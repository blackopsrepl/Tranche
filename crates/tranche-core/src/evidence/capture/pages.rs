//! Validating one page and deciding what it says about completeness.
//!
//! Each collection has its own envelope, so each is read in its own terms:
//! check-runs answer with `{total_count, check_runs}` while the other lists are
//! bare arrays. A terminal cursor on a truncated collection is not completeness,
//! which is why the reported total is recorded and compared against what was
//! actually observed rather than assumed to match.

use serde_json::Value;

use super::super::read::Response;
use super::super::selection::Member;
use super::endpoints::{CHECK_RUN_CAP, FILE_LIST_CAP};

/// What one accepted page says: where to continue, whether the group is done, and
/// why not when it is not.
pub struct Accepted {
    pub next: Option<String>,
    pub complete: bool,
    pub reason: Option<String>,
}

impl Accepted {
    fn continuing(next: String) -> Self {
        Self {
            next: Some(next),
            complete: false,
            reason: None,
        }
    }

    fn done() -> Self {
        Self {
            next: None,
            complete: true,
            reason: None,
        }
    }
}

/// The observed and reported item counts a group accumulates across pages.
#[derive(Debug, Default, Clone)]
pub struct Counts {
    pub observed: u64,
    pub reported: Option<u64>,
}

/// Validate one page of the diff representation.
///
/// An oversized diff is blocked rather than truncated: half a patch presented as a
/// diff would be worse than a named gap, and GitHub itself omits patches for large
/// or unsupported files, which the file list reports separately.
pub fn diff(response: &Response, max_bytes: usize, max_lines: usize) -> Result<Accepted, String> {
    let bytes = response.body.len();
    let lines = response.body.iter().filter(|byte| **byte == b'\n').count();
    if bytes > max_bytes || lines > max_lines {
        return Err(format!(
            "diff exceeds the review bound ({bytes} bytes, {lines} lines)"
        ));
    }
    if response
        .next_url()
        .map_err(|error| error.message)?
        .is_some()
    {
        // A diff is one representation, not a paginated list.
        return Err("the diff answered with a continuation".to_owned());
    }
    Ok(Accepted::done())
}

/// Validate one page of a list component, updating the group's counts.
pub fn list(
    member: &Member,
    component: &str,
    group: &str,
    response: &Response,
    page: u64,
    counts: &mut Counts,
) -> Result<Accepted, String> {
    let payload = response.json().map_err(|error| error.message)?;

    match (component, group) {
        ("metadata", _) => {
            // The metadata response is what proves the capture still describes the
            // revision it claims, so a mismatch is refused here as well as at the
            // live check.
            if !payload.is_object() {
                return Err("metadata response is not this pull request".to_owned());
            }
            if payload.get("number").and_then(Value::as_u64) != Some(member.number) {
                return Err("metadata response is not this pull request".to_owned());
            }
            if payload
                .get("head")
                .and_then(|head| head.get("sha"))
                .and_then(Value::as_str)
                != Some(&member.head_sha)
            {
                return Err("metadata response names a different head revision".to_owned());
            }
            return Ok(Accepted::done());
        }
        ("checks", "statuses" | "fork_statuses") => {
            let statuses = payload
                .as_object()
                .and_then(|object| object.get("statuses"))
                .and_then(Value::as_array)
                .ok_or_else(|| "combined status response carries no status list".to_owned())?;
            counts.observed += statuses.len() as u64;
            if let Some(total) = payload.get("total_count").and_then(Value::as_u64) {
                counts.reported = Some(total);
            }
        }
        ("checks", _) => {
            let runs = payload
                .as_object()
                .and_then(|object| object.get("check_runs"))
                .and_then(Value::as_array)
                .ok_or_else(|| "check-runs response carries no check-run list".to_owned())?;
            counts.observed += runs.len() as u64;
            if let Some(total) = payload.get("total_count").and_then(Value::as_u64) {
                counts.reported = Some(total);
            }
        }
        _ => {
            if !payload.is_array() {
                return Err(format!("{component} response is not a list"));
            }
        }
    }

    match response.next_url().map_err(|error| error.message)? {
        Some(next) => {
            // A continuation that does not advance would loop forever against a
            // live endpoint, so it is treated as no continuation at all.
            if super::super::read::url::page_number(&next).map_err(|error| error.message)? <= page {
                return Err("continuation URL does not advance the page number".to_owned());
            }
            Ok(Accepted::continuing(next))
        }
        None => Ok(Accepted::done()),
    }
}

/// A group that reported more items than it delivered is not complete.
///
/// A terminal cursor says the pagination ended; it does not say the collection was
/// fully observed. Both the reported-versus-observed count and GitHub's own
/// documented ceilings are checked, and either gap is named rather than smoothed
/// over.
pub fn shortfall(component: &str, group: &str, counts: &Counts) -> Option<String> {
    match counts.reported {
        None => None,
        Some(reported) => {
            if counts.observed < reported {
                return Some(format!(
                    "incomplete: GitHub reports {reported} items but {} were observed; \
                     the collection changed or is capped",
                    counts.observed
                ));
            }
            if component == "checks"
                && reported >= CHECK_RUN_CAP
                && (group == "check_runs" || group == "fork_check_runs")
            {
                return Some(format!(
                    "incomplete: GitHub caps check runs at the {CHECK_RUN_CAP} most recent \
                     suites; iterate the suites to see them all"
                ));
            }
            if component == "files" && reported >= FILE_LIST_CAP {
                return Some(format!(
                    "incomplete: GitHub caps the file list at {FILE_LIST_CAP} entries"
                ));
            }
            None
        }
    }
}
