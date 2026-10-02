//! `tranche dupes`, end to end, against a local model stub.
//!
//! The property worth proving is convergence: a comparison that has already been
//! made and is still current is not work, so a second pass spends nothing. The
//! opposite failure — re-running a pair whose verdict is still current — is what
//! makes a dupe pass never finish on a large backlog.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

const KEY: &str = "jev-test-key-not-real";

/// A model stub that answers every request and counts them.
fn model() -> (String, Arc<AtomicU32>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a free port");
    let port = listener.local_addr().expect("address").port();
    let asked = Arc::new(AtomicU32::new(0));
    let counter = Arc::clone(&asked);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            counter.fetch_add(1, Ordering::SeqCst);
            drain(&mut stream);
            let body = r#"{"answers":{"sameness":{"choice":"related_but_different","probabilities":{"same_change":0.2}}},"usage":{"input_tokens":100,"output_tokens":20},"model":"jev-1.2.3","request_id":"req-2"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    (format!("http://127.0.0.1:{port}"), asked)
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

/// A checkout with two PRs whose titles resemble each other, already judged.
///
/// Both a snapshot and a judgment log are needed: a dupe pass compares judged
/// PRs only, so the candidate set comes from the judgments.
fn root_with_pair() -> tempfile::TempDir {
    let root = tempfile::tempdir().expect("a root");
    let pages = root.path().join("data/pages");
    let out = root.path().join("out");
    fs::create_dir_all(&pages).expect("pages");
    fs::create_dir_all(&out).expect("out");

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

    // One judgment per PR, produced by the same code path the judge command uses.
    let root_handle = tranche_core::report::Root::new(root.path());
    let (_, endpoint) = ((), String::new());
    let _ = endpoint;
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_tranche"))
        .arg("--root")
        .arg(root.path())
        .arg("judge")
        .env("TYPESAFE_API_KEY", KEY)
        .env("TRANCHE_DEV_API_URL", judge_endpoint())
        .output()
        .expect("judge runs");
    assert!(
        output.status.success(),
        "judging the fixture failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let _ = root_handle;
    root
}

/// A stub endpoint for the judgment setup step.
fn judge_endpoint() -> String {
    let (endpoint, _) = model();
    endpoint
}

fn run(root: &Path, endpoint: &str, arguments: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_tranche"))
        .arg("--root")
        .arg(root)
        .args(arguments)
        .env("TYPESAFE_API_KEY", KEY)
        .env("TRANCHE_DEV_API_URL", endpoint)
        .output()
        .expect("the tranche binary runs")
}

fn verdicts(root: &Path) -> Vec<serde_json::Value> {
    match fs::read_to_string(root.join("out/pair_verdicts.jsonl")) {
        Ok(text) => text
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect(),
        Err(_) => Vec::new(),
    }
}

#[test]
fn comparing_writes_one_verdict_for_the_candidate_pair() {
    let root = root_with_pair();
    let (endpoint, asked) = model();
    let output = run(root.path(), &endpoint, &["dupes"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let records = verdicts(root.path());
    assert_eq!(records.len(), 1, "one verdict for the one candidate pair");
    assert_eq!(asked.load(Ordering::SeqCst), 1);
    let record = &records[0];
    assert_eq!(record["a"], 11);
    assert_eq!(record["b"], 22);
    assert!(record["binding"].is_string(), "a binding is a cache key");
    assert_eq!(record["verdict"], "related_but_different");
    // `normalize_pair` moves the token counts to the flat fields and drops the
    // nested block, so a reader does not have to know which shape it got.
    assert_eq!(record["input_tokens"], 100);
    assert_eq!(record["output_tokens"], 20);
    assert!(record.get("usage").is_none(), "the nested block is gone");
}

#[test]
fn a_second_pass_has_nothing_left_to_compare() {
    // Convergence: a current verdict is not work. Without this a dupe pass never
    // finishes, because every run would re-ask every pair it already knows.
    let root = root_with_pair();
    let (endpoint, asked) = model();
    run(root.path(), &endpoint, &["dupes"]);
    let after_first = asked.load(Ordering::SeqCst);
    assert!(after_first >= 1);

    let output = run(root.path(), &endpoint, &["dupes"]);
    assert!(output.status.success());
    assert_eq!(
        asked.load(Ordering::SeqCst),
        after_first,
        "a second pass spends nothing"
    );
    assert_eq!(verdicts(root.path()).len(), 1, "and appends nothing");
}

#[test]
fn max_pairs_bounds_the_pass() {
    let root = root_with_pair();
    let (endpoint, asked) = model();
    let output = run(root.path(), &endpoint, &["dupes", "--max-pairs", "1"]);
    assert!(output.status.success());
    assert!(
        asked.load(Ordering::SeqCst) <= 1 + 2,
        "at most the one pair, plus the judging setup"
    );
    assert_eq!(verdicts(root.path()).len(), 1);
}
