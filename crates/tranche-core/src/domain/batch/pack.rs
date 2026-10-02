//! The size of a tranche, the activity a batch is ordered by, and the reviewer
//! prompt each batch carries.

use serde_json::{Value, json};

use crate::domain::pr::{Pr, Prs};

/// PRs merged together as one tranche.
pub const BATCH_SIZE: usize = 5;

/// Whether a PR's head has moved since it was judged, and how long it has been quiet.
///
/// Head evidence is distinct from GitHub thread churn, including bots. A matching
/// recorded head establishes an idle lower bound at judgment time; an unjudged PR
/// has only its creation date as an age proxy, and `updated_at` is thread
/// metadata rather than priority.
pub fn pr_activity(pr: &Pr, record: Option<&Value>) -> Value {
    let bound_head = record
        .and_then(|record| record.get("head_sha"))
        .and_then(Value::as_str);
    let head_moved = bound_head
        .zip(pr.head_sha.as_deref())
        .is_some_and(|(bound, current)| bound != current);
    let matched = bound_head
        .zip(pr.head_sha.as_deref())
        .is_some_and(|(bound, current)| bound == current);
    let judged_at = record
        .and_then(|record| record.get("judged_at"))
        .and_then(Value::as_str);
    let idle_since = if head_moved {
        None
    } else if matched {
        judged_at.map(str::to_owned)
    } else {
        Some(pr.created.clone())
    };
    let basis = if head_moved || idle_since.is_none() {
        "unknown"
    } else if matched && judged_at.is_some() {
        "judgment"
    } else {
        "creation"
    };
    json!({
        "head_moved": head_moved,
        "idle_since": idle_since,
        "idle_basis": basis,
        "thread_updated": pr.updated,
    })
}

/// The reviewer prompt for one batch.
///
/// Deterministic, self-contained text: it points at every PR of the batch and
/// demands one unified proposal rather than per-PR verdicts.
pub fn batch_review_prompt(members: &[u64], batch_id: &str, prs: &Prs, subject: &str) -> String {
    let listed = members
        .iter()
        .filter_map(|number| prs.get(*number))
        .map(|pr| format!("- #{}: {} — {}", pr.number, pr.title, pr.url))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "You are reviewing {subject} pre-release batch {batch_id} ({} PRs to be merged together as one tranche).\n\n\
         Pull requests in this batch:\n{listed}\n\n\
         Work through the batch methodically:\n\
         1. Read every PR fully — description, diff, and review comments. For PRs Jev flagged as the same change, verify they truly overlap and identify the strongest implementation of each.\n\
         2. Map dependencies between the PRs (shared files, ordering constraints, conflicts) and check each PR's CI status.\n\
         3. Produce ONE unified proposal for the batch: what merges, in which order, what gets squashed or dropped, and why — as a single coherent plan, not per-PR verdicts.\n\
         4. Verify the plan: does the combined result still build and pass tests? Any PR that cannot be verified stays out — say so explicitly.\n\
         5. Deliver: (a) the unified proposal, (b) a step-by-step merge plan with exact commands, (c) risks with mitigations, (d) an explicit list of anything excluded and why.\n\n\
         Facts over plausibility: base every claim on the actual diffs and CI state, never on titles alone. You are proposing — the human decides.",
        members.len()
    )
}
