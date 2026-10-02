//! The pipeline commands that reach outside the process.
//!
//! Everything here is orchestration over `tranche-core`: the core decides what a
//! page means, this decides when to ask for one and where the result lands.

use std::collections::HashSet;
use std::time::Duration;

use serde_json::Value;
use tranche_core::domain::pr::validate_pr;
use tranche_core::gh::{self, Transport};
use tranche_core::report::Root;
use tranche_core::util::{atomic_json, digest};

/// The API's maximum page size.
const PER_PAGE: usize = 100;

/// Pause between pages. GitHub's guidance is to leave a gap rather than burst.
const BETWEEN_PAGES: Duration = Duration::from_millis(400);

/// The snapshot format the corpus is captured in.
const SNAPSHOT_VERSION: u64 = 1;

/// Refresh the observed open-PR membership.
///
/// One atomic commit at the end. A failed or partial capture returns an error and
/// writes nothing, so the last committed observation stays intact — a half-read
/// membership would make every report built from it quietly short.
pub fn fetch(
    root: &Root,
    transport: Transport,
    report: &mut dyn FnMut(&str),
) -> Result<Value, String> {
    let mut captured: Vec<Value> = Vec::new();
    let mut seen: HashSet<u64> = HashSet::new();
    let contract = tranche_core::policy::Contract::load(root.path())?;
    let repository = contract.repository().to_owned();
    let mut page = 1usize;
    loop {
        let url = format!(
            "{}/repos/{repository}/pulls?state=open&per_page={PER_PAGE}&page={page}",
            gh::API
        );
        let items = gh::page(transport, &url).map_err(|error| error.to_string())?;
        if items.is_empty() {
            break;
        }
        for item in &items {
            validate_pr(item).map_err(|error| error.0)?;
            let number = item["number"].as_u64().unwrap_or(0);
            if !seen.insert(number) {
                // A repeat means pagination is not behaving; continuing would
                // double-count a PR and inflate every count downstream.
                return Err(
                    "invalid or repeated PR during pagination; the previous snapshot is retained"
                        .to_owned(),
                );
            }
        }
        let count = items.len();
        captured.extend(items);
        report(&format!(
            "page {page}: {count} PRs (total {})",
            captured.len()
        ));
        // A short page is the last page, including an exactly-full final one.
        if count < PER_PAGE {
            break;
        }
        page += 1;
        std::thread::sleep(BETWEEN_PAGES);
    }

    let snapshot = serde_json::json!({
        "version": SNAPSHOT_VERSION,
        "repo": repository,
        "items": captured,
        // The digest covers exactly the items, which is what `load_prs` re-checks.
        "digest": digest(&Value::Array(captured.clone())),
        "observed_at": observed_at(),
    });
    let path = root.snapshot_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    atomic_json(&path, &snapshot).map_err(|error| format!("cannot write the snapshot: {error}"))?;
    report(&format!(
        "fetched {} observed open PRs into {}",
        captured.len(),
        path.display()
    ));
    Ok(snapshot)
}

/// The observation time, in the ISO-8601 form the snapshot records.
///
/// Not part of any digest: `load_prs` re-checks the version, the repository and
/// the digest over the items, and reads this only to report when it looked.
fn observed_at() -> String {
    let now = time::OffsetDateTime::now_utc();
    now.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| String::new())
}
