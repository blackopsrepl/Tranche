//! Park decisions: a hold with a named unblock path, never a close.
//!
//! Parking is what keeps a batch's one claim — that these PRs merge together —
//! true. Reasons come only from facts the pipeline already holds: the draft bit,
//! the finished-form score, the judgment's freshness. One predicate serves every
//! consumer (batches, parked.json, the workbench and MCP), because a second copy
//! would let them disagree about which PRs are held.

use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashMap};

use super::super::judge::Judgment;
use super::super::pr::{Pr, Prs};

/// Every park reason, with the action that clears it.
///
/// A parked PR re-enters automatically: the author pushes, the judgment
/// re-binds, the next refresh re-packs. Closing stays a maintainer decision.
pub const PARK_UNBLOCK: [(&str, &str); 4] = [
    (
        "draft",
        "Author marks the pull request ready for review; the next fetch recaptures it and the next batches run re-packs it.",
    ),
    (
        "finished_form",
        "Author adds the missing description or QA evidence; judge --resume re-binds the judgment and the PR re-enters on the next refresh.",
    ),
    (
        "unjudged_or_stale",
        "Run judge --resume (or refresh) to restore a current judgment; the PR re-enters on the next batches run.",
    ),
    (
        "same_change_hold",
        "Held with its same_change group: atomic units are never split, so the group re-enters together when every member clears its own park reason.",
    ),
];

/// The action that clears one reason, for the human report.
pub fn unblock_for(reason: &str) -> &'static str {
    PARK_UNBLOCK
        .iter()
        .find(|(name, _)| *name == reason)
        .map(|(_, text)| *text)
        .unwrap_or("")
}

/// A PR's own park reasons: the draft bit, the finished-form score, freshness.
///
/// A finished form just above 1 is not parked; a missing judgment parks under
/// its own reason rather than being read as a low score.
pub fn park_set(prs: &Prs, judgments: &HashMap<u64, Judgment>) -> BTreeMap<u64, Vec<String>> {
    let mut parks = BTreeMap::new();
    let mut ordered: Vec<&Pr> = prs.iter().collect();
    ordered.sort_by_key(|pr| pr.number);
    for pr in ordered {
        let mut reasons = Vec::new();
        if pr.draft {
            reasons.push("draft".to_owned());
        }
        match judgments.get(&pr.number) {
            None => reasons.push("unjudged_or_stale".to_owned()),
            Some(judgment) => {
                if judgment
                    .metric("finished_form", "score")
                    .is_some_and(|value| value <= 1.0)
                {
                    reasons.push("finished_form".to_owned());
                }
            }
        }
        if !reasons.is_empty() {
            parks.insert(pr.number, reasons);
        }
    }
    parks
}

/// One park predicate for every consumer.
///
/// A PR's own reasons plus `same_change_hold` for every member of a confirmed
/// group that waits for a parked member. Atomic units are never split: pulling
/// one member out would leave the rest of the group claimed twice or held
/// entirely.
pub fn park_state(
    dupes: &Value,
    judgments: &HashMap<u64, Judgment>,
    prs: &Prs,
) -> BTreeMap<u64, Vec<String>> {
    let mut parks = park_set(prs, judgments);
    let groups = dupes
        .get("confirmed_groups")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for group in groups {
        let members: Vec<u64> = group
            .as_array()
            .map(|items| items.iter().filter_map(Value::as_u64).collect())
            .unwrap_or_default();
        if members.iter().any(|member| parks.contains_key(member)) {
            for member in members {
                let entry = parks.entry(member).or_default();
                if !entry.iter().any(|reason| reason == "same_change_hold") {
                    entry.push("same_change_hold".to_owned());
                }
            }
        }
    }
    parks
}

/// The first-class park record.
///
/// Membership never replaces the PR's own category, exactly as the security
/// meta-category does not.
pub fn parked_payload(
    parks: &BTreeMap<u64, Vec<String>>,
    prs: &Prs,
    judgments: &HashMap<u64, Judgment>,
    dupes_digest: &str,
    repository: &str,
) -> Value {
    let members: Vec<Value> = parks
        .iter()
        .map(|(number, reasons)| {
            let pr = prs.get(*number);
            let unblock = reasons
                .iter()
                .map(|reason| unblock_for(reason))
                .collect::<Vec<_>>()
                .join(" ");
            json!({
                "number": number,
                "title": pr.map(|pr| pr.title.clone()).unwrap_or_default(),
                "author": pr.map(|pr| pr.author.clone()).unwrap_or_default(),
                "reasons": reasons,
                "unblock": unblock,
                "head_sha": pr.and_then(|pr| pr.head_sha.clone()),
                "url": pr.map(|pr| pr.url.clone()),
                "created": pr.map(|pr| pr.created.clone()).unwrap_or_default(),
                "security_flag": judgments.get(number).and_then(|j| j.metric("security_flag", "noul")),
            })
        })
        .collect();
    json!({
        "format_version": 1,
        "repo": repository,
        "dupes_digest": dupes_digest,
        "parked": members.len(),
        "meaning": "Parked before batching (issue #8): drafts, PRs without finished form, PRs without a current judgment, and same_change groups holding for a parked member. A hold with a named unblock path, never a close; re-entry is automatic when the reason clears.",
        "members": members,
    })
}

/// Park counts by reason, for the human report.
pub fn park_counts(parks: &BTreeMap<u64, Vec<String>>) -> Map<String, Value> {
    let mut counts = Map::new();
    for reason in [
        "draft",
        "finished_form",
        "unjudged_or_stale",
        "same_change_hold",
    ] {
        let total = parks
            .values()
            .filter(|reasons| reasons.iter().any(|value| value == reason))
            .count();
        counts.insert(reason.to_owned(), json!(total));
    }
    counts
}
