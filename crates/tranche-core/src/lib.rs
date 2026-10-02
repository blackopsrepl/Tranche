//! Tranche core.
//!
//! Every module here is transport-free: no stdout, no argv, no JSON-RPC. The
//! `tranche` and `tranche-mcp` binaries are thin adapters over this crate, so a
//! rule stated once cannot drift between the surfaces that must agree.

pub mod domain;
pub mod evidence;
pub mod gh;
pub mod jev;
pub mod report;
pub mod util;
