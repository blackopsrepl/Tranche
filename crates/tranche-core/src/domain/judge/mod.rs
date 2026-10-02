//! Stored judgments: what one records, and the binding that makes it reusable.
//!
//! A judgment is bound to the PR evidence, the repository and the model that
//! produced it. The binding is verified rather than trusted, because a mismatch does
//! not merely look stale — it makes every stored judgment unreusable, so a resume
//! pass silently re-asks the question for the whole corpus and re-bills it.

pub mod judgment;
pub mod records;
pub mod store;

pub use judgment::{
    BINDING_VERSION, Judgment, judgment_binding, judgment_is_current, reusable_judgment,
};
pub use records::{finite_json, normalize_judgment, read_lines, usage};
pub use store::{current_judgments, load_done};
