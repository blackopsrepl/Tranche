//! Clustering and batching, checked against the committed report.
//!
//! Rebuilding the committed `out/` files and comparing digests is the check that
//! matters: the report gate compares these same digests, so a mismatch would make
//! every report read as modified and every stored judgment foreign.

use tranche_core::domain::batch::{merge_batches, park_state, parked_payload};
use tranche_core::domain::cluster::cluster;
use tranche_core::domain::dupe::current_pairs;
use tranche_core::domain::judge::current_judgments;
use tranche_core::domain::pr::load_prs;
use tranche_core::report::{REPOSITORY, Root};
use tranche_core::util::digest;

const MODEL: &str = "jev-latest";
const CLUSTERS_DIGEST: &str = "8cddc06fc66c8123b960a44e7582770094e7cd348912a04e56a13787a7b4435e";
const DUPES_DIGEST: &str = "d241a64a38b2cfe5ebe5d8242b29e12f49b50f420e2e694eb2180fc60cf4fe70";
const REPORT_BINDING: &str = "d756202ba10f2e8a373c0a66b36ab23e745e42a22d8cd4ad97923d7eaa2ca661";

fn corpus() -> Root {
    Root::new(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("nested in the repository"),
    )
}

fn rebuilt() -> (
    Root,
    tranche_core::domain::pr::Prs,
    std::collections::HashMap<u64, tranche_core::domain::judge::Judgment>,
    Vec<serde_json::Value>,
) {
    let root = corpus();
    let prs = load_prs(&root, REPOSITORY).expect("the capture loads");
    let judgments = current_judgments(&root, &prs, REPOSITORY, MODEL, false).expect("judgments");
    let verdicts = current_pairs(&root, &prs, &judgments, REPOSITORY, MODEL, false).expect("pairs");
    (root, prs, judgments, verdicts)
}

#[test]
#[ignore = "pins the digests of the local 111 MiB corpus"]
fn the_cluster_output_matches_the_committed_report() {
    let (_root, prs, judgments, verdicts) = rebuilt();
    let report = cluster(&prs, &judgments, &verdicts, REPOSITORY, MODEL, false);
    assert_eq!(digest(&report.clusters), CLUSTERS_DIGEST, "clusters.json");
    assert_eq!(digest(&report.dupes), DUPES_DIGEST, "dupes.json");
    assert_eq!(
        report.summary["report_binding"].as_str(),
        Some(REPORT_BINDING),
        "report binding"
    );
}

#[test]
#[ignore = "pins the digests of the local 111 MiB corpus"]
fn the_summary_counts_match_the_committed_report() {
    let (_root, prs, judgments, verdicts) = rebuilt();
    let summary = cluster(&prs, &judgments, &verdicts, REPOSITORY, MODEL, false).summary;
    let counts = [
        ("prs_in_corpus", 2817),
        ("judged", 2817),
        ("unjudged_or_stale", 0),
        ("unbound_judgments", 0),
        ("dupe_groups", 84),
        ("review_groups", 10),
        ("uncertain_pairs", 57),
        ("prs_in_dupe_groups", 209),
        ("superseded", 0),
        ("ready_tranches", 10),
        ("ready_prs", 612),
        ("needs_author_followup", 87),
        ("escalate_review", 382),
        ("security_priority", 371),
        ("unknown_risk_or_security", 0),
    ];
    for (key, expected) in counts {
        assert_eq!(summary[key].as_u64(), Some(expected), "{key}");
    }
    assert_eq!(
        summary["tokens"]["input"].as_i64(),
        Some(4_484_571),
        "input tokens"
    );
    assert_eq!(
        summary["tokens"]["output"].as_i64(),
        Some(587_861),
        "output tokens"
    );
}

#[test]
#[ignore = "pins the digests of the local 111 MiB corpus"]
fn the_batch_plan_matches_the_committed_report() {
    let (root, prs, judgments, verdicts) = rebuilt();
    let report = cluster(&prs, &judgments, &verdicts, REPOSITORY, MODEL, false);
    let batches = merge_batches(&report.dupes, &judgments, &prs, DUPES_DIGEST, REPOSITORY)
        .expect("batches pack");
    let committed = serde_json::from_str::<serde_json::Value>(
        &std::fs::read_to_string(root.batches_path()).expect("committed batches"),
    )
    .expect("valid JSON");

    // Every field except the digests and prompts is compared structurally; the
    // prompt text is compared through the whole-file digest below.
    assert_eq!(
        batches["batches"].as_array().map(Vec::len),
        Some(522),
        "batch count"
    );
    assert_eq!(
        batches["security_batches"].as_u64(),
        Some(66),
        "security batches"
    );
    assert_eq!(batches["parked_prs"].as_u64(), Some(187), "parked");
    assert_eq!(
        batches["excluded_review_prs"].as_u64(),
        Some(37),
        "excluded"
    );
    assert_eq!(
        batches["same_change_groups"].as_u64(),
        Some(75),
        "same-change groups"
    );
    assert_eq!(
        batches["batches"][0]["members"], committed["batches"][0]["members"],
        "first batch members"
    );

    // The committed file is the contract; the rebuild must reproduce it byte for
    // byte apart from the digest of dupes, which is passed in identically.
    let produced = digest(&batches);
    let expected = digest(&committed);
    assert_eq!(produced, expected, "batches.json digest");

    let parks = park_state(&report.dupes, &judgments, &prs);
    assert_eq!(parks.len(), 187, "parked PRs");
    let parked = parked_payload(&parks, &prs, &judgments, DUPES_DIGEST, REPOSITORY);
    assert_eq!(parked["parked"].as_u64(), Some(187), "park record count");
}
