//! The payload the workbench fetches at boot.
//!
//! Every field is copied from a report rather than derived here, so the page can
//! only show what `cluster`, `dupes` and `batches` already decided.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde_json::{Map, Value, json};
use tranche_core::domain::batch::pr_activity;
use tranche_core::domain::cluster::{escalated, review_candidate, security_priority};
use tranche_core::domain::judge::{Judgment, load_done};
use tranche_core::domain::pr::{Pr, Prs};
use tranche_core::policy::Contract;
use tranche_core::report::Root;

/// Build the payload.
///
/// The contract supplies the category labels the picker shows and the judge
/// policy the categories come from; the page invents neither.
pub(super) fn payload(
    corpus: &Prs,
    judgments: &HashMap<u64, Judgment>,
    dupes: &Value,
    batches: Option<&Value>,
    parked: Option<&Value>,
    root: &Root,
    contract: &Contract,
) -> Value {
    let grouped = grouped(dupes);
    let mut related = grouped.clone();
    for pair in dupes["uncertain_pairs"].as_array().into_iter().flatten() {
        for side in ["a", "b"] {
            if let Some(number) = pair[side].as_u64() {
                related.insert(number);
            }
        }
    }
    let parked_by_number: HashMap<u64, &Value> = (parked)
        .and_then(|parked| parked["members"].as_array())
        .into_iter()
        .flatten()
        .filter_map(|member| member["number"].as_u64().map(|number| (number, member)))
        .collect();
    let mut batch_of: BTreeMap<u64, Vec<Value>> = BTreeMap::new();
    for batch in batches
        .and_then(|batches| batches["batches"].as_array())
        .into_iter()
        .flatten()
    {
        let tag = json!({"id": batch["id"], "count": batch["count"]});
        for number in batch["members"].as_array().into_iter().flatten() {
            if let Some(number) = number.as_u64() {
                batch_of.entry(number).or_default().push(tag.clone());
            }
        }
    }
    let latest = load_done(root, contract.judge_questions()).unwrap_or_default();

    // The page is ordered by number, not by the captured order.
    let mut ordered: Vec<&Pr> = corpus.iter().collect();
    ordered.sort_by_key(|pr| pr.number);
    let categories = contract.judge_questions()["category"]["criteria"].clone();
    let security_label = contract.display_str("security_label", "Security (meta)");
    let context = RowContext {
        grouped: &grouped,
        related: &related,
        parked: &parked_by_number,
        batch_of: &batch_of,
        latest: &latest,
        categories: &categories,
    };
    let rows: Vec<Value> = ordered
        .into_iter()
        .map(|pr| row(pr, judgments, &context))
        .collect();

    let mut labels: Map<String, Value> = contract
        .display()
        .get("category_labels")
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .map(|(key, label)| (key.clone(), label.clone()))
                .collect()
        })
        .unwrap_or_default();
    for (key, label) in [
        ("security-review", security_label),
        ("unknown", "Unknown".to_owned()),
    ] {
        labels.entry(key.to_owned()).or_insert(json!(label));
    }
    let shipped: Vec<Value> = batches
        .and_then(|batches| batches["batches"].as_array())
        .into_iter()
        .flatten()
        .map(|batch| {
            json!({
                "id": batch["id"], "count": batch["count"], "members": batch["members"],
                "security_members": batch["security_members"],
                "average_risk": batch["average_risk"], "created": batch["created"],
                "review_prompt": batch["review_prompt"],
            })
        })
        .collect();

    json!({
        "repository": contract.repository(),
        "prs": rows,
        "categories": Value::Object(labels),
        "groups": dupes,
        "batches": shipped,
        "batches_available": batches.is_some(),
        "parked": parked.cloned().unwrap_or(Value::Null),
    })
}

/// The lookup tables one row reads, shared across the page.
struct RowContext<'a> {
    grouped: &'a HashSet<u64>,
    related: &'a HashSet<u64>,
    parked: &'a HashMap<u64, &'a Value>,
    batch_of: &'a BTreeMap<u64, Vec<Value>>,
    latest: &'a HashMap<u64, Value>,
    categories: &'a Value,
}

/// One PR, with everything the page filters on.
fn row(pr: &Pr, judgments: &HashMap<u64, Judgment>, context: &RowContext<'_>) -> Value {
    let RowContext {
        grouped,
        related,
        parked,
        batch_of,
        latest,
        categories,
    } = context;
    let judgment = judgments.get(&pr.number);
    let finished = judgment.and_then(|judgment| judgment.metric("finished_form", "score"));
    let security = judgment.map(security_priority).unwrap_or(false);
    json!({
        "number": pr.number,
        "title": pr.title,
        "body": pr.body,
        "body_truncated": pr.body_truncated,
        "author": pr.author,
        "created": pr.created,
        "draft": pr.draft,
        "activity": pr_activity(pr, latest.get(&pr.number)),
        "category": judgment
            .map(|judgment| judgment.category(categories))
            .unwrap_or_else(|| "unknown".to_owned()),
        // The `security-review` key is the meta category: a security-flagged PR
        // is in both its own category and this one.
        "categories": if security {
            json!(["security-review"])
        } else {
            json!([])
        },
        "freshness": judgment
            .map(|judgment| judgment.freshness().to_owned())
            .unwrap_or_else(|| "unjudged or stale".to_owned()),
        "risk": judgment.and_then(|judgment| judgment.metric("risk", "score")),
        "security": judgment.and_then(|judgment| judgment.metric("security_flag", "noul")),
        "security_priority": security,
        "finished": finished,
        "effort": judgment.and_then(|judgment| judgment.metric("review_effort", "score")),
        "is_fix": judgment.and_then(|judgment| judgment.metric("is_fix", "noul")),
        "diffstat": diffstat(pr),
        "candidate": judgment
            .is_some_and(|judgment| review_candidate(pr, judgment, grouped)),
        "senior": judgment.is_some_and(escalated),
        "followup": finished.is_some_and(|value| value <= 1.0) && !grouped.contains(&pr.number),
        "related": related.contains(&pr.number),
        "parked": parked
            .get(&pr.number)
            .and_then(|member| member.get("reasons").cloned())
            .unwrap_or_else(|| json!([])),
        "batches": batch_of.get(&pr.number).cloned().unwrap_or_default(),
    })
}

/// Which PRs sit in a grouping, confirmed or under review.
fn grouped(dupes: &Value) -> HashSet<u64> {
    let mut grouped = HashSet::new();
    for group in dupes["confirmed_groups"].as_array().into_iter().flatten() {
        for number in group.as_array().into_iter().flatten() {
            if let Some(number) = number.as_u64() {
                grouped.insert(number);
            }
        }
    }
    for group in dupes["review_groups"].as_array().into_iter().flatten() {
        for number in group["members"].as_array().into_iter().flatten() {
            if let Some(number) = number.as_u64() {
                grouped.insert(number);
            }
        }
    }
    grouped
}

/// The diffstat the captured list supplied, or that it supplied none.
fn diffstat(pr: &Pr) -> String {
    match (pr.files, pr.additions, pr.deletions) {
        (Some(files), Some(additions), Some(deletions))
            if files >= 0 && additions >= 0 && deletions >= 0 =>
        {
            format!("{files} files changed, +{additions}/-{deletions}")
        }
        _ => "unknown (not supplied by the captured PR list)".to_owned(),
    }
}
