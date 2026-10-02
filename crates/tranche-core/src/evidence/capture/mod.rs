//! Acquiring a batch's public evidence: read-only, model-free, resumable.
//!
//! The capture is the one operation that spends a GitHub budget, so its structure is
//! all about being honest under partial progress. It never certifies a source it did
//! not re-check, never presents a capped collection as complete, and never treats a
//! previously public repository as ongoing permission to share.
//!
//! `lifecycle` is the orchestration; `endpoints` says where each component is read from,
//! `pages` decides what one page says about completeness, `records` moves the
//! manifest, and `groups` reads one group. `setup` holds identity, storage and the lock.

pub mod checks;
pub mod endpoints;
pub mod groups;
pub mod lifecycle;
pub mod outcome;
pub mod pages;
pub mod records;
pub mod setup;
pub mod state;

pub use lifecycle::Capture;
pub use outcome::{Stopped, Verify};

pub use setup::{
    accepted_url, fresh_manifest, generation_of, lock_for, new_capture_id, selection_of,
    stored_manifest,
};
