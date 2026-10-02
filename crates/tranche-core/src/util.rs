//! Shared primitives used across the core.

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::io::{self, Write};
use std::path::Path;

/// SHA-256 over the canonical JSON projection of a value.
///
/// Compatibility is load-bearing: these digests are report bindings and cache
/// keys. `tranche.py:63` hashes `sort_keys=True, separators=(",", ":")`, and the
/// `out/` files on disk were written with it. A different encoding makes every
/// stored judgment unreusable and every existing report foreign.
pub fn digest(value: &serde_json::Value) -> String {
    sha256_hex(canonical_json(value).as_bytes())
}

/// As [`digest`], but ordering integer-like keys numerically.
///
/// Keys are compared as *objects*, and which order that
/// produces depends on the type the caller used. The report binding is keyed by
/// PR number as an **integer** (`dict[int, dict]`), so `3507` sorts before
/// `10002`; the capture generation's `revision` map is keyed by the **string**
/// `str(number)`, so it sorts lexicographically. JSON has only string keys, so
/// the two cases are indistinguishable from the value alone - the caller has to
/// say which one it is.
pub fn digest_numeric_keys(value: &serde_json::Value) -> String {
    sha256_hex(canonical_json_with(value, KeyOrder::Numeric).as_bytes())
}

/// The key ordering a canonical encoding uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyOrder {
    /// Compare keys as written. This is the ordering for string keys, and
    /// the default because most maps in the format are string-keyed.
    Lexicographic,
    /// Compare keys that are all integers numerically. Used where the map is keyed by
    /// map by an integer.
    Numeric,
}

/// The exact JSON encoding `tranche.digest()` hashes: sorted keys, no
/// whitespace, non-ASCII preserved.
pub fn canonical_json(value: &serde_json::Value) -> String {
    canonical_json_with(value, KeyOrder::Lexicographic)
}

/// The canonical encoding under a chosen key order.
pub fn canonical_json_with(value: &serde_json::Value, order: KeyOrder) -> String {
    let mut out = String::new();
    write_canonical(value, &mut out, order);
    out
}

/// The key comparison the report format produces for one map.
fn key_order(a: &str, b: &str, order: KeyOrder) -> std::cmp::Ordering {
    match order {
        KeyOrder::Lexicographic => a.cmp(b),
        KeyOrder::Numeric => match (a.parse::<i64>(), b.parse::<i64>()) {
            (Ok(left), Ok(right)) => left.cmp(&right),
            _ => a.cmp(b),
        },
    }
}

fn write_canonical(value: &serde_json::Value, out: &mut String, order: KeyOrder) {
    match value {
        serde_json::Value::Null => out.push_str("null"),
        serde_json::Value::Bool(flag) => out.push_str(if *flag { "true" } else { "false" }),
        serde_json::Value::Number(number) => out.push_str(&number.to_string()),
        serde_json::Value::String(text) => {
            out.push_str(&serde_json::to_string(text).expect("strings are serializable"))
        }
        serde_json::Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_canonical(item, out, order);
            }
            out.push(']');
        }
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_by(|a, b| key_order(a, b, order));
            out.push('{');
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(key).expect("keys are strings"));
                out.push(':');
                write_canonical(&map[key], out, order);
            }
            out.push('}');
        }
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// SHA-256 of a file's bytes, or `None` when it is absent.
pub fn file_digest(path: &Path, limit: u64) -> io::Result<Option<(String, u64)>> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut buffer = Vec::new();
    io::Read::read_to_end(&mut file, &mut buffer)?;
    if buffer.len() as u64 > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "input file exceeds the byte limit",
        ));
    }
    let size = buffer.len() as u64;
    Ok(Some((sha256_hex(&buffer), size)))
}

/// Write a file atomically: temporary sibling, fsync, rename.
///
/// A partial report must never be observable, so everything that publishes
/// state under `out/` goes through here.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}

/// Serialize a value and publish it atomically.
///
/// The report's writer puts a space
/// after each comma and colon. `serde_json::to_vec` writes compact JSON, so a
/// ported writer would silently rewrite every artifact in a different style —
/// the digests still match, because they sort and compact explicitly, but the
/// files a reader diffs would all churn.
pub fn atomic_json<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    publish_json(path, value, false)
}

/// Serialize with the report's one-space indent, for stdout.
pub fn indented_json<T: Serialize>(value: &T) -> Result<String, serde_json::Error> {
    let encoded = spaced_json(value, true)?;
    Ok(encoded)
}

fn publish_json<T: Serialize>(path: &Path, value: &T, indent: bool) -> io::Result<()> {
    let encoded = spaced_json(value, indent).map_err(io::Error::other)?;
    atomic_write(path, encoded.as_bytes())
}

/// JSON with the report's separators, optionally at one-space indent.
pub fn spaced_json<T: Serialize>(value: &T, indent: bool) -> Result<String, serde_json::Error> {
    let compact = serde_json::to_string(value)?;
    Ok(space_separators(&compact, indent))
}

/// Insert the report's separator spacing outside string literals.
///
/// A byte-level pass rather than a re-serialization: the compact form is already
/// the right value in the right order, and walking it keeps number formatting
/// exactly as `serde_json` wrote it.
fn space_separators(compact: &str, indent: bool) -> String {
    let mut out = String::with_capacity(compact.len() + compact.len() / 8);
    let mut in_string = false;
    let mut escaped = false;
    let mut depth = 0usize;
    for character in compact.chars() {
        if in_string {
            out.push(character);
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        match character {
            '"' => {
                in_string = true;
                out.push(character);
            }
            ',' => {
                out.push(',');
                out.push(' ');
                if indent {
                    out.push('\n');
                    out.extend(std::iter::repeat_n(' ', depth));
                }
            }
            ':' => {
                out.push(':');
                out.push(' ');
            }
            '{' | '[' => {
                depth += 1;
                out.push(character);
                if indent {
                    out.push('\n');
                    out.extend(std::iter::repeat_n(' ', depth));
                }
            }
            '}' | ']' => {
                depth = depth.saturating_sub(1);
                if indent {
                    out.push('\n');
                    out.extend(std::iter::repeat_n(' ', depth));
                }
                out.push(character);
            }
            character => out.push(character),
        }
    }
    out
}

/// Parse a JSON Lines file into records, refusing malformed lines.
pub fn parse_jsonl<T: for<'de> Deserialize<'de>>(text: &str) -> Result<Vec<T>, String> {
    let mut records = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        records.push(
            serde_json::from_str(line)
                .map_err(|error| format!("malformed record on line {}: {error}", index + 1))?,
        );
    }
    Ok(records)
}
