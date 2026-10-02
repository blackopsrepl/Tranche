//! The manifest a capture writes, and the validation that proves a stored one
//! describes itself.
//!
//! A manifest is untrusted input: it is a file under `out/` a writer could have
//! edited. Reading one re-derives every identity it claims — the generation from
//! its own selection, each source's id from its own record — rather than
//! believing the stored value. Nothing here consults the current report, so a
//! historical capture stays readable after the report changes.

use serde_json::{Map, Value, json};

use super::components::groups_for;
use super::paths::{atomic_manifest, manifest_path, validate_body_digest};
use super::selection::{Selection, generation_id, source_id};
use super::store::strict_json_loads;
use super::{COMPONENTS, FORMAT, PROFILE, RESERVED_BUDGET};
use crate::report::Root;

/// The empty manifest a fresh capture starts from.
pub fn build_manifest(
    selection: &Selection,
    capture_id: &str,
    request_limit: i64,
    observed_at: Option<&str>,
) -> Value {
    let observed = observed_at.map(str::to_owned).unwrap_or_else(super::now);
    let thread_updated: Map<String, Value> = selection
        .members
        .iter()
        .map(|member| {
            (
                member.number.to_string(),
                member
                    .updated_at
                    .clone()
                    .map(Value::String)
                    .unwrap_or(Value::Null),
            )
        })
        .collect();
    json!({
        "format": FORMAT,
        "profile": PROFILE,
        "capture_id": capture_id,
        "generation": generation_id(selection, capture_id),
        "created_at": observed,
        "updated_at": observed,
        "selection": selection.as_json(),
        "code_observation": {
            "observed_at": observed,
            "thread_updated_at": Value::Object(thread_updated),
        },
        "capture": {
            "observed_at": observed,
            "request_limit": request_limit,
            "requests_used": 0,
            "reserved": RESERVED_BUDGET,
            "failures": 0,
            "retries": 0,
            "identity_checks": 0,
            "stop_reason": Value::Null,
            "bytes_stored": 0,
        },
        "sources": [],
        "components": super::coverage::fresh_components(selection),
        "citations": [],
    })
}

/// Publish a manifest, stamping the update time.
pub fn write_manifest(
    root: &Root,
    capture_id: &str,
    manifest: &mut Value,
) -> Result<(), super::Error> {
    if let Some(object) = manifest.as_object_mut() {
        object.insert("updated_at".to_owned(), json!(super::now()));
    }
    let path = manifest_path(root, capture_id)?;
    atomic_manifest(&path, manifest).map_err(Into::into)
}

/// Load and structurally verify one capture checkpoint.
pub fn read_manifest(root: &Root, capture_id: &str) -> Result<Value, super::Error> {
    let path = manifest_path(root, capture_id)?;
    if !path.exists() {
        return Err(super::refuse(format!(
            "no capture {capture_id} under {}",
            super::evidence_root(root).display()
        ))
        .into());
    }
    let data = std::fs::read(&path)
        .map_err(|error| super::refuse(format!("cannot read capture manifest: {error}")))?;
    let manifest = strict_json_loads(&data)?;
    check_shape(&manifest, capture_id)?;
    let selection =
        super::selection::selection_from_json(manifest.get("selection").expect("checked above"))?;
    let generation = manifest
        .get("generation")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    if generation != generation_id(&selection, capture_id) {
        return Err(
            super::refuse("capture manifest generation does not match its own selection").into(),
        );
    }
    let ids = check_sources(&manifest, &selection, &generation)?;
    check_citations(&manifest, &ids)?;
    Ok(manifest)
}

fn check_shape(manifest: &Value, capture_id: &str) -> Result<(), super::Error> {
    if !manifest.is_object() {
        return Err(super::refuse("capture manifest is not a JSON object").into());
    }
    for field in [
        "format",
        "profile",
        "capture_id",
        "generation",
        "created_at",
        "updated_at",
        "selection",
        "code_observation",
        "capture",
        "sources",
        "components",
        "citations",
    ] {
        if manifest.get(field).is_none() {
            return Err(super::refuse(format!("capture manifest is missing {field:?}")).into());
        }
    }
    if manifest.get("format").and_then(Value::as_str) != Some(FORMAT)
        || manifest.get("profile").and_then(Value::as_str) != Some(PROFILE)
    {
        return Err(
            super::refuse("capture manifest declares a different format or profile").into(),
        );
    }
    if manifest.get("capture_id").and_then(Value::as_str) != Some(capture_id) {
        return Err(super::refuse("capture manifest does not match its directory").into());
    }
    if !manifest.get("sources").is_some_and(Value::is_array)
        || !manifest.get("components").is_some_and(Value::is_array)
        || !manifest.get("citations").is_some_and(Value::is_array)
    {
        return Err(super::refuse("capture manifest records are malformed").into());
    }
    Ok(())
}

