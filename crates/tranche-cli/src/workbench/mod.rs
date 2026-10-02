//! The workbench: `docs/index.html` and the payload it fetches.
//!
//! The renderer refuses inconsistent inputs rather than showing them. A report
//! whose binding no longer matches, a `batches.json` from a different dupe run, a
//! park record that does not equal the one the current judgments produce — all of
//! those would put a maintainer in front of numbers nobody can vouch for, so none
//! of them render.
//!
//! Eligibility stays in the pipeline's own predicates. This never reclassifies a
//! judgment; it arranges what the report already says.

mod labels;
mod page;
mod payload;
mod writing;

pub use labels::CATEGORY_LABELS;
pub use page::page;
