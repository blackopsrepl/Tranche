//! The batch plan and park record as written into the published report.
//!
//! These are the human-readable half of the same facts the JSON files carry, so
//! they read the packed value rather than recomputing anything.

use serde_json::Value;
use std::collections::BTreeMap;

use super::super::pr::Prs;
use super::pack::BATCH_SIZE;
use super::park::unblock_for;

/// The batch-plan section appended to the published report.
pub fn batch_plan_section(batches: &Value) -> String {
    let list = batches
        .get("batches")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let batch_size = batches
        .get("batch_size")
        .and_then(Value::as_u64)
        .unwrap_or(BATCH_SIZE as u64);
    let mut lines = vec![
        String::new(),
        "# Suggested pre-release batches (issue #4)".to_owned(),
        String::new(),
        format!("A batch is **{batch_size} PRs merged together as one tranche**. Jev determines"),
        "the composition: model-consistent same_change groups are atomic — their PRs combine into"
            .to_owned(),
        "ONE pull request inside the batch. Batches are disjoint (every PR is in at most one"
            .to_owned(),
        "batch); review groups are excluded on purpose. Ordered security-first, then".to_owned(),
        "risk band and evidenced head idle time. Model-suggested, never verified safe to merge."
            .to_owned(),
        String::new(),
        format!(
            "Batches: {} · security-first batches: {} · same-change groups: {} · review-group PRs excluded: {}.",
            list.len(),
            batches
                .get("security_batches")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            batches
                .get("same_change_groups")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            batches
                .get("excluded_review_prs")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        ),
        String::new(),
    ];
    if !list.is_empty() {
        lines.push("| Batch | Size | Security | Avg risk | Groups | Members |".to_owned());
        lines.push("|---|---|---|---|---|---|".to_owned());
        for batch in &list {
            let members = batch
                .get("members")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_u64)
                        .map(|number| format!("#{number}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default();
            let risk = match batch.get("average_risk").and_then(Value::as_f64) {
                None => "unknown".to_owned(),
                Some(value) => format!("{value:.1}"),
            };
            let security = batch
                .get("security_members")
                .and_then(Value::as_u64)
                .filter(|value| *value > 0)
                .map(|value| value.to_string())
                .unwrap_or_else(|| "—".to_owned());
            let groups = batch
                .get("same_change_groups")
                .and_then(Value::as_u64)
                .filter(|value| *value > 0)
                .map(|value| value.to_string())
                .unwrap_or_else(|| "—".to_owned());
            lines.push(format!(
                "| {} | {} | {} | {} | {} | {} |",
                batch.get("id").and_then(Value::as_str).unwrap_or("?"),
                batch.get("count").and_then(Value::as_u64).unwrap_or(0),
                security,
                risk,
                groups,
                members,
            ));
        }
        lines.push(String::new());
        lines.push("## Reviewer agent prompts".to_owned());
        lines.push(String::new());
        lines.push(
            "Copy-paste a prompt into an agent to start a thorough, methodical review that"
                .to_owned(),
        );
        lines.push("produces one unified proposal + merge plan for the batch.".to_owned());
        lines.push(String::new());
        for batch in &list {
            lines.push(format!(
                "### {}",
                batch.get("id").and_then(Value::as_str).unwrap_or("?")
            ));
            lines.push(String::new());
            lines.push("```".to_owned());
            lines.push(
                batch
                    .get("review_prompt")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
            );
            lines.push("```".to_owned());
            lines.push(String::new());
        }
    }
    trimmed(lines)
}

/// The park-record section appended to the published report.
pub fn park_section(parks: &BTreeMap<u64, Vec<String>>, prs: &Prs) -> String {
    let count = |reason: &str| {
        parks
            .values()
            .filter(|r| r.iter().any(|r| r == reason))
            .count()
    };
    let mut lines = vec![
        String::new(),
        "# Parked before batching (issue #8)".to_owned(),
        String::new(),
        format!(
            "{} PRs are parked: drafts, PRs without finished form, PRs without a",
            parks.len()
        ),
        "current judgment, and same_change groups holding for a parked member. Park is a"
            .to_owned(),
        "**hold with a named unblock path, never a close** — re-entry is automatic when the"
            .to_owned(),
        "reason clears and the next refresh re-packs. No batch lists a parked PR.".to_owned(),
        String::new(),
        format!(
            "Reasons: draft {} · finished_form {} · unjudged_or_stale {} · same_change_hold {}.",
            count("draft"),
            count("finished_form"),
            count("unjudged_or_stale"),
            count("same_change_hold"),
        ),
        String::new(),
        "| PR | Reasons | Unblocked by |".to_owned(),
        "|---|---|---|".to_owned(),
    ];
    for (number, reasons) in parks {
        let unblock = reasons
            .iter()
            .map(|reason| unblock_for(reason))
            .collect::<Vec<_>>()
            .join(" ");
        lines.push(format!(
            "| #{number} | {} | {} |",
            reasons.join(", "),
            unblock
        ));
    }
    let _ = prs;
    lines.push(String::new());
    lines.push(
        "Parked does not remove a security-flagged PR from the security meta-category;".to_owned(),
    );
    lines.push("it only removes it from merge batches. Full record: out/parked.json.".to_owned());
    lines.push(String::new());
    trimmed(lines)
}

/// Join lines, trailing newline only.
fn trimmed(lines: Vec<String>) -> String {
    let mut section = lines.join("\n");
    while section.ends_with('\n') {
        section.pop();
    }
    section.push('\n');
    section
}
