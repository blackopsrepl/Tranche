//! Captured pull requests: the record, the corpus, and how one is read and checked.
//!
//! Everything downstream is bound to these bytes, so the corpus is loaded and
//! validated once at the boundary rather than trusted as it flows through the
//! pipeline.

pub mod error;
pub mod invalidating;
pub mod projection;
pub mod record;
pub mod references;
pub mod validating;

pub use error::{PrError, load_prs};

pub use projection::{pr_evidence_digest, pr_state, read_json};
pub use record::{BODY_CHARS, Pr, Prs};
pub use references::reference_numbers;
pub use validating::validate_pr;
