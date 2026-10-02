use solverforge::prelude::*;
use tranche_core::domain::assignment::{AssignmentPlan, AssignmentTask, Member};
use tranche_core::domain::questions::judge_questions;

#[test]
fn assignment_does_not_rebill_the_seven_question_contract() {
    assert_eq!(judge_questions().as_object().unwrap().len(), 7);
}

#[test]
fn capacity_is_a_hard_per_member_pr_limit() {
    let mut plan = AssignmentPlan::new(
        vec![Member::new(
            "synthetic-a",
            "Synthetic A",
            vec!["docs".into()],
            1,
            "",
        )],
        vec![
            AssignmentTask::new(
                1,
                "".into(),
                vec!["docs".into()],
                false,
                None,
                "docs".into(),
            ),
            AssignmentTask::new(
                2,
                "".into(),
                vec!["docs".into()],
                false,
                None,
                "docs".into(),
            ),
        ],
    );
    for task in &mut plan.tasks {
        task.member_idx = Some(0);
    }
    let score =
        tranche_core::domain::assignment::constraints::create_constraints().evaluate_all(&plan);
    assert!(score < HardSoftScore::of(0, i64::MIN));
}

#[test]
fn solver_completes_and_leaves_excess_work_unassigned() {
    let plan = AssignmentPlan::new(
        vec![Member::new(
            "synthetic-a",
            "Synthetic A",
            vec!["docs".into()],
            1,
            "",
        )],
        (1..=3)
            .map(|n| {
                AssignmentTask::new(
                    n,
                    "".into(),
                    vec!["docs".into()],
                    false,
                    None,
                    "docs".into(),
                )
            })
            .collect(),
    );
    let solved = tranche_core::domain::assignment::solve::solve(plan).unwrap();
    assert_eq!(
        solved
            .tasks
            .iter()
            .filter(|t| t.member_idx.is_some())
            .count(),
        1
    );
}
