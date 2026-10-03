//! The corpus-level check a captured PR must pass.

use serde_json::Value;

use super::error::PrError;

/// Refuse missing or foreign identity in legacy shards. A snapshot provides its
/// own repository binding; legacy records must establish theirs individually.
/// Forks in `head` are not the reviewed repository and are ignored.
pub(super) fn validate_repository(item: &Value, repository: &str) -> Result<(), PrError> {
    if item.pointer("/base/repo/full_name").is_none()
        && ["html_url", "url"]
            .iter()
            .all(|field| item.get(field).is_none_or(Value::is_null))
    {
        return Err(PrError(
            "legacy captured PR has no repository identity; fetch again".into(),
        ));
    }
    if let Some(name) = item.pointer("/base/repo/full_name")
        && name.as_str() != Some(repository)
    {
        return Err(PrError(
            "captured PR repository identity differs; fetch again".into(),
        ));
    }
    for (field, prefix, segment) in [
        ("html_url", "https://github.com/", "pull"),
        ("url", "https://api.github.com/repos/", "pulls"),
    ] {
        if let Some(value) = item.get(field).filter(|v| !v.is_null()) {
            let expected = format!("{prefix}{repository}/{segment}/");
            if !value.as_str().is_some_and(|s| {
                s.strip_prefix(&expected)
                    .is_some_and(|n| n.parse::<u64>().ok() == item["number"].as_u64())
            }) {
                return Err(PrError(
                    "captured PR repository identity differs; fetch again".into(),
                ));
            }
        }
    }
    Ok(())
}

/// A captured PR must carry the fields a judgment reads, or nothing may use it.
pub fn validate_pr(item: &Value) -> Result<(), PrError> {
    let object = item
        .as_object()
        .ok_or_else(|| PrError("invalid captured PR shape; fetch again".into()))?;
    match object.get("number") {
        Some(Value::Number(number)) if number.as_u64().is_some_and(|n| n > 0) => {}
        _ => return Err(PrError("invalid captured PR shape; fetch again".into())),
    }
    if !object.get("title").is_some_and(Value::is_string) {
        return Err(PrError("invalid captured PR shape; fetch again".into()));
    }
    match object.get("body") {
        None | Some(Value::Null) | Some(Value::String(_)) => {}
        Some(_) => return Err(PrError("invalid captured PR shape; fetch again".into())),
    }
    for field in ["head", "user", "author"] {
        match object.get(field) {
            None | Some(Value::Null) | Some(Value::Object(_)) => {}
            Some(_) => {
                return Err(PrError(format!("invalid captured PR {field}; fetch again")));
            }
        }
    }
    if let Some(labels) = object.get("labels").filter(|v| !v.is_null()) {
        let items = labels
            .as_array()
            .ok_or_else(|| PrError("invalid captured PR labels; fetch again".into()))?;
        for label in items {
            let valid = label
                .as_object()
                .and_then(|object| object.get("name"))
                .is_some_and(Value::is_string);
            if !valid {
                return Err(PrError("invalid captured PR labels; fetch again".into()));
            }
        }
    }
    Ok(())
}
