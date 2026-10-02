//! Check runs and statuses, and the groups a component acquires.
//!
//! A pull request's own checks live in its base repository while a fork's head
//! can carry its own, so `checks` is four separately recorded groups inside one
//! component rather than a flattened coverage entry. A missing group does not
//! mean "no CI" — it means the capture did not observe that repository.

/// Check runs and statuses for the base repository, the fork's, or both.
pub const CHECKS_GROUPS: [&str; 4] = ["check_runs", "statuses", "fork_check_runs", "fork_statuses"];

/// The base-repository checks every member has.
pub const BASE_CI_GROUPS: [&str; 2] = ["check_runs", "statuses"];

/// The fork's own checks, present only when the head repository differs.
pub const FORK_CI_GROUPS: [&str; 2] = ["fork_check_runs", "fork_statuses"];

/// The groups a component acquires, in report order.
pub fn groups_for(component: &str) -> Vec<String> {
    match component {
        "checks" => CHECKS_GROUPS
            .iter()
            .map(|name| (*name).to_owned())
            .collect(),
        _ => vec![component.to_owned()],
    }
}
