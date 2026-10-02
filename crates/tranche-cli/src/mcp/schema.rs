//! The six read-only tools, as JSON Schema the client sees.

use serde_json::{Value, json};

/// The tools' own descriptions, verbatim in intent from the reference server.
const DESCRIPTIONS: [(&str, &str); 6] = [
    (
        "surface",
        "Inspect bound report coverage; model suggestions, never merge approval.",
    ),
    (
        "query",
        "Security-first PR search. Exact filters, finished_form score; offset/limit pagination.",
    ),
    (
        "pick",
        "Inspect one exact batch id with source-bound PRs and the unchanged review prompt.",
    ),
    (
        "next_prompt",
        "Next batch in report order; after is an existing ordinal/id, 0 starts.",
    ),
    (
        "related",
        "Inspect model relationship evidence, conflicts and missing pairs; no survivor selected.",
    ),
    (
        "digests",
        "Read current report binding, output checksums and before/after checked input byte digests.",
    ),
];

fn pagination() -> Value {
    json!({
        "offset": {"type": "integer", "minimum": 0, "maximum": 100000, "default": 0},
        "limit": {"type": "integer", "minimum": 1, "maximum": 100, "default": 25},
    })
}

fn batch_id() -> Value {
    json!({"type": "string", "pattern": "^B[0-9]{3,6}$"})
}

/// The tool definitions a `tools/list` response carries.
pub fn definitions(judge_categories: &[String]) -> Vec<Value> {
    let mut categories: Vec<Value> = judge_categories
        .iter()
        .map(|category| json!(category))
        .collect();
    categories.extend([json!("security-review"), json!("unknown"), Value::Null]);
    let properties = json!({
        "surface": {},
        "query": {
            "text": {"type": "string", "maxLength": 512, "default": ""},
            "category": {"type": ["string", "null"], "default": null, "enum": categories},
            "risk_band": {"type": ["string", "null"], "default": null,
                          "enum": ["low", "core", "danger", "unknown", null]},
            "security": {"type": ["boolean", "null"], "default": null},
            "finished_form": {"type": ["number", "null"], "minimum": 0, "maximum": 3, "default": null},
            "batch": {"type": ["string", "null"], "default": null, "pattern": "^B[0-9]{3,6}$"},
            "queue": {"type": "string", "default": "all",
                      "enum": ["all", "security", "candidates", "senior", "followup", "parked", "related", "assigned"]},
            "offset": pagination()["offset"],
            "limit": pagination()["limit"],
        },
        "pick": {"batch_id": batch_id()},
        "next_prompt": {"after": {"anyOf": [
            {"type": "integer", "minimum": 0}, batch_id(), {"type": "null"}],
            "default": null}},
        "related": {"number": {"type": "integer", "minimum": 1},
                    "offset": pagination()["offset"], "limit": pagination()["limit"]},
        "digests": {},
    });
    let required = json!({"pick": ["batch_id"], "related": ["number"]});
    let object = properties.as_object().expect("a literal object");
    DESCRIPTIONS
        .iter()
        .map(|(name, description)| {
            json!({
                "name": name,
                "description": description,
                "inputSchema": {
                    "type": "object",
                    "properties": object.get(*name).cloned().unwrap_or(json!({})),
                    "additionalProperties": false,
                    "required": required.get(*name).cloned().unwrap_or(json!([])),
                },
                "annotations": {
                    "readOnlyHint": true, "destructiveHint": false,
                    "idempotentHint": true, "openWorldHint": false,
                },
            })
        })
        .collect()
}

