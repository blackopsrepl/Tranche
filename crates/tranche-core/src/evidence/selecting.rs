//! Resolving a batch id into the selection a capture binds to.
//!
//! Every refusal here is a refusal to attach evidence to the wrong thing: an unknown
//! ordinal, a batch the report does not contain, a batch that lists parked PRs, a
//! member whose revisions or repository identities are unavailable, or members that
//! do not share one base repository. Attaching a capture to the wrong batch would
//! produce evidence that is internally consistent and describes the wrong work, which
//! is worse than no evidence at all.

use serde_json::Value;

use super::errors::SelectionError;
use super::selection::{Member, Selection, is_batch_id};
use crate::domain::pr::Prs;
use crate::report::Root;

/// One member's revision identity, taken from the captured item rather than inferred.
struct Revision {
    member: Member,
}

/// The batch plan and park record a selection is checked against.
///
/// Owns its data because it is assembled from files read at the call site and then
/// used after those bindings could move.
pub struct Plan {
    pub batches: Option<Value>,
    pub parked: Option<Value>,
    pub binding: Option<String>,
    pub clusters_digest: Option<String>,
    pub dupes_digest: Option<String>,
    pub batches_digest: Option<String>,
    pub parked_digest: Option<String>,
}

impl Plan {
    /// Read the plan from the report on disk.
    pub fn read(root: &Root) -> Result<Self, SelectionError> {
        let read = |path: std::path::PathBuf, name: &str| -> Result<Value, SelectionError> {
            let text = std::fs::read_to_string(&path)
                .map_err(|error| SelectionError(format!("{name}: {error}")))?;
            serde_json::from_str(&text).map_err(|error| SelectionError(format!("{name}: {error}")))
        };
        let summary = read(root.summary_path(), "summary.json")?;
        let digests = summary["output_digests"].clone();
        let text = |name: &str| digests[name].as_str().map(str::to_owned);
        Ok(Self {
            batches: read(root.batches_path(), "batches.json").ok(),
            parked: read(root.parked_path(), "parked.json").ok(),
            binding: summary["report_binding"].as_str().map(str::to_owned),
            clusters_digest: text("clusters.json"),
            dupes_digest: text("dupes.json"),
            batches_digest: text("batches.json"),
            parked_digest: text("parked.json"),
        })
    }
}

