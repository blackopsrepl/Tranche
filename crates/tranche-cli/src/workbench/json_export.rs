use serde_json::Value;
use tranche_core::report::Root;
use tranche_core::util::{atomic_write, indented_json};

/// Write the versioned standalone JSON contract.
pub(super) fn write(root: &Root, payload: &Value, report_binding: &str) -> Result<usize, String> {
    let document = super::export_data::document(payload, report_binding);
    let mut encoded = indented_json(&document).map_err(|error| error.to_string())?;
    encoded.push('\n');
    let path = root.docs_dir().join("data/report.json");
    atomic_write(&path, encoded.as_bytes())
        .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    Ok(encoded.len())
}
