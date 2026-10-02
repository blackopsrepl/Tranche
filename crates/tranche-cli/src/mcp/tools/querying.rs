//! `query`: security-first PR search with exact filters and pagination.

use serde_json::{Value, json};

use super::super::error::{QueryArguments, ReportError};
use super::View;

impl View {
    pub fn query(&self, arguments: QueryArguments<'_>) -> Result<Value, ReportError> {
        let QueryArguments {
            text,
            category,
            risk_band,
            security,
            finished_form,
            batch,
            queue,
            offset,
            limit,
        } = arguments;
        let members: Option<Vec<u64>> = match batch {
            Some(batch_id) => Some(
                self.find_batch(batch_id)?["members"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_u64)
                    .collect(),
            ),
            None => None,
        };
        let terms: Vec<String> = text
            .to_lowercase()
            .split_whitespace()
            .map(str::to_owned)
            .collect();
        let mut matched: Vec<Value> = Vec::new();
        for row in self.rows() {
            let haystack = format!(
                "#{} @{author} {title} {body}",
                row["number"],
                author = row["author"].as_str().unwrap_or(""),
                title = row["title"].as_str().unwrap_or(""),
                body = row["body"].as_str().unwrap_or(""),
            )
            .to_lowercase();
            if terms.iter().any(|term| !haystack.contains(term)) {
                continue;
            }
            if let Some(category) = category {
                let row_category = row["category"].as_str().unwrap_or("unknown");
                let matches = row_category == category
                    || (category == "security-review"
                        && row["security_priority"].as_bool() == Some(true));
                if !matches {
                    continue;
                }
            }
            if let Some(band) = risk_band
                && row["risk_band"].as_str() != Some(band)
            {
                continue;
            }
            if let Some(security) = security
                && (row["security"].is_null()
                    || row["security_priority"].as_bool() != Some(security))
            {
                continue;
            }
            if let Some(finished) = finished_form
                && row["finished_form"].as_f64() != Some(finished)
            {
                continue;
            }
            if let Some(members) = &members
                && !members.contains(&row["number"].as_u64().unwrap_or(0))
            {
                continue;
            }
            if queue != "all" {
                let field = match queue {
                    "security" => "security_priority",
                    "candidates" => "candidate",
                    "senior" => "senior",
                    "followup" => "followup",
                    "related" => "related",
                    "parked" => "parked",
                    other => return Err(ReportError(format!("unknown queue {other}"))),
                };
                if !row[field].as_bool().unwrap_or(false) {
                    continue;
                }
            }
            matched.push(row);
        }
        // Issue #8: an empty reason array never parks.
        if queue == "parked" {
            matched.retain(|row| {
                row["parked"]
                    .as_array()
                    .is_some_and(|reasons| !reasons.is_empty())
            });
        }
        let total = matched.len() as u64;
        let end = ((offset + limit) as usize).min(matched.len());
        let page: Vec<Value> = matched.get(offset as usize..end).unwrap_or(&[]).to_vec();
        self.envelope(json!({
            "items": page,
            "total": total,
            "offset": offset,
            "next_offset": if (offset + limit) < total { json!(offset + limit) } else { Value::Null },
        }))
    }
}
