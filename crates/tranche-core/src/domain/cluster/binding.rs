//! The binding that identifies one report.
//!
//! This is what makes a report a specific observation rather than any report: it
//! digests the sources, the judgments, the pairs and the question policy, so a
//! stored report that does not match is refused rather than served. The key
//! order is the numeric one, because this map is keyed by PR number as an
//! integer.

use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashMap};

use crate::domain::judge::{BINDING_VERSION, Judgment};
use crate::domain::pr::Prs;

/// The binding over the whole report: sources, judgments, pairs and policy.
///
/// The maps here are keyed by PR number as an *integer*, so their key
/// order is numeric, not lexicographic. The policy enters as the two-question
/// array `[judge, pair]`, exactly as the pipeline has always digested it; the
/// values come from the deployment's contract rather than the compiler.
pub fn report_binding(
    prs: &Prs,
    judgments: &HashMap<u64, Judgment>,
    verdicts: &[Value],
    repository: &str,
    model: &str,
    judge_questions: &Value,
    pair_questions: &Value,
) -> String {
    let sources: Map<String, Value> = prs
        .iter()
        .map(|pr| {
            (
                pr.number.to_string(),
                json!({
                    "source": pr.source_digest,
                    "evidence": pr.evidence_digest,
                    "references": pr.ref_digest,
                }),
            )
        })
        .collect();
    let ordered: BTreeMap<String, &Value> = judgments
        .iter()
        .map(|(number, judgment)| (number.to_string(), &judgment.record))
        .collect();
    crate::util::digest_numeric_keys(&json!({
        "version": BINDING_VERSION,
        "repo": repository,
        "sources": sources,
        "judgments": ordered,
        "pairs": verdicts,
        "questions": [judge_questions, pair_questions],
        "model": model,
    }))
}
