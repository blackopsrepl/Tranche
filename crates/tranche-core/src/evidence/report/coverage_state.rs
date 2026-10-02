//! Per-member and per-component coverage, from the recorded state alone.
//!
//! This is the view a reader checks a capture against: the counts and statuses that
//! say what was actually observed, before anything is interpreted.

use serde_json::{Value, json};

use super::super::coverage::is_complete;

/// The default window, in bytes.
pub const DEFAULT_WINDOW_BYTES: usize = 16 * 1024;

/// The per-member and per-component coverage, from the recorded state alone.
pub fn coverage(manifest: &Value) -> Value {
    let components = manifest["components"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let citations = manifest["citations"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let members: Vec<Value> = manifest["selection"]["members"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|member| {
            let number = member["number"].as_u64().unwrap_or(0);
            let entries: Vec<Value> = components
                .iter()
                .filter(|entry| entry["number"].as_u64() == Some(number))
                .map(|entry| {
                    json!({
                        "component": entry["component"],
                        "status": entry["status"],
                        "pages": entry["groups"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .map(|group| group["pages"].as_u64().unwrap_or(0))
                            .sum::<u64>(),
                        "reason": entry["reason"],
                        "groups": entry["groups"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .map(|group| json!({
                                "group": group["group"],
                                "status": group["status"],
                                "pages": group["pages"],
                                "reason": group["reason"],
                            }))
                            .collect::<Vec<_>>(),
                        "citations": citations
                            .iter()
                            .filter(|citation| {
                                citation["number"].as_u64() == Some(number)
                                    && citation["component"] == entry["component"]
                            })
                            .count(),
                    })
                })
                .collect();
            json!({
                "number": number,
                "base_sha": member["base_sha"],
                "head_sha": member["head_sha"],
                "components": entries,
            })
        })
        .collect();

    let capture = &manifest["capture"];
    json!({
        "capture_id": manifest["capture_id"],
        "generation": manifest["generation"],
        "batch": manifest["selection"]["batch"]["id"],
        "observed_at": capture["observed_at"],
        "stop_reason": capture["stop_reason"],
        "complete": is_complete(manifest),
        "requests": {
            "request_limit": capture["request_limit"],
            "requests_used": capture["requests_used"],
            "reserved": capture["reserved"],
            "failures": capture["failures"],
            "retries": capture["retries"],
            "identity_checks": capture["identity_checks"],
        },
        "bytes_stored": capture["bytes_stored"],
        "citations": citations.len(),
        "members": members,
    })
}
