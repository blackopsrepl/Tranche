//! The predicates that decide what a report recommends.
//!
//! These read model answers, never raw text, and every one of them treats an
//! unknown as unknown: a missing score is never coerced into a low one.

use crate::domain::judge::Judgment;
use crate::domain::pr::Pr;
use std::collections::HashSet;

/// The probability at or above which security is a top-priority meta-category.
pub const SECURITY_PRIORITY: f64 = 0.5;

/// True when a PR belongs in the review-candidate tranches.
///
/// Every input must be known and current: a stale judgment, an unusable answer
/// or a draft all disqualify, because a candidate that cannot be verified is not
/// a candidate.
pub fn review_candidate(pr: &Pr, judgment: &Judgment, grouped: &HashSet<u64>) -> bool {
    let risk = judgment.metric("risk", "score");
    let finished = judgment.metric("finished_form", "score");
    let fix = judgment.metric("is_fix", "noul");
    let security = judgment.metric("security_flag", "noul");
    let effort = judgment.metric("review_effort", "score");
    if judgment.freshness() != "current"
        || !crate::domain::judge::reusable_judgment(judgment)
        || pr.draft
        || grouped.contains(&pr.number)
        || risk.is_none()
        || finished.is_none()
        || fix.is_none()
        || security.is_none()
        || effort.is_none()
    {
        return false;
    }
    let (risk, finished, fix, security) = (
        risk.unwrap_or_default(),
        finished.unwrap_or_default(),
        fix.unwrap_or_default(),
        security.unwrap_or_default(),
    );
    risk <= 1.5 && finished >= 1.8 && fix >= 0.6 && security < 0.5
}

/// True when risk or security warrants human escalation.
pub fn escalated(judgment: &Judgment) -> bool {
    let risk = judgment.metric("risk", "score");
    let security = judgment.metric("security_flag", "noul");
    risk.is_some_and(|value| value >= 3.0) || security.is_some_and(|value| value >= 0.5)
}

/// A cross-cutting meta-category ranked above every other category.
///
/// Membership uses the same bar as senior escalation; unlike a category choice
/// it never replaces the PR's own area. Unknown stays unknown.
pub fn security_priority(judgment: &Judgment) -> bool {
    judgment
        .metric("security_flag", "noul")
        .is_some_and(|value| value >= SECURITY_PRIORITY)
}

/// The risk band a PR is filed under.
pub fn risk_band(risk: Option<f64>) -> &'static str {
    match risk {
        None => "unknown",
        Some(value) if value <= 1.5 => "low",
        Some(value) if value <= 2.5 => "core",
        Some(_) => "danger",
    }
}
