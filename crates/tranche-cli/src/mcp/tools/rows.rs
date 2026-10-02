//! The row projection every tool reads, and the envelope every answer travels in.
//!
//! One row per captured PR: judgment metrics flattened, queues computed, sorted
//! security-first then by risk. The envelope carries the report's identity and the
//! disclaimer, and enforces the 1 MiB response bound.

use std::collections::HashMap;

use serde_json::{Value, json};
use tranche_core::domain::cluster::{escalated, review_candidate, security_priority};
use tranche_core::domain::judge::Judgment;
use tranche_core::domain::pr::Prs;
use tranche_core::report::REPOSITORY;

use super::super::error::ReportError;

/// The tools' own response bound, as the reference server advertised.
pub const MAX_RESULT_BYTES: usize = 1024 * 1024;

/// What every response carries before the tool's own data.
pub const DISCLAIMER: &str = "Model suggestions from titles and shortened descriptions, not merge/close \
approval. Patches, CI, reproductions and security have not been verified.";

/// The tools' view of one loaded report.
///
/// It owns its data: a long-lived server loads fresh per request and drops the old
/// view whole, so memory tracks the report rather than the session.
pub struct View {
    pub dupes: Value,
    pub batches: Option<Value>,
    pub parked: Option<Value>,
    pub prs: Prs,
    pub judgments: HashMap<u64, Judgment>,
    pub pairs: Vec<Value>,
    pub latest_judgments: HashMap<u64, Value>,
    pub identity: Value,
}

impl View {
    pub(super) fn envelope(&self, data: Value) -> Result<Value, ReportError> {
        let mut result = json!({
            "repo": REPOSITORY,
            "disclaimer": DISCLAIMER,
            "digests": self.identity,
        });
        if let (Some(object), Some(extra)) = (result.as_object_mut(), data.as_object()) {
            for (key, value) in extra {
                object.insert(key.clone(), value.clone());
            }
        }
        let text = serde_json::to_string(&result)
            .map_err(|error| ReportError(format!("a tool answer cannot be serialized: {error}")))?;
        if text.len() > MAX_RESULT_BYTES {
            return Err(ReportError(
                "Response exceeds byte limit; narrow filters or lower limit".to_owned(),
            ));
        }
        Ok(result)
    }

    pub(super) fn activity(&self, number: u64) -> Value {
        self.prs.get(number).map_or_else(
            || Value::Null,
            |pr| tranche_core::domain::batch::pr_activity(pr, self.latest_judgments.get(&number)),
        )
    }

    /// Every PR as a searchable row, sorted security-first then by risk.
    pub(super) fn rows(&self) -> Vec<Value> {
        let grouped: std::collections::HashSet<u64> = self.dupes["confirmed_groups"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|group| group.as_array().into_iter().flatten())
            .filter_map(Value::as_u64)
            .chain(
                self.dupes["review_groups"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .flat_map(|group| group["members"].as_array().into_iter().flatten())
                    .filter_map(Value::as_u64),
            )
            .collect();
        let related: std::collections::HashSet<u64> = grouped
            .iter()
            .copied()
            .chain(
                self.dupes["uncertain_pairs"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .flat_map(|pair| ["a", "b"].iter().filter_map(|side| pair[*side].as_u64())),
            )
            .collect();
        let parked: HashMap<u64, &Value> = self
            .parked
            .as_ref()
            .and_then(|parked| parked["members"].as_array())
            .into_iter()
            .flatten()
            .filter_map(|member| member["number"].as_u64().map(|n| (n, member)))
            .collect();

        let mut rows: Vec<Value> = self
            .prs
            .iter()
            .map(|pr| {
                let judgment = self.judgments.get(&pr.number);
                let risk = judgment.and_then(|j| j.metric("risk", "score"));
                let finished = judgment.and_then(|j| j.metric("finished_form", "score"));
                let security = judgment.and_then(|j| j.metric("security_flag", "noul"));
                let priority = judgment.is_some_and(security_priority);
                let risk_band = match risk {
                    None => "unknown",
                    Some(risk) if risk <= 1.5 => "low",
                    Some(risk) if risk <= 2.5 => "core",
                    Some(_) => "danger",
                };
                json!({
                    "number": pr.number,
                    "title": pr.title,
                    "body": pr.body,
                    "author": pr.author,
                    "created": pr.created,
                    "url": pr.url,
                    "head_sha": pr.head_sha,
                    "draft": pr.draft,
                    "answers": judgment.map(|j| j.answers().clone()).unwrap_or_default(),
                    "category": judgment
                        .map(|j| j.category())
                        .unwrap_or_else(|| "unknown".to_owned()),
                    "risk": risk,
                    "finished_form": finished,
                    "security": security,
                    "security_priority": priority,
                    "freshness": judgment.map(|j| j.freshness().to_owned())
                        .unwrap_or_else(|| "unjudged".to_owned()),
                    "judgment_binding": judgment
                        .and_then(|j| j.record.get("binding").cloned())
                        .unwrap_or(Value::Null),
                    "risk_band": risk_band,
                    "candidate": judgment.is_some_and(|j| review_candidate(pr, j, &grouped)),
                    "senior": judgment.is_some_and(escalated),
                    "followup": finished.is_some_and(|value| value <= 1.0)
                        && !grouped.contains(&pr.number),
                    "related": related.contains(&pr.number),
                    "parked": parked
                        .get(&pr.number)
                        .and_then(|member| member.get("reasons").cloned())
                        .unwrap_or_else(|| json!([])),
                    "activity": self.activity(pr.number),
                })
            })
            .collect();
        rows.sort_by(|a, b| {
            let key = |row: &Value| {
                (
                    !row["security_priority"].as_bool().unwrap_or(false),
                    row["risk"].as_f64().unwrap_or(99.0).to_bits(),
                    row["created"].as_str().unwrap_or("").to_owned(),
                    row["number"].as_u64().unwrap_or(0),
                )
            };
            key(a).cmp(&key(b))
        });
        rows
    }

    pub fn judge_categories(&self) -> Vec<String> {
        tranche_core::domain::questions::judge_questions()
            .get("category")
            .and_then(|category| category.get("criteria"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|criterion| criterion.get("name").and_then(Value::as_str))
            .map(str::to_owned)
            .collect()
    }
}
