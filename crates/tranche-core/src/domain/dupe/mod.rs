//! Pair verdicts: what one records, which pairs are worth comparing, and the binding
//! that makes a stored verdict reusable.
//!
//! `pair_binding()` is a cache key exactly as the judgment binding is. A verdict is
//! only reusable while it still describes the same two revisions under the same
//! question policy, and the candidate set is what decides which pairs are worth a
//! paid comparison at all.

pub mod normalizing;
pub mod similarity;
pub mod verdict;
pub mod verdicts;

pub use normalizing::normalize_pair;
pub use similarity::{candidate_pairs, lexical_pairs};
pub use verdict::{BINDING_VERSION, brief, p_same, pair_binding, pair_is_current, reusable_pair};
pub use verdicts::{Verdicts, current_pairs, current_verdicts, pair_cache};
