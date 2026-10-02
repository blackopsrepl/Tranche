//! Attempt accounting and the request loop.
//!
//! Every attempt is charged, failures included, because the budget exists to bound
//! what a capture costs rather than what it achieves.

use super::ReadError;
use super::gh_call::{header_of, invoke};
use super::url;

/// The largest response body this transport will keep.
pub const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
/// How long `gh` may take before it is killed.
pub const GH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
/// The most stderr kept for a diagnostic. Overflow is dropped, never fatal.
pub const MAX_STDERR_BYTES: usize = 64 * 1024;
/// Redirects followed before the read is refused.
pub const MAX_REDIRECTS: usize = 3;

/// Statuses this transport follows itself.
const REDIRECT_STATUSES: [u16; 5] = [301, 302, 303, 307, 308];

/// What one request is allowed to spend.
///
/// `reserved` capacity is held back from ordinary requests so work that must not
/// be starved — the final identity checks a capture performs before it certifies
/// anything — can still run. A caller spending that capacity says so with
/// `reserve = Some(0)`.
#[derive(Debug, Clone)]
pub struct Budget {
    pub limit: u64,
    pub reserved: u64,
    pub used: u64,
    pub failures: u64,
    pub identity_checks: u64,
}

impl Budget {
    pub fn new(limit: u64, reserved: u64) -> Result<Self, ReadError> {
        if reserved > limit {
            return Err(ReadError::new(
                "reserved budget must be within the request budget",
            ));
        }
        Ok(Self {
            limit,
            reserved,
            used: 0,
            failures: 0,
            identity_checks: 0,
        })
    }

    /// Attempts left, whatever is reserved.
    pub fn remaining(&self) -> u64 {
        self.limit.saturating_sub(self.used)
    }

    /// Whether one more request fits while keeping `reserve` (the reserved floor
    /// by default).
    pub fn can(&self, reserve: Option<u64>) -> bool {
        let keep = reserve.unwrap_or(self.reserved);
        self.used + 1 + keep <= self.limit
    }

    /// Consume one attempt, keeping `reserve` requests available for later work.
    pub fn charge(&mut self, reserve: Option<u64>) -> Result<(), ReadError> {
        let keep = reserve.unwrap_or(self.reserved);
        if !self.can(Some(keep)) {
            return Err(ReadError::new(format!(
                "request budget exhausted ({}/{}, {keep} held back)",
                self.used, self.limit
            )));
        }
        self.used += 1;
        Ok(())
    }

    pub fn record_failure(&mut self) {
        self.failures += 1;
    }
}

/// One HTTP response: the exact bytes plus the metadata they arrived with.
#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub url: String,
    /// Lowercased header names.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub truncated: bool,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        header_of(&self.headers, name)
    }

    /// The media type without its parameters.
    pub fn media_type(&self) -> String {
        self.header("content-type")
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_owned()
    }

    /// The continuation URL the response advertises, validated.
    pub fn next_url(&self) -> Result<Option<String>, ReadError> {
        url::next_link(self.header("link"))
    }

    /// Parse the body as UTF-8 JSON. A truncated body is never parsed silently.
    pub fn json(&self) -> Result<serde_json::Value, ReadError> {
        if self.truncated {
            return Err(ReadError::too_large(
                "truncated response body cannot be parsed as JSON",
            ));
        }
        serde_json::from_slice(&self.body)
            .map_err(|error| ReadError::new(format!("response body is not UTF-8 JSON ({error})")))
    }
}

/// What one `gh` invocation produced, bounded and possibly incomplete.
pub struct Raw {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub truncated: bool,
    pub hops: usize,
}

