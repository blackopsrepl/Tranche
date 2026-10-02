//! The commands that rebuild a report from the corpus and write it out.
//!
//! Each one is a thin shell over `tranche-core`: read the inputs, call the one
//! entry point, publish the artifacts in the order a reader can tolerate. Nothing
//! here decides what a report means.

use std::collections::HashMap;
use std::io::Write;

use tranche_core::domain::batch::{
    batch_plan_section, merge_batches, park_section, park_state, parked_payload,
};
use tranche_core::domain::cluster::{cluster, render};
use tranche_core::domain::dupe::current_pairs;
use tranche_core::domain::judge::{Judgment, current_judgments};
use tranche_core::domain::pr::{Prs, load_prs};
use tranche_core::policy::Contract;
use tranche_core::report::Root;
use tranche_core::util::{atomic_json, digest};

use crate::commands::Outcome;
use crate::report_files::read_json;

/// The deployment contract, or the refusal that explains why there is none.
fn contract_of(root: &Root) -> Result<Contract, Outcome> {
    Contract::load(root.path()).map_err(|error| Outcome::refusal(error, 1))
}

/// Pack the review candidates into pre-release batches and write the park record.
///
/// `batches` refuses to run against a report it did not produce: the recorded
/// digests have to describe the `clusters.json` and `dupes.json` on disk, or the
/// batches would be packed from a different observation than the one published.
pub fn batches(root: &Root, json: bool) -> Outcome {
    let contract = match contract_of(root) {
        Ok(contract) => contract,
        Err(outcome) => return outcome,
    };
    let repository = contract.repository().to_owned();
    let summary = match read_json(&root.summary_path()) {
        Ok(summary) => summary,
        Err(error) => return Outcome::refusal(format!("summary.json: {error}"), 1),
    };
    if summary["format_version"].as_u64() != Some(2)
        || summary["repo"].as_str() != Some(repository.as_str())
    {
        return Outcome::refusal("unrecognized cluster observation; run cluster first", 1);
    }
    let dupes = match read_json(&root.dupes_path()) {
        Ok(dupes) => dupes,
        Err(error) => return Outcome::refusal(format!("dupes.json: {error}"), 1),
    };
    let clusters = match read_json(&root.clusters_path()) {
        Ok(clusters) => clusters,
        Err(error) => return Outcome::refusal(format!("clusters.json: {error}"), 1),
    };
    let recorded = &summary["output_digests"];
    if recorded["clusters.json"].as_str() != Some(digest(&clusters).as_str()) {
        return Outcome::refusal(
            "clusters.json does not match the recorded digest; run cluster first",
            1,
        );
    }
    if recorded["dupes.json"].as_str() != Some(digest(&dupes).as_str()) {
        return Outcome::refusal(
            "dupes.json does not match the recorded digest; run cluster first",
            1,
        );
    }
    let dupes_digest = digest(&dupes);

    let corpus = match load_prs(root, &repository) {
        Ok(corpus) => corpus,
        Err(error) => return Outcome::refusal(format!("corpus: {}", error.0), 1),
    };
    let judgments: HashMap<u64, Judgment> = match current_judgments(
        root,
        &corpus,
        &repository,
        contract.model(),
        contract.judge_questions(),
        false,
    ) {
        Ok(judgments) => judgments,
        Err(error) => return Outcome::refusal(format!("judgments: {error}"), 1),
    };
    let batches = match merge_batches(&dupes, &judgments, &corpus, &dupes_digest, &contract) {
        Ok(batches) => batches,
        Err(error) => return Outcome::refusal(error, 1),
    };
    let parks = park_state(&dupes, &judgments, &corpus);

    let out = root.out_dir();
    if let Err(error) = std::fs::create_dir_all(&out) {
        return Outcome::refusal(format!("cannot create {}: {error}", out.display()), 1);
    }
    if let Err(error) = atomic_json(&root.batches_path(), &batches) {
        return Outcome::refusal(format!("cannot write batches.json: {error}"), 1);
    }
    if let Err(error) = atomic_json(
        &root.parked_path(),
        &parked_payload(&parks, &corpus, &judgments, &dupes_digest, &repository),
    ) {
        return Outcome::refusal(format!("cannot write parked.json: {error}"), 1);
    }
    // Batches append to the report; `cluster` owns the file and wrote the part
    // above. A missing report is skipped rather than invented.
    if let Err(error) = append_plan(root, &batches, &parks, &corpus) {
        return Outcome::refusal(format!("cannot append to tranches.md: {error}"), 1);
    }

    let count = |field: &str| batches[field].as_u64().unwrap_or(0);
    if json {
        return Outcome::success(format!(
            "{}\n",
            tranche_core::util::indented_json(&batches).unwrap_or_default()
        ));
    }
    Outcome::success(format!(
        "{} batches of ≤ {} PRs ({} security-first, {} same-change groups, {} review-group PRs excluded, {} parked)\n\
         wrote {}/batches.json, {}/parked.json\n",
        batches["batches"].as_array().map(Vec::len).unwrap_or(0),
        count("batch_size"),
        count("security_batches"),
        count("same_change_groups"),
        count("excluded_review_prs"),
        count("parked_prs"),
        out.display(),
        out.display(),
    ))
}

