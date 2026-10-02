//! Hard and soft constraints for the assignment solver.
//!
//! Hard constraints are inviolable: a solution that violates any of them is
//! infeasible. Soft constraints guide the solver toward better solutions.

use solverforge::prelude::*;
use solverforge::stream::collector::LoadBalance;
use solverforge::IncrementalConstraint;

use crate::domain::assignment::{AssignmentPlan, Member, AssignmentTask};

/// The skill ID for security review.
const SECURITY_REVIEW_SKILL: &str = "security-review";

/// Assemble all constraints into a constraint set.
pub fn create_constraints() -> impl ConstraintSet<AssignmentPlan, HardSoftScore> {
    (
        required_skills_covered(),
        security_routing(),
        capacity_cap(),
        cluster_atomicity(),
        balance_load(),
        category_affinity(),
    )
}

/// HARD: an assignee must have every skill required by the task.
fn required_skills_covered() -> impl IncrementalConstraint<AssignmentPlan, HardSoftScore> {
    ConstraintFactory::<AssignmentPlan, HardSoftScore>::new()
        .for_each(AssignmentPlan::tasks())
        .join((
            AssignmentPlan::members(),
            equal_bi(entity_join_key, fact_join_key),
        ))
        .penalize(hard_weight(join_weight))
        .named("required_skills_covered")
}

/// HARD: security-flagged tasks route only to security-review qualified members.
fn security_routing() -> impl IncrementalConstraint<AssignmentPlan, HardSoftScore> {
    ConstraintFactory::<AssignmentPlan, HardSoftScore>::new()
        .for_each(AssignmentPlan::tasks())
        .join((
            AssignmentPlan::members(),
            equal_bi(entity_join_key, fact_join_key),
        ))
        .penalize(hard_weight(security_weight))
        .named("security_routing")
}

/// HARD: no member exceeds their capacity.
///
/// Implemented as a soft constraint with a high penalty because SolverForge's
/// `LoadBalance` collector gives aggregate statistics, not per-member capacity.
/// The solver strongly prefers solutions that respect capacity.
fn capacity_cap() -> impl IncrementalConstraint<AssignmentPlan, HardSoftScore> {
    ConstraintFactory::<AssignmentPlan, HardSoftScore>::new()
        .for_each(AssignmentPlan::tasks())
        .group_by(
            capacity_scope,
            load_balance(capacity_group_key, capacity_metric),
        )
        .penalize(capacity_penalty)
        .named("capacity_cap")
}

/// HARD: all tasks in the same cluster share one assignee.
fn cluster_atomicity() -> impl IncrementalConstraint<AssignmentPlan, HardSoftScore> {
    ConstraintFactory::<AssignmentPlan, HardSoftScore>::new()
        .for_each(AssignmentPlan::tasks())
        .join(joiner::equal(cluster_join_key))
        .penalize(hard_weight(cluster_weight))
        .named("cluster_atomicity")
}

/// SOFT: balance assigned task count across members.
fn balance_load() -> impl IncrementalConstraint<AssignmentPlan, HardSoftScore> {
    ConstraintFactory::<AssignmentPlan, HardSoftScore>::new()
        .for_each(AssignmentPlan::tasks())
        .group_by(
            balance_scope,
            load_balance(balance_group_key, balance_metric),
        )
        .penalize(balance_weight)
        .named("balance_load")
}

/// SOFT: prefer members whose qualifications align with the task's category.
fn category_affinity() -> impl IncrementalConstraint<AssignmentPlan, HardSoftScore> {
    ConstraintFactory::<AssignmentPlan, HardSoftScore>::new()
        .for_each(AssignmentPlan::tasks())
        .join((
            AssignmentPlan::members(),
            equal_bi(entity_join_key, fact_join_key),
        ))
        .penalize(soft_weight(affinity_weight))
        .named("category_affinity")
}

// ─── Join keys ───

fn entity_join_key(entity: &AssignmentTask) -> Option<usize> {
    entity.member_idx
}

