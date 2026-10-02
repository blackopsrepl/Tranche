//! Rendering the workbench from the bound reports.
//!
//! Every gate that decides whether the page renders at all lives here, because a
//! reader seeing numbers is the thing they can no longer un-see. The arrangement of
//! what the report already says is in `payload`; this file decides whether to.

use std::collections::HashMap;

use tranche_core::domain::batch::{park_state, parked_payload};
use tranche_core::domain::cluster::report_binding;
use tranche_core::domain::dupe::current_pairs;
use tranche_core::domain::judge::{Judgment, current_judgments};
use tranche_core::domain::pr::{Prs, load_prs};
use tranche_core::report::{MODEL, REPOSITORY, Root};
use tranche_core::util::digest;

use crate::commands::Outcome;
use crate::report_files::read_json;

/// Render the workbench from the bound reports.
pub fn page(root: &Root, report: &mut dyn FnMut(&str)) -> Outcome {
    let corpus: Prs = match load_prs(root, REPOSITORY) {
        Ok(corpus) => corpus,
        Err(error) => return Outcome::refusal(format!("corpus: {}", error.0), 1),
    };
    let summary = match read_json(&root.summary_path()) {
        Ok(summary) => summary,
        Err(error) => return Outcome::refusal(format!("summary.json: {error}"), 1),
    };
    let allow_unbound = summary["allow_unbound"].as_bool() == Some(true);
    let judgments: HashMap<u64, Judgment> =
        match current_judgments(root, &corpus, REPOSITORY, MODEL, allow_unbound) {
            Ok(judgments) => judgments,
            Err(error) => return Outcome::refusal(format!("judgments: {error}"), 1),
        };
    let verdicts = match current_pairs(root, &corpus, &judgments, REPOSITORY, MODEL, allow_unbound)
    {
        Ok(verdicts) => verdicts,
        Err(error) => return Outcome::refusal(format!("pairs: {error}"), 1),
    };
    let clusters = match read_json(&root.clusters_path()) {
        Ok(clusters) => clusters,
        Err(error) => return Outcome::refusal(format!("clusters.json: {error}"), 1),
    };
    let dupes = match read_json(&root.dupes_path()) {
        Ok(dupes) => dupes,
        Err(error) => return Outcome::refusal(format!("dupes.json: {error}"), 1),
    };

    // The report has to be the one these judgments produce.
    let binding = report_binding(&corpus, &judgments, &verdicts, REPOSITORY, MODEL);
    let recorded = &summary["output_digests"];
    if summary["format_version"].as_u64() != Some(2)
        || summary["repo"].as_str() != Some(REPOSITORY)
        || summary["report_binding"].as_str() != Some(binding.as_str())
        || recorded["clusters.json"].as_str() != Some(digest(&clusters).as_str())
        || recorded["dupes.json"].as_str() != Some(digest(&dupes).as_str())
    {
        return Outcome::refusal(
            "report inputs changed or are legacy/mixed; rerun cluster before rendering",
            1,
        );
    }

    // Batches and the park record ship with the workbench when present, and a
    // stale one must never be shown against a newer dupe run.
    let batches = match read_json(&root.batches_path()) {
        Ok(batches) => {
            if batches["format_version"].as_u64() != Some(3)
                || batches["repo"].as_str() != Some(REPOSITORY)
                || batches["dupes_digest"].as_str() != recorded["dupes.json"].as_str()
            {
                return Outcome::refusal(
                    "batches.json is stale or foreign; run `tranche batches` before rendering",
                    1,
                );
            }
            Some(batches)
        }
        Err(_) => None,
    };
    let parks = park_state(&dupes, &judgments, &corpus);
    let parked = match read_json(&root.parked_path()) {
        Ok(parked) => {
            let expected = parked_payload(
                &parks,
                &corpus,
                &judgments,
                recorded["dupes.json"].as_str().unwrap_or(""),
                REPOSITORY,
            );
            if parked != expected {
                return Outcome::refusal(
                    "parked.json is stale, foreign or modified; run `tranche batches` before rendering",
                    1,
                );
            }
            Some(parked)
        }
        Err(_) => None,
    };
    let parked_prs = batches
        .as_ref()
        .and_then(|batches| batches["parked_prs"].as_u64())
        .unwrap_or(0);
    if parked_prs > 0 && parked.is_none() {
        return Outcome::refusal(
            "batches.json parked PRs but parked.json is missing; run `tranche batches`",
            1,
        );
    }

    let built = super::payload::payload(
        &corpus,
        &judgments,
        &dupes,
        batches.as_ref(),
        parked.as_ref(),
        root,
    );
    match super::writing::write(root, built) {
        Ok((html, json_bytes)) => {
            report(&format!(
                "wrote {}/index.html ({} KB) and {}/data/workbench.json ({} KB)",
                root.docs_dir().display(),
                html / 1024,
                root.docs_dir().display(),
                json_bytes / 1024
            ));
            Outcome::success(String::new())
        }
        Err(error) => Outcome::refusal(error, 1),
    }
}
