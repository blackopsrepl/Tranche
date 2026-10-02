//! Building the report and the binding that identifies it.
//!
//! The binding is what makes a report a specific observation rather than any
//! report: it digests the sources, the judgments, the pairs and the question
//! policy, so a report that does not match it is refused rather than served.

use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};

use super::groups::{Classification, duplicate_groups, pair_classification};
use super::predicates::{escalated, review_candidate, risk_band, security_priority};
use crate::domain::dupe::p_same;
use crate::domain::judge::{Judgment, usage};
use crate::domain::pr::{Pr, Prs};
use crate::policy::Contract;
use crate::util::digest;

use super::binding::report_binding;

/// The clustering output: the two published files and the summary.
pub struct Clustered {
    pub clusters: Value,
    pub dupes: Value,
    pub summary: Value,
    /// Review candidates per category, for the human report. Sorted by
    /// descending candidate count, the tie broken by name.
    pub tranches: Vec<(String, Vec<Value>)>,
    /// The security meta-category, ranked probability-first.
    pub security_review: Vec<Value>,
    /// Numbers needing escalation, with the title the report prints.
    pub escalate: Vec<(u64, String)>,
    /// Possible author follow-ups, with the title the report prints.
    pub follow_up: Vec<(u64, String)>,
}

/// Build the report from the current judgments and verdicts.
///
/// The deployment's contract supplies the repository, the model and the
/// question policy the report binds itself under.
pub fn cluster(
    prs: &Prs,
    judgments: &HashMap<u64, Judgment>,
    verdicts: &[Value],
    contract: &Contract,
    allow_unbound: bool,
) -> Clustered {
    let repository = contract.repository();
    let model = contract.model();
    let (dupe_groups, review_groups) = duplicate_groups(verdicts);
    let mut in_group: HashSet<u64> = HashSet::new();
    for group in &dupe_groups {
        in_group.extend(group.iter().copied());
    }
    for group in &review_groups {
        if let Some(members) = group.get("members").and_then(Value::as_array) {
            in_group.extend(members.iter().filter_map(Value::as_u64));
        }
    }

    let mut uncertain_pairs: Vec<Value> = verdicts
        .iter()
        .filter(|verdict| {
            matches!(
                pair_classification(verdict),
                Classification::Uncertain
                    | Classification::Contradictory
                    | Classification::Malformed
            )
        })
        .map(|verdict| {
            json!({
                "a": verdict.get("a").cloned().unwrap_or(Value::Null),
                "b": verdict.get("b").cloned().unwrap_or(Value::Null),
                "p_same": p_same(verdict),
                "similarity": verdict.get("similarity").cloned().unwrap_or(Value::Null),
                "verdict": verdict.get("verdict").cloned().unwrap_or(Value::Null),
                "classification": pair_classification(verdict).as_str(),
            })
        })
        .collect();
    // A stable sort: the input is in log order, and a stable sort preserves that
    // order across ties. `dupes.json` is digested, so it is a contract.
    uncertain_pairs.sort_by(|left, right| {
        let score = |value: &Value| value.get("p_same").and_then(Value::as_f64).unwrap_or(-1.0);
        score(right).total_cmp(&score(left))
    });

    // Sorted maps: the published file's key order is not observable through the
    // digest (which sorts keys), but determinism here keeps output stable.
    let mut clusters: BTreeMap<String, BTreeMap<String, Vec<Value>>> = BTreeMap::new();
    let mut tranches: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    let mut security_review: Vec<Value> = Vec::new();
    let mut follow_up: Vec<(u64, String)> = Vec::new();
    let mut escalate: Vec<(u64, String)> = Vec::new();
    let mut category_order: Vec<String> = Vec::new();
    let mut band_order: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut ordered: Vec<&Pr> = prs.iter().collect();
    ordered.sort_by_key(|pr| pr.number);
    for pr in ordered {
        let Some(judgment) = judgments.get(&pr.number) else {
            continue;
        };
        let risk = judgment.metric("risk", "score");
        let item = json!({
            "number": pr.number,
            "title": pr.title,
            "author": pr.author,
            "risk": risk,
            "finished_form": judgment.metric("finished_form", "score"),
            "is_fix": judgment.metric("is_fix", "noul"),
            "review_effort": judgment.metric("review_effort", "score"),
            "security_flag": judgment.metric("security_flag", "noul"),
            "freshness": judgment.freshness(),
            "superseded_by": Value::Null,
            "source_digest": pr.source_digest,
            "evidence_digest": pr.evidence_digest,
            "head_sha": pr.head_sha,
            "url": pr.url,
        });
        let categories = contract.judge_questions()["category"]["criteria"].clone();
        let category = judgment.category(&categories);
        let band = risk_band(risk);
        // The report's category order breaks count ties by first appearance:
        // the sort is stable over an object built in visit order.
        if !category_order.iter().any(|seen| seen == &category) {
            category_order.push(category.clone());
        }
        let bands = band_order.entry(category.clone()).or_default();
        if !bands.iter().any(|seen| seen == band) {
            bands.push(band.to_owned());
        }
        clusters
            .entry(category.clone())
            .or_default()
            .entry(band.to_owned())
            .or_default()
            .push(item.clone());
        if review_candidate(pr, judgment, &in_group) {
            tranches.entry(category).or_default().push(item.clone());
        }
        if security_priority(judgment) {
            security_review.push(item.clone());
        }
        let finished = judgment.metric("finished_form", "score");
        if finished.is_some_and(|value| value <= 1.0) && !in_group.contains(&pr.number) {
            follow_up.push((pr.number, pr.title.clone()));
        }
        if escalated(judgment) {
            escalate.push((pr.number, pr.title.clone()));
        }
    }

    security_review.sort_by(|left, right| {
        let score = |value: &Value| {
            value
                .get("security_flag")
                .and_then(Value::as_f64)
                .unwrap_or(-1.0)
        };
        score(right)
            .total_cmp(&score(left))
            .then_with(|| number(left, "number").cmp(&number(right, "number")))
    });
    let mut clusters_payload: Map<String, Value> = Map::new();
    // The security meta-category is a first-class key, ordered before every
    // category when consumers read clusters.json.
    clusters_payload.insert(
        "security-review".to_owned(),
        Value::Array(security_review.clone()),
    );
    for category in &category_order {
        let Some(stored) = clusters.get(category) else {
            continue;
        };
        let mut bands: Map<String, Value> = Map::new();
        // An object keeps first-appearance order, so the bands of a category
        // are not alphabetical.
        for band in band_order.get(category).into_iter().flatten() {
            if let Some(items) = stored.get(band) {
                bands.insert(band.clone(), Value::Array(items.clone()));
            }
        }
        clusters_payload.insert(category.clone(), Value::Object(bands));
    }
    let clusters_payload = Value::Object(clusters_payload);

    let mut tokens = (0i64, 0i64);
    for record in judgments
        .values()
        .map(|judgment| &judgment.record)
        .chain(verdicts.iter())
    {
        let (input, output) = usage(record);
        tokens.0 += input;
        tokens.1 += output;
    }

    let dupes = json!({
        "confirmed_groups": dupe_groups,
        "review_groups": review_groups,
        "uncertain_pairs": uncertain_pairs,
        "meaning": "Model-consistent candidate groups, not verified duplicates. No survivor selected.",
    });
    // Categories by descending candidate count, the tie broken by name, so the
    // report's section order is deterministic.
    let ready_prs_list: Vec<Value> = tranches.values().flatten().cloned().collect();
    let mut tranche_sections: Vec<(String, Vec<Value>)> = tranches
        .into_iter()
        .filter(|(_, items)| !items.is_empty())
        .collect();
    tranche_sections.sort_by(|left, right| {
        let rank = |name: &str| {
            category_order
                .iter()
                .position(|seen| seen == name)
                .unwrap_or(usize::MAX)
        };
        right
            .1
            .len()
            .cmp(&left.1.len())
            .then_with(|| rank(&left.0).cmp(&rank(&right.0)))
    });
    let unknown = judgments
        .values()
        .filter(|judgment| {
            judgment.metric("risk", "score").is_none()
                || judgment.metric("security_flag", "noul").is_none()
        })
        .count();
    let summary = json!({
        "format_version": 2,
        "repo": repository,
        "prs_in_corpus": prs.len(),
        "judged": judgments.len(),
        "unjudged_or_stale": prs.len() - judgments.len(),
        "unbound_judgments": judgments.values().filter(|j| j.freshness() == "unbound").count(),
        "allow_unbound": allow_unbound,
        "report_binding": report_binding(
            prs,
            judgments,
            verdicts,
            repository,
            model,
            contract.judge_questions(),
            contract.pair_questions(),
        ),
        "dupe_groups": dupes["confirmed_groups"].as_array().map(Vec::len).unwrap_or(0),
        "review_groups": dupes["review_groups"].as_array().map(Vec::len).unwrap_or(0),
        "uncertain_pairs": dupes["uncertain_pairs"].as_array().map(Vec::len).unwrap_or(0),
        "prs_in_dupe_groups": in_group.len(),
        "superseded": 0,
        "ready_tranches": tranche_sections.len(),
        "ready_prs": ready_prs_list.len(),
        "recommendation_kind": "review-candidates",
        "needs_author_followup": follow_up.len(),
        "escalate_review": escalate.len(),
        "security_priority": security_review.len(),
        "unknown_risk_or_security": unknown,
        "tokens": {"input": tokens.0, "output": tokens.1},
        "output_digests": {
            "clusters.json": digest(&clusters_payload),
            "dupes.json": digest(&dupes),
        },
    });

    Clustered {
        clusters: clusters_payload,
        dupes,
        summary,
        tranches: tranche_sections,
        security_review,
        escalate,
        follow_up,
    }
}

/// A numeric field of a record, absent meaning zero.
fn number(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or(0)
}
