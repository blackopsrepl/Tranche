//! The report and its summary reproduce from the stored corpus.
//!
//! Everything under `out/` is the interface every reader depends on, so code that
//! reads the same logs must arrive at the same bytes — not merely similar ones.
//! The expected values are read from the committed files rather than restated, so
//! this fails if either side drifts.

mod support;

use support::{built, contract, corpus, read, reconstructed};
use tranche_core::domain::cluster::cluster;
use tranche_core::util::digest;

#[test]
fn the_report_reconstructs_from_the_stored_corpus() {
    let root = corpus();
    let (_, _, report) = built(&root);

    let committed_clusters = read(&root, "clusters.json");
    let committed_dupes = read(&root, "dupes.json");
    let committed_summary = read(&root, "summary.json");

    assert_eq!(
        digest(&report.clusters),
        digest(&committed_clusters),
        "clusters.json reproduces byte for byte"
    );
    assert_eq!(
        digest(&report.dupes),
        digest(&committed_dupes),
        "dupes.json reproduces byte for byte"
    );
    assert_eq!(
        report.summary["report_binding"], committed_summary["report_binding"],
        "the report binding identifies the same observation"
    );
    assert_eq!(
        report.summary["output_digests"], committed_summary["output_digests"],
        "the recorded output digests match"
    );
}

#[test]
fn the_summary_agrees_on_every_count() {
    let root = corpus();
    let (_, _, report) = built(&root);
    let committed = read(&root, "summary.json");

    for field in [
        "format_version",
        "repo",
        "prs_in_corpus",
        "judged",
        "unjudged_or_stale",
        "unbound_judgments",
        "allow_unbound",
        "dupe_groups",
        "review_groups",
        "uncertain_pairs",
        "prs_in_dupe_groups",
        "superseded",
        "ready_tranches",
        "ready_prs",
        "recommendation_kind",
        "needs_author_followup",
        "escalate_review",
        "security_priority",
        "unknown_risk_or_security",
        "tokens",
    ] {
        assert_eq!(
            report.summary.get(field),
            committed.get(field),
            "summary.{field} disagrees with the published report"
        );
    }
}

#[test]
fn the_security_meta_category_leads_the_report() {
    let root = corpus();
    let (_, _, report) = built(&root);

    // A first-class key, ordered before every category.
    let keys: Vec<&String> = report
        .clusters
        .as_object()
        .expect("clusters is an object")
        .keys()
        .collect();
    assert_eq!(
        keys.first().map(|key| key.as_str()),
        Some("security-review")
    );

    // Ranked probability-first, then by number.
    let mut previous: Option<(f64, u64)> = None;
    for item in report.clusters["security-review"]
        .as_array()
        .expect("a list of items")
    {
        let flag = item["security_flag"].as_f64().unwrap_or(-1.0);
        let number = item["number"].as_u64().expect("a number");
        if let Some((last_flag, last_number)) = previous {
            assert!(
                flag < last_flag || (flag == last_flag && number > last_number),
                "#{number} is out of order: {flag} after {last_flag}"
            );
        }
        previous = Some((flag, number));
    }
}

#[test]
fn an_unbound_report_admits_what_it_cannot_establish() {
    let root = corpus();
    let (_, _, report) = built(&root);
    let summary = &report.summary;

    assert_eq!(summary["allow_unbound"], false);
    // Unjudged is a count, never an assumption: every observed PR is accounted
    // for as either judged or not.
    assert_eq!(
        summary["prs_in_corpus"].as_u64().unwrap(),
        summary["judged"].as_u64().unwrap() + summary["unjudged_or_stale"].as_u64().unwrap()
    );
}

#[test]
fn rebuilding_twice_reaches_the_same_bytes() {
    // The pipeline is deterministic: two runs over the same logs must agree, or
    // the digests that gate every reader are meaningless.
    let root = corpus();
    let (prs, judgments, verdicts) = reconstructed(&root);
    let first = cluster(&prs, &judgments, &verdicts, contract(), false);
    let second = cluster(&prs, &judgments, &verdicts, contract(), false);
    assert_eq!(digest(&first.clusters), digest(&second.clusters));
    assert_eq!(digest(&first.dupes), digest(&second.dupes));
    assert_eq!(first.summary, second.summary);
}
