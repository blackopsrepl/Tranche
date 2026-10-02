//! The append-only judgment log, read newest-per-PR.
//!
//! A malformed tail — a truncated write — never costs the earlier records.

use serde_json::{Value, json};
use std::collections::HashMap;

use super::super::judge::{Judgment, judgment_binding, judgment_is_current, normalize_judgment};
use super::super::pr::Prs;
use crate::report::Root;

pub fn load_done(root: &Root, questions: &Value) -> Result<HashMap<u64, Value>, String> {
    let path = root.judgments_path();
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let text = std::fs::read_to_string(&path).map_err(|error| error.to_string())?;
    let mut latest: HashMap<u64, Value> = HashMap::new();
    for line in text.lines() {
        if !line.contains("\"number\"") {
            continue;
        }
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(object) = record.as_object() else {
            continue;
        };
        if object.contains_key("error") {
            continue;
        }
        let number = match object.get("number").and_then(Value::as_u64) {
            Some(number) if number > 0 => number,
            _ => continue,
        };
        latest.insert(number, record);
    }
    Ok(latest
        .into_iter()
        .map(|(number, record)| (number, normalize_judgment(&record, questions)))
        .collect())
}

/// The judgments a report may publish, newest per PR, keyed by PR number.
///
/// `questions` is the deployment's judge-question policy; the binding it feeds
/// is what decides whether a stored record still answers today's question.
pub fn current_judgments(
    root: &Root,
    prs: &Prs,
    repository: &str,
    model: &str,
    questions: &Value,
    allow_unbound: bool,
) -> Result<HashMap<u64, Judgment>, String> {
    let mut current = HashMap::new();
    for (number, record) in load_done(root, questions)? {
        let Some(pr) = prs.get(number) else {
            continue;
        };
        let binding = judgment_binding(pr, repository, model, questions);
        let matches = judgment_is_current(&record, &binding);
        let legacy = record.get("binding").is_none() && allow_unbound;
        if matches || legacy {
            let mut stored = record.clone();
            if let Some(object) = stored.as_object_mut() {
                object.insert(
                    "freshness".to_owned(),
                    json!(if matches { "current" } else { "unbound" }),
                );
            }
            current.insert(
                number,
                Judgment {
                    number,
                    record: stored,
                },
            );
        }
    }
    Ok(current)
}
