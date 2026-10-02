//! Shared setup for the tests that rebuild the report from a stored corpus.
//!
//! Included by each of those test binaries rather than built as its own, so a
//! binary is created only for the cases it actually runs.
//!
//! The tests read `tests/fixture/`, a committed slice of a real corpus. The full
//! corpus is 111 MiB and stays local, so a test that read it would pass on a
//! developer's machine and fail in a fresh checkout — which is exactly what
//! happened in CI. Regenerate the fixture with:
//!
//! ```text
//! cargo run --release -p tranche-core --example slice_fixture
//! ```

#![allow(dead_code)]

use std::collections::HashMap;
use std::path::Path;

use serde_json::Value;
use tranche_core::domain::cluster::{Clustered, cluster};
use tranche_core::domain::dupe::current_pairs;
use tranche_core::domain::judge::current_judgments;
use tranche_core::domain::pr::load_prs;
use tranche_core::report::Root;

pub use tranche_core::domain::judge::Judgment;
pub use tranche_core::domain::pr::Prs;

pub const REPO: &str = "omacom/omarchy";
pub const MODEL: &str = "jev-latest";

/// How many PRs the committed fixture holds. Asserted so a truncated or
/// regenerated fixture cannot quietly weaken every comparison below.
pub const FIXTURE_PRS: usize = 12;

/// The checkout holding the committed fixture.
pub fn corpus() -> Root {
    Root::new(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixture"),
    )
}

/// One committed output, read from disk.
pub fn read(root: &Root, name: &str) -> Value {
    let path = root.out_dir().join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    serde_json::from_str(&text).expect("the committed output parses")
}

/// The fixture, judged and pair-compared.
pub fn reconstructed(root: &Root) -> (Prs, HashMap<u64, Judgment>, Vec<Value>) {
    let prs = load_prs(root, REPO).expect("the fixture loads");
    let judgments = current_judgments(root, &prs, REPO, MODEL, false).expect("judgments bind");
    let verdicts =
        current_pairs(root, &prs, &judgments, REPO, MODEL, false).expect("verdicts bind");
    // A read that returned nothing would satisfy every equality below.
    assert_eq!(
        prs.len(),
        FIXTURE_PRS,
        "the fixture holds the PRs it claims"
    );
    assert!(!judgments.is_empty(), "the fixture's judgments were read");
    (prs, judgments, verdicts)
}

/// The report the fixture produces, with the inputs it was built from.
pub fn built(root: &Root) -> (Prs, HashMap<u64, Judgment>, Clustered) {
    let (prs, judgments, verdicts) = reconstructed(root);
    let report = cluster(&prs, &judgments, &verdicts, REPO, MODEL, false);
    (prs, judgments, report)
}
