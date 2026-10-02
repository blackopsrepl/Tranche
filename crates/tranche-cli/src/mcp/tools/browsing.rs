//! `pick` and `next_prompt`: batches as the reviewer consumes them.

use std::collections::HashMap;

use serde_json::{Map, Value, json};
use tranche_core::domain::cluster::{escalated, review_candidate, security_priority};
use tranche_core::domain::judge::Judgment;
use tranche_core::report::REPOSITORY;

use super::super::error::{QueryArguments, ReportError};
use super::View;

impl View {
    pub(super) fn find_batch(&self, batch_id: &str) -> Result<&Value, ReportError> {
        if !batch_id.starts_with('B')
            || batch_id.len() < 4
            || batch_id.len() > 7
            || !batch_id[1..].chars().all(|c| c.is_ascii_digit())
        {
            return Err(ReportError(
                "Expected exact batch id such as B001".to_owned(),
            ));
        }
        self.batches
            .as_ref()
            .and_then(|batches| batches["batches"].as_array())
            .into_iter()
            .flatten()
            .find(|batch| batch["id"].as_str() == Some(batch_id))
            .ok_or_else(|| ReportError("Unknown batch id".to_owned()))
    }

    pub fn pick(&self, batch_id: &str) -> Result<Value, ReportError> {
        let batch = self.find_batch(batch_id)?;
        let rows: HashMap<u64, Value> = self
            .rows()
            .into_iter()
            .filter_map(|row| row["number"].as_u64().map(|n| (n, row)))
            .collect();
        let members: Vec<Value> = batch["members"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_u64)
            .filter_map(|number| rows.get(&number).cloned())
            .collect();
        let activity: Map<String, Value> = batch["members"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_u64)
            .map(|number| (number.to_string(), self.activity(number)))
            .collect();
        self.envelope(json!({
            "batch": batch,
            "prs": members,
            "activity": Value::Object(activity),
        }))
    }

    pub fn next_prompt(&self, after: Option<&str>) -> Result<Value, ReportError> {
        let Some(batches) = self.batches.as_ref() else {
            return Err(ReportError(
                "batches.json is unavailable; run `tranche batches`".to_owned(),
            ));
        };
        let all = batches["batches"].as_array().cloned().unwrap_or_default();
        let after_ordinal: u64 = match after {
            Some(text) if text.starts_with('B') => {
                self.find_batch(text)?["ordinal"].as_u64().unwrap_or(0)
            }
            Some(text) => text.parse().map_err(|_| {
                ReportError("after must be an existing batch id or ordinal (0 starts)".to_owned())
            })?,
            None => 0,
        };
        if after_ordinal > all.len() as u64 {
            return Err(ReportError(
                "after must be an existing batch id or ordinal (0 starts)".to_owned(),
            ));
        }
        let batch = all
            .iter()
            .find(|batch| batch["ordinal"].as_u64().unwrap_or(0) > after_ordinal);
        let activity: Map<String, Value> = batch
            .and_then(|batch| batch["members"].as_array())
            .into_iter()
            .flatten()
            .filter_map(Value::as_u64)
            .map(|number| (number.to_string(), self.activity(number)))
            .collect();
        self.envelope(json!({
            "batch": batch.cloned().unwrap_or(Value::Null),
            "activity": Value::Object(activity),
        }))
    }
}
