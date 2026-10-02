//! The six tools' answers, over the same bound report the CLI serves.
//!
//! Every answer carries the report's identity and the disclaimer, so a client can
//! quote which report it was served from and cannot mistake model suggestions for
//! approval. The tools are split by what they answer: the shared row projection in
//! `rows`, coverage in `surface`, search in `querying`, batches in `browsing`, and
//! relationship evidence plus the digests in `relations`.

pub mod browsing;
pub mod querying;
pub mod relations;
pub mod rows;
pub mod surface;

pub use rows::{DISCLAIMER, MAX_RESULT_BYTES, View};
