//! Membership projection and digest compatibility, pinned against the real corpus.
//!
//! These are cache keys and report bindings, so equality is the contract: a
//! different projection silently invalidates every stored judgment and makes
//! every report on disk foreign.

use serde_json::{Value, json};
use tranche_core::domain::pr::{Prs, load_prs, pr_evidence_digest, reference_numbers};
use tranche_core::report::Root;

const REPOSITORY: &str = "omacom/omarchy";

fn corpus() -> Root {
    Root::new(env!("CARGO_MANIFEST_DIR"))
        .path()
        .parent()
        .and_then(std::path::Path::parent)
        .map(Root::new)
        .expect("the core crate is nested in the repository")
}

#[test]
#[ignore = "reads the local 111 MiB corpus; the committed fixture covers the rest"]
fn membership_digest_matches_the_committed_capture() {
    let prs = load_prs(&corpus(), REPOSITORY).expect("the committed capture loads");
    assert_eq!(prs.len(), 2817);
    assert_eq!(
        prs.membership_digest(),
        "1a832988a90e0791597020cb03c76db08d3d80d6aac02dce8666c0f8bc01788a"
    );
}

#[test]
#[ignore = "reads the local 111 MiB corpus; the committed fixture covers the rest"]
fn per_pr_digests_match_the_committed_capture() {
    // First, second, a late middle row and the last row of the real capture.
    let expected = [
        (
            13971,
            "4a9d73c55fdff092d18598cd6eadee07c0e8d0885cc4b046cdfd0c94eb53f18f",
            "6b3961369a0736c6ccabaf3d1bee39ea420baf46987b9508cba9ccbd1b75975e",
            "f2449ae08c8ceb97029a1bfcb708a1ce27672d6bb746dca96f44bbec7f558c9a",
        ),
        (
            13970,
            "cbbefe4ad8aacb6d756e1802e52a0806979c44eaeb441f936fbc515f22941bae",
            "386e85d0b4f0f849227c8aaefca373811068ef369a3e5c5e89b940ed3ed06513",
            "2b9dce10b60cde23b12216093cb4a3da7186602c5a3eee2964fb52751fae157f",
        ),
        (
            11918,
            "6aeb5b29f37316ab962bde2f684460a356f3ad4c071dc0c465b52067e14008ba",
            "f25204ef6440b9b0a97a05621a1b4af476907320ab3b83099bd7d8ac40b47b1a",
            "be5e839f89f27cb4241cac4d66a01dee471c3f302aee1ff2bb36594295647d05",
        ),
        (
            3507,
            "0a652d6fedb6f17020d24adc5d1480577c1cb79b63457bb7160e95d535bcddc3",
            "f1a7677be8286acf0d3ee8d77168175c968fda26894453eec432eeedfa9e3591",
            "2b9dce10b60cde23b12216093cb4a3da7186602c5a3eee2964fb52751fae157f",
        ),
    ];
    let prs = load_prs(&corpus(), REPOSITORY).expect("the committed capture loads");
    for (number, source, evidence, reference) in expected {
        let pr = prs
            .get(number)
            .unwrap_or_else(|| panic!("PR {number} is in the capture"));
        assert_eq!(pr.source_digest, source, "source digest for {number}");
        assert_eq!(pr.evidence_digest, evidence, "evidence digest for {number}");
        assert_eq!(pr.ref_digest, reference, "reference digest for {number}");
    }
}

#[test]
#[ignore = "reads the local 111 MiB corpus; the committed fixture covers the rest"]
fn the_projection_keeps_capture_order() {
    let prs = load_prs(&corpus(), REPOSITORY).expect("the committed capture loads");
    // Order is part of the membership digest; a set would silently reorder it.
    assert_eq!(prs.numbers().first(), Some(&13971));
    assert_eq!(prs.numbers().last(), Some(&3507));
}

#[test]
#[ignore = "reads the local 111 MiB corpus; the committed fixture covers the rest"]
fn references_resolve_only_in_repository_mentions() {
    // A bare number belongs here; a qualified or linked mention names its own
    // repository and must never be re-read as a bare number.
    let body = "see #12 and other/repo#34 and https://github.com/omacom/omarchy/pull/56 \
                plus https://github.com/other/repo/pull/78";
    assert_eq!(reference_numbers(body, REPOSITORY, None), vec![12, 56]);
}

#[test]
#[ignore = "reads the local 111 MiB corpus; the committed fixture covers the rest"]
fn a_pr_never_references_itself() {
    assert_eq!(
        reference_numbers("follow-up to #42", REPOSITORY, Some(42)),
        Vec::<u64>::new()
    );
    assert_eq!(
        reference_numbers("#42 and #43", REPOSITORY, Some(42)),
        vec![43]
    );
}

#[test]
#[ignore = "reads the local 111 MiB corpus; the committed fixture covers the rest"]
fn evidence_projection_ignores_unrelated_envelope_churn() {
    // Repository-wide counters move on unrelated events; the evidence digest
    // must not, or every PR looks changed after every fetch.
    let base: Value = json!({
        "number": 1, "title": "t", "body": "b", "state": "open", "draft": false,
        "labels": [{"name": "bug", "color": "ff0000"}],
        "user": {"login": "someone", "id": 7, "site_admin": false},
        "head": {"sha": "abc", "ref": "f", "label": "o:f", "repo": {"stargazers_count": 1}},
        "changed_files": 2
    });
    let mut churned = base.clone();
    churned["head"]["repo"]["stargazers_count"] = json!(999);
    churned["user"]["site_admin"] = json!(true);
    churned["labels"][0]["color"] = json!("00ff00");
    churned["_links"] = json!({"self": {"href": "x"}});
    churned["statuses_url"] = json!("y");
    assert_eq!(pr_evidence_digest(&base), pr_evidence_digest(&churned));

    // A real change must move it.
    let mut changed = base.clone();
    changed["title"] = json!("different");
    assert_ne!(pr_evidence_digest(&base), pr_evidence_digest(&changed));
}

#[test]
#[ignore = "reads the local 111 MiB corpus; the committed fixture covers the rest"]
fn a_malformed_capture_is_refused_rather_than_used() {
    let root = tempfile::tempdir().expect("a temporary root");
    let pages = root.path().join("data").join("pages");
    std::fs::create_dir_all(&pages).expect("the pages directory");
    let snapshot = pages.join("snapshot.json");
    std::fs::write(
        &snapshot,
        serde_json::to_vec(&json!({
            "version": 1,
            "repo": REPOSITORY,
            "digest": "not-the-digest",
            "items": []
        }))
        .expect("serializable"),
    )
    .expect("written");

    let report_root = Root::new(root.path());
    let error = load_prs(&report_root, REPOSITORY).expect_err("a bad checksum is refused");
    assert!(error.0.contains("checksum differs"), "{error}");
}

#[test]
#[ignore = "reads the local 111 MiB corpus; the committed fixture covers the rest"]
fn prs_are_indexed_by_number_without_reordering() {
    let prs = Prs::default();
    assert!(prs.is_empty());
    assert_eq!(
        prs.membership_digest(),
        tranche_core::util::digest(&json!([] as [u64; 0]))
    );
}
