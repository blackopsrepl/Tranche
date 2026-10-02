//! A PR that needs a reviewer assignment.

use serde::{Deserialize, Serialize};
use solverforge::prelude::*;

/// A PR requiring assignment, with Jev-judged required skills.
#[planning_entity]
#[derive(Serialize, Deserialize)]
pub struct AssignmentTask {
    #[planning_id]
    pub id: String,
    pub pr_number: u64,
    pub title: String,
    /// Skills required to review this PR (from Jev's 8th question).
    pub required_skills: Vec<String>,
    /// True when security_flag >= 0.5 (routes only to security-review qualified).
    pub security_flag: bool,
    /// Cluster group ID for atomic assignment (all members share one assignee).
    pub cluster_id: Option<String>,
    /// Coarse category from the existing judgment.
    pub category: String,
    /// One planning entity per atomic group; capacity counts every PR.
    pub numbers: Vec<u64>,
    /// Unknown evidence is never eligible.
    pub evidence_known: bool,
    // @solverforge:begin entity-variables
    #[planning_variable(value_range_provider = "members", allows_unassigned = true)]
    pub member_idx: Option<usize>,
    // @solverforge:end entity-variables
}

impl AssignmentTask {
    pub fn new(
        pr_number: u64,
        title: String,
        required_skills: Vec<String>,
        security_flag: bool,
        cluster_id: Option<String>,
        category: String,
    ) -> Self {
        Self {
            id: format!("pr-{pr_number}"),
            pr_number,
            title,
            required_skills,
            security_flag,
            cluster_id,
            category,
            numbers: vec![pr_number],
            evidence_known: true,
            member_idx: None,
        }
    }
}
