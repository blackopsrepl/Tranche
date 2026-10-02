//! Clustering: duplicate groups, review candidates and the summary.
//!
//! The predicates here are the pipeline's judgement of its own output, so they
//! are written once and read by everything: `cluster` publishes them, the report
//! gate recomputes them, and the batch packer consumes them. A second copy would
//! let the workbench, the MCP server and the batches disagree.

pub mod binding;
pub mod groups;
pub mod predicates;
pub mod render;
pub mod report;
pub mod style;

pub use binding::report_binding;
pub use groups::{Classification, accepted_pair, duplicate_groups, pair_classification};
pub use predicates::{
    SECURITY_PRIORITY, escalated, review_candidate, risk_band, security_priority,
};
pub use render::render;
pub use report::{Clustered, cluster};
