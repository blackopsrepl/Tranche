//! The refusal type for unusable captured input, and the loader that raises it.
//!
//! Validation is not decoration: a PR missing an identity would otherwise enter
//! every downstream digest as a hole, so it is refused at the boundary.

use serde_json::Value;

use super::invalidating::invalid;
use super::projection::project;
use super::record::Prs;
use super::validating::validate_pr;
use crate::report::Root;
use crate::util::digest;

/// A captured PR is unusable, so nothing downstream may treat it as data.
#[derive(Debug)]
pub struct PrError(pub String);
impl std::fmt::Display for PrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for PrError {}
/// Read the captured membership: one snapshot, or the sorted page shards.
pub fn load_prs(root: &Root, repository: &str) -> Result<Prs, PrError> {
    let pages = membership_pages(root, repository)?;
    let mut prs = Prs::default();
    for page in pages {
        let items = page
            .as_array()
            .ok_or_else(|| PrError("captured PR membership must be a list; fetch again".into()))?;
        for item in items {
            validate_pr(item)?;
            let number = item["number"]
                .as_u64()
                .expect("validated as a positive int");
            if prs.by_number.contains_key(&number) {
                return Err(PrError(
                    "repeated PR in captured membership; fetch again".into(),
                ));
            }
            prs.insert(project(item, repository));
        }
    }
    Ok(prs)
}
pub(crate) fn membership_pages(root: &Root, repository: &str) -> Result<Vec<Value>, PrError> {
    let snapshot = root.snapshot_path();
    if snapshot.exists() {
        let text = std::fs::read_to_string(&snapshot).map_err(invalid)?;
        let value: Value = serde_json::from_str(&text).map_err(invalid)?;
        let version = value.get("version").and_then(Value::as_u64);
        let repo = value.get("repo").and_then(Value::as_str);
        let items = value.get("items").cloned().unwrap_or(Value::Null);
        let recorded = value.get("digest").and_then(Value::as_str);
        if version != Some(1) || repo != Some(repository) || recorded != Some(&digest(&items)) {
            return Err(PrError(
                "fetched snapshot identity or checksum differs; fetch again".into(),
            ));
        }
        return Ok(vec![items]);
    }
    // Existing page caches stay readable until the next fetch.
    let mut pages = Vec::new();
    for path in crate::report::read_page_paths(&root.pages_dir()) {
        let text = std::fs::read_to_string(&path).map_err(invalid)?;
        let page: Value = serde_json::from_str(&text).map_err(invalid)?;
        if let Some(items) = page.as_array() {
            for item in items {
                super::validating::validate_repository(item, repository)?;
            }
        }
        pages.push(page);
    }
    Ok(pages)
}
