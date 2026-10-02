use serde_json::{Value, json};
use tranche_core::report::REPOSITORY;

/// Stable standalone export envelope; the workbench payload remains private to the page.
pub(super) fn document(payload: &Value, report_binding: &str) -> Value {
    json!({
        "format": "tranche.page-export",
        "schema_version": 1,
        "repository": REPOSITORY,
        "report_binding": report_binding,
        "pull_requests": payload["prs"].clone(),
        "categories": payload["categories"].clone(),
        "groups": payload["groups"].clone(),
        "batches_available": payload["batches_available"].clone(),
        "batches": payload["batches"].clone(),
        "parked": payload["parked"].clone(),
    })
}
