//! Pre-release batches and the park record.
//!
//! A batch claims exactly one thing: these PRs merge together. That claim is why
//! a draft, a PR without finished form or a PR without a current judgment must
//! never appear in one, and why a same-change group is atomic — splitting it
//! would leave the remaining members claimed twice or held entirely.

pub mod pack;
pub mod packing;
pub mod park;
pub mod report_section;

pub use pack::{BATCH_SIZE, batch_review_prompt, pr_activity};
pub use packing::merge_batches;
pub use park::{PARK_UNBLOCK, park_counts, park_set, park_state, parked_payload, unblock_for};
pub use report_section::{batch_plan_section, park_section};
