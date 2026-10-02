//! Turning a captured item into what the pipeline works with.
//!
//! The evidence digest covers only the fields a judgment depends on. Everything
//! else in a GitHub pull-list envelope churns without the PR changing:
//! repository-wide counters, URL scaffolding and nested remote objects all move
//! on unrelated events, so digesting the whole envelope made every PR look
//! changed after every fetch.

use regex::Regex;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

use super::error::PrError;
use super::invalidating::invalid;
use super::record::{BODY_CHARS, Pr, Prs};
use super::references::reference_numbers;
use crate::util::digest;

/// Digest of the captured fields a judgment actually depends on.
///
/// The whole envelope churns without the PR changing: repository-wide counters,
/// URL scaffolding and nested remote objects all move on unrelated events.
pub fn pr_evidence_digest(item: &Value) -> String {
    const FIELDS: [&str; 21] = [
        "number",
        "title",
        "body",
        "created_at",
        "updated_at",
        "state",
        "draft",
        "labels",
        "milestone",
        "requested_reviewers",
        "requested_teams",
        "changed_files",
        "additions",
        "deletions",
        "commits",
        "comments",
        "review_comments",
        "merged_at",
        "merge_commit_sha",
        "user",
        "head",
    ];
    let Some(object) = item.as_object() else {
        return digest(&Value::Object(Map::new()));
    };
    let mut evidence = Map::new();
    for field in FIELDS {
        let Some(value) = object.get(field) else {
            continue;
        };
        let projected = match field {
            "user" => project_object(value, &["login"]),
            "head" => project_object(value, &["sha", "ref", "label"]),
            "labels" => project_labels(value),
            _ => value.clone(),
        };
        evidence.insert(field.to_owned(), projected);
    }
    digest(&Value::Object(evidence))
}

fn project_object(value: &Value, keep: &[&str]) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            keep.iter()
                .filter(|key| object.contains_key(**key))
                .map(|key| ((*key).to_owned(), object[*key].clone()))
                .collect(),
        ),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| match item {
                    Value::Object(object) => Value::Object(
                        keep.iter()
                            .filter(|key| object.contains_key(**key))
                            .map(|key| ((*key).to_owned(), object[*key].clone()))
                            .collect(),
                    ),
                    other => other.clone(),
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

fn project_labels(value: &Value) -> Value {
    match value.as_array() {
        Some(items) => Value::Array(
            items
                .iter()
                .map(
                    |label| match label.as_object().and_then(|o| o.get("name")) {
                        Some(name) => name.clone(),
                        None => label.clone(),
                    },
                )
                .collect(),
        ),
        None => value.clone(),
    }
}

/// Project one captured item into the fields the pipeline works with.
pub(super) fn project(item: &Value, repository: &str) -> Pr {
    let object = item.as_object().expect("validated before projection");
    let number = object["number"]
        .as_u64()
        .expect("validated as a positive int");
    let raw_body = object.get("body").and_then(Value::as_str).unwrap_or("");
    let refs = reference_numbers(raw_body, repository, Some(number));
    let body = clean_body(raw_body);
    let text = |key: &str| {
        object
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let nested = |key: &str, field: &str| {
        object
            .get(key)
            .and_then(Value::as_object)
            .and_then(|value| value.get(field))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    let size = |key: &str| object.get(key).and_then(Value::as_i64);
    let author = nested("user", "login")
        .or_else(|| nested("author", "login"))
        .unwrap_or_else(|| "unknown".to_owned());
    let url = text("html_url");
    Pr {
        number,
        title: text("title").trim().to_owned(),
        body_truncated: body.chars().count() > BODY_CHARS,
        body: body.chars().take(BODY_CHARS).collect(),
        author,
        created: text("created_at"),
        updated: text("updated_at"),
        draft: object
            .get("draft")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        files: size("changed_files"),
        additions: size("additions"),
        deletions: size("deletions"),
        labels: object
            .get("labels")
            .and_then(Value::as_array)
            .map(|labels| {
                labels
                    .iter()
                    .filter_map(|label| {
                        label
                            .as_object()
                            .and_then(|value| value.get("name"))
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                    })
                    .collect()
            })
            .unwrap_or_default(),
        refs: refs.clone(),
        head_sha: nested("head", "sha"),
        url: if url.is_empty() {
            format!("https://github.com/{repository}/pull/{number}")
        } else {
            url
        },
        source_digest: digest(item),
        evidence_digest: pr_evidence_digest(item),
        ref_digest: digest(&serde_json::json!({ "references": refs })),
    }
}

/// Strip comments, images and bare links, then collapse whitespace.
///
/// The judged body is prose: markup and URLs are noise the model should not be
/// charged for.
fn clean_body(raw: &str) -> String {
    static COMMENT: OnceLock<Regex> = OnceLock::new();
    static IMAGE: OnceLock<Regex> = OnceLock::new();
    static LINK: OnceLock<Regex> = OnceLock::new();
    static SPACE: OnceLock<Regex> = OnceLock::new();
    let comment = COMMENT.get_or_init(|| Regex::new(r"(?s)<!--.*?-->").expect("static pattern"));
    let image = IMAGE.get_or_init(|| Regex::new(r"!\[[^\]]*\]\([^)]*\)").expect("static pattern"));
    let link = LINK.get_or_init(|| Regex::new(r"https?://\S+").expect("static pattern"));
    let space = SPACE.get_or_init(|| Regex::new(r"\s+").expect("static pattern"));
    let text = comment.replace_all(raw, "");
    let text = image.replace_all(&text, "");
    let text = link.replace_all(&text, "");
    space.replace_all(&text, " ").trim().to_owned()
}

/// PR number → the references it contributes as comparison candidates.
pub fn ref_index(prs: &Prs) -> HashMap<u64, Vec<u64>> {
    prs.iter().map(|pr| (pr.number, pr.refs.clone())).collect()
}

/// The state handed to the model for one PR.
pub fn pr_state(pr: &Pr) -> Value {
    let sizes = [pr.files, pr.additions, pr.deletions];
    let known = sizes
        .iter()
        .all(|size| size.is_some_and(|value| value >= 0));
    let diffstat = match (pr.files, pr.additions, pr.deletions) {
        (Some(files), Some(additions), Some(deletions)) if known => {
            format!("{files} files changed, +{additions}/-{deletions}")
        }
        _ => "unknown (not supplied by the captured PR list)".to_owned(),
    };
    serde_json::json!({
        "pr": {
            "title": pr.title,
            "body": if pr.body.is_empty() { "(empty body)".to_owned() } else { pr.body.clone() },
            "author": pr.author,
            "diffstat": diffstat,
            "draft": pr.draft,
            "evidence_basis": "title and shortened description; patches, CI and reproduction results not verified",
            "diffstat_available": known,
            "body_truncated": pr.body_truncated,
        }
    })
}

/// Read a JSON file, or report that the report inputs are unusable.
pub fn read_json(path: &Path) -> Result<Value, PrError> {
    let text = std::fs::read_to_string(path).map_err(invalid)?;
    serde_json::from_str(&text).map_err(invalid)
}
