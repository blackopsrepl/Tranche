//! Reading GitHub's open-PR list, with the credential staying inside `gh`.
//!
//! `gh api` is preferred because it is already authenticated: unauthenticated
//! GitHub allows 60 requests an hour and one capture of this backlog costs about
//! 29, so an unauthenticated refresh fails on its third pass. The token is never
//! read by this process and never placed in an argv, where it would be visible to
//! every user on the host.
//!
//! A failed or partial capture reports and leaves the previous snapshot alone.
//! Nothing here writes.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use serde_json::Value;

/// The GitHub REST base.
pub const API: &str = "https://api.github.com";

/// How one page is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Transport {
    /// The authenticated `gh` CLI.
    #[default]
    Gh,
    /// The `curl` binary.
    Curl,
    /// The standard library HTTP client.
    Urllib,
}

impl Transport {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Gh => "gh",
            Self::Curl => "curl",
            Self::Urllib => "urllib",
        }
    }
}

#[derive(Debug)]
pub struct GitHubError(pub String);

impl std::fmt::Display for GitHubError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for GitHubError {}

impl From<String> for GitHubError {
    fn from(message: String) -> Self {
        Self(message)
    }
}

/// One page of the open-PR list.
///
/// The body must be a JSON array. Anything else is a refusal rather than an empty
/// page, because an error document read as "no more PRs" would silently truncate
/// the corpus.
pub fn page(transport: Transport, url: &str) -> Result<Vec<Value>, GitHubError> {
    page_with(transport, url, None)
}

/// One page, optionally resolving the command through one directory.
///
/// The search path is a parameter rather than global state: a caller that wants
/// a specific `gh` says so, and a test cannot change what a test beside it sees.
pub fn page_with(
    transport: Transport,
    url: &str,
    command_dir: Option<&std::path::Path>,
) -> Result<Vec<Value>, GitHubError> {
    let timeout = match transport {
        Transport::Gh => Duration::from_secs(120),
        _ => Duration::from_secs(90),
    };
    page_with_timeout(transport, url, command_dir, timeout)
}

/// One page with an explicit subprocess deadline, including output draining.
/// The HTTP transport retains its own client timeout.
pub fn page_with_timeout(
    transport: Transport,
    url: &str,
    command_dir: Option<&std::path::Path>,
    timeout: Duration,
) -> Result<Vec<Value>, GitHubError> {
    let body = match transport {
        Transport::Gh => run(
            "gh",
            &["api", url],
            timeout,
            "gh api fetch failed",
            command_dir,
        )?,
        Transport::Curl => run(
            "curl",
            &[
                "--fail",
                "--silent",
                "--show-error",
                "--max-time",
                "60",
                url,
            ],
            timeout,
            "curl fetch failed",
            command_dir,
        )?,
        // The one transport with no external binary: useful for a caller that has
        // no `gh` and a token in the environment, and the only one that needs no
        // subprocess at all.
        Transport::Urllib => return via_http(url),
    };
    let parsed: Value = serde_json::from_str(&body)
        .map_err(|error| GitHubError(format!("GitHub returned unreadable JSON: {error}")))?;
    match parsed {
        Value::Array(items) => Ok(items),
        _ => Err(GitHubError(
            "GitHub did not return a PR list; the previous snapshot is retained".to_owned(),
        )),
    }
}

/// Run one command to completion and return its stdout.
///
/// The last line of stderr is what the operator needs; the whole thing is usually
/// a progress line plus a cause.
fn run(
    program: &str,
    arguments: &[&str],
    timeout: Duration,
    context: &str,
    command_dir: Option<&std::path::Path>,
) -> Result<String, GitHubError> {
    let program_path = match command_dir {
        Some(directory) => directory.join(program),
        None => PathBuf::from(program),
    };
    let mut command = Command::new(&program_path);
    // A descendant can inherit the pipes. Keep the command tree together so a
    // timeout also closes those writers before the reader threads are joined.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| GitHubError(format!("{context}: cannot run {program}: {error}")))?;

    let output = drain(&mut child, timeout).map_err(|error| {
        GitHubError(format!(
            "{context}: {error}; the previous snapshot is retained"
        ))
    })?;
    if !output.0.success() {
        let stderr = String::from_utf8_lossy(&output.2);
        let detail = stderr
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("no detail");
        return Err(GitHubError(format!(
            "{context} ({detail}); the previous snapshot is retained"
        )));
    }
    Ok(String::from_utf8_lossy(&output.1).into_owned())
}

