//! Reading GitHub's open-PR list, with the credential staying inside `gh`.
//!
//! `gh api` is preferred because it is already authenticated: unauthenticated
//! GitHub allows 60 requests an hour and one capture of this backlog costs about
//! 29, so an unauthenticated refresh fails on its third pass. The token is never
//! read by this process and never placed in an argv, where it would be visible to
//! every user on the host.
//!
//! A failed or partial capture reports and leaves the previous snapshot alone.
//! Nothing here writes.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use serde_json::Value;

/// The GitHub REST base.
pub const API: &str = "https://api.github.com";

/// How one page is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Transport {
    /// The authenticated `gh` CLI.
    #[default]
    Gh,
    /// The `curl` binary.
    Curl,
    /// The standard library HTTP client.
    Urllib,
}

impl Transport {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Gh => "gh",
            Self::Curl => "curl",
            Self::Urllib => "urllib",
        }
    }
}

#[derive(Debug)]
pub struct GitHubError(pub String);

impl std::fmt::Display for GitHubError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for GitHubError {}

impl From<String> for GitHubError {
    fn from(message: String) -> Self {
        Self(message)
    }
}

/// One page of the open-PR list.
///
/// The body must be a JSON array. Anything else is a refusal rather than an empty
/// page, because an error document read as "no more PRs" would silently truncate
/// the corpus.
pub fn page(transport: Transport, url: &str) -> Result<Vec<Value>, GitHubError> {
    page_with(transport, url, None)
}

/// One page, optionally resolving the command through one directory.
///
/// The search path is a parameter rather than global state: a caller that wants
/// a specific `gh` says so, and a test cannot change what a test beside it sees.
pub fn page_with(
    transport: Transport,
    url: &str,
    command_dir: Option<&std::path::Path>,
) -> Result<Vec<Value>, GitHubError> {
    let body = match transport {
        Transport::Gh => run(
            "gh",
            &["api", url],
            Duration::from_secs(120),
            "gh api fetch failed",
            command_dir,
        )?,
        Transport::Curl => run(
            "curl",
            &[
                "--fail",
                "--silent",
                "--show-error",
                "--max-time",
                "60",
                url,
            ],
            Duration::from_secs(90),
            "curl fetch failed",
            command_dir,
        )?,
        // The one transport with no external binary: useful for a caller that has
        // no `gh` and a token in the environment, and the only one that needs no
        // subprocess at all.
        Transport::Urllib => return via_http(url),
    };
    let parsed: Value = serde_json::from_str(&body)
        .map_err(|error| GitHubError(format!("GitHub returned unreadable JSON: {error}")))?;
    match parsed {
        Value::Array(items) => Ok(items),
        _ => Err(GitHubError(
            "GitHub did not return a PR list; the previous snapshot is retained".to_owned(),
        )),
    }
}

/// Run one command to completion and return its stdout.
///
/// The last line of stderr is what the operator needs; the whole thing is usually
/// a progress line plus a cause.
fn run(
    program: &str,
    arguments: &[&str],
    timeout: Duration,
    context: &str,
    command_dir: Option<&std::path::Path>,
) -> Result<String, GitHubError> {
    let program_path = match command_dir {
        Some(directory) => directory.join(program),
        None => PathBuf::from(program),
    };
    let mut child = Command::new(&program_path)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| GitHubError(format!("{context}: cannot run {program}: {error}")))?;

    // A hung fetch must not hang the pipeline. `gh` and `curl` both take their
    // own timeouts, so this is the backstop.
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(GitHubError(format!(
                    "{context}: timed out after {timeout:?}"
                )));
            }
            Err(error) => {
                return Err(GitHubError(format!("{context}: {error}")));
            }
        }
    }
    let output = child
        .wait_with_output()
        .map_err(|error| GitHubError(format!("{context}: {error}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("no detail");
        return Err(GitHubError(format!(
            "{context} ({detail}); the previous snapshot is retained"
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Read a page over HTTP, for a caller with no `gh`.
///
/// A token is used only when the environment carries one; without it this is the
/// unauthenticated path the module documents as limited.
fn via_http(url: &str) -> Result<Vec<Value>, GitHubError> {
    let blocking = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60))
        .user_agent("tranche")
        .build()
        .map_err(|error| GitHubError(format!("cannot build the HTTP client: {error}")))?;
    let mut request = blocking
        .get(url)
        .header("Accept", "application/vnd.github+json");
    if let Ok(token) = std::env::var("GITHUB_TOKEN")
        && !token.trim().is_empty()
    {
        request = request.bearer_auth(token.trim());
    }
    let response = request
        .send()
        .map_err(|error| GitHubError(format!("network: {error}")))?;
    let status = response.status().as_u16();
    if status != 200 {
        return Err(GitHubError(format!("HTTP {status} from GitHub")));
    }
    response
        .json::<Value>()
        .map_err(|error| GitHubError(format!("GitHub returned unreadable JSON: {error}")))
        .and_then(|parsed| match parsed {
            Value::Array(items) => Ok(items),
            _ => Err(GitHubError(
                "GitHub did not return a PR list; the previous snapshot is retained".to_owned(),
            )),
        })
}
