//! The report gate.
//!
//! Every reader — the CLI, the workbench and the MCP server — obtains its
//! observation through `load`, so these tests are the ones that decide whether a
//! stale, foreign or modified report can leak into a surface. A false accept
//! here is worse than a false refuse: it publishes an observation that is not
//! the one the report describes.

use tranche_core::domain::cluster::cluster;
use tranche_core::domain::dupe::current_pairs;
use tranche_core::domain::judge::current_judgments;
use tranche_core::domain::pr::load_prs;
use tranche_core::policy::Contract;
use tranche_core::report::{BoundReport, Limits, Root, input_digests, load};
use tranche_core::util::{atomic_json, digest};

#[test]
fn deployment_contract_is_fingerprinted_with_report_inputs() {
    let root = Root::new(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixture"));
    let digests = input_digests(&root, &Limits::default()).unwrap();
    assert!(
        digests
            .get(root.path().join(Contract::FILE_NAME).to_str().unwrap())
            .is_some()
    );
}

fn contract() -> Contract {
    Contract::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("nested in the repository"),
    )
    .expect("the deployment contract loads")
}

fn corpus() -> Root {
    Root::new(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("nested in the repository"),
    )
}

/// A disposable copy of the real report, so a tampered file never touches the
/// checkout.
fn copy_of_corpus() -> (tempfile::TempDir, Root) {
    let source = corpus();
    let temp = tempfile::tempdir().expect("a temporary root");
    let dest = Root::new(temp.path());
    std::fs::copy(
        source.path().join(Contract::FILE_NAME),
        dest.path().join(Contract::FILE_NAME),
    )
    .expect("the deployment contract");
    for path in source.source_paths() {
        let Some(name) = path.file_name() else {
            continue;
        };
        let target = if name == Contract::FILE_NAME {
            dest.path().join(Contract::FILE_NAME)
        } else if name == "snapshot.json" {
            dest.snapshot_path()
        } else if let Some(stem) = name.to_str().filter(|n| n.starts_with("page_")) {
            dest.pages_dir().join(stem)
        } else {
            dest.out_dir().join(name)
        };
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).expect("the destination directory");
        }
        std::fs::copy(&path, &target).expect("the report file");
    }
    (temp, dest)
}

#[test]
#[ignore = "reads the local 111 MiB corpus; the committed fixture covers the rest"]
fn the_committed_report_is_accepted() {
    let report = load(&corpus(), &Limits::default()).expect("the committed report is valid");
    assert_eq!(report.prs.len(), 2817);
    assert_eq!(report.judgments.len(), 2817);
    assert_eq!(report.pairs.len(), 890);
    assert!(report.batches.is_some(), "a batch plan is bound");
    assert!(report.parked.is_some(), "a park record is bound");
    // The identity a reader quotes back to prove which report it saw.
    assert_eq!(
        report.identity["clusters.json"].as_str(),
        Some("8cddc06fc66c8123b960a44e7582770094e7cd348912a04e56a13787a7b4435e")
    );
    assert_eq!(
        report.identity["report_binding"].as_str(),
        Some("d756202ba10f2e8a373c0a66b36ab23e745e42a22d8cd4ad97923d7eaa2ca661")
    );
    assert!(
        report.identity["batches.json"].as_str().is_some(),
        "the batch digest is part of the identity"
    );
}

#[test]
#[ignore = "reads the local 111 MiB corpus; the committed fixture covers the rest"]
fn a_foreign_report_is_refused() {
    let (_temp, root) = copy_of_corpus();
    let mut summary: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root.summary_path()).expect("summary"))
            .expect("valid JSON");
    summary["repo"] = serde_json::json!("someone/else");
    atomic_json(&root.summary_path(), &summary).expect("written");
    let error = load(&root, &Limits::default()).expect_err("a foreign report is refused");
    assert!(error.0.contains("foreign or modified"), "{error}");
}

#[test]
#[ignore = "reads the local 111 MiB corpus; the committed fixture covers the rest"]
fn a_modified_cluster_output_is_refused() {
    let (_temp, root) = copy_of_corpus();
    let mut clusters: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root.clusters_path()).expect("clusters"))
            .expect("valid JSON");
    // Append one PR to a category: the recorded digest no longer describes it.
    clusters["docs"]["low"]
        .as_array_mut()
        .expect("a band array")
        .pop();
    atomic_json(&root.clusters_path(), &clusters).expect("written");
    let error = load(&root, &Limits::default()).expect_err("a modified report is refused");
    assert!(error.0.contains("foreign or modified"), "{error}");
}

#[test]
#[ignore = "reads the local 111 MiB corpus; the committed fixture covers the rest"]
fn a_stale_report_binding_is_refused() {
    let (_temp, root) = copy_of_corpus();
    let mut summary: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root.summary_path()).expect("summary"))
            .expect("valid JSON");
    summary["report_binding"] = serde_json::json!("0".repeat(64));
    atomic_json(&root.summary_path(), &summary).expect("written");
    let error = load(&root, &Limits::default()).expect_err("a stale binding is refused");
    assert!(error.0.contains("foreign or modified"), "{error}");
}

