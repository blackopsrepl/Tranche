//! Positive penalty magnitudes; SolverForge subtracts them from the score.
use solverforge::prelude::*;
use solverforge::stream::{ConstraintFactory, joiner};
use solverforge::stream::collector::sum;
use super::{AssignmentPlan, AssignmentTask, Member};

pub fn create_constraints() -> impl ConstraintSet<AssignmentPlan, HardSoftScore> {
    let invalid = ConstraintFactory::<AssignmentPlan, HardSoftScore>::new()
        .for_each(AssignmentPlan::tasks())
        .join((AssignmentPlan::members(), joiner::equal_bi(
            |t: &AssignmentTask| t.member_idx, |m: &Member| Some(m.index))))
        .filter(|t: &AssignmentTask, m: &Member| !t.evidence_known || !m.evidence_known || !m.covers(&t.required_skills)
            || (t.security_flag && !m.qualified_skills.iter().any(|s| s == "security-review")))
        .penalize(HardSoftScore::of_hard(1)).named("skills and security coverage");
    let capacity = ConstraintFactory::<AssignmentPlan, HardSoftScore>::new()
        .for_each(AssignmentPlan::tasks())
        .join((AssignmentPlan::members(), joiner::equal_bi(
            |t: &AssignmentTask| t.member_idx, |m: &Member| Some(m.index))))
        .group_by(|_t: &AssignmentTask, m: &Member| (m.index, m.capacity),
            sum(|(t, _m): (&AssignmentTask, &Member)| t.numbers.len() as i64))
        .penalize(|key: &(usize, usize), count: &i64| HardSoftScore::of_hard((*count - key.1 as i64).max(0)))
        .named("hard per-member PR capacity");
    let unassigned = ConstraintFactory::<AssignmentPlan, HardSoftScore>::new()
        .for_each(AssignmentPlan::tasks()).filter(|t: &AssignmentTask| t.member_idx.is_none())
        .penalize(|t: &AssignmentTask| HardSoftScore::of_soft(10000 * t.numbers.len() as i64))
        .named("prefer covered work");
    let balance = ConstraintFactory::<AssignmentPlan, HardSoftScore>::new()
        .for_each(AssignmentPlan::tasks()).filter(|t: &AssignmentTask| t.member_idx.is_some())
        .group_by(|t: &AssignmentTask| t.member_idx,
            sum(|t: &AssignmentTask| t.numbers.len() as i64))
        .penalize(|_key: &Option<usize>, count: &i64| HardSoftScore::of_soft(count * count))
        .named("balance open load");
    let affinity = ConstraintFactory::<AssignmentPlan, HardSoftScore>::new()
        .for_each(AssignmentPlan::tasks())
        .join((AssignmentPlan::members(), joiner::equal_bi(
            |t: &AssignmentTask| t.member_idx, |m: &Member| Some(m.index))))
        .filter(|t: &AssignmentTask, m: &Member| !m.qualified_skills.contains(&t.category))
        .penalize(HardSoftScore::of_soft(1)).named("category affinity");
    (invalid, capacity, unassigned, balance, affinity)
}
