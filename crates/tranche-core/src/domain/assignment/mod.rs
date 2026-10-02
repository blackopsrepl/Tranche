//! Skill-based PR assignment (issue #5).
//!
//! The domain types for the SolverForge assignment solver. Jev judges
//! qualifications and required skills; this module defines the planning
//! model that SolverForge solves. Jev never assigns; the solver never
//! calls Jev.

pub mod member;
pub mod plan;
pub mod task;

pub use member::Member;
pub use plan::AssignmentPlan;
pub use task::AssignmentTask;
