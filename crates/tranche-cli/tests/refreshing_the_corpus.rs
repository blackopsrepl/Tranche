//! `tranche refresh`, end to end, against local stubs.
//!
//! Two properties decide whether this command is trustworthy. A dry run must
//! write nothing and count the same work the real pass performs, and a real run
//! must stop at the first step that fails rather than publishing a report built
//! from a half-finished pipeline.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

const KEY: &str = "jev-test-key-not-real";

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
                "category": {"type": "choice", "criteria": {"fix-misc": "a fix", "unclear": "unclear"}},
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

/// A stub that answers both the model and GitHub, and counts each.
struct Stubs {
    directory: tempfile::TempDir,
    model: String,
    asked: Arc<AtomicU32>,
    judged: Arc<AtomicU32>,
    compared: Arc<AtomicU32>,
}

fn stubs() -> Stubs {
    failing_stubs("")
}

fn failing_stubs(fail: &'static str) -> Stubs {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a free port");
    let port = listener.local_addr().expect("address").port();
    let asked = Arc::new(AtomicU32::new(0));
    let judged = Arc::new(AtomicU32::new(0));
    let judged_counter = Arc::clone(&judged);
    let compared = Arc::new(AtomicU32::new(0));
    let compared_counter = Arc::clone(&compared);
    let counter = Arc::clone(&asked);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            counter.fetch_add(1, Ordering::SeqCst);
            let body = drain(&mut stream);
            // The model endpoint answers the questions; the GitHub path answers a
            // PR list. One stub, told apart by the body it received.
            // One stub serves both model calls and the GitHub read. A judge
            // request asks about `category`; a pair comparison asks about
            // `sameness`. Counting them together made a dupe call look like a
            // re-judgment.
            let reply = if body.contains("\"category\"") {
                judged_counter.fetch_add(1, Ordering::SeqCst);
                r#"{"answers":{"category":{"choice":"fix-misc"},"risk":{"score":1},"is_fix":{"noul":0.9},"dupe_signal":{"noul":0.1},"finished_form":{"score":2},"review_effort":{"score":1},"security_flag":{"noul":0.0}},"usage":{"input_tokens":10,"output_tokens":5},"model":"jev-1.2.3"}"#.to_owned()
            } else if body.contains("\"sameness\"") {
                compared_counter.fetch_add(1, Ordering::SeqCst);
                r#"{"answers":{"sameness":{"choice":"related_but_different","probabilities":{"same_change":0.2}}},"usage":{"input_tokens":100,"output_tokens":20},"model":"jev-1.2.3"}"#.to_owned()
            } else {
                format!(
                    "[{{\"number\": 11, \"title\": \"Fix panel crash on startup\", \"body\": \"a\", \"head\": {{\"sha\": \"{}\"}}, \"user\": {{\"login\": \"octo\"}}}}, {{\"number\": 22, \"title\": \"Fix panel crash on startup\", \"body\": \"b\", \"head\": {{\"sha\": \"{}\"}}, \"user\": {{\"login\": \"octo\"}}}}]",
                    "1".repeat(40),
                    "2".repeat(40),
                )
            };
            let invalid = (fail == "judge"
                && body.contains("\"category\"")
                && judged_counter.load(Ordering::SeqCst) == 1)
                || (fail == "dupes" && body.contains("\"sameness\""));
            let reply = if invalid {
                r#"{"answers":{}}"#.to_owned()
            } else {
                reply
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                reply.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });

    // A fake `gh` pointing at the same stub, so the fetch step reads it.
    let directory = tempfile::tempdir().expect("a directory");
    let gh = directory.path().join("gh");
    fs::write(
        &gh,
        format!("#!/bin/sh\nexec curl --silent --fail 'http://127.0.0.1:{port}/pulls'\n"),
    )
    .expect("write the fake gh");
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).expect("runnable");
    }

    Stubs {
        directory,
        model: format!("http://127.0.0.1:{port}"),
        asked,
        judged,
        compared,
    }
}

fn drain(stream: &mut TcpStream) -> String {
    let Ok(handle) = stream.try_clone() else {
        return String::new();
    };
    let mut reader = BufReader::new(handle);
    let mut length = 0usize;
    let mut line = String::new();
    while reader.read_line(&mut line).unwrap_or(0) > 0 {
        if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
            length = value.trim().parse().unwrap_or(0);
        }
        if line == "\r\n" {
            break;
        }
        line.clear();
    }
    let mut body = vec![0u8; length];
    let _ = reader.read_exact(&mut body);
    String::from_utf8_lossy(&body).into_owned()
}

