//! Execute the published SolverForge scalar API, not a hand-written router.
use solverforge::{SolverEvent, SolverManager};
use solverforge::prelude::*;
use super::{AssignmentPlan, constraints::create_constraints};

pub fn solve(plan: AssignmentPlan) -> Result<AssignmentPlan, String> {
    static MANAGER: SolverManager<AssignmentPlan> = SolverManager::new();
    let (id, mut receiver) = MANAGER.solve(plan).map_err(|e| e.to_string())?;
    let mut result = Err("solver ended without a completed solution".into());
    while let Some(event) = receiver.blocking_recv() {
        match event {
            SolverEvent::Completed { solution, .. } => {
                // Never publish infeasible solutions, even if the search times out.
                if create_constraints().evaluate_all(&solution) < HardSoftScore::of(0, i64::MIN) {
                    result = Err("solver returned a hard-infeasible proposal".into());
                } else { result = Ok(solution); }
                break;
            }
            SolverEvent::Failed { error, .. } => { result = Err(error.to_string()); break; }
            _ => {}
        }
    }
    MANAGER.delete(id).map_err(|e| e.to_string())?;
    result
}
