//! The evidence vocabulary: what a capture is made of, and what it costs.
//!
//! These constants are the shared language of the whole evidence service. They live
//! together because they only make sense against each other: the component list is
//! what the reserved budget protects, and the exit codes are what a caller branches
//! on when the budget runs out.

use crate::report::Root;

/// The packet envelope this capture format writes.
pub const FORMAT: &str = "tranche.evidence-packet/v1";

/// The component profile every capture claims to satisfy.
pub const PROFILE: &str = "pr-review/v1";

/// Components whose bytes name a code revision.
pub const CODE_COMPONENTS: [&str; 3] = ["metadata", "diff", "files"];

/// Components that are timestamped observations of a moving conversation.
pub const MUTABLE_COMPONENTS: [&str; 5] = [
    "discussion",
    "review_comments",
    "reviews",
    "checks",
    "closing_issues",
];

/// Every component a member must account for, in report order.
pub const COMPONENTS: [&str; 8] = [
    "metadata",
    "diff",
    "files",
    "discussion",
    "review_comments",
    "reviews",
    "checks",
    "closing_issues",
];

/// Requests held back for the completion checks, so a shortfall stops the capture
/// explicitly instead of certifying an unverified batch.
pub const RESERVED_BUDGET: i64 = 3;

/// A writer lock older than this is reclaimable once its process is gone.
pub const LOCK_TTL: f64 = 900.0;

/// The exit code each outcome maps to, stable for scripts.
pub const EXIT_OK: i32 = 0;
/// Usable but incomplete.
pub const EXIT_USABLE: i32 = 1;
/// The command was called wrongly.
pub const EXIT_USAGE: i32 = 2;
/// The request was refused.
pub const EXIT_REFUSED: i32 = 3;
/// The capture is incomplete.
pub const EXIT_INCOMPLETE: i32 = 4;

/// The evidence root beneath a report checkout.
///
/// Capture writes land under an ignored directory, never in the report itself: the
/// report describes a backlog, while a capture is a local observation of public
/// content.
pub fn evidence_root(root: &Root) -> std::path::PathBuf {
    root.evidence_dir()
}

/// UTC now, in the format every manifest timestamp uses.
///
/// Whole seconds, one trailing `Z`: a manifest is a statement about a moment, not a
/// high-resolution log, and the recorded format is part of what packets compare.
pub fn now() -> String {
    let text = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned());
    // Rfc3339 may carry fractional seconds or an offset; the recorded format is
    // whole seconds with one trailing `Z`.
    match text.split_once('.') {
        Some((head, _)) => format!("{head}Z"),
        None => text,
    }
}
