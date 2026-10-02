//! Binding a report to the inputs it was built from.

use super::paths::{BoundReport, Limits, ReportError, Root};
use super::reading::read_report;

/// Read and validate the bound report, or refuse it.
///
/// Refuses before returning anything a caller could mistake for a usable
/// observation: a stale, unbound, foreign or modified input never leaves here.
pub fn load(root: &Root, limits: &Limits) -> Result<BoundReport, ReportError> {
    read_report(root, limits, false)
}
/// Read a report for inspection only, admitting legacy judgments without a
/// binding. The result is explicitly not freshness-checked, so it must never be
/// presented as current.
pub fn load_for_inspection(
    root: &Root,
    limits: &Limits,
    allow_unbound: bool,
) -> Result<BoundReport, ReportError> {
    read_report(root, limits, allow_unbound)
}
