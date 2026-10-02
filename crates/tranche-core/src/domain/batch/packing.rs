//! Packing PRs into security-first batches of five.
//!
//! A batch claims exactly one thing: these PRs merge together as one tranche.
//! That claim is why a parked PR never appears in one, and why a same-change
//! group is atomic — pulling out one member would leave the rest of the group
//! claimed twice or held entirely.

use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

use super::pack::{BATCH_SIZE, batch_review_prompt, pr_activity};
use super::park::park_state;
use crate::domain::cluster::SECURITY_PRIORITY;
use crate::domain::judge::Judgment;
use crate::domain::pr::Prs;
use crate::policy::Contract;

/// Pack PRs into security-first batches of five.
///
/// The deployment's contract supplies the repository the batches bind to and
/// the display name the reviewer prompts address.
pub fn merge_batches(
    dupes: &Value,
    judgments: &HashMap<u64, Judgment>,
    prs: &Prs,
    dupes_digest: &str,
    contract: &Contract,
) -> Result<Value, String> {
    let repository = contract.repository();
    let subject = contract.batch_subject();
    let parks = park_state(dupes, judgments, prs);

    let security_count = |members: &[u64]| -> u64 {
        members
            .iter()
            .filter(|number| {
                judgments
                    .get(number)
                    .and_then(|judgment| judgment.metric("security_flag", "noul"))
                    .unwrap_or(0.0)
                    >= SECURITY_PRIORITY
            })
            .count() as u64
    };
    let unit_stats = |members: &[u64]| -> (u64, Option<f64>, String) {
        let risks: Vec<f64> = members
            .iter()
            .filter_map(|number| judgments.get(number))
            .filter_map(|judgment| judgment.metric("risk", "score"))
            .collect();
        let average = if risks.is_empty() {
            None
        } else {
            let mean = risks.iter().sum::<f64>() / risks.len() as f64;
            Some((mean * 100.0).round() / 100.0)
        };
        let created = members
            .iter()
            .filter_map(|number| prs.get(*number))
            .map(|pr| pr.created.clone())
            .min()
            .unwrap_or_default();
        (security_count(members), average, created)
    };

    let mut grouped: HashSet<u64> = HashSet::new();
    let mut units: Vec<Value> = Vec::new();
    for group in dupes
        .get("confirmed_groups")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let members: Vec<u64> = group
            .as_array()
            .map(|items| items.iter().filter_map(Value::as_u64).collect())
            .unwrap_or_default();
        grouped.extend(members.iter().copied());
        if members.iter().any(|member| parks.contains_key(member)) {
            continue; // the whole atomic unit waits; it re-enters together
        }
        let (security, risk, created) = unit_stats(&members);
        units.push(json!({
            "members": members, "same_change": true,
            "security": security, "risk": risk, "created": created,
        }));
    }
    let mut excluded: HashSet<u64> = HashSet::new();
    for group in dupes
        .get("review_groups")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        if let Some(members) = group.get("members").and_then(Value::as_array) {
            excluded.extend(members.iter().filter_map(Value::as_u64));
        }
    }
    let mut ordered: Vec<u64> = prs.numbers().to_vec();
    ordered.sort_unstable();
    for number in ordered {
        if grouped.contains(&number) || excluded.contains(&number) || parks.contains_key(&number) {
            continue;
        }
        let (security, risk, created) = unit_stats(&[number]);
        units.push(json!({
            "members": [number], "same_change": false,
            "security": security, "risk": risk, "created": created,
        }));
    }

    // Security outranks every risk band; within a band, the longest evidenced
    // quiet head goes first. The newest bound in an atomic unit is its floor.
    //
    // Every component is ascending to match the key the ordering sorts by, so the
    // first field is negated security rather than reversed comparison.
    let priority = |unit: &Value| -> (i64, i64, bool, String, String, u64) {
        let members: Vec<u64> = unit
            .get("members")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_u64).collect())
            .unwrap_or_default();
        let activities: Vec<Value> = members
            .iter()
            .filter_map(|number| prs.get(*number).map(|pr| (pr, *number)))
            .map(|(pr, number)| {
                pr_activity(pr, judgments.get(&number).map(|judgment| &judgment.record))
            })
            .collect();
        let moving = activities.iter().any(|activity| {
            activity
                .get("head_moved")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        });
        let idle = activities
            .iter()
            .filter_map(|activity| {
                activity
                    .get("idle_since")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .max()
            .unwrap_or_else(|| "9999".to_owned());
        let risk = unit.get("risk").and_then(Value::as_f64);
        let band = match risk {
            None => 3,
            Some(value) if value <= 1.5 => 0,
            Some(value) if value <= 2.5 => 1,
            Some(_) => 2,
        };
        let security = unit.get("security").and_then(Value::as_i64).unwrap_or(0);
        let created = unit
            .get("created")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        (
            -security,
            band,
            moving,
            idle,
            created,
            members.first().copied().unwrap_or(0),
        )
    };
    units.sort_by_key(priority);

    let mut packed: Vec<Vec<u64>> = Vec::new();
    let mut current: Vec<u64> = Vec::new();
    for unit in &units {
        let members: Vec<u64> = unit
            .get("members")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_u64).collect())
            .unwrap_or_default();
        if !current.is_empty() && current.len() + members.len() > BATCH_SIZE {
            packed.push(std::mem::take(&mut current));
        }
        current.extend(members);
    }
    if !current.is_empty() {
        packed.push(current);
    }

    if packed
        .iter()
        .flatten()
        .any(|number| parks.contains_key(number))
    {
        return Err(
            "internal error: a parked PR entered a batch (issue #8 gate failed)".to_owned(),
        );
    }

    let mut batches = Vec::new();
    for (index, members) in packed.iter().enumerate() {
        let ordinal = index + 1;
        let id = format!("B{ordinal:03}");
        let (security, average_risk, created) = unit_stats(members);
        let member_set: HashSet<u64> = members.iter().copied().collect();
        let groups = units
            .iter()
            .filter(|unit| unit.get("same_change").and_then(Value::as_bool) == Some(true))
            .filter(|unit| {
                unit.get("members")
                    .and_then(Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(Value::as_u64)
                            .all(|number| member_set.contains(&number))
                    })
                    .unwrap_or(false)
            })
            .count();
        batches.push(json!({
            "ordinal": ordinal,
            "id": id,
            "members": members,
            "count": members.len(),
            "target": "merged together as one tranche",
            "same_change_groups": groups,
            "security_members": security,
            "average_risk": average_risk,
            "created": created,
            "review_prompt": batch_review_prompt(
                members,
                &format!("B{ordinal:03}"),
                prs,
                &subject,
            ),
        }));
    }
    let security_batches = batches
        .iter()
        .filter(|batch| {
            batch
                .get("security_members")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                > 0
        })
        .count();
    let same_change_groups: u64 = batches
        .iter()
        .map(|batch| {
            batch
                .get("same_change_groups")
                .and_then(Value::as_u64)
                .unwrap_or(0)
        })
        .sum();
    Ok(json!({
        "format_version": 3,
        "repo": repository,
        "dupes_digest": dupes_digest,
        "batch_size": BATCH_SIZE,
        "meaning": "A batch is 5 PRs merged together as one tranche (issue #4). Jev determines the composition: same_change groups are atomic and combine into ONE pull request inside their batch. Batches are disjoint: every PR belongs to at most one batch. Ordered security-first, then risk band and evidenced idle lower bound. Parked PRs are excluded before packing (issue #8). Model-suggested, not verified safe to merge.",
        "batches": batches,
        "security_batches": security_batches,
        "same_change_groups": same_change_groups,
        "excluded_review_prs": excluded.len(),
        "parked_prs": parks.len(),
    }))
}
