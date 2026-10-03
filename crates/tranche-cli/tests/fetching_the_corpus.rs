//! `tranche fetch`, end to end, with a fake `gh`.
//!
//! Fetching is the one command that reaches outside the process, so the thing
//! worth proving is what it leaves behind: a snapshot that the corpus loader
//! accepts, and nothing at all when the capture fails.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output};

/// A fake `gh` on a directory placed first on the child's own path.
///
/// The path is set on the child process, never in this process: a test that
/// rewrote the test runner's `PATH` would change what every test beside it saw.
fn fake_gh(script_body: &str) -> tempfile::TempDir {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let binary: PathBuf = directory.path().join("gh");
    fs::write(&binary, format!("#!/bin/sh\n{script_body}\n")).expect("write the fake");
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).expect("make it runnable");
    directory
}

fn run(root: &std::path::Path, path: &tempfile::TempDir) -> Output {
    let current = std::env::var("PATH").unwrap_or_default();
    Command::new(env!("CARGO_BIN_EXE_tranche"))
        .arg("--root")
        .arg(root)
        .arg("fetch")
        .env("PATH", format!("{}:{current}", path.path().display()))
        .output()
        .expect("the tranche binary runs")
}

/// One page of two PRs, in the shape the API returns and `validate_pr` accepts.
const TWO_PRS: &str = r#"[{"number": 11, "title": "first", "body": "b", "head": {"sha": "a"}, "user": {"login": "octo"}}, {"number": 22, "title": "second", "body": null, "head": {"sha": "b"}, "user": {"login": "octo"}}]"#;

/// A minimal deployment contract for a synthetic root: just the identity and
/// the question shape the loader and normalizer read.
fn contract_file(root: &std::path::Path) {
    let mut contract = serde_json::json!({
        "version": 1,
        "repository": "omacom/omarchy",
        "model": "jev-latest",
        "policy": {
            "version": 1,
            "judge": {
                "category": {"type": "choice", "criteria": {"fix": "a fix", "unclear": "unclear"}},
                "risk": {"type": "score", "criteria": ["a", "b", "c", "d", "e"]},
                "is_fix": {"type": "noul"},
                "dupe_signal": {"type": "noul"},
                "finished_form": {"type": "score", "criteria": ["a", "b", "c", "d"]},
                "review_effort": {"type": "score", "criteria": ["a", "b", "c", "d"]},
                "security_flag": {"type": "noul"}
            },
            "pair": {
                "sameness": {
                    "type": "choice",
                    "criteria": {
                        "same_change": "same",
                        "related_but_different": "related",
                        "unrelated": "unrelated"
                    }
                }
            }
        }
    });
    for section in ["judge", "pair"] {
        for question in contract["policy"][section]
            .as_object_mut()
            .unwrap()
            .values_mut()
        {
            question["instructions"] =
                serde_json::json!("Answer this question from the supplied evidence.");
        }
    }
    fs::write(
        root.join("tranche.json"),
        serde_json::to_string_pretty(&contract).expect("encode"),
    )
    .expect("contract");
}

#[test]
fn fetching_writes_a_snapshot_the_corpus_loader_accepts() {
    let root = tempfile::tempdir().expect("a root");
    contract_file(root.path());
    let path = fake_gh(&format!("printf '%s' '{TWO_PRS}'"));
    let output = run(root.path(), &path);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let snapshot: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(root.path().join("data/pages/snapshot.json")).expect("a snapshot"),
    )
    .expect("the snapshot is JSON");
    assert_eq!(snapshot["version"], 1);
    assert_eq!(snapshot["repo"], "omacom/omarchy");
    assert_eq!(snapshot["items"].as_array().expect("items").len(), 2);

    // The digest covers exactly the items: the loader re-checks this and refuses
    // a snapshot that does not describe itself.
    let digest = tranche_core::util::digest(&snapshot["items"]);
    assert_eq!(snapshot["digest"], digest, "the snapshot verifies");

    // And the loader really reads it.
    let corpus = tranche_core::domain::pr::load_prs(
        &tranche_core::report::Root::new(root.path()),
        "omacom/omarchy",
    )
    .expect("the corpus loads from what fetch wrote");
    assert_eq!(corpus.len(), 2);
}

#[test]
fn a_failed_capture_leaves_the_previous_snapshot_untouched() {
    let root = tempfile::tempdir().expect("a root");
    contract_file(root.path());
    let pages = root.path().join("data/pages");
    fs::create_dir_all(&pages).expect("a pages directory");
    let existing = r#"{"version":1,"repo":"omacom/omarchy","items":[],"digest":"x"}"#;
    fs::write(pages.join("snapshot.json"), existing).expect("a previous snapshot");

    let path = fake_gh("exit 1");
    let output = run(root.path(), &path);
    assert!(!output.status.success(), "a failed fetch refuses");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("previous snapshot"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let after = fs::read_to_string(pages.join("snapshot.json")).expect("still there");
    assert_eq!(after, existing, "the previous snapshot was not touched");
}

#[test]
fn a_body_that_is_not_a_pr_list_refuses_rather_than_empty() {
    // An error document read as "no open PRs" would publish an empty corpus.
    let root = tempfile::tempdir().expect("a root");
    contract_file(root.path());
    let path = fake_gh(r#"printf '%s' '{"message":"Not Found"}'"#);
    let output = run(root.path(), &path);
    assert!(!output.status.success(), "a non-array refuses");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("did not return a PR list"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !root.path().join("data/pages/snapshot.json").exists(),
        "nothing was written"
    );
}
