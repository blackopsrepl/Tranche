//! Solver-owned proposals; Jev runs only before this boundary.
pub mod preprocessing;
pub mod proposal;
pub mod team;

solverforge::planning_model! {
    root = "src/domain/assignment";
    pub mod member;
    pub mod task;
    pub mod plan;
    pub mod constraints;
    pub mod solve;
    pub use member::Member;
    pub use task::AssignmentTask;
    pub use plan::AssignmentPlan;
}
