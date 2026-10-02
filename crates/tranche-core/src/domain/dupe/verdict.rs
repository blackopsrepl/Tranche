//! Pair verdicts, their binding, and the candidate set.
//!
//! `pair_binding()` is a cache key exactly as the judgment binding is, and the
//! candidate set is what decides which pairs are worth a paid comparison.

use serde_json::{Value, json};

use crate::domain::pr::Pr;
use crate::domain::questions::pair_questions;
use crate::util::digest;

/// Bumped only when the binding's meaning changes.
pub const BINDING_VERSION: u64 = 1;

/// The short state a pair comparison is shown for one side.
pub fn brief(pr: &Pr) -> Value {
    json!({
        "number": pr.number,
        "title": pr.title,
        "body": pr.body.chars().take(400).collect::<String>(),
        "evidence_basis": "shortened descriptions only; source equivalence not verified",
    })
}

/// The binding digest for a pair, under today's evidence, policy and model.
///
/// The pair is ordered by number, so a verdict recorded as (b, a) still matches.
pub fn pair_binding(a: &Pr, b: &Pr, repository: &str, model: &str) -> String {
    let (first, second) = if a.number <= b.number { (a, b) } else { (b, a) };
    digest(&json!({
        "version": BINDING_VERSION,
        "repo": repository,
        "model": model,
        "sources": [first.evidence_digest, second.evidence_digest],
        "state": [brief(first), brief(second)],
        "questions": pair_questions(),
    }))
}

/// The probability of the `same_change` verdict, or `None` when unusable.
pub fn p_same(record: &Value) -> Option<f64> {
    let value = record
        .get("probabilities")
        .and_then(Value::as_object)
        .and_then(|probabilities| probabilities.get("same_change"))
        .and_then(Value::as_f64)?;
    (value.is_finite() && (0.0..=1.0).contains(&value)).then_some(value)
}

/// True when the record carries a verdict that can be reused.
pub fn reusable_pair(record: &Value) -> bool {
    record
        .get("verdict")
        .is_some_and(|verdict| !verdict.is_null())
        && p_same(record).is_some()
}

/// True when a stored verdict still answers today's question for today's evidence.
pub fn pair_is_current(record: &Value, binding: &str) -> bool {
    record.get("binding").and_then(Value::as_str) == Some(binding)
}