/// Read one URL, following validated redirects and charging every attempt.
///
/// The redirects `gh` does not follow are followed here, and each one is validated
/// before it is requested.
pub fn get(
    url: &str,
    accept: Option<&str>,
    mut budget: Option<&mut Budget>,
    reserve: Option<u64>,
    repo_prefix: Option<&str>,
) -> Result<Response, ReadError> {
    let mut attempt_url = url::validate(url, repo_prefix)?;
    let mut hops = 0usize;
    loop {
        if let Some(inner) = budget.as_deref_mut()
            && let Err(error) = inner.charge(reserve)
        {
            return Err(error);
        }
        let mut arguments = vec![
            "api".to_owned(),
            "--include".to_owned(),
            "--method".to_owned(),
            "GET".to_owned(),
            attempt_url.clone(),
        ];
        if let Some(accept) = accept {
            arguments.push("-H".to_owned());
            arguments.push(format!("accept: {accept}"));
        }
        let raw = match invoke(&arguments, None, MAX_RESPONSE_BYTES) {
            Ok(raw) => raw,
            Err(error) => {
                // An oversized response is not a failed request: the remedy is a
                // larger bound, so it is not charged as a failure.
                if !error.too_large
                    && let Some(inner) = budget.as_deref_mut()
                {
                    inner.record_failure();
                }
                return Err(error);
            }
        };
        hops += raw.hops.saturating_sub(1);
        if REDIRECT_STATUSES.contains(&raw.status)
            && let Some(location) = header_of(&raw.headers, "location")
        {
            if hops >= MAX_REDIRECTS {
                return Err(ReadError::new(format!(
                    "too many redirects from {attempt_url}"
                )));
            }
            attempt_url = url::validate(&url::absolute(location, &attempt_url)?, repo_prefix)?;
            continue;
        }
        if raw.status >= 400 {
            if let Some(inner) = budget.as_deref_mut() {
                inner.record_failure();
            }
            return Err(ReadError::new(format!(
                "HTTP {} from {attempt_url}",
                raw.status
            )));
        }
        if raw.truncated {
            return Err(ReadError::too_large(format!(
                "response from {attempt_url} exceeded the {MAX_RESPONSE_BYTES} byte bound"
            )));
        }
        return Ok(Response {
            status: raw.status,
            url: attempt_url,
            headers: raw.headers,
            body: raw.body,
            truncated: false,
        });
    }
}

/// The GraphQL endpoint.
pub const GRAPHQL_URL: &str = "https://api.github.com/graphql";

/// The largest GraphQL request body this transport will send.
pub const MAX_GRAPHQL_BODY_BYTES: usize = 256 * 1024;

/// Run one GraphQL query. Mutations and subscriptions are refused by construction.
///
/// The document travels on stdin rather than in the argv, so the query text never
/// appears in `ps` output and no credential-shaped value can be mistaken for an
/// argument. This is a read path: a document that is not a query, or that mentions
/// a mutation or subscription anywhere, is refused before a request is made.
pub fn graphql(
    query: &str,
    variables: &serde_json::Value,
    mut budget: Option<&mut Budget>,
    reserve: Option<u64>,
) -> Result<Response, ReadError> {
    let trimmed = query.trim_start();
    if query.trim().is_empty() || !(trimmed.starts_with("query") || trimmed.starts_with('{')) {
        return Err(ReadError::new(
            "refusing a GraphQL document that is not a query",
        ));
    }
    let lowered = query.to_ascii_lowercase();
    if lowered.contains("mutation") || lowered.contains("subscription") {
        return Err(ReadError::new(
            "refusing a GraphQL document mentioning mutation/subscription",
        ));
    }
    let body = serde_json::to_vec(&serde_json::json!({
        "query": query,
        "variables": variables,
    }))
    .map_err(|error| ReadError::new(format!("cannot encode the GraphQL request: {error}")))?;
    if body.len() > MAX_GRAPHQL_BODY_BYTES {
        return Err(ReadError::new("GraphQL request body exceeds the bound"));
    }
    if let Some(inner) = budget.as_deref_mut()
        && let Err(error) = inner.charge(reserve)
    {
        return Err(error);
    }
    let arguments = vec![
        "api".to_owned(),
        "--include".to_owned(),
        "--method".to_owned(),
        "POST".to_owned(),
        GRAPHQL_URL.to_owned(),
        "--input".to_owned(),
        "-".to_owned(),
    ];
    let raw = match invoke(&arguments, Some(&body), MAX_RESPONSE_BYTES) {
        Ok(raw) => raw,
        Err(error) => {
            if !error.too_large
                && let Some(inner) = budget.as_deref_mut()
            {
                inner.record_failure();
            }
            return Err(error);
        }
    };
    if raw.status >= 400 {
        if let Some(inner) = budget {
            inner.record_failure();
        }
        return Err(ReadError::new(format!(
            "HTTP {} from {GRAPHQL_URL}",
            raw.status
        )));
    }
    if raw.truncated {
        return Err(ReadError::too_large(format!(
            "GraphQL response exceeded the {MAX_RESPONSE_BYTES} byte bound"
        )));
    }
    Ok(Response {
        status: raw.status,
        url: GRAPHQL_URL.to_owned(),
        headers: raw.headers,
        body: raw.body,
        truncated: false,
    })
}