/// Resolve one batch through the bound report, or refuse.
pub fn select(
    root: &Root,
    batch_id: &str,
    corpus: &Prs,
    plan: &Plan,
) -> Result<Selection, SelectionError> {
    if !is_batch_id(batch_id) {
        return Err(SelectionError(
            "expected an exact batch id such as B001".to_owned(),
        ));
    }
    let Some(batches) = plan.batches.as_ref() else {
        return Err(SelectionError(
            "batches.json is unavailable; run `tranche batches`".to_owned(),
        ));
    };
    let all = batches["batches"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let Some(batch) = all
        .iter()
        .find(|batch| batch["id"].as_str() == Some(batch_id))
    else {
        return Err(SelectionError(format!(
            "unknown batch id {batch_id}; the current report has {} batches",
            all.len()
        )));
    };
    let members_of_batch: Vec<u64> = batch["members"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_u64)
        .collect();

    // A batch that lists parked PRs means the plan and the park record disagree,
    // and the disagreement is the report's, not something to work around here.
    let parked: Vec<u64> = plan
        .parked
        .as_ref()
        .and_then(|parked| parked["members"].as_array())
        .into_iter()
        .flatten()
        .filter_map(|member| member["number"].as_u64())
        .collect();
    let inconsistent: Vec<u64> = members_of_batch
        .iter()
        .copied()
        .filter(|number| parked.contains(number))
        .collect();
    if !inconsistent.is_empty() {
        let listed = inconsistent
            .iter()
            .map(|number| format!("#{number}"))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(SelectionError(format!(
            "batch {batch_id} lists parked PRs {listed}; the batch plan and the park \
             record disagree - rerun `tranche batches`"
        )));
    }

    let items = captured_items(root)?;
    let mut members: Vec<Member> = Vec::with_capacity(members_of_batch.len());
    for number in &members_of_batch {
        let revision = revision(*number, &items, corpus)?;
        members.push(revision.member);
    }
    let Some(first) = members.first() else {
        return Err(SelectionError(format!("batch {batch_id} has no members")));
    };

    let repository = serde_json::json!({
        "id": first.base_repo_id,
        "full_name": first.base_repo_name,
        // Visibility is checked live at export rather than assumed here, because a
        // capture records what it read, not what it was permitted to share.
        "visibility": "unknown",
    });
    if members.iter().any(|member| {
        member.base_repo_id != first.base_repo_id || member.base_repo_name != first.base_repo_name
    }) {
        return Err(SelectionError(
            "batch members do not share one base repository".to_owned(),
        ));
    }

    let report = serde_json::json!({
        "report_binding": plan.binding,
        "output_digests": {
            "clusters.json": plan.clusters_digest,
            "dupes.json": plan.dupes_digest,
            "batches.json": plan.batches_digest,
            "parked.json": plan.parked_digest,
        },
    });

    let ordinal = batch["ordinal"].as_u64().unwrap_or(0);
    let prompt = batch["review_prompt"].as_str().unwrap_or("").to_owned();
    Ok(Selection {
        repository,
        report,
        batch_id: batch_id.to_owned(),
        batch_ordinal: ordinal,
        prompt,
        members,
    })
}

/// The captured raw items, keyed by number.
///
/// The revision identity is read from the captured item, because the projection the
/// report carries has already dropped the repository objects the identity needs.
fn captured_items(root: &Root) -> Result<std::collections::HashMap<u64, Value>, SelectionError> {
    let snapshot = root.snapshot_path();
    if snapshot.exists() {
        let text = std::fs::read_to_string(&snapshot)
            .map_err(|error| SelectionError(format!("cannot read the capture: {error}")))?;
        let value: Value = serde_json::from_str(&text)
            .map_err(|error| SelectionError(format!("the capture is unreadable: {error}")))?;
        if let Some(items) = value.get("items").and_then(Value::as_array) {
            return Ok(index(items));
        }
    }
    let mut items = Vec::new();
    for path in crate::report::read_page_paths(&root.pages_dir()) {
        let text = std::fs::read_to_string(&path)
            .map_err(|error| SelectionError(format!("cannot read {}: {error}", path.display())))?;
        let value: Value = serde_json::from_str(&text).map_err(|error| {
            SelectionError(format!("{} is unreadable: {error}", path.display()))
        })?;
        if let Some(page) = value.as_array() {
            items.extend(page.iter().cloned());
        }
    }
    Ok(index(&items))
}

fn index(items: &[Value]) -> std::collections::HashMap<u64, Value> {
    items
        .iter()
        .filter_map(|item| {
            item.get("number")
                .and_then(Value::as_u64)
                .map(|n| (n, item.clone()))
        })
        .collect()
}

/// Full base/head identity for one member, or refuse the selection.
///
/// A short or missing revision is never substituted: the whole point of the code
/// observation is that it names an exact pair of commits.
fn revision(
    number: u64,
    items: &std::collections::HashMap<u64, Value>,
    corpus: &Prs,
) -> Result<Revision, SelectionError> {
    let item = items.get(&number).ok_or_else(|| {
        SelectionError(format!(
            "#{number} is not in the captured membership; fetch again"
        ))
    })?;
    let head = item.get("head").cloned().unwrap_or(Value::Null);
    let base = item.get("base").cloned().unwrap_or(Value::Null);
    let (head_repo, base_repo) = (
        head.get("repo").cloned().unwrap_or(Value::Null),
        base.get("repo").cloned().unwrap_or(Value::Null),
    );
    if !head_repo.is_object() || !base_repo.is_object() {
        return Err(SelectionError(format!(
            "#{number} no longer exposes its base/head repository (a deleted fork cannot \
             be captured); the selection cannot bind this member"
        )));
    }
    let sha = |value: &Value, name: &str| -> Result<String, SelectionError> {
        let text = value.get("sha").and_then(Value::as_str).unwrap_or("");
        if text.len() == 40 && text.chars().all(|c| c.is_ascii_hexdigit()) {
            Ok(text.to_owned())
        } else {
            Err(SelectionError(format!(
                "#{number} has no full 40-character {name}; fetch again"
            )))
        }
    };
    let repo_id = |value: &Value, name: &str| -> Result<u64, SelectionError> {
        match value.get("id").and_then(Value::as_u64) {
            Some(id) if id > 0 => Ok(id),
            _ => Err(SelectionError(format!(
                "#{number} has no usable {name}; fetch again"
            ))),
        }
    };
    let repo_name = |value: &Value| {
        value
            .get("full_name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned()
    };

    let pr = corpus.get(number).ok_or_else(|| {
        SelectionError(format!("#{number} is missing from the report projection"))
    })?;
    Ok(Revision {
        member: Member {
            number,
            source_digest: pr.source_digest.clone(),
            evidence_digest: pr.evidence_digest.clone(),
            base_sha: sha(&base, "base_sha")?,
            head_sha: sha(&head, "head_sha")?,
            base_repo_id: repo_id(&base_repo, "base_repo_id")?,
            base_repo_name: repo_name(&base_repo),
            head_repo_id: repo_id(&head_repo, "head_repo_id")?,
            head_repo_name: repo_name(&head_repo),
            head_fork: head_repo
                .get("fork")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            updated_at: item
                .get("updated_at")
                .and_then(Value::as_str)
                .map(str::to_owned),
        },
    })
}
