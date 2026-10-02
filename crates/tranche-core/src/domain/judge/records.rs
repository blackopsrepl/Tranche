//! Normalizing a stored judgment to the shape the report reads, and the
//! per-request token usage the log carries.

use serde_json::{Map, Value, json};
use std::path::Path;

use super::super::questions::{answer_field, metric_ceiling};
use crate::util::parse_jsonl;

pub fn finite_json(value: &Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, value)| (key.clone(), finite_json(value)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(finite_json).collect()),
        Value::Number(number) => match number.as_f64() {
            Some(value) if !value.is_finite() => Value::Null,
            _ => value.clone(),
        },
        other => other.clone(),
    }
}

/// Normalize a stored judgment to the shape the rest of the pipeline reads.
///
/// A field the model left unusable becomes `null` and is listed in
/// `normalization_errors`; it is never coerced to a value, because an unknown
/// category and an invented one are different facts. `questions` is the
/// deployment's judge-question policy, which decides what a usable answer is.
pub fn normalize_judgment(record: &Value, questions: &Value) -> Value {
    let mut object = normalized_record(record);
    let mut errors: Vec<String> = Vec::new();
    let mut answers = object
        .get("answers")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    if let Some(policy) = questions.as_object() {
        for (name, question) in policy {
            let Some(field) = question
                .get("type")
                .and_then(Value::as_str)
                .and_then(answer_field)
            else {
                continue;
            };
            let mut answer = answers
                .get(name)
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            let value = if field == "choice" {
                let chosen = answer.get(field).and_then(Value::as_str);
                let valid = chosen.is_some_and(|value| {
                    question
                        .get("criteria")
                        .and_then(|criteria| criteria.get(value))
                        .is_some()
                });
                match valid {
                    true => answer.get(field).cloned().unwrap_or(Value::Null),
                    false => {
                        errors.push(format!("answers.{name}.{field}: invalid or missing"));
                        Value::Null
                    }
                }
            } else {
                let ceiling = metric_ceiling(name, field).unwrap_or(0.0);
                let usable = answer
                    .get(field)
                    .and_then(Value::as_f64)
                    .filter(|value| value.is_finite() && (0.0..=ceiling).contains(value));
                match usable.and_then(serde_json::Number::from_f64) {
                    Some(number) => Value::Number(number),
                    None => {
                        errors.push(format!("answers.{name}.{field}: invalid or missing"));
                        Value::Null
                    }
                }
            };
            answer.insert(field.to_owned(), value);
            answers.insert(name.clone(), Value::Object(answer));
        }
    }

    object.insert("answers".to_owned(), Value::Object(answers));
    object.insert("normalization_errors".to_owned(), json!(errors));
    Value::Object(object)
}

/// Token accounting as clustering reads it: the nested `usage` block only.
///
/// This deliberately ignores the flat fields. `normalize_pair` moves a verdict's
/// tokens to the flat fields and drops `usage`, so clustering counts pair
/// verdicts as zero — and reproducing that is required, because the total is
/// published in `summary.json`.
pub fn usage(record: &Value) -> (i64, i64) {
    let nested = record.get("usage").and_then(Value::as_object);
    let read = |field: &str| -> i64 {
        nested
            .and_then(|usage| usage.get(field))
            .and_then(Value::as_i64)
            .filter(|value| *value >= 0)
            .unwrap_or(0)
    };
    (read("input_tokens"), read("output_tokens"))
}

fn normalized_record(record: &Value) -> Map<String, Value> {
    let mut object: Map<String, Value> = record
        .as_object()
        .map(|object| {
            object
                .iter()
                .map(|(key, value)| (key.clone(), finite_json(value)))
                .collect()
        })
        .unwrap_or_default();
    let usage = object.get("usage").and_then(Value::as_object);
    let counts = ["input_tokens", "output_tokens"]
        .iter()
        .map(|field| {
            let value = usage
                .and_then(|usage| usage.get(*field))
                .and_then(Value::as_i64);
            (
                (*field).to_owned(),
                json!(value.filter(|value| *value >= 0).unwrap_or(0)),
            )
        })
        .collect();
    object.insert("usage".to_owned(), Value::Object(counts));
    object
}

/// Read a JSON Lines file into normalized records, or report why it cannot be used.
pub fn read_lines<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Vec<T>, String> {
    let text = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    parse_jsonl(&text)
}
