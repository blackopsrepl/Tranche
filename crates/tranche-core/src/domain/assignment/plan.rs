//! The SolverForge planning solution for skill-based PR assignment.

use serde::{Deserialize, Serialize};
use solverforge::prelude::*;

use super::member::Member;
use super::task::AssignmentTask;

/// The assignment plan: members, tasks, and the solver's score.
#[planning_solution(
    constraints = "crate::domain::assignment::constraints::create_constraints",
    solver_toml = "solver.toml"
)]
#[derive(Serialize, Deserialize)]
pub struct AssignmentPlan {
    #[problem_fact_collection]
    pub members: Vec<Member>,
    #[planning_entity_collection]
    pub tasks: Vec<AssignmentTask>,
    #[planning_score]
    pub score: Option<HardSoftScore>,
}

impl AssignmentPlan {
    pub fn new(members: Vec<Member>, tasks: Vec<AssignmentTask>) -> Self {
        let mut plan = Self {
            members,
            tasks,
            score: None,
        };
        plan.normalize();
        plan
    }

    /// Restores dense indexes and range-safe values after transport.
    pub fn normalize(&mut self) {
        for (index, member) in self.members.iter_mut().enumerate() {
            member.index = index;
        }
        for task in &mut self.tasks {
            if task.member_idx.is_some_and(|idx| idx >= self.members.len()) {
                task.member_idx = None;
            }
        }
    }
}
