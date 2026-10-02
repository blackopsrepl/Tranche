//! The packet's own encoding, which is this project's object digest rather than a
//! general canonicalization standard.
//!
//! Sorted keys, compact separators, no ASCII escaping, no trailing newline. The
//! distinction matters: `tranche.digest()` is an encoding this project defines,
//! and the packets it hashes are read back by this project's own tooling.

use crate::util::digest;
use serde_json::{Map, Value};

/// The packet format this module produces.
pub const FORMAT: &str = "tranche.evidence-packet/v1";
/// The profile whose components and sources the packet describes.
pub const PROFILE: &str = "pr-review/v1";
/// The export bound, applied to the exact serialized bytes. Base64 expansion and
/// metadata count against it.
pub const DEFAULT_EXPORT_BYTES: usize = 64 * 1024 * 1024;

/// The object digest of a packet before its own digest is added.
pub fn packet_digest(packet: &Value) -> String {
    digest(packet)
}

/// The exact serialized bytes, or a refusal when they exceed the bound.
///
/// The size is checked on the bytes rather than on an estimate, because base64
/// expansion is what makes a packet large and it is not predictable from the
/// record count.
pub fn packet_bytes(packet: &Value, limit: usize) -> Result<Vec<u8>, String> {
    let Value::Object(object) = packet else {
        return Err("a packet must be an object".to_owned());
    };
    let mut sorted = Map::new();
    let mut keys: Vec<&String> = object.keys().collect();
    keys.sort();
    for key in keys {
        sorted.insert(key.clone(), object[key].clone());
    }
    let text = serde_json::to_string(&Value::Object(sorted))
        .map_err(|error| format!("packet cannot be serialized ({error})"))?;
    let bytes = text.into_bytes();
    if bytes.len() > limit {
        return Err(format!(
            "packet serializes to {} bytes, over the {limit} byte export bound; \
             narrow the batch or raise the bound deliberately",
            bytes.len()
        ));
    }
    Ok(bytes)
}

/// The explanatory text every packet carries.
///
/// It is part of the packet rather than documentation on purpose: a reader holding
/// only the file must not conclude that a digest proves authenticity, that a
/// captured CI run was a test Tranche ran, or that completeness is approval.
pub const NOTES: &str = "Captured from public GitHub by Tranche; read-only and model-free. \
Digests prove integrity, not authenticity: they show the bytes are the bytes recorded, not \
that GitHub served them. A captured CI run is an observation, not a test performed by \
Tranche. Complete means every endpoint group reported a terminal page within the recorded \
budgets; it is not a review, a test run or approval to merge.";
