//! The capture's own shape: how it stops, and why a member could not be certified.
//!
//! Every outcome here is a named state rather than a boolean, because a stopped
//! capture is normal — a batch may be larger than one run's budget — and the reason
//! is what tells a reader whether to resume, retry or start over.

/// The largest diff a capture will keep, in bytes.
pub const MAX_DIFF_BYTES: usize = 3 * 1024 * 1024;
/// The largest diff a capture will keep, in lines.
pub const MAX_DIFF_LINES: usize = 50_000;
/// Pages one group may hold before the group is reported as capped.
pub const MAX_PAGES: u64 = 60;

/// What a run did and why it stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stopped {
    /// Everything the batch needed was acquired.
    Finished,
    /// The request budget could not cover both acquisition and the final checks.
    Budget,
    /// The transport failed in a way that was not one group's problem.
    Transport,
    /// A member moved, so the generation no longer describes it.
    RevisionDrift,
}

impl Stopped {
    /// The stop reason recorded in the manifest, or none when the run finished.
    pub fn reason(&self) -> Option<&'static str> {
        match self {
            Self::Finished => None,
            Self::Budget => Some("request_budget"),
            Self::Transport => Some("transport_error"),
            Self::RevisionDrift => Some("revision_drift"),
        }
    }
}

/// Why a member could not be certified.
pub enum Verify {
    /// The member moved; the generation no longer describes it.
    Moved,
    /// The check could not be made, which is a transport problem rather than a
    /// statement about the member.
    Failed(String),
}
