//! Default workbench resources shipped inside the binary.
//!
//! Deployment-local templates and assets are explicit overrides. Missing defaults
//! are installed on render, so a release archive does not need a source checkout.

use std::path::Path;

use tranche_core::report::Root;
use tranche_core::util::atomic_write;

const TEMPLATE: &str = include_str!("../../../../page/template.html");
const ASSETS: &[(&str, &[u8])] = &[
    (
        "workbench.css",
        include_bytes!("../../../../docs/assets/workbench.css"),
    ),
    (
        "workbench.js",
        include_bytes!("../../../../docs/assets/workbench.js"),
    ),
    (
        "tranche-title.png",
        include_bytes!("../../../../docs/assets/tranche-title.png"),
    ),
    (
        "tranche.gif",
        include_bytes!("../../../../docs/assets/tranche.gif"),
    ),
    (
        "tranche-mascot.png",
        include_bytes!("../../../../docs/assets/tranche-mascot.png"),
    ),
];

pub(super) fn template(root: &Root) -> Result<String, String> {
    match std::fs::read_to_string(root.template_path()) {
        Ok(template) => Ok(template),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(TEMPLATE.to_owned()),
        Err(error) => Err(format!(
            "cannot read {}: {error}",
            root.template_path().display()
        )),
    }
}

pub(super) fn install(root: &Root) -> Result<(), String> {
    let directory = root.docs_dir().join("assets");
    for (name, bytes) in ASSETS {
        install_missing(&directory.join(name), bytes)?;
    }
    Ok(())
}

fn install_missing(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if path.exists() {
        if !path.is_file() {
            return Err(format!("asset override is not a file: {}", path.display()));
        }
        return Ok(());
    }
    atomic_write(path, bytes).map_err(|error| format!("cannot install {}: {error}", path.display()))
}
