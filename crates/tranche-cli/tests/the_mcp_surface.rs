//! The MCP stdio surface, end to end, against the committed fixture.
//!
//! The contract a client depends on: initialize echoes a known protocol version,
//! the six tools answer through the same gates the CLI answers pass, unknown tools
//! are `-32602`, and a tool that cannot answer is an `isError` result rather than a
//! protocol error.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::Value;

/// Spawn `tranche mcp` over the fixture and exchange one round of messages.
fn exchange(root: &std::path::Path, requests: &[Value]) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_tranche"))
        .arg("--root")
        .arg(root)
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("the tranche binary runs");
    {
        let mut stdin = child.stdin.take().expect("stdin is piped");
        for request in requests {
            writeln!(stdin, "{request}").expect("a request is written");
        }
    }
    let output = child.wait_with_output().expect("the server exits");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("every response is valid JSON"))
        .collect()
}

/// The fixture checkout, copied so a session never touches it.
fn root() -> tempfile::TempDir {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("nested in the repository");
    let root = tempfile::tempdir().expect("a root");
    copy(
        &repository.join("crates/tranche-core/tests/fixture"),
        root.path(),
    );
    root
}

fn copy(source: &Path, destination: &Path) {
    std::fs::create_dir_all(destination).expect("a directory");
    for entry in std::fs::read_dir(source).expect("the fixture reads") {
        let entry = entry.expect("an entry");
        let target = destination.join(entry.file_name());
        if entry.file_type().expect("a type").is_dir() {
            copy(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("a file");
        }
    }
}

fn text_of(response: &Value) -> Value {
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or("");
    serde_json::from_str(text).unwrap_or(Value::Null)
}

#[test]
fn initialize_echoes_a_known_protocol_version_and_lists_the_tools() {
    let root = root();
    let responses = exchange(
        root.path(),
        &[
            serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}),
            serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        ],
    );
    assert_eq!(responses.len(), 2, "a notification gets no response");
    assert_eq!(
        responses[0]["result"]["protocolVersion"], "2025-06-18",
        "the client's version is echoed when known"
    );
    assert_eq!(responses[0]["result"]["serverInfo"]["name"], "tranche");
    let tools: Vec<&str> = responses[1]["result"]["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert_eq!(
        tools,
        [
            "surface",
            "query",
            "pick",
            "next_prompt",
            "related",
            "digests"
        ]
    );
    // The read-only annotation is part of the contract with the client.
    for tool in responses[1]["result"]["tools"].as_array().expect("tools") {
        assert_eq!(tool["annotations"]["readOnlyHint"], true);
    }
}

#[test]
fn the_tools_answer_through_the_report_gates() {
    let root = root();
    let responses = exchange(
        root.path(),
        &[
            serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"digests","arguments":{}}}),
            serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"query","arguments":{"queue":"all","limit":5}}}),
            serde_json::json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"next_prompt","arguments":{}}}),
            serde_json::json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"surface","arguments":{}}}),
        ],
    );
    // digests: the bound report's identity, the thing a client quotes.
    let digests = text_of(&responses[0]);
    assert!(
        digests["digests"]["report_binding"].is_string(),
        "the report binding travels"
    );
    // query: the disclaimer travels and pagination is bounded, whichever queue
    // the fixture exercises. The 'all' queue always has the whole corpus.
    let query = text_of(&responses[1]);
    assert!(
        query["disclaimer"]
            .as_str()
            .is_some_and(|text| !text.is_empty())
    );
    let items = query["items"].as_array().expect("a page of items");
    assert!(!items.is_empty(), "the whole corpus is non-empty");
    assert!(items.len() <= 5, "the limit bound the page");
    assert_eq!(query["offset"], 0);
    // next_prompt: the first batch, with its unchanged review prompt.
    let next = text_of(&responses[2]);
    assert_eq!(next["batch"]["id"], "B001");
    assert!(
        next["batch"]["review_prompt"]
            .as_str()
            .is_some_and(|text| !text.is_empty()),
        "the review prompt is provenance, not decoration"
    );
    // surface: coverage and queues over the whole report.
    let surface = text_of(&responses[3]);
    assert!(surface["summary"]["prs_in_corpus"].as_u64().unwrap_or(0) > 0);
    assert!(surface["queues"]["security"]["count"].is_u64());
}

#[test]
fn unknown_tools_are_protocol_errors_and_bad_arguments_are_tool_results() {
    let root = root();
    let responses = exchange(
        root.path(),
        &[
            serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"nope","arguments":{}}}),
            serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"pick","arguments":{"batch_id":"B999"}}}),
            serde_json::json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"pick","arguments":{}}}),
        ],
    );
    // An unknown tool never runs: the spec puts that behind -32602.
    assert_eq!(responses[0]["error"]["code"], -32602);
    // A known tool refusing its arguments is a result with isError, not an error.
    assert_eq!(responses[1]["result"]["isError"], true);
    assert!(
        responses[1]["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .contains("Unknown batch id")
    );
    assert_eq!(responses[2]["result"]["isError"], true);
    assert!(
        responses[2]["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .contains("Missing required argument")
    );
}

#[test]
fn a_broken_report_is_refused_not_half_served() {
    let root = root();
    // Corrupt the summary so the gates must refuse the report: a tool answer built
    // from a half-read report would be worse than an error.
    let summary = root.path().join("out/summary.json");
    std::fs::write(&summary, "{not json").expect("the summary is overwritten");
    let responses = exchange(
        root.path(),
        &[
            serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"digests","arguments":{}}}),
        ],
    );
    assert_eq!(responses[0]["result"]["isError"], true);
    let message = responses[0]["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or("")
        .to_lowercase();
    assert!(
        message.contains("invalid") || message.contains("summary") || message.contains("bound"),
        "the refusal names the report: {message}"
    );
}

#[test]
fn a_parse_error_is_a_protocol_error_with_null_id() {
    let root = root();
    let mut child = Command::new(env!("CARGO_BIN_EXE_tranche"))
        .arg("--root")
        .arg(root.path())
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("the tranche binary runs");
    {
        let mut stdin = child.stdin.take().expect("stdin is piped");
        writeln!(stdin, "{{not json").expect("the bad line is written");
        writeln!(
            stdin,
            "{{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"ping\"}}"
        )
        .expect("the ping is written");
    }
    let output = child.wait_with_output().expect("the server exits");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let lines: Vec<&str> = stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    assert_eq!(lines.len(), 2, "both lines are answered");
    // An unparseable line has no id, so the error carries null per JSON-RPC.
    let parse_error: Value = serde_json::from_str(lines[0]).expect("valid JSON");
    assert_eq!(parse_error["error"]["code"], -32700);
    assert!(parse_error["id"].is_null());
    // The ping that follows is still answered: one bad line does not end the session.
    let ping: Value = serde_json::from_str(lines[1]).expect("valid JSON");
    assert_eq!(ping["id"], 9);
    assert_eq!(ping["result"], serde_json::json!({}));
}
