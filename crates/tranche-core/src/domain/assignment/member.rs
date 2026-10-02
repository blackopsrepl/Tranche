//! A team member discovered from a resume and classified by Jev.

use serde::{Deserialize, Serialize};
use solverforge::prelude::*;

/// A triage team member with Jev-judged qualifications.
#[planning_entity]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Member {
    #[planning_id]
    pub id: String,
    pub name: String,
    /// Skills this member is qualified for (from Jev qualification pass).
    pub qualified_skills: Vec<String>,
    /// Maximum PRs this member can handle.
    pub capacity: usize,
    /// Source resume file.
    pub resume_source: String,
    /// Dense index for the solver join, set during normalize.
    pub index: usize,
}

impl Member {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        qualified_skills: Vec<String>,
        capacity: usize,
        resume_source: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            qualified_skills,
            capacity,
            resume_source: resume_source.into(),
            index: 0,
        }
    }

    /// True when this member is qualified for every required skill.
    pub fn covers(&self, required: &[String]) -> bool {
        required
            .iter()
            .all(|skill| self.qualified_skills.iter().any(|q| q == skill))
    }
}
