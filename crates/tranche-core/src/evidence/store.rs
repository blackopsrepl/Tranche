//! Content-addressed storage for captured response bytes.
//!
//! Bytes are written and fsynced before any manifest may cite them, so a crash
//! between the two leaves an unreferenced file — swept on the next completed run
//! — and never a checkpoint pointing at bytes that are not there.

use super::{EvidenceError, refuse};
use crate::evidence::paths::{atomic_bytes, body_path, confined_file};
use crate::report::Root;
use crate::util::sha256_hex;

/// Persist immutable source bytes and return their digest.
///
/// An existing file with the right name is not proof: it is read back and
/// verified before it is trusted, because the name is derived from the digest
/// and nothing enforces that the bytes still match.
pub fn store_body(root: &Root, data: &[u8]) -> Result<String, EvidenceError> {
    let digest = sha256_hex(data);
    let path = body_path(root, &digest)?;
    if path.exists() {
        read_body(root, &digest)?;
        return Ok(digest);
    }
    atomic_bytes(&path, data)?;
    Ok(digest)
}

/// Read stored bytes, verifying them against their digest before returning.
pub fn read_body(root: &Root, digest: &str) -> Result<Vec<u8>, EvidenceError> {
    let path = body_path(root, digest)?;
    let confined = confined_file(&super::evidence_root(root), &format!("bodies/{digest}.bin"))?;
    if !confined.exists() {
        return Err(refuse(format!(
            "stored source {}… is missing",
            &digest[..12.min(digest.len())]
        )));
    }
    let data = std::fs::read(&confined)
        .map_err(|error| refuse(format!("cannot read stored source: {error}")))?;
    let actual = sha256_hex(&data);
    if actual != digest {
        return Err(refuse(format!(
            "stored source {}… is corrupted (computed {}…)",
            &digest[..12.min(digest.len())],
            &actual[..12]
        )));
    }
    let _ = path;
    Ok(data)
}

/// Load JSON refusing the shapes that make a record ambiguous.
///
/// Duplicate keys and non-finite numbers are refusal cases: a checkpoint that
/// can be read two ways cannot be trusted to resume from.
pub fn strict_json_loads(data: &[u8]) -> Result<serde_json::Value, EvidenceError> {
    let text = std::str::from_utf8(data)
        .map_err(|error| refuse(format!("evidence record is not valid UTF-8 ({error})")))?;
    // `serde_json` keeps the last of a duplicated key rather than refusing it, so
    // the ambiguity is detected before parsing.
    reject_duplicate_keys(text)?;
    let value: serde_json::Value = serde_json::from_str(text)
        .map_err(|error| refuse(format!("evidence record is not valid JSON ({error})")))?;
    reject_non_finite(&value)?;
    Ok(value)
}

/// Refuse a duplicated object key anywhere in the document.
///
/// A checkpoint that can be read two ways is not a checkpoint. This walks the
/// text structurally — tracking object and array nesting, and which position is
/// a key — because a value-level parser has already resolved the duplicate by
/// the time it returns a map.
fn reject_duplicate_keys(text: &str) -> Result<(), EvidenceError> {
    struct Frame {
        object: bool,
        seen: std::collections::HashSet<String>,
        expect_key: bool,
    }
    let bytes = text.as_bytes();
    let mut stack: Vec<Frame> = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'{' => stack.push(Frame {
                object: true,
                seen: std::collections::HashSet::new(),
                expect_key: true,
            }),
            b'[' => stack.push(Frame {
                object: false,
                seen: std::collections::HashSet::new(),
                expect_key: false,
            }),
            b'}' | b']' => {
                stack.pop();
                // The closed collection is a completed value for its parent.
                if let Some(parent) = stack.last_mut() {
                    parent.expect_key = false;
                }
            }
            b',' => {
                if let Some(frame) = stack.last_mut()
                    && frame.object
                {
                    frame.expect_key = true;
                }
            }
            b'"' => {
                let start = index;
                index += 1;
                while index < bytes.len() {
                    match bytes[index] {
                        b'\\' => index += 1,
                        b'"' => break,
                        _ => {}
                    }
                    index += 1;
                }
                let literal = &text[start..=index.min(bytes.len() - 1)];
                let is_key = stack
                    .last()
                    .is_some_and(|frame| frame.object && frame.expect_key);
                if is_key {
                    let key: String = serde_json::from_str(literal).map_err(|error| {
                        refuse(format!("evidence record has an unreadable key ({error})"))
                    })?;
                    let frame = stack.last_mut().expect("checked above");
                    if !frame.seen.insert(key.clone()) {
                        return Err(refuse(format!("duplicate JSON key {key:?}")));
                    }
                    frame.expect_key = false;
                } else if let Some(frame) = stack.last_mut() {
                    frame.expect_key = false;
                }
            }
            byte if byte.is_ascii_whitespace() || byte == b':' => {}
            _ => {
                // A scalar value: skip to the end of the token.
                while index < bytes.len()
                    && !matches!(
                        bytes[index],
                        b',' | b'}' | b']' | b' ' | b'\n' | b'\t' | b'\r'
                    )
                {
                    index += 1;
                }
                continue;
            }
        }
        index += 1;
    }
    Ok(())
}

/// Refuse NaN and Infinity, which JSON cannot express and a digest cannot cover.
fn reject_non_finite(value: &serde_json::Value) -> Result<(), EvidenceError> {
    match value {
        serde_json::Value::Number(number) => {
            if number.as_f64().is_some_and(|value| !value.is_finite()) {
                return Err(refuse(format!("non-finite JSON number {number}")));
            }
            Ok(())
        }
        serde_json::Value::Array(items) => items.iter().try_for_each(reject_non_finite),
        serde_json::Value::Object(map) => map.values().try_for_each(reject_non_finite),
        _ => Ok(()),
    }
}

/// Every body digest any stored capture cites, for the orphan sweep.
pub fn referenced_bodies(root: &Root) -> Result<std::collections::HashSet<String>, EvidenceError> {
    let mut referenced = std::collections::HashSet::new();
    let directory = super::evidence_root(root);
    let entries = match std::fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(referenced),
        Err(error) => return Err(refuse(format!("cannot read the evidence root: {error}"))),
    };
    for entry in entries.flatten() {
        let manifest_path = entry.path().join("manifest.json");
        if !manifest_path.is_file() {
            continue;
        }
        // A manifest that cannot be read is skipped rather than fatal: the sweep
        // must not delete bytes because one record is damaged.
        let Ok(text) = std::fs::read(&manifest_path) else {
            continue;
        };
        let Ok(manifest) = strict_json_loads(&text) else {
            continue;
        };
        let Some(sources) = manifest
            .get("sources")
            .and_then(serde_json::Value::as_array)
        else {
            continue;
        };
        for source in sources {
            if let Some(digest) = source
                .get("body_sha256")
                .and_then(serde_json::Value::as_str)
            {
                referenced.insert(digest.to_owned());
            }
        }
    }
    Ok(referenced)
}

/// Remove stored bodies no capture cites.
///
/// A crash can leave bytes written but not yet cited. They are harmless, but
/// they are also unreferenced state, so a completed run tidies them.
pub fn sweep_orphans(root: &Root) -> Result<usize, EvidenceError> {
    let directory = super::evidence_root(root).join("bodies");
    let entries = match std::fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(refuse(format!("cannot read the body store: {error}"))),
    };
    let keep = referenced_bodies(root)?;
    let mut removed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("bin") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
            continue;
        };
        if !keep.contains(stem) && std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}