#[test]
#[ignore = "reads the local 111 MiB corpus; the committed fixture covers the rest"]
fn a_modified_batch_plan_is_refused() {
    let (_temp, root) = copy_of_corpus();
    let mut batches: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root.batches_path()).expect("batches"))
            .expect("valid JSON");
    batches["batches"][0]["members"]
        .as_array_mut()
        .expect("members")
        .pop();
    batches["batches"][0]["count"] = serde_json::json!(3);
    atomic_json(&root.batches_path(), &batches).expect("written");
    let error = load(&root, &Limits::default()).expect_err("a modified batch plan is refused");
    assert!(error.0.contains("batches.json is stale"), "{error}");
}

#[test]
#[ignore = "reads the local 111 MiB corpus; the committed fixture covers the rest"]
fn a_modified_park_record_is_refused() {
    let (_temp, root) = copy_of_corpus();
    let mut parked: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root.parked_path()).expect("parked"))
            .expect("valid JSON");
    parked["parked"] = serde_json::json!(0);
    atomic_json(&root.parked_path(), &parked).expect("written");
    let error = load(&root, &Limits::default()).expect_err("a modified park record is refused");
    assert!(error.0.contains("parked.json is stale"), "{error}");
}

#[test]
#[ignore = "reads the local 111 MiB corpus; the committed fixture covers the rest"]
fn a_missing_park_record_is_refused_when_batches_parked_prs() {
    let (_temp, root) = copy_of_corpus();
    std::fs::remove_file(root.parked_path()).expect("removed");
    let error = load(&root, &Limits::default()).expect_err("a missing park record is refused");
    assert!(error.0.contains("parked.json is missing"), "{error}");
}

#[test]
#[ignore = "reads the local 111 MiB corpus; the committed fixture covers the rest"]
fn a_malformed_judgment_log_is_refused() {
    let (_temp, root) = copy_of_corpus();
    let mut log = std::fs::read_to_string(root.judgments_path()).expect("the log");
    // A line that parses as JSON but is not a record makes the whole log unsafe.
    log.push_str("[1, 2, 3]\n");
    std::fs::write(root.judgments_path(), log).expect("written");
    let error = load(&root, &Limits::default()).expect_err("a malformed log is refused");
    assert!(error.0.contains("Invalid or missing"), "{error}");
}

#[test]
#[ignore = "reads the local 111 MiB corpus; the committed fixture covers the rest"]
fn load_produces_the_same_projection_the_builder_does() {
    // The gate rebuilds the report to check the binding. This proves the rebuild
    // and the direct construction agree, so the check cannot pass by comparing a
    // value with itself.
    let root = corpus();
    let contract = contract();
    let repository = contract.repository().to_owned();
    let report = load(&root, &Limits::default()).expect("valid");
    let prs = load_prs(&root, &repository).expect("capture");
    let judgments = current_judgments(
        &root,
        &prs,
        &repository,
        contract.model(),
        contract.judge_questions(),
        false,
    )
    .expect("judgments");
    let pairs = current_pairs(
        &root,
        &prs,
        &judgments,
        &repository,
        contract.model(),
        contract.pair_questions(),
        false,
    )
    .expect("pairs");
    let rebuilt = cluster(&prs, &judgments, &pairs, &contract, false);
    assert_eq!(digest(&report.clusters), digest(&rebuilt.clusters));
    assert_eq!(digest(&report.dupes), digest(&rebuilt.dupes));
}

#[test]
#[ignore = "reads the local 111 MiB corpus; the committed fixture covers the rest"]
fn input_digests_fingerprint_every_bound_input() {
    let root = corpus();
    let digests = input_digests(&root, &Limits::default()).expect("digests");
    let map = digests.as_object().expect("a map");
    // The contract, snapshot, two logs and five report outputs. The page shards
    // exist but are not fingerprinted while a snapshot is present.
    assert_eq!(map.len(), 9, "{:?}", map.keys().collect::<Vec<_>>());
    assert!(map.contains_key("/srv/lab/hack/omarchy-pr-jev-triage/data/pages/snapshot.json"));
    assert!(map.contains_key("/srv/lab/hack/omarchy-pr-jev-triage/out/summary.json"));
    assert!(map.values().all(|value| value.is_string()));

    // Absent files are omitted rather than recorded as null, so the key set is
    // the set that exists.
    let temp = tempfile::tempdir().expect("a temporary root");
    let empty = Root::new(temp.path());
    let digests = input_digests(&empty, &Limits::default()).expect("digests");
    assert_eq!(digests.as_object().expect("a map").len(), 0);
}

#[test]
#[ignore = "reads the local 111 MiB corpus; the committed fixture covers the rest"]
fn a_root_without_the_deployment_contract_is_refused() {
    let temp = tempfile::tempdir().expect("a temporary root");
    let root = Root::new(temp.path());
    let error = load(&root, &Limits::default()).expect_err("an empty root is refused");
    assert!(error.0.contains("no usable deployment contract"), "{error}");
    // Nothing leaks as a usable observation.
    let refused: Result<BoundReport, _> = load(&root, &Limits::default());
    assert!(refused.is_err());
}
