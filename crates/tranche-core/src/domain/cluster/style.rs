//! Markdown formatting the published report depends on.
//!
//! These are not stylistic choices — the published `tranches.md` is compared
//! against the committed file, so a space in the wrong place is a failed test.

use serde_json::Value;

/// Escape a cell so a pipe or newline cannot break the table row.
pub fn cell(value: &str) -> String {
    value.replace('|', "\\|").replace('\n', " ")
}

/// A cell read from a JSON string, absent meaning empty.
pub fn text(value: &Value) -> String {
    cell(value.as_str().unwrap_or_default())
}

/// One decimal place, or an em dash when the score is absent.
pub fn one_decimal(value: Option<f64>) -> String {
    match value {
        Some(number) => format!("{number:.1}"),
        None => "—".to_owned(),
    }
}

/// Two decimal places, or an em dash when the score is absent.
pub fn two_decimals(value: Option<f64>) -> String {
    match value {
        Some(number) => format!("{number:.2}"),
        None => "—".to_owned(),
    }
}

/// A float as the report writes it: the shortest form that round-trips.
///
/// Both languages print the shortest decimal that round-trips, so `{}` agrees
/// with `repr()` on every value the pipeline stores.
pub fn number(value: &Value) -> String {
    match value.as_f64() {
        Some(number) => format!("{number}"),
        None => "None".to_owned(),
    }
}

/// JSON with the report's separators: a space after each comma and colon.
///
/// `serde_json::to_string` writes compact JSON (`{"a":1}`); the report writes
/// `{"a": 1}`, with a space after each comma and colon. The relationship
/// diagnostics are printed through this, and they are compared against the
/// committed report, so the spacing is part of the output.
pub fn spaced_json(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(true) => "true".to_owned(),
        Value::Bool(false) => "false".to_owned(),
        Value::Number(number) => number.to_string(),
        Value::String(text) => json_string(text),
        Value::Array(items) => {
            let rendered: Vec<String> = items.iter().map(spaced_json).collect();
            format!("[{}]", rendered.join(", "))
        }
        Value::Object(map) => {
            let rendered: Vec<String> = map
                .iter()
                .map(|(key, value)| format!("{}: {}", json_string(key), spaced_json(value)))
                .collect();
            format!("{{{}}}", rendered.join(", "))
        }
    }
}

/// A JSON string literal, escaped as `json.dumps` escapes it.
fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            character if (character as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => out.push(character),
        }
    }
    out.push('"');
    out
}
