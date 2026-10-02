//! `related` and `digests`: dupe evidence, and the identity a reader quotes.

use serde_json::{Map, Value, json};

use super::super::error::ReportError;
use super::View;

impl View {
    pub fn related(&self, number: u64, offset: u64, limit: u64) -> Result<Value, ReportError> {
        if self.prs.get(number).is_none() {
            return Err(ReportError(
                "Expected captured PR number and bounded pagination".to_owned(),
            ));
        }
        let mut items: Vec<Value> = Vec::new();
        for group in self.dupes["confirmed_groups"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if group
                .as_array()
                .is_some_and(|members| members.contains(&json!(number)))
            {
                items.push(json!({"kind": "confirmed_group", "members": group}));
            }
        }
        for group in self.dupes["review_groups"].as_array().into_iter().flatten() {
            if group["members"]
                .as_array()
                .is_some_and(|members| members.contains(&json!(number)))
            {
                let mut item = group.clone();
                item["kind"] = json!("review_group");
                items.push(item);
            }
        }
        for pair in &self.pairs {
            if pair["a"].as_u64() == Some(number) || pair["b"].as_u64() == Some(number) {
                let mut item = pair.clone();
                item["kind"] = json!("pair");
                item["classification"] =
                    json!(tranche_core::domain::cluster::pair_classification(pair).as_str());
                items.push(item);
            }
        }
        let total = items.len() as u64;
        let end = ((offset + limit) as usize).min(items.len());
        let mut page: Vec<Value> = items.get(offset as usize..end).unwrap_or(&[]).to_vec();
        for item in &mut page {
            let members: Vec<u64> = match item.get("members").and_then(Value::as_array) {
                Some(members) => members.iter().filter_map(Value::as_u64).collect(),
                None => vec![
                    item["a"].as_u64().unwrap_or(0),
                    item["b"].as_u64().unwrap_or(0),
                ],
            };
            let sources: Map<String, Value> = members
                .iter()
                .filter_map(|n| {
                    self.prs.get(*n).map(|pr| {
                        (
                            n.to_string(),
                            json!({
                                "number": pr.number,
                                "source_digest": pr.source_digest,
                                "evidence_digest": pr.evidence_digest,
                                "head_sha": pr.head_sha,
                                "url": pr.url,
                            }),
                        )
                    })
                })
                .collect();
            item["sources"] = Value::Object(sources);
        }
        self.envelope(json!({
            "items": page,
            "total": total,
            "offset": offset,
            "next_offset": if (offset + limit) < total { json!(offset + limit) } else { Value::Null },
        }))
    }

    pub fn digests(&self) -> Result<Value, ReportError> {
        self.envelope(json!({}))
    }
}
