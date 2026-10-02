//! Reading what is already on disk.
//!
//! `info` and the digest check `batches` performs both go through here, so a
//! malformed or absent file is reported the same way wherever it is needed.

use serde_json::Value;

use tranche_core::report::Root;

/// Read one JSON file, or report that the pipeline's input is unusable.
pub fn read_json(path: &std::path::Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    serde_json::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))
}

/// The summary of the report currently on disk, if it is one this pipeline made.
///
/// Read rather than recomputed, so a caller reports what was published instead of
/// what a fresh run would produce.
pub fn current_summary(root: &Root) -> Result<Value, String> {
    let summary = read_json(&root.summary_path())?;
    if summary["format_version"].as_u64() != Some(2) {
        return Err("unrecognized cluster observation; run cluster first".to_owned());
    }
    Ok(summary)
}
