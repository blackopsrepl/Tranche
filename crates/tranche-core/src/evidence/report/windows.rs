//! Bounded windows and citation resolution, answered from the stored bytes.
//!
//! Both are the operations a reader must be able to perform without the state store,
//! the network or a second application.

use serde_json::{Value, json};

use super::super::store::read_body;
use super::coverage_state::DEFAULT_WINDOW_BYTES;
use super::printing::printable;
use crate::report::Root;

pub fn window(
    root: &Root,
    manifest: &Value,
    source_id: &str,
    start: usize,
    length: usize,
) -> Result<Value, String> {
    if length == 0 || length > DEFAULT_WINDOW_BYTES {
        return Err(format!("length must be 1..{DEFAULT_WINDOW_BYTES} bytes"));
    }
    let source = source_for(manifest, source_id)?;
    let sha = source["body_sha256"].as_str().unwrap_or("");
    let body = read_body(root, sha).map_err(|error| error.0)?;
    if start > body.len() {
        return Err(format!(
            "start byte {start} is past the end of a {} byte source",
            body.len()
        ));
    }
    let end = body.len().min(start + length);
    let chunk = &body[start..end];
    Ok(json!({
        "capture_id": manifest["capture_id"],
        "source_id": source_id,
        "number": source["number"],
        "component": source["component"],
        "group": source["group"],
        "page": source["page"],
        "url": source["url"],
        "media_type": source["media_type"],
        "body_sha256": sha,
        "body_bytes": body.len(),
        "start_byte": start,
        "end_byte": end,
        "next_start": if end < body.len() { json!(end) } else { Value::Null },
        "window_sha256": crate::util::sha256_hex(chunk),
        "content": printable(chunk),
    }))
}

/// Resolve one citation against the stored bytes it names, offline.
///
/// This is the operation a reader must be able to perform without the state store,
/// the network or another application: look the source up, verify its bytes, and
/// re-derive the excerpt digest that was recorded. A mismatch is reported rather
/// than shown, because a citation that no longer matches is worse than none.
pub fn resolve_citation(root: &Root, manifest: &Value, citation_id: &str) -> Result<Value, String> {
    let citations = manifest["citations"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let citation = citations
        .iter()
        .find(|citation| citation["id"].as_str() == Some(citation_id))
        .ok_or_else(|| {
            format!(
                "no citation {citation_id:?} in capture {}",
                manifest["capture_id"].as_str().unwrap_or("")
            )
        })?;
    let source_sha = citation["source_sha256"].as_str().unwrap_or("");
    let body = read_body(root, source_sha).map_err(|error| error.0)?;
    let start = citation["start_byte"].as_u64().unwrap_or(0) as usize;
    let end = citation["end_byte"].as_u64().unwrap_or(0) as usize;
    if start > end || end > body.len() {
        return Err(format!(
            "citation {citation_id:?} names a range outside its stored bytes"
        ));
    }
    let excerpt = &body[start..end];
    let digest = crate::util::sha256_hex(excerpt);
    if digest != citation["excerpt_sha256"].as_str().unwrap_or("") {
        return Err(format!(
            "citation {} no longer matches its stored bytes",
            &citation_id[..citation_id.len().min(12)]
        ));
    }
    let source = source_for(manifest, citation["source_id"].as_str().unwrap_or(""))?;
    Ok(json!({
        "citation": citation,
        "source": source,
        "excerpt_sha256": digest,
        "excerpt": printable(excerpt),
    }))
}

/// One source by its id.
fn source_for<'a>(manifest: &'a Value, source_id: &str) -> Result<&'a Value, String> {
    manifest["sources"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|source| source["id"].as_str() == Some(source_id))
        .ok_or_else(|| {
            format!(
                "no source {source_id:?} in capture {}",
                manifest["capture_id"].as_str().unwrap_or("")
            )
        })
}
