//! Contract boundary checks; validation never reshapes the frozen policy.
use serde_json::Value;

pub(super) fn validate(value: &Value) -> Result<(), String> {
    for pointer in ["/version", "/policy/version"] {
        if value.pointer(pointer).and_then(Value::as_u64) != Some(1) {
            return Err(format!("`{pointer}` must be supported version 1"));
        }
    }
    if value["model"].as_str().is_none_or(|s| s.trim().is_empty()) {
        return Err("`model` must be a nonempty string".into());
    }
    for (half, expected) in [("judge", 7), ("pair", 1)] {
        if value["policy"][half]
            .as_object()
            .is_none_or(|questions| questions.len() != expected)
        {
            return Err(format!(
                "`policy.{half}` must contain exactly the {expected} supported v1 questions"
            ));
        }
    }
    for (half, name, kind, length) in [
        ("judge", "category", "choice", 0),
        ("judge", "risk", "score", 5),
        ("judge", "is_fix", "noul", 0),
        ("judge", "dupe_signal", "noul", 0),
        ("judge", "finished_form", "score", 4),
        ("judge", "review_effort", "score", 4),
        ("judge", "security_flag", "noul", 0),
        ("pair", "sameness", "choice", 0),
    ] {
        let field = format!("policy.{half}.{name}");
        let spec = &value["policy"][half][name];
        if spec["type"].as_str() != Some(kind) {
            return Err(format!("`{field}.type` must be `{kind}`"));
        }
        let instructions = &spec["instructions"];
        if !nonempty(instructions) && !nonempty(&instructions["question"]) {
            return Err(format!(
                "`{field}.instructions` must contain a nonempty question"
            ));
        }
        if kind == "choice" {
            let criteria = spec["criteria"]
                .as_object()
                .filter(|c| {
                    !c.is_empty() && c.iter().all(|(k, v)| !k.trim().is_empty() && nonempty(v))
                })
                .ok_or_else(|| {
                    format!("`{field}.criteria` must be a nonempty object of choice descriptions")
                })?;
            if half == "pair"
                && (criteria.len() != 3
                    || ["same_change", "related_but_different", "unrelated"]
                        .iter()
                        .any(|k| !criteria.contains_key(*k)))
            {
                return Err(format!(
                    "`{field}.criteria` must name same_change, related_but_different and unrelated"
                ));
            }
        } else if kind == "score"
            && spec["criteria"]
                .as_array()
                .is_none_or(|c| c.len() != length || !c.iter().all(nonempty))
        {
            return Err(format!(
                "`{field}.criteria` must contain {length} nonempty score descriptions"
            ));
        }
    }
    Ok(())
}

fn nonempty(value: &Value) -> bool {
    value.as_str().is_some_and(|s| !s.trim().is_empty())
}
