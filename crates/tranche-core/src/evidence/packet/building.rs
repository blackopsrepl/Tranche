//! Building the packet: the projection, and the one place the fields are chosen.
//!
//! The packet is a projection of the manifest, not a copy of it. A reader holding
//! only the packet can resolve every citation with no state store, no network and
//! no second application: the selection is frozen in, each body is carried exactly
//! once, and every identity is recomputed from what the packet itself holds.
//!
//! What is deliberately absent matters as much. Manifest bookkeeping such as
//! `created_at`, `updated_at` and `bytes_stored` is not exported, because a reader
//! verifying evidence has no use for it. The manifest's own flattened coverage
//! projection is not exported either: the component state travels as held, with its
//! endpoint groups intact.

use std::collections::BTreeMap;

use crate::report::Root;
use serde_json::{Map, Value, json};

use super::base64;
use super::encoding::{FORMAT, NOTES, PROFILE, packet_bytes, packet_digest};
use crate::evidence::coverage::is_complete;
use crate::evidence::selection::selection_from_json;
use crate::evidence::store::read_body;

/// The capture accounting keys the packet exports, in this order.
const ACCOUNTED: [&str; 8] = [
    "observed_at",
    "request_limit",
    "requests_used",
    "reserved",
    "failures",
    "retries",
    "identity_checks",
    "stop_reason",
];

/// The source keys the packet exports, in this order.
const SOURCE_FIELDS: [&str; 13] = [
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
];

/// Build the packet and its own digest.
pub fn build(root: &Root, manifest: &Value) -> Result<Value, String> {
    let selection = selection_from_json(
        manifest
            .get("selection")
            .ok_or_else(|| "the manifest records no selection".to_owned())?,
    )
    .map_err(|error| error.to_string())?;

    // Each distinct body is read and carried once, keyed by its digest, so
    // identical bytes are not repeated.
    let mut bodies: BTreeMap<String, Value> = BTreeMap::new();
    for source in manifest["sources"].as_array().into_iter().flatten() {
        let Some(sha) = source["body_sha256"].as_str() else {
            continue;
        };
        if bodies.contains_key(sha) {
            continue;
        }
        let raw = read_body(root, sha).map_err(|error| error.0)?;
        bodies.insert(
            sha.to_owned(),
            json!({
                "sha256": sha,
                "bytes": raw.len(),
                "base64": base64(&raw),
            }),
        );
    }

    let capture = ACCOUNTED
        .iter()
        .map(|key| {
            (
                (*key).to_owned(),
                manifest["capture"]
                    .get(*key)
                    .cloned()
                    .unwrap_or(Value::Null),
            )
        })
        .collect::<Map<String, Value>>();
    let sources: Vec<Value> = manifest["sources"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|source| {
            SOURCE_FIELDS
                .iter()
                .map(|key| {
                    (
                        (*key).to_owned(),
                        source.get(*key).cloned().unwrap_or(Value::Null),
                    )
                })
                .collect::<Map<String, Value>>()
                .into()
        })
        .collect();

    let mut packet = Map::new();
    packet.insert("format".to_owned(), json!(FORMAT));
    packet.insert("profile".to_owned(), json!(PROFILE));
    packet.insert("capture_id".to_owned(), manifest["capture_id"].clone());
    packet.insert("generation".to_owned(), manifest["generation"].clone());
    packet.insert("selection".to_owned(), selection.as_json());
    packet.insert(
        "membership_digest".to_owned(),
        json!(selection.membership_digest()),
    );
    packet.insert("complete".to_owned(), json!(is_complete(manifest)));
    packet.insert("capture".to_owned(), Value::Object(capture));
    packet.insert(
        "components".to_owned(),
        manifest.get("components").cloned().unwrap_or(json!([])),
    );
    packet.insert("sources".to_owned(), json!(sources));
    packet.insert(
        "bodies".to_owned(),
        Value::Object(bodies.into_iter().collect()),
    );
    packet.insert(
        "citations".to_owned(),
        manifest.get("citations").cloned().unwrap_or(json!([])),
    );
    packet.insert("notes".to_owned(), json!(NOTES));

    let value = Value::Object(packet);
    // The digest is taken before the field is added: it covers the packet, not
    // itself.
    let computed = packet_digest(&value);
    let mut with_digest = match value {
        Value::Object(object) => object,
        _ => unreachable!("built as an object"),
    };
    with_digest.insert("packet_digest".to_owned(), json!(computed));
    Ok(Value::Object(with_digest))
}

/// Serialize a built packet under the export bound.
pub fn bytes(root: &Root, manifest: &Value, limit: usize) -> Result<(Value, Vec<u8>), String> {
    let packet = build(root, manifest)?;
    let serialized = packet_bytes(&packet, limit)?;
    Ok((packet, serialized))
}
