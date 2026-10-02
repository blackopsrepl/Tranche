//! Deriving citations from what a capture recorded.
//!
//! Citations are a projection of the capture, computed once and stored, so every
//! reader — export, a windowed source read, a future adapter — resolves the same
//! offsets without reparsing anything or inventing its own eligibility rules.
//! Discussion and review bodies are JSON, so their text is quoted exactly and the
//! offsets are computed against the stored bytes of the response that carried them.

use serde_json::Value;

use super::store::read_body;
use crate::report::Root;

/// The citation ceiling a capture will record.
pub const MAX_CITATIONS: usize = 5_000;
/// The most file entries one page may cite.
pub const MAX_CITATIONS_PER_PAGE: usize = 32;

/// The components whose raw bytes are a document a person reads.
const CONTENT_COMPONENTS: [&str; 5] = ["files", "diff", "discussion", "review_comments", "reviews"];

/// Derive the citations for one capture's sources.
///
/// Deliberately absent: the pull request's own description, which is the report's
/// evidence and not source evidence, and any citation into a `patch` field GitHub
/// omitted for a large or unsupported file, which the file list reports separately
/// as a gap.
pub fn extract_citations(root: &Root, manifest: &Value) -> Result<Vec<Value>, String> {
    let mut citations: Vec<Value> = Vec::new();
    for source in manifest["sources"].as_array().into_iter().flatten() {
        let component = source["component"].as_str().unwrap_or("");
        if !CONTENT_COMPONENTS.contains(&component) || citations.len() >= MAX_CITATIONS {
            continue;
        }
        let Some(sha) = source["body_sha256"].as_str() else {
            continue;
        };
        let body = read_body(root, sha).map_err(|error| error.0)?;
        let payload = parse(&body);
        let Some(payload) = payload else { continue };
        let number = source["number"].as_u64().unwrap_or(0);
        let Some(source_id) = source["id"].as_str() else {
            continue;
        };
        match component {
            "files" => {
                for entry in payload
                    .as_array()
                    .into_iter()
                    .flatten()
                    .take(MAX_CITATIONS_PER_PAGE)
                {
                    for key in ["patch", "filename"] {
                        if let Some(text) = entry.get(key).and_then(Value::as_str)
                            && !text.is_empty()
                        {
                            add(&mut citations, number, "files", source_id, &body, text);
                        }
                    }
                }
            }
            component @ ("discussion" | "review_comments" | "reviews") => {
                for entry in payload.as_array().into_iter().flatten() {
                    if let Some(text) = entry.get("body").and_then(Value::as_str) {
                        add(&mut citations, number, component, source_id, &body, text);
                    }
                }
            }
            _ => {}
        }
    }
    // The identity covers everything except itself, like every other digest here.
    for citation in &mut citations {
        let computed = super::super::util::digest(citation);
        citation["id"] = serde_json::json!(computed);
    }
    Ok(citations)
}

/// One citation, appended only when the text is found byte-exactly.
fn add(
    citations: &mut Vec<Value>,
    number: u64,
    component: &str,
    source_id: &str,
    body: &[u8],
    text: &str,
) {
    if text.is_empty() || citations.len() >= MAX_CITATIONS {
        return;
    }
    let encoded = json_string_encoded(text);
    let Some(start) = find(body, &encoded) else {
        return;
    };
    citations.push(serde_json::json!({
        "id": "",
        "number": number,
        "component": component,
        "source_id": source_id,
        "source_sha256": crate::util::sha256_hex(body),
        "start_byte": start,
        "end_byte": start + encoded.len(),
        "excerpt_sha256": crate::util::sha256_hex(&encoded),
        "kind": "body",
    }));
}

/// Parse the stored body as JSON, or none when it is not JSON.
fn parse(body: &[u8]) -> Option<Value> {
    serde_json::from_slice(body).ok()
}

/// The text encoded the way a JSON string would carry it, without the quotes.
fn json_string_encoded(text: &str) -> Vec<u8> {
    let quoted = serde_json::to_string(text).unwrap_or_default();
    quoted.as_bytes()[1..quoted.len() - 1].to_vec()
}

/// Find the first offset of `needle` in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    (0..=haystack.len() - needle.len())
        .find(|offset| &haystack[*offset..offset + needle.len()] == needle)
}
