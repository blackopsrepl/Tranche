//! Resume input is private. Public projections contain synthetic IDs, never text or paths.
use super::{
    Member,
    preprocessing::{QUALIFIED, Taxonomy, current, probabilities, records},
};
use crate::report::Root;
use serde_json::{Value, json};
use std::path::PathBuf;

pub struct Resume {
    pub id: String,
    pub text: String,
    pub capacity: usize,
}
impl Resume {
    pub fn state(&self) -> Value {
        json!({"resume_text":self.text})
    }
}
pub fn resume_paths(root: &Root) -> Result<Vec<PathBuf>, String> {
    let dir = root.path().join("input/team");
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(&dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.file_type().map_err(|e| e.to_string())?.is_file()
            && matches!(
                entry.path().extension().and_then(|s| s.to_str()),
                Some("md" | "txt")
            )
        {
            paths.push(entry.path());
        }
    }
    paths.sort();
    Ok(paths)
}
pub fn resumes(root: &Root) -> Result<Vec<Resume>, String> {
    let mut resumes = Vec::new();
    let mut ids = std::collections::HashSet::new();
    for path in resume_paths(root)? {
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or("invalid resume filename")?;
        if stem.is_empty()
            || !stem
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        {
            return Err("resume filenames must be lowercase ASCII identifiers".into());
        }
        let id = format!("synthetic-{stem}");
        if !ids.insert(id.clone()) {
            return Err(format!("duplicate resume ID: {id}"));
        }
        let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        if text.len() > 65536 {
            return Err("resume exceeds 64 KiB".into());
        }
        let caps: Vec<_> = text
            .lines()
            .filter_map(|l| l.trim().strip_prefix("Capacity:"))
            .collect();
        if caps.len() != 1 {
            return Err(format!("{id}: exactly one Capacity: line required"));
        }
        let capacity: usize = caps[0]
            .trim()
            .parse()
            .map_err(|_| format!("{id}: invalid capacity"))?;
        if capacity > 10000 {
            return Err(format!("{id}: capacity exceeds 10000"));
        }
        resumes.push(Resume { id, text, capacity });
    }
    Ok(resumes)
}
pub fn members(root: &Root, taxonomy: &Taxonomy) -> Result<Vec<Member>, String> {
    let cache = records(&root.out_dir().join("qualifications.jsonl"))?;
    let mut members = Vec::new();
    for resume in resumes(root)? {
        let record = current(&cache, &resume.id, &resume.state(), taxonomy, true);
        let probs = record.map(|r| probabilities(&r.answers, taxonomy));
        let skills = probs
            .as_ref()
            .map(|p| {
                p.iter()
                    .filter(|(_, p)| p.is_some_and(|p| p >= QUALIFIED))
                    .map(|(id, _)| id.clone())
                    .collect()
            })
            .unwrap_or_default();
        let mut member = Member::new(&resume.id, &resume.id, skills, resume.capacity, "");
        member.evidence_known =
            record.is_some() && probs.is_some_and(|p| p.values().all(Option::is_some));
        member.index = members.len();
        members.push(member);
    }
    Ok(members)
}