/// Append the batch plan and park record to the published report.
fn append_plan(
    root: &Root,
    batches: &serde_json::Value,
    parks: &std::collections::BTreeMap<u64, Vec<String>>,
    corpus: &Prs,
) -> std::io::Result<()> {
    let path = root.tranches_path();
    if !path.exists() {
        return Ok(());
    }
    let mut section = batch_plan_section(batches);
    if !parks.is_empty() {
        section.push_str(&park_section(parks, corpus));
    }
    let mut stream = std::fs::OpenOptions::new().append(true).open(&path)?;
    stream.write_all(section.as_bytes())
}

/// Rebuild the report from the stored corpus and write every artifact `cluster`
/// owns.
///
/// This stage writes four files, and the manifest last, so a reader never
/// sees a report whose binding does not yet exist.
pub fn cluster_report(root: &Root, allow_unbound: bool, json: bool) -> Outcome {
    let contract = match contract_of(root) {
        Ok(contract) => contract,
        Err(outcome) => return outcome,
    };
    let repository = contract.repository().to_owned();
    let corpus = match load_prs(root, &repository) {
        Ok(corpus) => corpus,
        Err(error) => return Outcome::refusal(format!("corpus: {}", error.0), 1),
    };
    let judgments: HashMap<u64, Judgment> = match current_judgments(
        root,
        &corpus,
        &repository,
        contract.model(),
        contract.judge_questions(),
        allow_unbound,
    ) {
        Ok(judgments) => judgments,
        Err(error) => return Outcome::refusal(format!("judgments: {error}"), 1),
    };
    let verdicts = match current_pairs(
        root,
        &corpus,
        &judgments,
        &repository,
        contract.model(),
        contract.pair_questions(),
        allow_unbound,
    ) {
        Ok(verdicts) => verdicts,
        Err(error) => return Outcome::refusal(format!("pairs: {error}"), 1),
    };

    if corpus.is_empty() {
        // Writing here would publish an empty report over whatever is already
        // stored, and `summary.json` is the manifest a reader trusts.
        return Outcome::refusal(
            format!(
                "no captured PR membership under {}; fetch first",
                root.pages_dir().display()
            ),
            1,
        );
    }

    let built = cluster(&corpus, &judgments, &verdicts, &contract, allow_unbound);
    let out = root.out_dir();
    if let Err(error) = std::fs::create_dir_all(&out) {
        return Outcome::refusal(format!("cannot create {}: {error}", out.display()), 1);
    }
    for (name, value) in [
        ("clusters.json", &built.clusters),
        ("dupes.json", &built.dupes),
    ] {
        if let Err(error) = atomic_json(&out.join(name), value) {
            return Outcome::refusal(format!("cannot write {name}: {error}"), 1);
        }
    }
    if let Err(error) = std::fs::write(root.tranches_path(), render(&built, &repository)) {
        return Outcome::refusal(format!("cannot write tranches.md: {error}"), 1);
    }
    if let Err(error) = atomic_json(&root.summary_path(), &built.summary) {
        return Outcome::refusal(format!("cannot write summary.json: {error}"), 1);
    }

    if json {
        let rendered = tranche_core::util::indented_json(&built.summary).unwrap_or_default();
        return Outcome::success(format!("{rendered}\n"));
    }
    Outcome::success(format!(
        "wrote {}/clusters.json dupes.json tranches.md summary.json\n",
        out.display()
    ))
}
