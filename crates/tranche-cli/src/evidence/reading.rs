//! Finding a stored capture, and printing what it recorded.

use serde_json::Value;
use tranche_core::domain::pr::load_prs;
use tranche_core::evidence::capture;
use tranche_core::evidence::{Plan, select};
use tranche_core::policy::Contract;
use tranche_core::report::Root;

/// The capture a batch currently resolves to, if one exists.
pub fn resume_target(
    root: &Root,
    selection: &tranche_core::evidence::selection::Selection,
) -> Option<String> {
    let wanted = selection.as_json();
    let directory = root.evidence_dir();
    let entries = std::fs::read_dir(&directory).ok()?;
    let mut found: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let id = entry.file_name().to_string_lossy().into_owned();
        if let Ok(manifest) = capture::stored_manifest(root, &id)
            && manifest["selection"] == wanted
        {
            found.push(id);
        }
    }
    found.sort();
    found.into_iter().next()
}
/// The manifest a show or export command reads.
pub fn load_manifest(
    root: &Root,
    capture_id: Option<&str>,
    batch: Option<&str>,
) -> Result<(Value, Option<String>), tranche_core::evidence::EvidenceError> {
    if let Some(id) = capture_id {
        // Historical inspection consults no report at all.
        let manifest =
            capture::stored_manifest(root, id).map_err(tranche_core::evidence::refuse)?;
        return Ok((manifest, None));
    }
    let Some(batch_id) = batch else {
        return Err(tranche_core::evidence::refuse(
            "expected --batch or --capture",
        ));
    };
    let repository = Contract::load(root.path())
        .map(|contract| contract.repository().to_owned())
        .map_err(tranche_core::evidence::refuse)?;
    let corpus =
        load_prs(root, &repository).map_err(|error| tranche_core::evidence::refuse(error.0))?;
    let plan = Plan::read(root).map_err(|error| tranche_core::evidence::refuse(error.0))?;
    let selection = select(root, batch_id, &corpus, &plan)
        .map_err(|error| tranche_core::evidence::refuse(error.0))?;
    match resume_target(root, &selection) {
        Some(id) => {
            let manifest =
                capture::stored_manifest(root, &id).map_err(tranche_core::evidence::refuse)?;
            Ok((manifest, None))
        }
        None => Err(tranche_core::evidence::refuse(format!(
            "no capture is associated with the current {batch_id}; \
             run `tranche evidence capture --batch {batch_id}`"
        ))),
    }
}
/// How a caller resumes an incomplete capture.
pub fn resume_hint(batch: &str, capture_id: &str) -> String {
    format!("resume: tranche evidence capture --batch {batch} --reuse-capture {capture_id}")
}
/// The coverage report as the operator reads it.
pub fn print_coverage(coverage: &Value) {
    let state = if coverage["complete"].as_bool() == Some(true) {
        "COMPLETE"
    } else {
        "INCOMPLETE"
    };
    let generation = coverage["generation"].as_str().unwrap_or("");
    println!(
        "capture {}  batch {}  generation {}...  {state}",
        coverage["capture_id"].as_str().unwrap_or(""),
        coverage["batch"].as_str().unwrap_or(""),
        &generation[..generation.len().min(12)]
    );
    let requests = &coverage["requests"];
    println!(
        "  observed {}  stop {}  requests {}/{} (failures {}, identity checks {})",
        coverage["observed_at"].as_str().unwrap_or(""),
        coverage["stop_reason"].as_str().unwrap_or("none"),
        requests["requests_used"],
        requests["request_limit"],
        requests["failures"],
        requests["identity_checks"]
    );
    println!(
        "  {} bytes stored, {} citations",
        coverage["bytes_stored"], coverage["citations"]
    );
    for member in coverage["members"].as_array().into_iter().flatten() {
        let head = member["head_sha"].as_str().unwrap_or("");
        let base = member["base_sha"].as_str().unwrap_or("");
        println!(
            "  #{}  head {}...  base {}...",
            member["number"],
            &head[..head.len().min(12)],
            &base[..base.len().min(12)]
        );
        for component in member["components"].as_array().into_iter().flatten() {
            let detail = match (component["status"].as_str(), component["reason"].as_str()) {
                (Some("complete"), _) | (_, None) => String::new(),
                (_, Some(reason)) => format!(" - {reason}"),
            };
            let groups = component["groups"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|group| {
                    format!(
                        "{}={}",
                        group["group"].as_str().unwrap_or(""),
                        group["status"].as_str().unwrap_or("")
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            println!(
                "      {:<16} {:<9} {:>3}p {:>4}c  [{groups}]{detail}",
                component["component"].as_str().unwrap_or(""),
                component["status"].as_str().unwrap_or(""),
                component["pages"],
                component["citations"]
            );
        }
    }
}
