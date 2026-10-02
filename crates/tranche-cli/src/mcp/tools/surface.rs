//! `surface`: coverage, queues, category counts and the batch overview.

use serde_json::{Map, Value, json};

use super::super::error::ReportError;
use super::View;

impl View {
    pub fn surface(&self) -> Result<Value, ReportError> {
        let overview: Vec<Value> = self
            .batches
            .as_ref()
            .and_then(|batches| batches["batches"].as_array())
            .into_iter()
            .flatten()
            .map(|batch| {
                json!({
                    "id": batch["id"], "ordinal": batch["ordinal"], "count": batch["count"],
                    "security_members": batch["security_members"],
                    "average_risk": batch["average_risk"], "created": batch["created"],
                })
            })
            .collect();
        let rows = self.rows();
        let mut category_counts: Map<String, Value> = Map::new();
        for row in &rows {
            let key = row["category"].as_str().unwrap_or("unknown").to_owned();
            let next = category_counts
                .get(&key)
                .and_then(Value::as_u64)
                .unwrap_or(0)
                + 1;
            category_counts.insert(key, json!(next));
        }
        let mut queues = Map::new();
        for (queue, field) in [
            ("all", ""),
            ("security", "security_priority"),
            ("candidates", "candidate"),
            ("related", "related"),
            ("assigned", "assigned"),
            ("senior", "senior"),
            ("followup", "followup"),
        ] {
            let members: Vec<Value> = rows
                .iter()
                .filter(|row| field.is_empty() || row[field].as_bool().unwrap_or(false))
                .map(|row| row["number"].clone())
                .collect();
            queues.insert(
                queue.to_owned(),
                json!({"count": members.len(), "members": members}),
            );
        }
        let parked_members: Vec<Value> = rows
            .iter()
            .filter(|row| {
                row["parked"]
                    .as_array()
                    .is_some_and(|reasons| !reasons.is_empty())
            })
            .map(|row| row["number"].clone())
            .collect();
        let mut parked_block = json!({"count": parked_members.len(), "members": parked_members});
        if let Some(parked) = self.parked.as_ref()
            && let Some(object) = parked_block.as_object_mut()
        {
            let unblock: Map<String, Value> = parked["members"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|member| {
                    member["number"]
                        .as_u64()
                        .map(|n| (n.to_string(), member["unblock"].clone()))
                })
                .collect();
            object.insert("unblock".to_owned(), Value::Object(unblock));
        }

        // The activity summary distinguishes evidenced idleness from creation age:
        // only a judgment's freshness says the head was still observed.
        let now = time::OffsetDateTime::now_utc();
        let mut durations: Vec<i64> = Vec::new();
        for pr in self.prs.iter() {
            let item = self.activity(pr.number);
            if item["idle_basis"].as_str() != Some("judgment") {
                continue;
            }
            if let Some(idle_since) = item["idle_since"].as_str()
                && let Ok(parsed) = time::OffsetDateTime::parse(
                    idle_since,
                    &time::format_description::well_known::Rfc3339,
                )
            {
                durations.push((now - parsed).whole_days().max(0));
            }
        }
        let count = |threshold: i64| durations.iter().filter(|days| **days >= threshold).count();
        let activity_counts = json!({
            "head_moved": self.prs.iter().filter(|pr| {
                self.activity(pr.number)["head_moved"].as_bool() == Some(true)
            }).count(),
            "idle_since_known": durations.len(),
            "idle_7d": count(7),
            "idle_30d": count(30),
            "idle_60d": count(60),
            "as_of": tranche_core::evidence::now(),
        });
        let coverage = json!({
            "prs_in_corpus": self.prs.len(),
            "judged": self.judgments.len(),
            "unjudged": self.prs.len() - self.judgments.len(),
        });
        self.envelope(json!({
            "summary": coverage,
            "activity": activity_counts,
            "batches_available": self.batches.is_some(),
            "category_counts": Value::Object(category_counts),
            "queues": Value::Object(queues),
            "parked": parked_block,
            "assignments": self.assignments,
            "batches": overview,
            "filters": {
                "categories": [{"security-review": true}, self.judge_categories(), {"unknown": true}],
                "risk_bands": ["low", "core", "danger", "unknown"],
                "queues": ["security", "all", "candidates", "senior", "followup", "parked", "related", "assigned"],
            },
        }))
    }
}
