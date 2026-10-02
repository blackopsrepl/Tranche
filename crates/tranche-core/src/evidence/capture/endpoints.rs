//! Where each component is read from, and how its pages are addressed.
//!
//! The component map is the whole reason a capture is checkable: a reviewer asking
//! "was the CI observed?" gets an answer that names the repository the answer came
//! from, rather than an absence one source could not have seen.

use serde_json::Value;

use super::super::read::transport::GRAPHQL_URL;
use super::super::read::url;
use super::super::selection::Member;

/// The media type that returns a patch rather than JSON.
pub const DIFF_ACCEPT: &str = "application/vnd.github.diff";
/// The media type the other REST components ask for.
pub const JSON_MEDIA: &str = "application/vnd.github+json";
/// The page size every list request asks for.
///
/// It is always sent rather than assumed: GitHub's default is 30, and a page whose
/// size we do not control cannot be reasoned about.
pub const PER_PAGE: u64 = 100;
/// GitHub's own ceiling on a check-run collection.
pub const CHECK_RUN_CAP: u64 = 1000;
/// GitHub's own ceiling on a file list.
pub const FILE_LIST_CAP: u64 = 3000;

/// The base repository's CI groups, which every member has.
pub const BASE_CI_GROUPS: [&str; 2] = ["check_runs", "statuses"];
/// The fork's CI groups, present only when the head repository differs.
pub const FORK_CI_GROUPS: [&str; 2] = ["fork_check_runs", "fork_statuses"];

/// The groups one component is made of, for this member.
///
/// `checks` is the one component whose groups depend on the member: a PR's own
/// checks live in its base repository, and a fork's head can carry its own.
pub fn groups_for(member: &Member, component: &str) -> Vec<String> {
    if component == "checks" {
        return ci_targets(member)
            .into_iter()
            .map(|(group, _, _)| group)
            .collect();
    }
    vec![component.to_owned()]
}

/// Every repository whose CI is worth reading for this member, and which groups
/// come from it.
pub fn ci_targets(member: &Member) -> Vec<(String, String, u64)> {
    let mut targets: Vec<(String, String, u64)> = BASE_CI_GROUPS
        .iter()
        .map(|group| {
            (
                (*group).to_owned(),
                member.base_repo_name.clone(),
                member.base_repo_id,
            )
        })
        .collect();
    if member.head_repo_name != member.base_repo_name {
        targets.extend(FORK_CI_GROUPS.iter().map(|group| {
            (
                (*group).to_owned(),
                member.head_repo_name.clone(),
                member.head_repo_id,
            )
        }));
    }
    targets
}

/// The repository a CI group is read from: the PR's own, or the linked fork.
pub fn ci_repo(member: &Member, group: &str) -> String {
    if FORK_CI_GROUPS.contains(&group) {
        member.head_repo_name.clone()
    } else {
        member.base_repo_name.clone()
    }
}

/// That repository's numeric identity.
pub fn ci_repo_id(member: &Member, group: &str) -> u64 {
    if FORK_CI_GROUPS.contains(&group) {
        member.head_repo_id
    } else {
        member.base_repo_id
    }
}

/// The repository a component's URL must stay inside.
pub fn repo_prefix(member: &Member, component: &str, group: &str) -> String {
    if component == "checks" {
        ci_repo(member, group)
    } else {
        member.base_repo_name.clone()
    }
}

/// The first URL of a component's group.
pub fn source_url(member: &Member, component: &str, group: &str) -> String {
    let base = &member.base_repo_name;
    let number = member.number;
    match component {
        // Metadata and the diff are the same resource read as two
        // representations: one JSON, one patch.
        "metadata" | "diff" => format!("{}/repos/{base}/pulls/{number}", url::ORIGIN),
        "files" => format!("{}/repos/{base}/pulls/{number}/files", url::ORIGIN),
        "discussion" => format!("{}/repos/{base}/issues/{number}/comments", url::ORIGIN),
        "review_comments" => format!("{}/repos/{base}/pulls/{number}/comments", url::ORIGIN),
        "reviews" => format!("{}/repos/{base}/pulls/{number}/reviews", url::ORIGIN),
        "checks" => {
            let suffix = if group.ends_with("check_runs") {
                "check-runs"
            } else {
                "status"
            };
            // Check runs are read at the head revision, which is the revision whose
            // CI the PR is claiming.
            format!(
                "{}/repos/{}/commits/{}/{suffix}",
                url::ORIGIN,
                ci_repo(member, group),
                member.head_sha
            )
        }
        "closing_issues" => GRAPHQL_URL.to_owned(),
        other => format!("{}/repos/{base}/pulls/{number}/{other}", url::ORIGIN),
    }
}

