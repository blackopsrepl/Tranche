//! The checks a response must pass, and the small URL helpers.

use serde_json::Value;

use super::super::read::Response;
use super::super::selection::Member;
use super::endpoints::{ci_repo, ci_repo_id, list_params};

/// A CI response must name the repository and revision that was asked about.
pub(crate) fn verify_ci_scope(
    member: &Member,
    group: &str,
    response: &Response,
) -> Result<(), String> {
    let payload = response.json().map_err(|error| error.message)?;
    let expected_repo = ci_repo(member, group);
    let expected_id = ci_repo_id(member, group);
    if let Some(repo) = payload.get("repository").filter(|repo| repo.is_object()) {
        if repo.get("full_name").and_then(Value::as_str) != Some(&expected_repo) {
            return Err(format!(
                "CI for #{} came from {:?}, not {:?}",
                member.number,
                repo.get("full_name").and_then(Value::as_str).unwrap_or(""),
                expected_repo
            ));
        }
        if let Some(id) = repo.get("id").and_then(Value::as_u64)
            && id != expected_id
        {
            return Err(format!(
                "CI for #{} came from repository id {id}, not {expected_id}",
                member.number
            ));
        }
    }
    if let Some(sha) = payload.get("sha").and_then(Value::as_str)
        && sha != member.head_sha
    {
        return Err(format!(
            "CI for #{} names revision {sha}, not {}",
            member.number, member.head_sha
        ));
    }
    for run in payload
        .get("check_runs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if !run.is_object() {
            return Err(format!(
                "CI for #{} returned a malformed check run",
                member.number
            ));
        }
        if let Some(sha) = run.get("head_sha").and_then(Value::as_str)
            && sha != member.head_sha
        {
            return Err(format!(
                "a check run for #{} names revision {sha}",
                member.number
            ));
        }
    }
    Ok(())
}
/// Append pagination parameters to a URL.
pub(crate) fn with_params(at: &str, component: &str, group: &str, page: u64) -> String {
    let params = list_params(component, group, page);
    if params.is_empty() {
        return at.to_owned();
    }
    let query: Vec<String> = params
        .into_iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect();
    let separator = if at.contains('?') { '&' } else { '?' };
    format!("{at}{separator}{}", query.join("&"))
}
/// Whether a transport message is the budget running out rather than a failure.
pub(crate) fn is_exhausted(message: &str) -> bool {
    message.contains("budget exhausted")
}
