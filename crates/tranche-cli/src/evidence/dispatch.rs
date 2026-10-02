//! Dispatch for the evidence verbs.

use tranche_core::report::Root;

use crate::cli::Evidence;
use crate::commands::Outcome;

use super::{acquiring, inspecting};

/// Dispatch one evidence verb.
pub fn run(root: &Root, verb: &Evidence, json_output: bool) -> Outcome {
    match verb {
        Evidence::Capture(args) => acquiring::capture_evidence(root, args, json_output),
        Evidence::Show(args) => inspecting::show(root, args, json_output),
        Evidence::Export(args) => inspecting::export(root, args, json_output),
    }
}
