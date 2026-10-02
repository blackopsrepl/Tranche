//! Starting and resuming a capture: identity, files and the writer lock.
//!
//! A capture is addressed by a random identity and bound to one generation, so
//! resuming is a lookup rather than a guess. Every path here stays under the ignored
//! evidence directory.

use serde_json::{Value, json};

use super::super::coverage::fresh_components;
use super::super::lock::Lock;
use super::super::manifest::{build_manifest, read_manifest};
use super::super::read::url;
use super::super::selection::{Selection, generation_id, selection_from_json};
use crate::report::Root;

/// An empty manifest for a fresh capture, with its component state prepared.
pub fn fresh_manifest(
    selection: &Selection,
    capture_id: &str,
    request_limit: u64,
    max_bytes: u64,
) -> Value {
    let mut manifest = build_manifest(selection, capture_id, request_limit as i64, None);
    manifest["components"] = json!(fresh_components(selection));
    manifest["citations"] = json!([]);
    manifest["capture"]["max_bytes"] = json!(max_bytes);
    manifest
}

/// Read a stored manifest, or refuse a malformed one.
pub fn stored_manifest(root: &Root, capture_id: &str) -> Result<Value, String> {
    read_manifest(root, capture_id).map_err(|error| error.to_string())
}

/// The generation a capture id belongs to.
pub fn generation_of(selection: &Selection, capture_id: &str) -> String {
    generation_id(selection, capture_id)
}

/// Rebuild the selection a stored capture is bound to.
pub fn selection_of(manifest: &Value) -> Result<Selection, String> {
    let stored = manifest
        .get("selection")
        .ok_or_else(|| "the capture records no selection".to_owned())?;
    selection_from_json(stored).map_err(|error| error.to_string())
}

/// A capture id derived from the clock and the process, as the native identity is.
pub fn new_capture_id() -> String {
    let seed = format!("{}:{}", super::super::now(), std::process::id());
    let digest = crate::util::digest(&json!(seed));
    digest[..32].to_owned()
}

/// Hold the writer lock for one capture, releasing it on drop.
pub fn lock_for(root: &Root, capture_id: &str) -> Result<Lock, String> {
    Lock::new(root, capture_id).map_err(|error| error.to_string())
}

/// A URL this transport will accept, for a caller checking one first.
pub fn accepted_url(at: &str, repo: &str) -> Result<String, String> {
    url::validate(at, Some(repo)).map_err(|error| error.message)
}
