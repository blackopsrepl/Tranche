//! The work a `dupes` pass performs, decided in one place.
//!
//! Defined once and consumed by both `dupes` and `refresh --dry-run`, so the
//! count a dry run reports can never disagree with the work actually done.
//!
//! A stored verdict that is still current is not work. Neither is a pair that has
//! stopped being a candidate — its similarity fell below the threshold, or the
//! body reference that nominated it was removed — even though its verdict is now
//! stale: nothing asks for that verdict, so re-running it would be work that
//! never converges.

use std::collections::{HashMap, HashSet};

use serde_json::Value;
use tranche_core::domain::dupe::{candidate_pairs, pair_cache, reusable_pair};
use tranche_core::domain::pr::{Pr, Prs};
use tranche_core::report::{MODEL, REPOSITORY, Root};

/// PR number → the references it contributes as comparison candidates.
///
/// The index is what lets a pair be nominated by a literal mention rather than by
/// prose resemblance, so it is built from the same projection the corpus uses.
pub fn ref_index(prs: &Prs) -> HashMap<u64, Vec<u64>> {
    prs.iter().map(|pr| (pr.number, pr.refs.clone())).collect()
}

/// The pairs worth asking about, best candidate first.
pub fn outstanding(
    root: &Root,
    prs: &Prs,
    judgments: &HashMap<u64, tranche_core::domain::judge::Judgment>,
) -> Result<Vec<(f64, u64, u64)>, String> {
    let cache = pair_cache(root, prs, REPOSITORY, MODEL)?;
    let done: HashSet<(u64, u64)> = cache
        .iter()
        .filter(|(_, record)| {
            reusable_pair(record) && record["freshness"].as_str() == Some("current")
        })
        .map(|(pair, _)| *pair)
        .collect();
    let candidates = candidate_pairs(prs, judgments, Some(&ref_index(prs)));
    let mut work: Vec<(f64, u64, u64)> = candidates
        .into_iter()
        .filter(|(pair, _)| !done.contains(pair))
        .map(|((a, b), score)| (score, a, b))
        .collect();
    // Best candidate first. `total_cmp` rather than a partial comparison, so an
    // equal score is still an order rather than a panic.
    work.sort_by(|left, right| {
        right
            .0
            .total_cmp(&left.0)
            .then_with(|| left.1.cmp(&right.1))
            .then_with(|| left.2.cmp(&right.2))
    });
    Ok(work)
}

/// The state one side of a comparison is shown as.
///
/// `brief` is the pair-comparison projection: a shortened description, because a
/// verdict is a judgement about what the authors said, not about their patches.
pub fn brief(pr: &Pr) -> Value {
    tranche_core::domain::dupe::brief(pr)
}

/// The judgments to compare under, keyed by PR number.
pub fn judgments(
    root: &Root,
    prs: &Prs,
) -> Result<HashMap<u64, tranche_core::domain::judge::Judgment>, String> {
    tranche_core::domain::judge::current_judgments(root, prs, REPOSITORY, MODEL, false)
}
