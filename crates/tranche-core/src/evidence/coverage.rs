//! Coverage accounting for a capture.
//!
//! A terminal cursor is not completeness. A group only counts as complete when
//! it is recorded complete and carries no blocking reason, and a component rolls
//! up to its weakest group — so an unknown can never be published as an empty
//! answer.

use serde_json::{Value, json};

use super::COMPONENTS;
use super::components::groups_for;
use super::selection::Selection;

/// A fresh component list: every slot present, nothing claimed as acquired.
pub fn fresh_components(selection: &Selection) -> Vec<Value> {
    let mut entries = Vec::new();
    for member in &selection.members {
        for component in COMPONENTS {
            let groups: Vec<Value> = groups_for(component)
                .into_iter()
                .map(|group| {
                    json!({
                        "group": group,
                        "status": "missing",
                        "pages": 0,
                        "next_url": Value::Null,
                        "source_ids": [],
                        "reason": "not acquired",
                    })
                })
                .collect();
            entries.push(json!({
                "number": member.number,
                "component": component,
                "status": "missing",
                "reason": "not acquired",
                "groups": groups,
            }));
        }
    }
    entries
}

/// Aggregate a component's groups into one honest status.
///
/// The precedence is deliberate: a blocked group is the most specific failure
/// and outranks a partial one, and a group that was never acquired outranks a
/// partial one because nothing about it is known.
pub fn roll_up(entry: &mut Value) {
    let groups = entry
        .get("groups")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let statuses: Vec<&str> = groups
        .iter()
        .filter_map(|group| group.get("status").and_then(Value::as_str))
        .collect();
    let status = if statuses.contains(&"blocked") {
        "blocked"
    } else if statuses.contains(&"missing") {
        "missing"
    } else if !statuses.is_empty() && statuses.iter().all(|status| *status == "complete") {
        "complete"
    } else {
        "partial"
    };
    let problems: Vec<String> = groups
        .iter()
        .filter(|group| group.get("status").and_then(Value::as_str) != Some("complete"))
        .filter_map(|group| {
            let name = group.get("group").and_then(Value::as_str).unwrap_or("?");
            let reason = group.get("reason").and_then(Value::as_str);
            reason
                .filter(|reason| !reason.is_empty())
                .map(|reason| format!("{name}: {reason}"))
        })
        .collect();
    let reason = if status == "complete" {
        Value::Null
    } else if problems.is_empty() {
        json!("incomplete")
    } else {
        json!(problems.join("; "))
    };
    if let Some(object) = entry.as_object_mut() {
        object.insert("status".to_owned(), json!(status));
        object.insert("reason".to_owned(), reason);
    }
}

/// Whether every group reached a terminal page with nothing blocking.
pub fn is_complete(manifest: &Value) -> bool {
    let stopped = manifest
        .get("capture")
        .and_then(|capture| capture.get("stop_reason"))
        .is_some_and(|reason| !reason.is_null());
    if stopped {
        return false;
    }
    manifest
        .get("components")
        .and_then(Value::as_array)
        .is_some_and(|components| {
            !components.is_empty()
                && components.iter().all(|component| {
                    component.get("status").and_then(Value::as_str) == Some("complete")
                })
        })
}
