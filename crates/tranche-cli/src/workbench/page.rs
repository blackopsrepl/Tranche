//! Rendering the workbench from the bound reports.
//!
//! Every gate that decides whether the page renders at all lives here, because a
//! reader seeing numbers is the thing they can no longer un-see. The arrangement of
//! what the report already says is in `payload`; this file decides whether to.

use tranche_core::policy::Contract;
use tranche_core::report::{Limits, Root};

use crate::commands::Outcome;

/// Render the workbench from the bound reports.
pub fn page(
    root: &Root,
    export_json: bool,
    export_xlsx: bool,
    report: &mut dyn FnMut(&str),
) -> Outcome {
    render(root, export_json, export_xlsx, true, report)
}

/// Write bound standalone exports without touching HTML resources.
pub fn export_only(
    root: &Root,
    export_json: bool,
    export_xlsx: bool,
    report: &mut dyn FnMut(&str),
) -> Outcome {
    render(root, export_json, export_xlsx, false, report)
}

fn render(
    root: &Root,
    export_json: bool,
    export_xlsx: bool,
    html: bool,
    report: &mut dyn FnMut(&str),
) -> Outcome {
    let contract = match Contract::load(root.path()) {
        Ok(contract) => contract,
        Err(error) => return Outcome::refusal(error, 1),
    };
    let repository = contract.repository().to_owned();
    let observation = match tranche_core::report::load(root, &Limits::default()) {
        Ok(observation) => observation,
        Err(error) => return Outcome::refusal(error.to_string(), 1),
    };
    let binding = observation.summary["report_binding"].as_str().unwrap_or("");

    let built = super::payload::payload(
        &observation.prs,
        &observation.judgments,
        &observation.dupes,
        observation.batches.as_ref(),
        observation.parked.as_ref(),
        root,
        &contract,
    );
    let mut message = if html {
        let (html, json_bytes) = match super::writing::write(root, &built, &contract) {
            Ok(sizes) => sizes,
            Err(error) => return Outcome::refusal(error, 1),
        };
        format!(
            "wrote {}/index.html ({} KB) and {}/data/workbench.json ({} KB)",
            root.docs_dir().display(),
            html / 1024,
            root.docs_dir().display(),
            json_bytes / 1024,
        )
    } else {
        "wrote standalone exports".to_owned()
    };
    if export_json {
        let bytes = match super::json_export::write(root, &built, binding, &repository) {
            Ok(bytes) => bytes,
            Err(error) => return Outcome::refusal(error, 1),
        };
        message.push_str(&format!(
            " and {}/data/report.json ({} KB)",
            root.docs_dir().display(),
            bytes / 1024
        ));
    }
    if export_xlsx {
        let bytes = match super::xlsx_export::write(root, &built, binding, &repository) {
            Ok(bytes) => bytes,
            Err(error) => return Outcome::refusal(error, 1),
        };
        message.push_str(&format!(
            " and {}/data/report.xlsx ({} KB)",
            root.docs_dir().display(),
            bytes / 1024
        ));
    }
    report(&message);
    Outcome::success(String::new())
}