/// Verify each source record recomputes its own identity and belongs to this
/// generation. Returns the set of ids the citations may refer to.
fn check_sources(
    manifest: &Value,
    selection: &Selection,
    generation: &str,
) -> Result<std::collections::HashSet<String>, super::Error> {
    let members = selection.numbers();
    let mut seen_ids = std::collections::HashSet::new();
    let mut seen_slots = std::collections::HashSet::new();
    for source in manifest
        .get("sources")
        .and_then(Value::as_array)
        .expect("checked by shape")
    {
        let object = source
            .as_object()
            .ok_or_else(|| super::refuse("capture manifest source record is malformed"))?;
        for field in [
            "id",
            "number",
            "component",
            "group",
            "page",
            "cursor",
            "next_cursor",
            "url",
            "accept",
            "media_type",
            "captured_at",
            "generation",
            "body_sha256",
        ] {
            if !object.contains_key(field) {
                return Err(super::refuse(format!("source record is missing {field:?}")).into());
            }
        }
        let number = object.get("number").and_then(Value::as_u64).unwrap_or(0);
        if !members.contains(&number) {
            return Err(super::refuse("source record names a member outside the selection").into());
        }
        let component = object
            .get("component")
            .and_then(Value::as_str)
            .unwrap_or("");
        let group = object.get("group").and_then(Value::as_str).unwrap_or("");
        if !COMPONENTS.contains(&component)
            || !groups_for(component).iter().any(|name| name == group)
        {
            return Err(super::refuse(
                "source record names an unknown component or a group not belonging to it",
            )
            .into());
        }
        let page = object.get("page").and_then(Value::as_u64).unwrap_or(0);
        if page < 1 {
            return Err(super::refuse("source record page is malformed").into());
        }
        let slot = (number, component.to_owned(), group.to_owned(), page);
        let id = object
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        if !seen_slots.insert(slot) || !seen_ids.insert(id.clone()) {
            return Err(super::refuse("capture manifest repeats a source slot or id").into());
        }
        if object.get("generation").and_then(Value::as_str) != Some(generation) {
            return Err(super::refuse("source record belongs to a different generation").into());
        }
        if id != source_id(generation, source) {
            return Err(super::refuse("source record id does not match its own contents").into());
        }
        let body = object
            .get("body_sha256")
            .and_then(Value::as_str)
            .unwrap_or("");
        validate_body_digest(body)
            .map_err(|_| super::refuse("source record has no usable body digest"))?;
    }
    Ok(seen_ids)
}

fn check_citations(
    manifest: &Value,
    seen_ids: &std::collections::HashSet<String>,
) -> Result<(), super::Error> {
    for citation in manifest
        .get("citations")
        .and_then(Value::as_array)
        .expect("checked by shape")
    {
        let Some(object) = citation.as_object() else {
            return Err(super::refuse("capture manifest citation is malformed").into());
        };
        let expected: std::collections::HashSet<&str> = [
            "id",
            "number",
            "component",
            "source_id",
            "source_sha256",
            "start_byte",
            "end_byte",
            "excerpt_sha256",
            "kind",
        ]
        .into_iter()
        .collect();
        let actual: std::collections::HashSet<&str> = object.keys().map(String::as_str).collect();
        if actual != expected {
            return Err(super::refuse("capture manifest citation is malformed").into());
        }
        let source_id_value = object
            .get("source_id")
            .and_then(Value::as_str)
            .unwrap_or("");
        if !seen_ids.contains(source_id_value) {
            return Err(super::refuse("citation references a source outside the capture").into());
        }
        let start = object.get("start_byte").and_then(Value::as_i64);
        let end = object.get("end_byte").and_then(Value::as_i64);
        match (start, end) {
            (Some(start), Some(end)) if start >= 0 && end > start => {}
            _ => {
                return Err(super::refuse(
                    "citation byte range is not a nonempty half-open interval",
                )
                .into());
            }
        }
    }
    Ok(())
}