/// The representation a component is read as.
pub fn accept_for(component: &str) -> &'static str {
    if component == "diff" {
        DIFF_ACCEPT
    } else {
        JSON_MEDIA
    }
}

/// Pagination parameters for a group. Single-object groups take none.
pub fn list_params(component: &str, group: &str, page: u64) -> Vec<(String, String)> {
    match component {
        "metadata" | "diff" => Vec::new(),
        // The combined status is one object, not a list.
        "checks" if group == "statuses" || group == "fork_statuses" => Vec::new(),
        _ => vec![
            ("per_page".to_owned(), PER_PAGE.to_string()),
            ("page".to_owned(), page.to_string()),
        ],
    }
}

/// The closing-issues query.
///
/// It carries a `totalCount` so the observed-versus-reported gap can be checked, and
/// a cursor so pagination is explicit rather than assumed complete.
pub fn closing_query() -> &'static str {
    // A single literal, not line-continuations: a swallowed space between tokens
    // becomes `idpullRequest`, and GitHub refuses the whole query.
    "query trancheClosingIssues($owner:String!,$name:String!,$number:Int!,$cursor:String){ repository(owner:$owner,name:$name){ nameWithOwner id pullRequest(number:$number){ number url closingIssuesReferences(first:100,after:$cursor){ totalCount pageInfo{hasNextPage endCursor} nodes{number url state title} } } } }"
}

/// The variables for one page of the closing-issues query.
pub fn closing_variables(member: &Member, cursor: Option<&str>) -> Value {
    let (owner, name) = member
        .base_repo_name
        .split_once('/')
        .unwrap_or((member.base_repo_name.as_str(), ""));
    serde_json::json!({
        "owner": owner,
        "name": name,
        "number": member.number,
        "cursor": cursor,
    })
}

/// Read one closing-issues page, or refuse a malformed or foreign answer.
///
/// Returns the next cursor and the observed item count. The query can answer for
/// the wrong repository or pull request when an endpoint misbehaves, so both are
/// checked rather than trusted.
pub fn parse_closing(payload: &Value, member: &Member) -> Result<(Option<String>, u64), String> {
    if !payload.is_object() {
        return Err("closing-issues query returned a non-object".to_owned());
    }
    if let Some(errors) = payload.get("errors").filter(|errors| !errors.is_null()) {
        return Err(format!("closing-issues query reported errors: {errors}"));
    }
    let repository = payload
        .get("data")
        .and_then(|data| data.get("repository"))
        .ok_or_else(|| "closing-issues query returned no repository".to_owned())?;
    if repository.get("nameWithOwner").and_then(Value::as_str) != Some(&member.base_repo_name) {
        return Err(format!(
            "closing-issues query answered for {:?}, not {:?}",
            repository
                .get("nameWithOwner")
                .and_then(Value::as_str)
                .unwrap_or(""),
            member.base_repo_name
        ));
    }
    let pull = repository
        .get("pullRequest")
        .ok_or_else(|| "closing-issues query answered about no pull request".to_owned())?;
    if pull.get("number").and_then(Value::as_u64) != Some(member.number) {
        return Err("closing-issues query answered about a different pull request".to_owned());
    }
    let connection = pull
        .get("closingIssuesReferences")
        .ok_or_else(|| "closing-issues query returned no connection".to_owned())?;
    let page_info = connection.get("pageInfo").cloned().unwrap_or(Value::Null);
    let nodes = connection
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| "closing-issues query returned no nodes".to_owned())?;
    let next = if page_info.get("hasNextPage").and_then(Value::as_bool) == Some(true) {
        page_info
            .get("endCursor")
            .and_then(Value::as_str)
            .map(str::to_owned)
    } else {
        None
    };
    Ok((next, nodes.len() as u64))
}
