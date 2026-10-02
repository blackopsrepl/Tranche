use serde_json::{Value, json};

use super::verdict::p_same;
use crate::domain::judge::finite_json;

/// Normalize a stored verdict to the shape the report reads.
pub fn normalize_pair(record: &Value) -> Value {
    let mut record = finite_json(record);
    let Some(object) = record.as_object_mut() else {
        return record;
    };
    let criteria = crate::domain::questions::sameness_criteria();
    let verdict = object.get("verdict").and_then(Value::as_str);
    if !verdict.is_some_and(|value| criteria.contains(&value)) {
        object.insert("verdict".to_owned(), Value::Null);
    }
    let mut probabilities = object
        .get("probabilities")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for (_, value) in probabilities.iter_mut() {
        let usable = value
            .as_f64()
            .is_some_and(|number| number.is_finite() && (0.0..=1.0).contains(&number));
        if !usable {
            *value = Value::Null;
        }
    }
    let normalized = Value::Object(probabilities.clone());
    let same = p_same(&Value::Object({
        let mut map = serde_json::Map::new();
        map.insert("probabilities".to_owned(), normalized);
        map
    }));
    probabilities.insert(
        "same_change".to_owned(),
        same.and_then(serde_json::Number::from_f64)
            .map(Value::Number)
            .unwrap_or(Value::Null),
    );
    let mut errors = Vec::new();
    let record_value = Value::Object(object.clone());
    if record_value.get("verdict").is_some_and(Value::is_null) {
        errors.push("verdict: invalid or missing".to_owned());
    }
    if same.is_none() {
        errors.push("probabilities.same_change: invalid or missing".to_owned());
    }
    object.insert("probabilities".to_owned(), Value::Object(probabilities));
    object.insert("normalization_errors".to_owned(), json!(errors));
    // The judgment block is what the report re-reads; per-request token usage is
    // not, and keeping it out lets cluster read the multi-megabyte log without
    // holding every usage block in memory.
    let has_counts = ["input_tokens", "output_tokens"]
        .iter()
        .all(|field| record_value.get(*field).and_then(Value::as_i64).is_some());
    if !has_counts {
        let nested = record_value.get("usage").and_then(Value::as_object);
        for field in ["input_tokens", "output_tokens"] {
            let value = nested
                .and_then(|usage| usage.get(field))
                .and_then(Value::as_i64)
                .filter(|value| *value >= 0)
                .unwrap_or(0);
            object.insert(field.to_owned(), json!(value));
        }
    }
    object.remove("usage");
    record
}
