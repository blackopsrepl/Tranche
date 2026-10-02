//! The human report reproduces from the stored corpus.
//!
//! The committed file is the report `cluster` writes followed by the plan and
//! park record `batches` appends, so reproducing it means running both stages in
//! order — which is also the only way to catch a trailing-newline difference,
//! because that shifts every appended line by one.

mod support;

use std::collections::BTreeMap;

use serde_json::Value;
use support::{Judgment, Prs, built, contract, corpus, read};
use tranche_core::domain::batch::{
    batch_plan_section, merge_batches, park_section, park_state, parked_payload,
};
use tranche_core::domain::cluster::{Clustered, render};
use tranche_core::util::digest;

/// The batches and the park state for one corpus read.
fn packed(
    report: &Clustered,
    judgments: &std::collections::HashMap<u64, Judgment>,
    prs: &Prs,
) -> (Value, BTreeMap<u64, Vec<String>>) {
    let dupes_digest = digest(&report.dupes);
    let batches =
        merge_batches(&report.dupes, judgments, prs, &dupes_digest, contract()).expect("packs");
    (batches, park_state(&report.dupes, judgments, prs))
}

#[test]
fn the_human_report_reproduces_from_the_same_corpus() {
    let root = corpus();
    let (prs, judgments, report) = built(&root);
    let (batches, parks) = packed(&report, &judgments, &prs);

    let mut rendered = render(&report, contract().repository());
    rendered.push_str(&batch_plan_section(&batches));
    rendered.push_str(&park_section(&parks, &prs));

    let published = std::fs::read_to_string(root.out_dir().join("tranches.md"))
        .expect("the published report reads");
    assert!(
        !published.is_empty(),
        "the committed report is not truncated"
    );
    assert_eq!(rendered, published, "tranches.md reproduces byte for byte");
}

#[test]
fn the_batch_and_park_records_reproduce() {
    let root = corpus();
    let (prs, judgments, report) = built(&root);
    let (batches, parks) = packed(&report, &judgments, &prs);
    let parked = parked_payload(
        &parks,
        &prs,
        &judgments,
        &digest(&report.dupes),
        contract().repository(),
    );

    assert_eq!(digest(&batches), digest(&read(&root, "batches.json")));
    assert_eq!(digest(&parked), digest(&read(&root, "parked.json")));
}

#[test]
fn a_batch_never_claims_a_parked_pr() {
    // The issue-#8 gate. A parked PR is a hold with an unblock path, so letting
    // one into a batch would claim work the pipeline deliberately set aside.
    let root = corpus();
    let (prs, judgments, report) = built(&root);
    let (batches, parks) = packed(&report, &judgments, &prs);

    let mut claimed: Vec<u64> = batches["batches"]
        .as_array()
        .expect("batches")
        .iter()
        .flat_map(|batch| batch["members"].as_array().cloned().unwrap_or_default())
        .filter_map(|number| number.as_u64())
        .collect();
    let overlap: Vec<u64> = parks
        .keys()
        .copied()
        .filter(|number| claimed.contains(number))
        .collect();
    assert!(
        overlap.is_empty(),
        "parked PRs entered a batch: {overlap:?}"
    );

    // Disjoint, and every declared size is the size actually present.
    claimed.sort_unstable();
    let mut unique = claimed.clone();
    unique.dedup();
    assert_eq!(unique.len(), claimed.len(), "a PR appears in two batches");
    assert_eq!(
        claimed.len(),
        batches["batches"]
            .as_array()
            .expect("batches")
            .iter()
            .map(|batch| batch["count"].as_u64().unwrap_or(0) as usize)
            .sum::<usize>(),
        "a batch declares a size it does not have"
    );
}
