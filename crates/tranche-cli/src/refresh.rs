//! The documented one-command pipeline.
//!
//! Runs the same steps in the same order every time and reuses everything the
//! caches can still support, so the work is proportional to what actually changed
//! since the last pass rather than to the size of the corpus.
//!
//! `refresh` is also where a dry run reports: the plan, what is already judged,
//! how many pair comparisons are outstanding, and — because fetching is the one
//! step that touches the network — that it would re-read GitHub. A dry run writes
//! nothing.

use serde_json::Value;
use tranche_core::domain::dupe::candidate_pairs;
use tranche_core::domain::pr::{Prs, load_prs};
use tranche_core::report::{REPOSITORY, Root};

use crate::commands::Outcome;
use crate::pairing::{judgments, outstanding, ref_index};
use crate::report_files::read_json;
use crate::reports::{batches as build_batches, cluster_report};

/// The steps, in the order they always run.
const STEPS: [&str; 6] = ["fetch", "judge", "dupes", "cluster", "batches", "page"];

/// The counts a refresh reports before and after, with the change marked.
const ACCOUNTED: [&str; 8] = [
    "prs_in_corpus",
    "judged",
    "dupe_groups",
    "review_groups",
    "uncertain_pairs",
    "ready_prs",
    "escalate_review",
    "security_priority",
];

/// Run the pipeline: fetch, judge, dupes, cluster, batches and the page.
pub fn refresh(
    root: &Root,
    max_pairs: u64,
    no_page: bool,
    dry_run: bool,
    report: &mut dyn FnMut(&str),
) -> Outcome {
    let steps: Vec<&str> = STEPS
        .iter()
        .copied()
        .filter(|step| !(*step == "page" && no_page))
        .collect();

    if dry_run {
        return plan(root, &steps, report);
    }

    let before = read_json(&root.summary_path()).ok();
    let before_batches = read_json(&root.batches_path()).ok();

    report(&format!("refresh: {}", steps.join(" -> ")));
    for step in &steps {
        let mut say = |line: &str| report(line);
        let outcome = match *step {
            "fetch" => {
                crate::pipeline::fetch(root, tranche_core::gh::Transport::Gh, &mut say).map(|_| ())
            }
            "judge" => crate::judgment::judge(root, true, None, &mut say).map(|_| ()),
            "dupes" => crate::dupes::dupes(root, max_pairs, &mut say).map(|_| ()),
            "cluster" => outcome_of(cluster_report(root, false, false)),
            "batches" => outcome_of(build_batches(root, false)),
            "page" => outcome_of(crate::workbench::page(root, false, false, &mut say)),
            other => Err(format!("unknown step {other}")),
        };
        if let Err(error) = outcome {
            return Outcome::refusal(format!("refresh stopped at {step}: {error}"), 1);
        }
    }

    report("");
    report("refresh complete");
    let after = read_json(&root.summary_path()).ok();
    let after_batches = read_json(&root.batches_path()).ok();
    for key in ACCOUNTED {
        report(&changed(key, before.as_ref(), after.as_ref()));
    }
    report(&changed(
        "parked_prs",
        before_batches.as_ref(),
        after_batches.as_ref(),
    ));
    Outcome::success(String::new())
}

/// What a refresh would do, writing nothing.
fn plan(root: &Root, steps: &[&str], report: &mut dyn FnMut(&str)) -> Outcome {
    let corpus: Prs = match load_prs(root, REPOSITORY) {
        Ok(corpus) => corpus,
        Err(error) => return Outcome::refusal(format!("corpus: {}", error.0), 1),
    };
    let judgments = match judgments(root, &corpus) {
        Ok(judgments) => judgments,
        Err(error) => return Outcome::refusal(format!("judgments: {error}"), 1),
    };
    let pending = match outstanding(root, &corpus, &judgments) {
        Ok(pending) => pending,
        Err(error) => return Outcome::refusal(format!("pairs: {error}"), 1),
    };
    // The denominator is the candidate set before the current verdicts are
    // discounted, so "n of m" reads as work remaining out of all candidates.
    let candidates = candidate_pairs(&corpus, &judgments, Some(&ref_index(&corpus))).len();

    report(&format!(
        "refresh plan ({}), no changes written:",
        steps.join(" -> ")
    ));
    report(&format!(
        "  judged now          {}/{} captured PRs",
        judgments.len(),
        corpus.len()
    ));
    report(&format!(
        "  pair verdicts to re-run  {} of {candidates} candidates",
        pending.len()
    ));
    if steps.contains(&"fetch") {
        report("  fetch               would re-read open-PR membership from GitHub");
    }
    Outcome::success(String::new())
}

/// One accounted count, with what it was when it moved.
fn changed(key: &str, before: Option<&Value>, after: Option<&Value>) -> String {
    let new = after.and_then(|value| value.get(key));
    let old = before.and_then(|value| value.get(key));
    let marker = if old == new || old.is_none() {
        String::new()
    } else {
        format!("   (was {})", old.map(Value::to_string).unwrap_or_default())
    };
    format!(
        "  {key:<22} {}{marker}",
        new.map(Value::to_string).unwrap_or_else(|| "—".to_owned())
    )
}

/// A step that reports its own progress through `Outcome`.
fn outcome_of(outcome: Outcome) -> Result<(), String> {
    if outcome.code == 0 {
        Ok(())
    } else {
        Err(outcome.stderr.trim().to_owned())
    }
}
