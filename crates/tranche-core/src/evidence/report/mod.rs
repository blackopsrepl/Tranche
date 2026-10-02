//! Reading a capture back: coverage, windows and citations.
//!
//! Everything here answers from the recorded state and the stored bodies alone — no
//! network, no second application, no report. That independence is the property being
//! preserved: a reader holding only the capture directory can verify it.
//!
//! Stored bytes are never altered. A window renders them for a terminal, escaping what
//! a terminal must not receive.

pub mod coverage_state;
pub mod printing;
pub mod windows;

pub use coverage_state::{DEFAULT_WINDOW_BYTES, coverage};
pub use printing::printable;
pub use windows::{resolve_citation, window};