/// Whether `value` has the JSON type `expected`.
///
/// JSON types, not Rust's: a bool is not a number and null is its own type.
fn type_matches(value: &Value, expected: &str) -> bool {
    match expected {
        "null" => value.is_null(),
        "boolean" => value.is_boolean(),
        "integer" => value.is_u64() || value.is_i64(),
        "number" => value.is_number(),
        "string" => value.is_string(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        _ => true,
    }
}

/// Validate one value against the schema subset these tools advertise.
fn check(value: &Value, schema: &Value, owner: &str, key: &str) -> Result<(), String> {
    let where_ = if key.is_empty() {
        owner.to_owned()
    } else {
        format!("{owner}.{key}")
    };
    if let Some(options) = schema.get("anyOf").and_then(Value::as_array) {
        return options
            .iter()
            .find_map(|option| check(value, option, owner, key).ok())
            .ok_or_else(|| format!("'{where_}' does not match any accepted form"));
    }
    let types: Vec<&str> = match schema.get("type") {
        Some(Value::String(text)) => vec![text.as_str()],
        Some(Value::Array(items)) => items.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    if !types.is_empty() && !types.iter().any(|expected| type_matches(value, expected)) {
        return Err(format!("'{where_}' must be {}", types.join("/")));
    }
    if value.is_null() {
        return Ok(());
    }
    if let Some(enum_values) = schema.get("enum").and_then(Value::as_array)
        && !enum_values.contains(value)
    {
        let allowed: Vec<String> = enum_values
            .iter()
            .filter(|item| !item.is_null())
            .map(|item| item.to_string())
            .collect();
        return Err(format!("'{where_}' must be one of: {}", allowed.join(", ")));
    }
    let number = || value.as_f64();
    if let Some(minimum) = schema.get("minimum").and_then(Value::as_f64)
        && number().is_some_and(|value| value < minimum)
    {
        return Err(format!("'{where_}' must be >= {minimum}"));
    }
    if let Some(maximum) = schema.get("maximum").and_then(Value::as_f64)
        && number().is_some_and(|value| value > maximum)
    {
        return Err(format!("'{where_}' must be <= {maximum}"));
    }
    if let Some(max_length) = schema.get("maxLength").and_then(Value::as_u64)
        && value
            .as_str()
            .is_some_and(|text| text.chars().count() as u64 > max_length)
    {
        return Err(format!(
            "'{where_}' must be at most {max_length} characters"
        ));
    }
    if let Some(pattern) = schema.get("pattern").and_then(Value::as_str)
        && value
            .as_str()
            .is_some_and(|text| !pattern_matches(pattern, text))
    {
        return Err(format!("'{where_}' does not match {pattern}"));
    }
    Ok(())
}

/// The two patterns the tools advertise, matched in full.
fn pattern_matches(pattern: &str, text: &str) -> bool {
    match pattern {
        "^B[0-9]{3,6}$" => {
            text.len() >= 4
                && text.len() <= 7
                && text.starts_with('B')
                && text[1..].chars().all(|c| c.is_ascii_digit())
        }
        _ => false,
    }
}

/// Enforce a tool's advertised input schema.
///
/// Clients are not required to validate — the spec puts the obligation on the
/// server — so an argument that arrives unchecked must still be refused in the
/// tool's own terms.
pub fn validate_arguments(name: &str, arguments: &Value, tools: &[Value]) -> Result<(), String> {
    if !arguments.is_object() {
        return Err("Arguments must be a JSON object".to_owned());
    }
    let Some(tool) = tools
        .iter()
        .find(|tool| tool["name"].as_str() == Some(name))
    else {
        return Err(format!("Unknown tool: {name}"));
    };
    let schema = &tool["inputSchema"];
    let properties = schema["properties"].as_object().expect("schema object");
    let unknown: Vec<String> = arguments
        .as_object()
        .expect("checked above")
        .keys()
        .filter(|key| !properties.contains_key(*key))
        .cloned()
        .collect();
    if !unknown.is_empty() {
        return Err(format!("Unknown argument(s): {}", unknown.join(", ")));
    }
    for required in schema["required"].as_array().into_iter().flatten() {
        if arguments.get(required.as_str().unwrap_or("")).is_none() {
            return Err(format!("Missing required argument(s): {}", required));
        }
    }
    for (key, value) in arguments.as_object().expect("checked above") {
        if let Some(spec) = properties.get(key) {
            check(value, spec, name, key)?;
        }
    }
    Ok(())
}