/// Upper bounds are per command, not per chunk. Refuse rather than parsing a
/// truncated response as a complete snapshot.
const STDOUT_LIMIT: usize = 32 * 1024 * 1024;
const STDERR_LIMIT: usize = 1024 * 1024;

type CapturedOutput = (std::process::ExitStatus, Vec<u8>, Vec<u8>);

fn drain(child: &mut std::process::Child, timeout: Duration) -> Result<CapturedOutput, String> {
    use std::io::Read;
    use std::sync::mpsc::{RecvTimeoutError, sync_channel};

    // Backpressure bounds queued chunks as well as the retained output. Readers
    // run independently: stderr cannot block a command still producing stdout.
    let (sender, receiver) = sync_channel(8);
    let mut readers = Vec::new();
    for (stream, mut pipe) in [
        (
            0,
            Box::new(child.stdout.take().expect("piped stdout")) as Box<dyn Read + Send>,
        ),
        (
            1,
            Box::new(child.stderr.take().expect("piped stderr")) as Box<dyn Read + Send>,
        ),
    ] {
        let sender = sender.clone();
        readers.push(std::thread::spawn(move || {
            let mut buffer = [0; 8192];
            loop {
                match pipe.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => {
                        if sender.send((stream, Ok(buffer[..count].to_vec()))).is_err() {
                            break;
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(error) => {
                        let _ = sender.send((stream, Err(error)));
                        break;
                    }
                }
            }
        }));
    }
    drop(sender);
    let started = std::time::Instant::now();
    let mut output = [Vec::new(), Vec::new()];
    let limits = [STDOUT_LIMIT, STDERR_LIMIT];
    let mut closed = false;
    let result = loop {
        if started.elapsed() >= timeout {
            break Err(format!("timed out after {timeout:?}"));
        }
        let status = match child.try_wait() {
            Ok(status) => status,
            Err(error) => break Err(error.to_string()),
        };
        if closed {
            if let Some(status) = status {
                break Ok((
                    status,
                    std::mem::take(&mut output[0]),
                    std::mem::take(&mut output[1]),
                ));
            }
            std::thread::sleep(Duration::from_millis(5));
            continue;
        }
        match receiver.recv_timeout(Duration::from_millis(5)) {
            Ok((stream, Ok(chunk))) => {
                if chunk.len() > limits[stream] - output[stream].len() {
                    let name = ["stdout", "stderr"][stream];
                    break Err(format!(
                        "{name} exceeded output limit of {} bytes",
                        limits[stream]
                    ));
                }
                output[stream].extend(chunk);
            }
            Ok((_, Err(error))) => break Err(format!("cannot read output: {error}")),
            Err(RecvTimeoutError::Disconnected) => closed = true,
            Err(RecvTimeoutError::Timeout) => {}
        }
    };
    // Drop the receiver before joining: a reader blocked by backpressure must
    // wake even if we refused the output before consuming all queued chunks.
    drop(receiver);
    #[cfg(unix)]
    unsafe {
        // The child leads the private group created above; never signal our own.
        libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
    for reader in readers {
        let _ = reader.join();
    }
    result
}

/// Read a page over HTTP, for a caller with no `gh`.
///
/// A token is used only when the environment carries one; without it this is the
/// unauthenticated path the module documents as limited.
fn via_http(url: &str) -> Result<Vec<Value>, GitHubError> {
    let blocking = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60))
        .user_agent("tranche")
        .build()
        .map_err(|error| GitHubError(format!("cannot build the HTTP client: {error}")))?;
    let mut request = blocking
        .get(url)
        .header("Accept", "application/vnd.github+json");
    if let Ok(token) = std::env::var("GITHUB_TOKEN")
        && !token.trim().is_empty()
    {
        request = request.bearer_auth(token.trim());
    }
    let response = request
        .send()
        .map_err(|error| GitHubError(format!("network: {error}")))?;
    let status = response.status().as_u16();
    if status != 200 {
        return Err(GitHubError(format!("HTTP {status} from GitHub")));
    }
    response
        .json::<Value>()
        .map_err(|error| GitHubError(format!("GitHub returned unreadable JSON: {error}")))
        .and_then(|parsed| match parsed {
            Value::Array(items) => Ok(items),
            _ => Err(GitHubError(
                "GitHub did not return a PR list; the previous snapshot is retained".to_owned(),
            )),
        })
}
