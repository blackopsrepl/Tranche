//! The three evidence verbs: capture, show and export.
//!
//! These are the only commands that touch the network for evidence, and the only ones
//! that write under `out/evidence/`. Every one refuses rather than guessing.

pub mod acquiring;
pub mod dispatch;
pub mod inspecting;
pub mod reading;

pub use dispatch::run;
