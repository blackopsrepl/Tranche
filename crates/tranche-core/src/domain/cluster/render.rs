//! The human report: `out/tranches.md`.
//!
//! Section order, wording and table layout follow the published report, because
//! the published file is what a maintainer reads and diffs. The JSON artifacts
//! are the machine contract; this is the prose that goes with them.

use serde_json::Value;

use super::SECURITY_PRIORITY;
use super::style::{cell, number, one_decimal, spaced_json, text, two_decimals};

/// The candidate table header, exactly as the published report writes it.
const CANDIDATE_TABLE: &str = "| PR | Title | Author | Model finished | Model effort | Model fix |\n\
                               |---|---|---|---|---|---|";

/// Render the whole report.
pub fn render(built: &super::Clustered, repository: &str) -> String {
    let summary = &built.summary;
    let dupes = &built.dupes;
    let confirmed = dupes["confirmed_groups"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let review = dupes["review_groups"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let uncertain = dupes["uncertain_pairs"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    let mut lines = vec![
        "# Tranche — PR review candidates".to_owned(),
        String::new(),
        format!(
            "Corpus: {} observed open PRs; {} matching judgments; {} unjudged/stale; {} unbound legacy judgments.",
            summary["prs_in_corpus"], summary["judged"], summary["unjudged_or_stale"],
            summary["unbound_judgments"],
        ),
        format!(
            "Review candidates: {}. Model-consistent groups: {}. Groups needing relationship review: {}. Security-priority items: {} (meta-category, reviewed first).",
            summary["ready_prs"], confirmed.len(), review.len(), summary["security_priority"],
        ),
        String::new(),
        "Evidence: titles and shortened descriptions (1200 characters per PR; 400 per pair). \
         Diffstat is unknown unless captured input supplies it. Patches, CI, reproductions, \
         fix coverage and security have not been verified. Model scores are suggestions, \
         not calibrated guarantees or approval to merge/close. Pagination records an observation, not a point-in-time GitHub snapshot."
            .to_owned(),
        String::new(),
    ];
    if summary["allow_unbound"].as_bool() == Some(true) {
        lines.push(
            "LEGACY INSPECTION: unbound judgments cannot establish freshness or enter review-candidate tranches."
                .to_owned(),
        );
        lines.push(String::new());
    }

    // Security ranks above every category.
    if !built.security_review.is_empty() {
        lines.push("## Security review — top priority (meta-category)".to_owned());
        lines.push(String::new());
        lines.push(
            "These PRs touch credentials, remote code execution, sudo/permissions, network"
                .to_owned(),
        );
        lines.push(
            format!(
                "exposure or crypto material (model probability ≥ {SECURITY_PRIORITY}). Review before any category batch."
            ),
        );
        lines.push(String::new());
        lines.push(CANDIDATE_TABLE.to_owned());
        lines.extend(built.security_review.iter().map(candidate_row));
        lines.push(String::new());
    }

    for (category, items) in &built.tranches {
        lines.push(format!(
            "## Review candidates: {category} — {} PRs",
            items.len()
        ));
        lines.push(String::new());
        lines.push(CANDIDATE_TABLE.to_owned());
        lines.extend(items.iter().map(candidate_row));
        lines.push(String::new());
    }

    lines.extend(group_section(
        "Model-consistent candidate groups — verify fix coverage; no survivor selected",
        &confirmed,
    ));
    lines.extend(group_section(
        "Candidate groups needing relationship review",
        &review,
    ));

    if !uncertain.is_empty() {
        lines.push("## Uncertain pairs — human comparison needed".to_owned());
        lines.push(String::new());
        for pair in &uncertain {
            lines.push(format!(
                "- #{} ↔ #{}: P(same)={}; verdict={}; {}",
                pair["a"],
                pair["b"],
                number(&pair["p_same"]),
                pair["verdict"].as_str().unwrap_or_default(),
                pair["classification"].as_str().unwrap_or_default(),
            ));
        }
        lines.push(String::new());
    }

    lines.extend(number_section(
        "Escalate for risk/security review",
        &built.escalate,
    ));
    lines.extend(number_section(
        "Possible author follow-up — verify before requesting changes",
        &built.follow_up,
    ));

    let _ = repository;
    // The report joins a line list whose last element is empty, so it ends
    // with exactly one newline. The blank line before the appended plan comes
    // from the plan section, not from here — adding one here shifts every
    // subsequent line of the published file.
    lines.join("\n")
}

/// One candidate row.
fn candidate_row(item: &Value) -> String {
    let title: String = text(&item["title"]).chars().take(80).collect();
    format!(
        "| [#{}]({}) | {} | {} | {} | {} | {} |",
        item["number"],
        text(&item["url"]),
        title,
        text(&item["author"]),
        one_decimal(item["finished_form"].as_f64()),
        one_decimal(item["review_effort"].as_f64()),
        two_decimals(item["is_fix"].as_f64()),
    )
}

/// Group lists: members, then whatever evidence keeps a group out of the
/// confirmed set.
fn group_section(label: &str, groups: &[Value]) -> Vec<String> {
    if groups.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![format!("## {label}"), String::new()];
    for group in groups {
        let members: Vec<String> = match group.get("members") {
            Some(_) => group["members"].as_array().cloned().unwrap_or_default(),
            None => group.as_array().cloned().unwrap_or_default(),
        }
        .iter()
        .filter_map(Value::as_u64)
        .map(|number| format!("#{number}"))
        .collect();
        lines.push(format!("- {}", members.join(", ")));
        if group.is_object() {
            for field in ["conflicting_pairs", "uncertain_pairs", "missing_pairs"] {
                if group[field]
                    .as_array()
                    .is_some_and(|items| !items.is_empty())
                {
                    lines.push(format!("  - {field}: {}", spaced_json(&group[field])));
                }
            }
            if group["unbound_evidence"].as_bool() == Some(true) {
                lines.push("  - Unbound legacy evidence; revisions cannot be checked.".to_owned());
            }
        }
    }
    lines.push(String::new());
    lines
}

/// Escalation and follow-up lists, each entry its number and title.
fn number_section(label: &str, entries: &[(u64, String)]) -> Vec<String> {
    if entries.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![format!("## {label}"), String::new()];
    lines.extend(
        entries
            .iter()
            .map(|(number, title)| format!("- #{number} {}", cell(title))),
    );
    lines.push(String::new());
    lines
}
