//! Solve from validated reports; export private evidence only as digests.
use crate::commands::Outcome;
use tranche_core::{
    domain::assignment::{
        proposal::{payload, prepare},
        solve::solve,
    },
    report::{Limits, Root, load},
    util::{atomic_json, indented_json},
};

pub fn assign(root: &Root, json: bool) -> Outcome {
    match build(root) {
        Ok(value) => Outcome::success(if json {
            format!("{}\n", indented_json(&value).unwrap_or_default())
        } else {
            "wrote out/assignments.json; proposed owners are synthetic, not real handles\n".into()
        }),
        Err(e) => Outcome::refusal(format!("assign: {e}"), 1),
    }
}
fn build(root: &Root) -> Result<serde_json::Value, String> {
    // Ignore only the prior assignment during regeneration, never the underlying reports.
    let report = tranche_core::report::reading::read_for_assignment(root, &Limits::default())
        .map_err(|e| e.to_string())?;
    let prepared = prepare(
        root,
        &report.prs,
        &report.judgments,
        &report.dupes,
        report.batches.as_ref(),
        &report.summary["report_binding"],
    )?;
    let solved = solve(prepared.plan.clone())?;
    let value = payload(&prepared, &solved)?;
    atomic_json(&root.assignments_path(), &value).map_err(|e| e.to_string())?;
    let existing = std::fs::read_to_string(root.tranches_path()).map_err(|e| e.to_string())?;
    let prefix = existing
        .split("\n<!-- tranche-assignments -->")
        .next()
        .unwrap_or(&existing)
        .trim_end();
    let count = value["assignments"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["member_id"].is_string())
        .count();
    let mut section = format!(
        "{prefix}\n\n<!-- tranche-assignments -->\n## Proposed skill assignments (synthetic simulation)\n\nAI-assisted proposals, not approvals or real GitHub handles. {count} PRs proposed; other work remains unassigned.\n\n"
    );
    for row in value["assignments"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["member_id"].is_string())
    {
        section.push_str(&format!(
            "- #{} → {} (proposed)\n",
            row["number"],
            row["member_id"].as_str().unwrap()
        ));
    }
    tranche_core::util::atomic_write(&root.tranches_path(), section.as_bytes())
        .map_err(|e| e.to_string())?;
    // Read back through the exact validator before reporting success.
    load(root, &Limits::default()).map_err(|e| e.to_string())?;
    Ok(value)
}
