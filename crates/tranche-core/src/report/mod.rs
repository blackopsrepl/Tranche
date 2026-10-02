//! The report root, its derived paths, and the loading that binds one.
//!
//! One module decides where everything lives, so a relocated checkout (tests,
//! `--root`, a copied report directory) cannot end up with two modules
//! disagreeing about which file they are reading.

pub mod inputs;
pub mod loading;
pub mod paths;
pub mod reading;

pub use inputs::{input_digests, read_page_paths};
pub use loading::{load, load_for_inspection};
pub use paths::{BoundReport, Limits, MODEL, REPOSITORY, ReportError, Root};
pub use reading::read_report;
