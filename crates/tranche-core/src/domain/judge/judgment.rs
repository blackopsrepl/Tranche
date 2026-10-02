//! The judgment record and the binding that makes one current.
//!
//! `judgment_binding()` is a cache key. A mismatch does not merely look stale — it
//! makes every stored judgment unreusable, so a resume pass silently re-asks the
//! question for the whole corpus and re-bills it.

use serde_json::{Map, Value, json};

use super::super::pr::{Pr, pr_state};
use super::super::questions::metric_ceiling;
use crate::util::digest;

/// The binding version every stored judgment is written under.
pub const BINDING_VERSION: u64 = 1;

#[derive(Debug, Clone)]
pub struct Judgment {
    pub number: u64,
    pub record: Value,
}

impl Judgment {
    pub fn freshness(&self) -> &str {
        self.record
            .get("freshness")
            .and_then(Value::as_str)
            .unwrap_or("current")
    }

    pub fn answers(&self) -> &Map<String, Value> {
        static EMPTY: std::sync::OnceLock<Map<String, Value>> = std::sync::OnceLock::new();
        self.record
            .get("answers")
            .and_then(Value::as_object)
            .unwrap_or_else(|| EMPTY.get_or_init(Map::new))
    }

    /// The chosen category, or `unclear` when the answer is unusable.
    ///
    /// `unclear` is a reserved label: the engine reads the category set from
    /// the deployment's policy, but a judgment whose choice the policy does
    /// not name is always reported as `unclear`, never dropped.
    pub fn category(&self, categories: &Value) -> String {
        let answer = self.answers().get("category").and_then(Value::as_object);
        let chosen = answer
            .and_then(|answer| answer.get("choice"))
            .and_then(Value::as_str);
        match chosen {
            Some(value) if categories.get(value).is_some() => value.to_owned(),
            _ => "unclear".to_owned(),
        }
    }

    /// A metric answer, or `None` when the model gave nothing usable.
    ///
    /// Absent, non-finite or out-of-range values are unknown, never zero.
    pub fn metric(&self, name: &str, field: &str) -> Option<f64> {
        let ceiling = metric_ceiling(name, field)?;
        let answer = self.answers().get(name).and_then(Value::as_object)?;
        let value = answer.get(field)?.as_f64()?;
        if value.is_finite() && (0.0..=ceiling).contains(&value) {
            Some(value)
        } else {
            None
        }
    }
}

/// The binding digest for one PR under today's evidence, policy and model.
///
/// `questions` is the judge-question value exactly as the deployment's policy
/// stores it; the digest covers that value, so the same wording produces the
/// same binding whichever file it came from.
pub fn judgment_binding(pr: &Pr, repository: &str, model: &str, questions: &Value) -> String {
    digest(&json!({
        "version": BINDING_VERSION,
        "repo": repository,
        "source": pr.evidence_digest,
        "state": pr_state(pr),
        "questions": questions,
        "model": model,
    }))
}

/// True when a stored record still answers today's question for today's evidence.
///
/// The binding is the single currency test, and it digests the full question
/// policy — so a judgment made under an older question set is never presented as
/// current. Completeness is deliberately not tested: an answer with an unknown
/// field is still the answer that was given, and dropping it would turn a known
/// category into an unknown one.
pub fn judgment_is_current(record: &Value, binding: &str) -> bool {
    record.get("binding").and_then(Value::as_str) == Some(binding)
}

/// True when every question has a usable answer, so the record can be reused.
pub fn reusable_judgment(judgment: &Judgment) -> bool {
    judgment
        .answers()
        .get("category")
        .and_then(Value::as_object)
        .and_then(|answer| answer.get("choice"))
        .is_some_and(|choice| !choice.is_null())
        && [
            ("risk", "score"),
            ("finished_form", "score"),
            ("review_effort", "score"),
            ("is_fix", "noul"),
            ("security_flag", "noul"),
        ]
        .iter()
        .all(|(name, field)| judgment.metric(name, field).is_some())
}