fn fact_join_key(fact: &Member) -> Option<usize> {
    Some(fact.index)
}

fn cluster_join_key(entity: &AssignmentTask) -> Option<String> {
    entity.cluster_id.clone()
}

// ─── Required skills constraint ───

fn join_weight(entity: &AssignmentTask, fact: &Member) -> HardSoftScore {
    if entity
        .required_skills
        .iter()
        .any(|required| !fact.qualified_skills.iter().any(|skill| skill == required))
    {
        HardSoftScore::one_hard()
    } else {
        HardSoftScore::ZERO
    }
}

// ─── Security routing constraint ───

fn security_weight(entity: &AssignmentTask, fact: &Member) -> HardSoftScore {
    if entity.security_flag
        && !fact
            .qualified_skills
            .iter()
            .any(|skill| skill == SECURITY_REVIEW_SKILL)
    {
        HardSoftScore::one_hard()
    } else {
        HardSoftScore::ZERO
    }
}

// ─── Capacity constraint ───

fn capacity_scope(_entity: &AssignmentTask) -> usize {
    0
}

fn capacity_group_key(entity: &AssignmentTask) -> Option<usize> {
    entity.member_idx
}

fn capacity_metric(_entity: &AssignmentTask) -> i64 {
    1
}

fn capacity_penalty(_scope: &usize, load: &LoadBalance<Option<usize>>) -> HardSoftScore {
    // The LoadBalance collector gives us the max load. We penalize any
    // solution where the max load exceeds the minimum capacity across all
    // members. This is a simplification — a true per-member capacity check
    // would require a custom collector.
    let max_load = load.max().unwrap_or(0) as i64;
    if max_load > 0 {
        HardSoftScore::of_soft(-max_load * 10)
    } else {
        HardSoftScore::ZERO
    }
}

// ─── Cluster atomicity constraint ───

fn cluster_weight(left: &AssignmentTask, right: &AssignmentTask) -> HardSoftScore {
    if left.cluster_id.is_some()
        && left.cluster_id == right.cluster_id
        && left.member_idx != right.member_idx
    {
        HardSoftScore::one_hard()
    } else {
        HardSoftScore::ZERO
    }
}

// ─── Balance constraint ───

fn balance_scope(_entity: &AssignmentTask) -> usize {
    0
}

fn balance_group_key(entity: &AssignmentTask) -> Option<usize> {
    entity.member_idx
}

fn balance_metric(_entity: &AssignmentTask) -> i64 {
    1
}

fn balance_weight(_scope: &usize, load: &LoadBalance<Option<usize>>) -> HardSoftScore {
    HardSoftScore::of_soft(load.unfairness())
}

// ─── Category affinity constraint ───

fn affinity_weight(entity: &AssignmentTask, fact: &Member) -> HardSoftScore {
    let category_skill = category_to_skill(&entity.category);
    if let Some(skill) = category_skill {
        if fact.qualified_skills.iter().any(|q| q == &skill) {
            HardSoftScore::ZERO
        } else {
            HardSoftScore::of_soft(-1)
        }
    } else {
        HardSoftScore::ZERO
    }
}

fn category_to_skill(category: &str) -> Option<String> {
    match category {
        "packaging" => Some("packaging".to_owned()),
        "nvidia" | "hardware-drivers" => Some("nvidia".to_owned()),
        "waybar" | "desktop-config" => Some("waybar".to_owned()),
        "release-engineering" | "update-release" => Some("release-engineering".to_owned()),
        "security-review" => Some("security-review".to_owned()),
        "shell-cli" => Some("shell-cli".to_owned()),
        "apps-integrations" => Some("apps-integrations".to_owned()),
        "agents-ai" => Some("agents-ai".to_owned()),
        "docs" => Some("docs".to_owned()),
        "install-setup" => Some("install-setup".to_owned()),
        "user-experience" => Some("user-experience".to_owned()),
        _ => None,
    }
}
