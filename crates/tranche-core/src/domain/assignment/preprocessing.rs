//! Supported independent Noul questions in one set per source.
use crate::{
    report::{MODEL, Root},
    util::digest,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;

/// Full PR evidence digest survives description truncation and binds repository.
pub fn requirement_state(pr: &crate::domain::pr::Pr) -> Value {
    let mut state = crate::domain::pr::pr_state(pr);
    state["source_digest"] = json!(pr.evidence_digest);
    state["repo"] = json!(crate::report::REPOSITORY);
    state
}
pub const QUALIFIED: f64 = 0.8;
pub const NOT_REQUIRED: f64 = 0.2;
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Skill {
    pub description: String,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Taxonomy {
    pub skills: BTreeMap<String, Skill>,
}
impl Taxonomy {
    pub fn parse(text: &str) -> Result<Self, String> {
        let parsed: Self = toml::from_str(text).map_err(|e| format!("skills.toml: {e}"))?;
        if parsed.skills.is_empty()
            || parsed.skills.len() > 64
            || parsed.skills.iter().any(|(id, skill)| {
                id.is_empty()
                    || !id.bytes().all(|c| c.is_ascii_lowercase() || c == b'-')
                    || skill.description.trim().is_empty()
            })
        {
            return Err("skills.toml requires 1..64 named skills with descriptions".into());
        }
        Ok(parsed)
    }
    pub fn load(root: &Root) -> Result<Self, String> {
        Self::parse(
            &std::fs::read_to_string(root.path().join("skills.toml")).map_err(|e| e.to_string())?,
        )
    }
}

pub fn questions(taxonomy: &Taxonomy, qualification: bool) -> Value {
    let main = if qualification {
        "Does resume_text provide concrete production evidence of practical experience with this skill? A declared Skills line alone is insufficient."
    } else {
        "Does reviewing this PR require this skill? Use pr.title, pr.body and pr.diffstat. Judge independently; insufficient detail is uncertainty, not absence."
    };
    let questions: serde_json::Map<String, Value> = taxonomy.skills.iter().map(|(id, s)| (id.clone(),
        json!({"type":"noul", "instructions":{"question":main, "skill":id, "description":s.description}}))).collect();
    Value::Object(questions)
}
pub fn binding(state: &Value, taxonomy: &Taxonomy, qualification: bool) -> String {
    digest(
        &json!({"version":1, "state":state, "questions":questions(taxonomy, qualification),
        "model":MODEL, "qualified":QUALIFIED, "not_required":NOT_REQUIRED}),
    )
}
pub fn probabilities(answer: &Value, taxonomy: &Taxonomy) -> BTreeMap<String, Option<f64>> {
    taxonomy
        .skills
        .keys()
        .map(|id| {
            let p = answer["answers"][id]["noul"]
                .as_f64()
                .filter(|p| p.is_finite() && (0.0..=1.0).contains(p));
            (id.clone(), p)
        })
        .collect()
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub id: String,
    pub binding: String,
    pub answers: Value,
    pub judged_at: String,
}
pub fn records(path: &Path) -> Result<BTreeMap<String, Record>, String> {
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut records = BTreeMap::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let row: Record =
            serde_json::from_str(line).map_err(|e| format!("invalid {}: {e}", path.display()))?;
        records.insert(row.id.clone(), row);
    }
    Ok(records)
}
pub fn current<'a>(
    cache: &'a BTreeMap<String, Record>,
    id: &str,
    state: &Value,
    taxonomy: &Taxonomy,
    qualification: bool,
) -> Option<&'a Record> {
    cache
        .get(id)
        .filter(|r| r.binding == binding(state, taxonomy, qualification))
}
