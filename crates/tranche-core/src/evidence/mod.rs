//! Evidence: capture, inspection, storage and export.
//!
//! The service is deliberately narrow. It reads GitHub, writes only under
//! `out/evidence/`, and never calls a model, mutates GitHub, executes captured
//! content or prints a packet into a terminal.

pub mod capture;
pub mod citations;
pub mod components;
pub mod coverage;
pub mod errors;
pub mod lock;
pub mod manifest;
pub mod packet;
pub mod paths;
pub mod read;
pub mod report;
pub mod selecting;
pub mod selection;
pub mod store;
pub mod vocabulary;

pub use errors::{
    Error, EvidenceError, IncompleteCapture, SelectionError, refuse, selection_refusal,
};
pub use selecting::{Plan, select};
pub use vocabulary::{
    CODE_COMPONENTS, COMPONENTS, EXIT_INCOMPLETE, EXIT_OK, EXIT_REFUSED, EXIT_USABLE, EXIT_USAGE,
    FORMAT, LOCK_TTL, MUTABLE_COMPONENTS, PROFILE, RESERVED_BUDGET, evidence_root, now,
};
