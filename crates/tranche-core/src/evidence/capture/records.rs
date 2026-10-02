//! Recording a page and moving a group's state.
//!
//! A source is appended and given an identity derived from the generation and its
//! own fields, so the same page recorded twice under one generation is the same
//! statement. The page number is the group's own count plus one, which is what makes
//! a resumed capture continue rather than restart.

use serde_json::{Value, json};

use super::super::coverage::roll_up;
use super::super::selection::source_id;
use super::super::store::store_body;
use crate::report::Root;

/// One page's provenance, as it arrives.
pub struct Recorded<'a> {
    pub number: u64,
    pub component: &'a str,
    pub group: &'a str,
    pub body: &'a [u8],
    pub url: &'a str,
    pub accept: &'a str,
    pub media_type: &'a str,
    pub cursor: Option<&'a str>,
    pub next_cursor: Option<&'a str>,
}

/// Append one source record and advance its group.
pub fn record(root: &Root, manifest: &mut Value, page: Recorded<'_>) -> Result<(), String> {
    let sha = store_body(root, page.body).map_err(|error| error.0)?;
    let generation = manifest["generation"].clone();
    let pages = group_entry(manifest, page.number, page.component, page.group)["pages"]
        .as_u64()
        .unwrap_or(0);

    let mut source = json!({
        "id": "",
        "number": page.number,
        "component": page.component,
        "group": page.group,
        "page": pages + 1,
        "cursor": page.cursor,
        "next_cursor": page.next_cursor,
        "url": page.url,
        "accept": page.accept,
        "media_type": page.media_type,
        "captured_at": super::super::now(),
        "generation": generation,
        "body_sha256": sha,
    });
    // The identity is recomputed rather than stored, so a rewritten manifest
    // cannot carry a source that names something else.
    let identity = source_id(generation.as_str().unwrap_or(""), &source);
    source["id"] = json!(identity);

    let sources = manifest["sources"]
        .as_array_mut()
        .ok_or_else(|| "the manifest has no source list".to_owned())?;
    sources.push(source);

    let entry = group_entry(manifest, page.number, page.component, page.group);
    entry["pages"] = json!(pages + 1);
    let ids = entry["source_ids"]
        .as_array_mut()
        .ok_or_else(|| "the group has no source list".to_owned())?;
    ids.push(json!(identity));
    entry["next_url"] = match page.next_cursor {
        Some(cursor) => json!(cursor),
        None => Value::Null,
    };
    spend_bytes(manifest, page.body.len())
}

/// Mark a group complete or partial, and roll the component up.
pub fn finish(
    manifest: &mut Value,
    number: u64,
    component: &str,
    group: &str,
    complete: bool,
    reason: Option<&str>,
) {
    let entry = group_entry(manifest, number, component, group);
    entry["status"] = json!(if complete { "complete" } else { "partial" });
    entry["reason"] = match reason {
        Some(reason) => json!(reason),
        None => Value::Null,
    };
    if complete {
        entry["next_url"] = Value::Null;
    }
    roll_up(component_entry(manifest, number, component));
}

/// Mark a group blocked. A blocked group is a named gap, not a failure of the run:
/// the rest of the capture continues.
pub fn block(manifest: &mut Value, number: u64, component: &str, group: &str, reason: &str) {
    let entry = group_entry(manifest, number, component, group);
    entry["status"] = json!("blocked");
    entry["reason"] = json!(reason);
    roll_up(component_entry(manifest, number, component));
}

/// Record the observed and reported counts a group finished with.
pub fn set_counts(
    manifest: &mut Value,
    number: u64,
    component: &str,
    group: &str,
    counts: &super::pages::Counts,
) {
    let entry = group_entry(manifest, number, component, group);
    if let serde_json::Value::Object(object) = entry {
        object.insert("items_observed".to_owned(), json!(counts.observed));
        if let Some(reported) = counts.reported {
            object.insert("items_reported".to_owned(), json!(reported));
        }
    }
}

/// The group state inside one component entry.
pub fn group_entry<'a>(
    manifest: &'a mut Value,
    number: u64,
    component: &str,
    group: &str,
) -> &'a mut Value {
    let entry = component_entry(manifest, number, component);
    entry["groups"]
        .as_array_mut()
        .and_then(|groups| {
            groups
                .iter_mut()
                .find(|state| state["group"].as_str() == Some(group))
        })
        .expect("the manifest declares every group the component has")
}

/// The component entry for one member.
pub fn component_entry<'a>(manifest: &'a mut Value, number: u64, component: &str) -> &'a mut Value {
    manifest["components"]
        .as_array_mut()
        .and_then(|components| {
            components.iter_mut().find(|entry| {
                entry["number"].as_u64() == Some(number)
                    && entry["component"].as_str() == Some(component)
            })
        })
        .expect("the manifest declares every component every member has")
}

/// Add stored bytes to the capture's running total.
fn spend_bytes(manifest: &mut Value, count: usize) -> Result<(), String> {
    let stored = manifest["capture"]["bytes_stored"].as_u64().unwrap_or(0);
    manifest["capture"]["bytes_stored"] = json!(stored + count as u64);
    Ok(())
}