fn root_with_corpus() -> tempfile::TempDir {
    let root = tempfile::tempdir().expect("a root");
    let pages = root.path().join("data/pages");
    fs::create_dir_all(&pages).expect("pages");
    contract_file(root.path());
    let items = serde_json::json!([
        {"number": 11, "title": "Fix panel crash on startup", "body": "a", "head": {"sha": "1".repeat(40)}, "user": {"login": "octo"}},
        {"number": 22, "title": "Fix panel crash on startup", "body": "b", "head": {"sha": "2".repeat(40)}, "user": {"login": "octo"}},
    ]);
    let snapshot = serde_json::json!({
        "version": 1,
        "repo": "omacom/omarchy",
        "items": items,
        "digest": tranche_core::util::digest(&items),
    });
    fs::write(
        pages.join("snapshot.json"),
        serde_json::to_string(&snapshot).expect("encode"),
    )
    .expect("snapshot");
    root
}

fn run(root: &Path, stubs: &Stubs, arguments: &[&str]) -> Output {
    let current = std::env::var("PATH").unwrap_or_default();
    Command::new(env!("CARGO_BIN_EXE_tranche"))
        .arg("--root")
        .arg(root)
        .args(arguments)
        .env("TYPESAFE_API_KEY", KEY)
        .env("TRANCHE_DEV_API_URL", &stubs.model)
        .env(
            "PATH",
            format!("{}:{current}", stubs.directory.path().display()),
        )
        .output()
        .expect("the tranche binary runs")
}

fn snapshot_of(root: &Path) -> Option<String> {
    fs::read_to_string(root.join("out/summary.json")).ok()
}

