//! The subprocess seam: one bounded `gh` invocation and its parsing.
//!
//! No shell is involved and no credential enters the argv — `gh` owns the token.
//! A redirect that `gh` follows itself appears as several header blocks in
//! `--include` output, and the last one is the response acted on.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::ReadError;
use super::transport::{GH_TIMEOUT, Raw};

/// Spawn `gh` with bounded output and return its final hop.
pub(super) fn invoke(
    arguments: &[String],
    stdin: Option<&[u8]>,
    max_bytes: usize,
) -> Result<Raw, ReadError> {
    let mut child = Command::new("gh")
        .args(arguments)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| ReadError::new(format!("cannot run gh: {error}")))?;
    if let Some(data) = stdin
        && let Some(mut handle) = child.stdin.take()
    {
        use std::io::Write;
        let _ = handle.write_all(data);
    }

    // Both pipes are drained on their own thread: a child that fills one while we
    // wait on the other would deadlock, and the read is capped so an unbounded
    // response cannot be buffered whole.
    let out = child.stdout.take().expect("stdout is piped");
    let err = child.stderr.take().expect("stderr is piped");
    let out_reader = std::thread::spawn(move || read_capped(out, max_bytes + 1));
    let err_reader =
        std::thread::spawn(move || read_capped(err, super::transport::MAX_STDERR_BYTES));
    let status = wait(&mut child)?;
    let (stdout, out_truncated) = out_reader
        .join()
        .map_err(|_| ReadError::new("the gh stdout reader panicked"))?
        .map_err(|error| ReadError::new(format!("gh output: {error}")))?;
    let (stderr, _) = err_reader
        .join()
        .map_err(|_| ReadError::new("the gh stderr reader panicked"))?
        .map_err(|error| ReadError::new(format!("gh error output: {error}")))?;

    if !status.success() && stdout.is_empty() {
        return Err(ReadError::new(format!(
            "gh failed ({})",
            diagnostic(&stderr)
        )));
    }
    let (blocks, body) = split_response(&stdout)?;
    let last = blocks.last().copied().unwrap_or(b"");
    let headers = header_block(last);
    if let Some(declared) = header_of(&headers, "content-length")
        && let Ok(declared) = declared.parse::<usize>()
        && declared > max_bytes
        && !out_truncated
    {
        return Err(ReadError::too_large(format!(
            "response declares {declared} bytes, over the {max_bytes} byte bound"
        )));
    }
    let code = status_of(last)?;
    if !status.success() {
        return Err(ReadError::new(format!(
            "gh failed with HTTP {code} ({})",
            diagnostic(&stderr)
        )));
    }
    if out_truncated {
        return Err(ReadError::too_large(format!(
            "response exceeded the {max_bytes} byte bound"
        )));
    }
    Ok(Raw {
        status: code,
        headers,
        body: body.to_vec(),
        truncated: out_truncated,
        hops: blocks.len(),
    })
}

/// Read a pipe to the end, keeping at most `limit` bytes.
fn read_capped(mut pipe: impl std::io::Read, limit: usize) -> std::io::Result<(Vec<u8>, bool)> {
    let mut kept = Vec::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut truncated = false;
    loop {
        let read = pipe.read(&mut buffer)?;
        if read == 0 {
            return Ok((kept, truncated));
        }
        let room = limit.saturating_sub(kept.len());
        kept.extend_from_slice(&buffer[..read.min(room)]);
        if read > room {
            truncated = true;
        }
    }
}

/// Wait for `gh`, killing it if it outlasts the timeout.
fn wait(child: &mut std::process::Child) -> Result<std::process::ExitStatus, ReadError> {
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) if started.elapsed() < GH_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ReadError::new(format!(
                    "gh timed out after {}s",
                    GH_TIMEOUT.as_secs()
                )));
            }
            Err(error) => return Err(ReadError::new(format!("gh wait: {error}"))),
        }
    }
}

/// Split `gh api --include` output into its header blocks and the body.
fn split_response(raw: &[u8]) -> Result<(Vec<&[u8]>, &[u8]), ReadError> {
    let mut blocks = Vec::new();
    let mut index = 0usize;
    while let Some(offset) = find_blank_line(&raw[index..]) {
        let end = index + offset;
        let block = &raw[index..end];
        let starts_with_status = block
            .iter()
            .position(|byte| !byte.is_ascii_whitespace())
            .is_some_and(|start| raw[index + start..].starts_with(b"HTTP/"));
        if !starts_with_status {
            break;
        }
        blocks.push(block);
        index = index + offset + blank_line_len(&raw[end..]);
    }
    if blocks.is_empty() {
        return Err(ReadError::new(
            "gh api --include produced no HTTP status line",
        ));
    }
    Ok((blocks, &raw[index..]))
}

/// The offset where a header block ends, at the first blank line.
fn find_blank_line(raw: &[u8]) -> Option<usize> {
    (0..raw.len())
        .find(|index| raw[*index..].starts_with(b"\r\n\r\n") || raw[*index..].starts_with(b"\n\n"))
}

/// How long the blank line at the start of `raw` is.
fn blank_line_len(raw: &[u8]) -> usize {
    if raw.starts_with(b"\r\n\r\n") {
        4
    } else if raw.starts_with(b"\n\n") {
        2
    } else {
        0
    }
}

/// Parse a header block's lowercased key/value pairs, skipping the status line.
fn header_block(block: &[u8]) -> Vec<(String, String)> {
    String::from_utf8_lossy(block)
        .lines()
        .skip(1)
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.trim().to_ascii_lowercase(), value.trim().to_owned()))
        .collect()
}

pub(super) fn header_of<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

/// Parse a status line's code.
fn status_of(block: &[u8]) -> Result<u16, ReadError> {
    String::from_utf8_lossy(block)
        .lines()
        .next()
        .unwrap_or("")
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| ReadError::new("unreadable HTTP status line from gh"))
}

/// A short, credential-free diagnostic line for the operator.
fn diagnostic(stderr: &[u8]) -> String {
    let detail = String::from_utf8_lossy(stderr)
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .unwrap_or("no detail")
        .to_owned();
    // A failure message is the likeliest place a token gets echoed back.
    let redacted = detail
        .split_whitespace()
        .map(|word| {
            let lower = word.to_ascii_lowercase();
            if ["token", "authorization", "password", "bearer"]
                .iter()
                .any(|key| lower.contains(key))
            {
                "REDACTED"
            } else {
                word
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    redacted.chars().take(200).collect()
}
