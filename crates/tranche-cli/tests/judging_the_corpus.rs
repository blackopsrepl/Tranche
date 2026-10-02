//! `tranche judge`, end to end, against a local model stub.
//!
//! The cost of a mistake here is money: if resume stops recognizing an existing
//! judgment it re-asks the whole backlog, and if it recognizes a stale one it
//! reuses an answer to a question nobody asked. Both directions are tested.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

/// The API key the client will pick up.
const KEY: &str = "jev-test-key-not-real";

/// A model stub that answers every request the same way and counts them.
struct Model {
    endpoint: String,
    asked: Arc<AtomicU32>,
}

fn model() -> Model {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a free port");
    let port = listener.local_addr().expect("address").port();
    let asked = Arc::new(AtomicU32::new(0));
    let counter = Arc::clone(&asked);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            counter.fetch_add(1, Ordering::SeqCst);
            drain(&mut stream);
            let body = r#"{"answers":{"category":{"choice":"fix-misc"},"risk":{"score":1},"is_fix":{"noul":0.9},"dupe_signal":{"noul":0.1},"finished_form":{"score":2},"review_effort":{"score":1},"security_flag":{"noul":0.0}},"usage":{"input_tokens":10,"output_tokens":5},"model":"jev-1.2.3","request_id":"req-1"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    Model {
        endpoint: format!("http://127.0.0.1:{port}"),
        asked,
    }
}

fn drain(stream: &mut TcpStream) {
    let Ok(handle) = stream.try_clone() else {
        return;
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
    if length > 0 {
        let mut body = vec![0u8; length];
        let _ = reader.read_exact(&mut body);
    }
}

/// A checkout holding a snapshot of two PRs.
fn root_with_corpus() -> tempfile::TempDir {
    let root = tempfile::tempdir().expect("a root");
    let pages = root.path().join("data/pages");
    fs::create_dir_all(&pages).expect("pages");
    let items = serde_json::json!([
        {"number": 11, "title": "first", "body": "b", "head": {"sha": "a"}, "user": {"login": "octo"}},
        {"number": 22, "title": "second", "body": null, "head": {"sha": "b"}, "user": {"login": "octo"}},
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

fn run(root: &Path, endpoint: &str, arguments: &[&str]) -> std::process::Output {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_tranche"));
    command
        .arg("--root")
        .arg(root)
        .args(arguments)
        .env("TYPESAFE_API_KEY", KEY)
        .env("TRANCHE_DEV_API_URL", endpoint);
    // The client reads the endpoint from the environment only when the tests set
    // it; production always uses the fixed one.
    command.output().expect("the tranche binary runs")
}

fn log(root: &Path) -> Vec<serde_json::Value> {
    let path = root.join("out/judgments.jsonl");
    match fs::read_to_string(&path) {
        Ok(text) => text
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect(),
        Err(_) => Vec::new(),
    }
}

fn binary_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_tranche"))
}

#[test]
fn the_binary_exists() {
    // A guard: every other test in this file depends on it.
    assert!(binary_path().exists());
}

#[test]
fn judging_appends_one_record_per_pr() {
    let root = root_with_corpus();
    let stub = model();
    let output = run(root.path(), &stub.endpoint, &["judge"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let records = log(root.path());
    assert_eq!(records.len(), 2, "one record per PR");
    assert_eq!(stub.asked.load(Ordering::SeqCst), 2, "one call per PR");
    // The record carries what a reader downstream needs.
    let first = &records[0];
    assert!(first["binding"].is_string(), "a binding is a cache key");
    assert!(first["answers"]["category"].is_object());
    assert_eq!(first["usage"]["input_tokens"], 10);
}

#[test]
fn resume_does_not_re_ask_a_current_judgment() {
    // The expensive failure: a resume that re-asks the whole backlog.
    let root = root_with_corpus();
    let stub = model();
    run(root.path(), &stub.endpoint, &["judge"]);
    assert_eq!(stub.asked.load(Ordering::SeqCst), 2);

    let output = run(root.path(), &stub.endpoint, &["judge", "--resume"]);
    assert!(output.status.success());
    assert_eq!(
        stub.asked.load(Ordering::SeqCst),
        2,
        "a resume with nothing new spends nothing"
    );
    assert_eq!(log(root.path()).len(), 2, "and appends nothing");
}

#[test]
fn a_changed_head_invalidates_the_judgment_and_re_asks() {
    // The other direction: an answer bound to old evidence must not be reused.
    let root = root_with_corpus();
    let stub = model();
    run(root.path(), &stub.endpoint, &["judge"]);
    assert_eq!(stub.asked.load(Ordering::SeqCst), 2);

    // The author pushed: the head SHA the judgment was bound to no longer matches.
    let pages = root.path().join("data/pages/snapshot.json");
    let text = fs::read_to_string(&pages).expect("snapshot");
    let mut snapshot: serde_json::Value = serde_json::from_str(&text).expect("parse");
    snapshot["items"][0]["head"]["sha"] = serde_json::json!("c".repeat(40));
    snapshot["digest"] = serde_json::json!(tranche_core::util::digest(&snapshot["items"]));
    fs::write(&pages, serde_json::to_string(&snapshot).expect("encode")).expect("write");

    run(root.path(), &stub.endpoint, &["judge", "--resume"]);
    assert_eq!(
        stub.asked.load(Ordering::SeqCst),
        3,
        "the moved PR is asked again, and only it"
    );
}

#[test]
fn a_fresh_pass_resets_the_log() {
    let root = root_with_corpus();
    let stub = model();
    run(root.path(), &stub.endpoint, &["judge"]);
    run(root.path(), &stub.endpoint, &["judge"]);
    // Without --resume the pass starts over rather than appending duplicates.
    assert_eq!(log(root.path()).len(), 2, "the log holds one pass");
    assert_eq!(stub.asked.load(Ordering::SeqCst), 4, "both PRs asked twice");
}

#[test]
fn a_root_without_a_corpus_refuses_rather_than_judging_nothing() {
    let root = tempfile::tempdir().expect("a root");
    let stub = model();
    let output = run(root.path(), &stub.endpoint, &["judge"]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("fetch first"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(stub.asked.load(Ordering::SeqCst), 0, "no request was made");
}
