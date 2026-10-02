//! The MCP surface: a read-only, model-free stdio server over the same bound
//! report the CLI serves.
//!
//! This is a front-end, not a second source of truth. Every answer is computed
//! through `report::load` — the same gates the CLI answers pass — so an agent asking
//! over MCP and a person asking through `tranche info` cannot be told different
//! stories by one checkout.

pub mod error;
pub mod schema;
pub mod server;
pub mod tools;

pub use error::{QueryArguments, ReportError};

/// Serve requests until stdin closes.
pub fn serve(root: &tranche_core::report::Root) -> Result<(), String> {
    server::serve(root)
}
