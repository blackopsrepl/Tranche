//! The Jev client's retry rules, against a local server.
//!
//! The classification is the whole contract: a 401 or a 422 must never be
//! retried, because they are question-shape bugs rather than load, and retrying
//! them would only spend the budget slower. A 429, a 529 and any 5xx must be
//! retried. The key must never appear in an error message.
//!
//! Each test runs its own server on its own port and passes that endpoint in, so
//! nothing here shares state with a test running beside it.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use std::time::Duration;

use tranche_core::jev::{JevError, ask_at};

/// A server that answers each request with the next scripted response, and
/// counts the requests it received.
struct Stub {
    endpoint: String,
    seen: Arc<AtomicU32>,
}

/// Responses are `(status, body)` in order; the last one repeats.
fn serve(script: &'static [(u16, &'static str)]) -> Stub {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a free port");
    let port = listener.local_addr().expect("address").port();
    let seen = Arc::new(AtomicU32::new(0));
    let counter = Arc::clone(&seen);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let index = counter.fetch_add(1, Ordering::SeqCst) as usize;
            let (status, body) = script[index.min(script.len() - 1)];
            drain(&mut stream);
            let reason = match status {
                200 => "OK",
                401 => "Unauthorized",
                422 => "Unprocessable Entity",
                429 => "Too Many Requests",
                _ => "Internal Server Error",
            };
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    Stub {
        endpoint: format!("http://127.0.0.1:{port}"),
        seen,
    }
}

/// Read the request head and body so the client does not see a reset.
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

fn call(stub: &Stub) -> Result<serde_json::Value, JevError> {
    // SAFETY: the key is a fixed test literal, set on the calling thread before
    // any request is made. Tests that touch the environment are serialised by
    // `cargo test`'s own thread-per-test model only for this one variable, and
    // every value written is the same, so a race cannot change the outcome.
    unsafe {
        std::env::set_var("TYPESAFE_API_KEY", "test-key-never-real");
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime")
        .block_on(ask_at(
            &stub.endpoint,
            &serde_json::json!({"pr": {"title": "x"}}),
            &serde_json::json!({"category": {}}),
            "jev-latest",
            // Milliseconds rather than the real seconds: this exercises the
            // exhaustion path without a sixty-second test.
            Duration::from_millis(1),
            Duration::from_millis(4),
        ))
}

#[test]
fn a_successful_answer_is_returned_without_retrying() {
    let stub = serve(&[(200, r#"{"answer":"ok"}"#)]);
    let answer = call(&stub).expect("the answer comes back");
    assert_eq!(answer["answer"], "ok");
    assert_eq!(stub.seen.load(Ordering::SeqCst), 1, "asked once");
}

#[test]
fn a_rejected_key_is_never_retried() {
    // Retrying a 401 would spend the budget six times on a key that cannot work.
    let stub = serve(&[(401, r#"{"error":"bad key"}"#)]);
    let error = call(&stub).expect_err("401 is fatal");
    assert!(error.to_string().contains("401"), "{error}");
    assert!(error.to_string().contains("check the key"), "{error}");
    assert_eq!(stub.seen.load(Ordering::SeqCst), 1, "asked exactly once");
}

#[test]
fn a_question_shape_bug_is_never_retried() {
    // A 422 means the request was malformed: retrying cannot fix it.
    let stub = serve(&[(422, r#"{"error":"bad question"}"#)]);
    let error = call(&stub).expect_err("422 is fatal");
    assert!(error.to_string().contains("422"), "{error}");
    assert_eq!(stub.seen.load(Ordering::SeqCst), 1, "asked exactly once");
}

#[test]
fn load_is_retried_until_it_succeeds() {
    // A 429 then a 500 then success: two retries, then the answer.
    let stub = serve(&[
        (429, r#"{"error":"slow down"}"#),
        (500, r#"{"error":"boom"}"#),
        (200, r#"{"answer":"third"}"#),
    ]);
    let answer = call(&stub).expect("the third attempt succeeds");
    assert_eq!(answer["answer"], "third");
    assert_eq!(stub.seen.load(Ordering::SeqCst), 3, "three attempts");
}

#[test]
fn exhausted_retries_report_the_last_error() {
    let stub = serve(&[(500, r#"{"error":"always down"}"#)]);
    let error = call(&stub).expect_err("five hundred is retried to exhaustion");
    assert!(error.to_string().contains("retries exhausted"), "{error}");
    assert!(error.to_string().contains("always down"), "{error}");
    assert_eq!(
        stub.seen.load(Ordering::SeqCst),
        6,
        "six attempts, the documented cap"
    );
}

#[test]
fn an_error_never_carries_the_key() {
    let stub = serve(&[(401, r#"{"error":"bad key"}"#)]);
    let error = call(&stub).expect_err("401 is fatal").to_string();
    assert!(
        !error.contains("test-key-never-real"),
        "the credential leaked into an error: {error}"
    );
}
