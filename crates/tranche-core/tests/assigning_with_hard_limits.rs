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

#[test]
fn unknown_requirements_and_unqualified_security_never_gain_an_owner() {
    let member = Member::new(
        "synthetic-docs",
        "Synthetic docs",
        vec!["docs".into()],
        10,
        "",
    );
    let mut unknown = AssignmentTask::new(1, "".into(), vec![], false, None, "docs".into());
    unknown.evidence_known = false;
    let security =
        AssignmentTask::new(2, "".into(), vec!["docs".into()], true, None, "docs".into());
    let solved = tranche_core::domain::assignment::solve::solve(AssignmentPlan::new(
        vec![member],
        vec![unknown, security],
    ))
    .unwrap();
    assert!(solved.tasks.iter().all(|t| t.member_idx.is_none()));
}

#[test]
fn atomic_units_consume_capacity_for_every_pr() {
    let mut unit = AssignmentTask::new(
        1,
        "".into(),
        vec!["docs".into()],
        false,
        Some("group".into()),
        "docs".into(),
    );
    unit.numbers = vec![1, 2, 3];
    let member = Member::new(
        "synthetic-docs",
        "Synthetic docs",
        vec!["docs".into()],
        2,
        "",
    );
    let solved = tranche_core::domain::assignment::solve::solve(AssignmentPlan::new(
        vec![member],
        vec![unit],
    ))
    .unwrap();
    assert!(solved.tasks[0].member_idx.is_none());
}

#[test]
fn asymmetric_capacity_is_not_the_minimum_capacity_across_the_team() {
    let members = vec![
        Member::new("synthetic-zero", "zero", vec!["docs".into()], 0, ""),
        Member::new("synthetic-docs", "docs", vec!["docs".into()], 3, ""),
    ];
    let tasks = (1..=3)
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
        .collect();
    let solved =
        tranche_core::domain::assignment::solve::solve(AssignmentPlan::new(members, tasks))
            .unwrap();
    assert!(solved.tasks.iter().all(|t| t.member_idx == Some(1)));
}
