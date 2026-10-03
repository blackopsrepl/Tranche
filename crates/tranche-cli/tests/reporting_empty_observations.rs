use serde_json::{Value, json};
use std::process::Command;
use tranche_core::report::Root;
use tranche_core::util::digest;

fn command(root: &std::path::Path, verb: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_tranche"))
        .args([verb, "--json", "--root"])
        .arg(root)
        .output()
        .unwrap()
}
fn setup() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("tranche.json"),
        include_bytes!("../../../tranche.json"),
    )
    .unwrap();
    std::fs::create_dir_all(root.path().join("out")).unwrap();
    std::fs::write(root.path().join("out/summary.json"), b"stale observation").unwrap();
    root
}

#[test]
fn an_observed_empty_snapshot_replaces_stale_reports_with_bound_zero_counts() {
    let temp = setup();
    let root = Root::new(temp.path());
    std::fs::create_dir_all(root.pages_dir()).unwrap();
    std::fs::write(
        root.snapshot_path(),
        json!({"version":1, "repo":"omacom/omarchy", "items":[], "digest":digest(&json!([]))})
            .to_string(),
    )
    .unwrap();
    let output = command(temp.path(), "cluster");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let summary: Value =
        serde_json::from_slice(&std::fs::read(root.summary_path()).unwrap()).unwrap();
    assert_eq!(summary["prs_in_corpus"], 0);
    assert_eq!(summary["judged"], 0);
    assert_eq!(summary["report_binding"].as_str().unwrap().len(), 64);
    let output = command(temp.path(), "batches");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let batches: Value =
        serde_json::from_slice(&std::fs::read(root.batches_path()).unwrap()).unwrap();
    assert_eq!(batches["batches"], json!([]));
    for verb in ["judge", "dupes", "all", "info", "page"] {
        let output = command(temp.path(), verb);
        assert!(
            output.status.success(),
            "{verb}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn absent_membership_does_not_replace_a_stale_report() {
    let temp = setup();
    let output = command(temp.path(), "cluster");
    assert!(!output.status.success());
    assert_eq!(
        std::fs::read(temp.path().join("out/summary.json")).unwrap(),
        b"stale observation"
    );
}
