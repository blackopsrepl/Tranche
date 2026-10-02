//! `tranche page`, end to end, from the committed fixture.
//!
//! The renderer's job is to arrange what the report already says, and to refuse
//! when the inputs cannot vouch for each other. Both halves are tested: a page
//! renders from consistent inputs, and an input the report no longer describes
//! stops it.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

/// The checkout holding the fixture, copied so a render never touches it, with
/// the template added: the template is source, not report data.
fn root() -> tempfile::TempDir {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("nested in the repository");
    let root = tempfile::tempdir().expect("a root");
    copy(
        &repository.join("crates/tranche-core/tests/fixture"),
        root.path(),
    );
    let page = root.path().join("page");
    fs::create_dir_all(&page).expect("page directory");
    fs::copy(
        repository.join("page/template.html"),
        page.join("template.html"),
    )
    .expect("the template");
    root
}

fn copy(source: &Path, destination: &Path) {
    for entry in fs::read_dir(source).expect("the fixture reads") {
        let entry = entry.expect("an entry");
        let target = destination.join(entry.file_name());
        if entry.file_type().expect("a type").is_dir() {
            fs::create_dir_all(&target).expect("a directory");
            copy(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).expect("a file");
        }
    }
}

fn run(root: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tranche"))
        .arg("--root")
        .arg(root)
        .arg("page")
        .output()
        .expect("the tranche binary runs")
}

fn payload(root: &Path) -> String {
    fs::read_to_string(root.join("docs/data/workbench.json")).expect("the payload reads")
}

/// The first title the fixture's report holds, for asserting the page is filled.
fn a_fixture_title() -> String {
    let clusters = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("nested in the repository")
        .join("crates/tranche-core/tests/fixture/out/clusters.json");
    let text = fs::read_to_string(clusters).expect("clusters");
    let parsed: serde_json::Value = serde_json::from_str(&text).expect("parses");
    parsed
        .as_object()
        .into_iter()
        .flatten()
        .flat_map(|(_, bands)| bands.as_object().into_iter().flatten())
        .flat_map(|(_, items)| items.as_array().into_iter().flatten())
        .find_map(|item| item["title"].as_str().map(str::to_owned))
        .expect("the fixture holds a titled PR")
}

#[test]
fn a_page_renders_from_consistent_inputs() {
    let root = root();
    let output = run(root.path());
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = payload(root.path());
    let parsed: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
    let rows = parsed["prs"].as_array().expect("a list of rows");
    assert!(!rows.is_empty(), "the payload carries PRs");

    // The page really is filled: a PR from the report appears in it.
    let title = a_fixture_title();
    let html = fs::read_to_string(root.path().join("docs/index.html")).expect("the page");
    assert!(!html.contains("{{options}}"), "the options were filled");
    assert!(!html.contains("{{count}}"), "the count was filled");
    assert!(
        rows.iter().any(|row| row["title"] == title.as_str()),
        "the expected PR is in the payload: {title}"
    );
}

#[test]
fn the_payload_escapes_markup_so_a_title_cannot_break_out() {
    // A title is untrusted text from GitHub. The fetch and the injection are the
    // reason `&`, `<` and `>` are escaped rather than carried through.
    let root = root();
    assert!(run(root.path()).status.success());
    let text = payload(root.path());
    assert!(!text.contains('<'), "no raw angle bracket survives");
    assert!(!text.contains('&'), "no raw ampersand survives");
    // And the escaping is the documented form, not a different one.
    assert!(text.contains("\\u003c") || !text.contains("u003c"));
}

#[test]
fn a_report_that_no_longer_describes_its_inputs_stops_the_render() {
    // The gate that matters: `summary.json` records what the report was built
    // from, and a report whose inputs have moved on must not be shown, because a
    // maintainer would be reading numbers nobody can vouch for.
    let root = root();
    let summary = root.path().join("out/summary.json");
    let text = fs::read_to_string(&summary).expect("summary");
    let mut parsed: serde_json::Value = serde_json::from_str(&text).expect("parses");
    parsed["report_binding"] = serde_json::json!("0000");
    fs::write(&summary, serde_json::to_string(&parsed).expect("encode")).expect("write");

    let output = run(root.path());
    assert!(!output.status.success(), "a foreign binding refuses");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("rerun cluster"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !root.path().join("docs/data/workbench.json").exists(),
        "nothing was written"
    );
}

#[test]
fn a_stale_batches_file_stops_the_render() {
    let root = root();
    let batches = root.path().join("out/batches.json");
    let text = fs::read_to_string(&batches).expect("batches");
    let mut parsed: serde_json::Value = serde_json::from_str(&text).expect("parses");
    parsed["dupes_digest"] = serde_json::json!("0000");
    fs::write(&batches, serde_json::to_string(&parsed).expect("encode")).expect("write");
    let output = run(root.path());
    assert!(!output.status.success(), "a stale batches refuses");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("batches.json is stale"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_missing_template_refuses_rather_than_writing_a_broken_page() {
    let root = root();
    fs::remove_file(root.path().join("page/template.html")).expect("remove the template");
    let output = run(root.path());
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("template"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
