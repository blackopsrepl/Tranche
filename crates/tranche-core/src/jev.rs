//! The Jev client: one question set per request, with the retry rules the
//! pipeline has always used.
//!
//! A 401 and a 422 are fatal and never retried — they are question-shape bugs,
//! not load, so retrying a bad request would only spend the budget slower. A 429,
//! a 529 and any 5xx are load, so they back off and try again. The key is read
//! from the environment first and the key file second, and it never appears in an
//! error message.

use std::path::PathBuf;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

/// Where the model answers.
pub const API_URL: &str = "https://api.typesafe.ai/v1/systemone";

/// Attempts per request, including the first.
const ATTEMPTS: u32 = 6;

/// The first backoff, doubled per attempt and capped.
const BACKOFF_START: Duration = Duration::from_secs(2);
const BACKOFF_CAP: Duration = Duration::from_secs(60);

/// Characters of a failing response body kept for the operator.
const BODY_CHARS: usize = 300;

/// Per-request timeout.
const TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Debug)]
pub struct JevError(pub String);

impl std::fmt::Display for JevError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for JevError {}

impl From<String> for JevError {
    fn from(message: String) -> Self {
        Self(message)
    }
}

/// The key file used when the environment carries no key.
pub fn key_file() -> PathBuf {
    let mut path = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    path.push("Documents");
    path.push("jevapi.txt");
    path
}

/// The API key, from the environment first and the key file second.
///
/// The returned value is the credential. It is never logged, never included in
/// an error and never written anywhere.
pub fn read_key() -> Result<String, JevError> {
    if let Ok(key) = std::env::var("TYPESAFE_API_KEY") {
        let key = key.trim().to_owned();
        if !key.is_empty() {
            return Ok(key);
        }
    }
    let path = key_file();
    match std::fs::read_to_string(&path) {
        Ok(key) if !key.trim().is_empty() => Ok(key.trim().to_owned()),
        _ => Err(JevError(format!(
            "no API key: set TYPESAFE_API_KEY or create {}",
            path.display()
        ))),
    }
}

/// One request's outcome, before retry classification.
enum Attempt {
    /// The model answered.
    Answered(Value),
    /// Worth trying again, with what went wrong for the final report.
    Retry(String),
    /// Never worth trying again.
    Fatal(String),
}

/// Ask the model one question set about one state.
///
/// The payload is `{"state", "model", "questions"}` — the shape the service
/// accepts, assembled here rather than by the caller so no request can drift.
pub async fn ask(state: &Value, questions: &Value, model: &str) -> Result<Value, JevError> {
    ask_with(
        &endpoint(),
        state,
        questions,
        model,
        BACKOFF_START,
        BACKOFF_CAP,
    )
    .await
}

/// The endpoint a request goes to.
///
/// The service URL is fixed, with one deliberate escape hatch: a caller with its
/// own environment can point a run at a local stub. That is how the retry and
/// resume paths are tested without a paid model call, and it is a whole-process
/// setting rather than mutable shared state — a test that points at its own stub
/// does so in the child process it spawns, so it cannot reach a test beside it.
fn endpoint() -> String {
    std::env::var("TRANCHE_DEV_API_URL").unwrap_or_else(|_| API_URL.to_owned())
}

/// Ask a specific endpoint with a specific backoff.
///
/// The backoff is a parameter so a test can exercise the exhaustion path without
/// waiting the real sixty seconds; production always passes the documented ones.
pub async fn ask_with(
    endpoint: &str,
    state: &Value,
    questions: &Value,
    model: &str,
    backoff_start: Duration,
    backoff_cap: Duration,
) -> Result<Value, JevError> {
    ask_at(
        endpoint,
        state,
        questions,
        model,
        backoff_start,
        backoff_cap,
    )
    .await
}

/// Ask the endpoint given, for tests and for a local stub.
pub async fn ask_at(
    endpoint: &str,
    state: &Value,
    questions: &Value,
    model: &str,
    backoff_start: Duration,
    backoff_cap: Duration,
) -> Result<Value, JevError> {
    let key = read_key()?;
    let payload = serde_json::json!({
        "state": state,
        "model": model,
        "questions": questions,
    });
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .build()
        .map_err(|error| JevError(format!("cannot build the HTTP client: {error}")))?;

    let mut backoff = backoff_start;
    let mut last = String::new();
    for attempt in 0..ATTEMPTS {
        match post(&client, endpoint, &payload, &key).await {
            Attempt::Answered(value) => return Ok(value),
            Attempt::Fatal(message) => return Err(JevError(message)),
            Attempt::Retry(message) => {
                last = message;
                if attempt + 1 == ATTEMPTS {
                    break;
                }
                // Jittered, so a fleet of retries does not arrive together.
                let jitter = Duration::from_millis((now_millis() % 50) as u64);
                tokio::time::sleep(backoff + jitter).await;
                backoff = (backoff * 2).min(backoff_cap);
            }
        }
    }
    Err(JevError(format!("retries exhausted; last error: {last}")))
}

/// One POST, classified for retry.
async fn post(
    client: &reqwest::Client,
    endpoint: &str,
    payload: &impl Serialize,
    key: &str,
) -> Attempt {
    let response = match client
        .post(endpoint)
        .header("Authorization", format!("Bearer {key}"))
        .header("Content-Type", "application/json")
        .json(payload)
        .send()
        .await
    {
        Ok(response) => response,
        // A transport failure is load until proven otherwise.
        Err(error) => return Attempt::Retry(format!("network: {error}")),
    };
    let status = response.status().as_u16();
    if status == 200 {
        return match response.json::<Value>().await {
            Ok(value) => Attempt::Answered(value),
            // A malformed body is not a shape the caller can fix by retrying.
            Err(error) => Attempt::Fatal(format!("HTTP 200 with an unreadable body: {error}")),
        };
    }
    let body = response.text().await.unwrap_or_default();
    let excerpt: String = body.chars().take(BODY_CHARS).collect();
    match status {
        401 => Attempt::Fatal(format!(
            "auth rejected (401); check the key. HTTP 401: {excerpt}"
        )),
        422 => Attempt::Fatal(format!(
            "request rejected (422) — question shape bug. HTTP 422: {excerpt}"
        )),
        429 | 529 => Attempt::Retry(format!("HTTP {status}: {excerpt}")),
        status if status >= 500 => Attempt::Retry(format!("HTTP {status}: {excerpt}")),
        status => Attempt::Fatal(format!("HTTP {status}: {excerpt}")),
    }
}

/// Milliseconds since the epoch, for jitter only.
fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0)
}