#[test]
fn a_dry_run_writes_nothing_and_reports_the_work() {
    let root = root_with_corpus();
    let stub = stubs();
    let output = run(root.path(), &stub, &["refresh", "--dry-run"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("no changes written"), "{stdout}");
    assert!(stdout.contains("judged now"), "{stdout}");
    assert!(stdout.contains("pair verdicts to re-run"), "{stdout}");
    assert!(
        stdout.contains("would re-read open-PR membership"),
        "{stdout}"
    );

    assert!(snapshot_of(root.path()).is_none(), "nothing was written");
    assert!(
        !root.path().join("out/judgments.jsonl").exists(),
        "no judgment was made"
    );
    assert_eq!(
        stub.asked.load(Ordering::SeqCst),
        0,
        "a dry run spends nothing"
    );
}

#[test]
fn refresh_runs_every_step_in_order_and_writes_the_page() {
    // The page is a step like any other now, so a plain refresh completes and
    // leaves the workbench behind.
    let root = root_with_corpus();
    let stub = stubs();
    let template = root.path().join("page");
    fs::create_dir_all(&template).expect("page directory");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("nested in the repository")
            .join("page/template.html"),
        template.join("template.html"),
    )
    .expect("the template");

    let output = run(root.path(), &stub, &["refresh", "--max-pairs", "5"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("refresh:"), "{stdout}");
    assert!(
        stdout.contains("fetch -> judge -> dupes -> cluster -> batches -> page"),
        "{stdout}"
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Every step left a real artifact, the page included.
    assert!(root.path().join("out/judgments.jsonl").exists());
    assert!(root.path().join("out/clusters.json").exists());
    assert!(root.path().join("out/batches.json").exists());
    assert!(root.path().join("docs/index.html").exists());
    assert!(root.path().join("docs/data/workbench.json").exists());
}

#[test]
fn no_page_completes_the_pipeline() {
    let root = root_with_corpus();
    let stub = stubs();
    let output = run(root.path(), &stub, &["refresh", "--no-page"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("refresh complete"), "{stdout}");
    // The before/after accounting is the point of the summary block.
    assert!(stdout.contains("prs_in_corpus"), "{stdout}");
    assert!(stdout.contains("parked_prs"), "{stdout}");

    let summary = snapshot_of(root.path()).expect("a summary");
    let parsed: serde_json::Value = serde_json::from_str(&summary).expect("parses");
    assert_eq!(parsed["prs_in_corpus"], 2);
    assert_eq!(parsed["judged"], 2);
}

#[test]
fn a_second_refresh_judges_nothing_new() {
    // The incremental promise: work is proportional to what changed, not to the
    // corpus size.
    let root = root_with_corpus();
    let stub = stubs();
    run(root.path(), &stub, &["refresh", "--no-page"]);
    assert_eq!(
        stub.judged.load(Ordering::SeqCst),
        2,
        "the first pass judges both PRs"
    );
    assert!(
        stub.compared.load(Ordering::SeqCst) >= 1,
        "and compares the candidate pair"
    );

    let output = run(root.path(), &stub, &["refresh", "--no-page"]);
    assert!(output.status.success());
    assert_eq!(
        stub.judged.load(Ordering::SeqCst),
        2,
        "a second refresh re-judges nothing"
    );
}

#[test]
fn composites_stop_after_a_partial_judgment_failure() {
    for arguments in [&["refresh", "--no-page"][..], &["all"][..]] {
        let root = root_with_corpus();
        let stub = failing_stubs("judge");
        let output = run(root.path(), &stub, arguments);
        assert!(
            !output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("stopped at judge") && stderr.contains("1 failed"),
            "{stderr}"
        );
        assert_eq!(stub.compared.load(Ordering::SeqCst), 0);
        assert!(
            snapshot_of(root.path()).is_none(),
            "no downstream publication"
        );
        assert!(!String::from_utf8_lossy(&output.stdout).contains("refresh complete"));
    }
}

#[test]
fn composites_stop_after_an_invalid_pair_answer_preserving_publication() {
    for arguments in [&["refresh", "--no-page"][..], &["all"][..]] {
        let root = root_with_corpus();
        let good = stubs();
        assert!(
            run(root.path(), &good, &["refresh", "--no-page"])
                .status
                .success()
        );
        let prior = snapshot_of(root.path());
        fs::remove_file(root.path().join("out/pair_verdicts.jsonl")).unwrap();
        let stub = failing_stubs("dupes");
        let output = run(root.path(), &stub, arguments);
        assert!(
            !output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("stopped at dupes") && stderr.contains("1 failed"),
            "{stderr}"
        );
        assert_eq!(
            snapshot_of(root.path()),
            prior,
            "previous publication retained"
        );
    }
}

#[test]
fn info_refuses_stale_or_foreign_reports_instead_of_printing_cached_counts() {
    for foreign in [false, true] {
        let root = root_with_corpus();
        let stub = stubs();
        assert!(
            run(root.path(), &stub, &["refresh", "--no-page"])
                .status
                .success()
        );
        assert!(
            run(root.path(), &stub, &["--json", "info"])
                .status
                .success()
        );
        let path = root.path().join("tranche.json");
        let mut contract: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        if foreign {
            contract["repository"] = serde_json::json!("elsewhere/project");
        } else {
            contract["policy"]["judge"]["category"]["instructions"] =
                serde_json::json!("A different question.");
        }
        fs::write(path, serde_json::to_vec(&contract).unwrap()).unwrap();
        let output = run(root.path(), &stub, &["--json", "info"]);
        assert!(
            !output.status.success(),
            "info must share the bound report gate"
        );
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn no_html_dispatch_exports_without_rendering_a_page() {
    let root = root_with_corpus();
    let stub = stubs();
    assert!(
        run(root.path(), &stub, &["refresh", "--no-page"])
            .status
            .success()
    );
    let output = run(root.path(), &stub, &["page", "--export-json", "--no-html"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !root.path().join("docs/index.html").exists(),
        "export only must not render HTML"
    );
    assert!(root.path().join("docs/data/report.json").exists());
}

#[test]
fn initialized_second_repository_completes_without_checkout_resources() {
    let root = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_tranche"))
        .args(["init", "sample/widgets", "--root"])
        .arg(root.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    let path = root.path().join("tranche.json");
    let mut contract: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    contract["policy"]["judge"]["category"]["criteria"] =
        serde_json::json!({"fix-misc":"Repairs a bug", "unclear":"Insufficient evidence"});
    fs::write(path, serde_json::to_vec(&contract).unwrap()).unwrap();
    let stub = stubs();
    let output = run(root.path(), &stub, &["refresh"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: serde_json::Value =
        serde_json::from_slice(&fs::read(root.path().join("docs/data/workbench.json")).unwrap())
            .unwrap();
    assert_eq!(payload["repository"], "sample/widgets");
    assert_eq!(payload["prs"].as_array().unwrap().len(), 2);
    assert!(root.path().join("docs/assets/workbench.js").is_file());
    assert!(!root.path().join("page/template.html").exists());
    let judged = stub.judged.load(Ordering::SeqCst);
    let compared = stub.compared.load(Ordering::SeqCst);
    assert!(run(root.path(), &stub, &["refresh"]).status.success());
    assert_eq!(stub.judged.load(Ordering::SeqCst), judged);
    assert_eq!(stub.compared.load(Ordering::SeqCst), compared);
}

#[test]
fn the_binary_is_the_one_under_test() {
    assert!(PathBuf::from(env!("CARGO_BIN_EXE_tranche")).exists());
}
