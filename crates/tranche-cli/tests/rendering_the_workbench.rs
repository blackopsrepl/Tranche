//! `tranche page`, end to end, from the committed fixture.
//!
//! The renderer's job is to arrange what the report already says, and to refuse
//! when the inputs cannot vouch for each other. Both halves are tested: a page
//! renders from consistent inputs, and an input the report no longer describes
//! stops it.

use std::fs;
use std::io::Read;
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
    run_with(root, &[])
}

fn run_with(root: &Path, page_args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tranche"))
        .arg("--root")
        .arg(root)
        .arg("page")
        .args(page_args)
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
    assert!(
        !root.path().join("docs/data/report.json").exists(),
        "standalone JSON stays opt-in"
    );
    assert!(
        !root.path().join("docs/data/report.xlsx").exists(),
        "Excel stays opt-in"
    );
}

#[test]
fn standalone_json_export_carries_the_bound_report_and_grouping_context() {
    let root = root();
    let output = run_with(root.path(), &["--export-json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let path = root.path().join("docs/data/report.json");
    let text = fs::read_to_string(&path).expect("the standalone export reads");
    let export: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("nested in the repository");
    let schema_text = fs::read_to_string(repository.join("docs/page-export.schema.json"))
        .expect("the published schema reads");
    let schema: serde_json::Value = serde_json::from_str(&schema_text).expect("valid schema JSON");
    let validator = jsonschema::validator_for(&schema).expect("the schema compiles");
    let errors: Vec<String> = validator
        .iter_errors(&export)
        .map(|error| error.to_string())
        .collect();
    assert!(errors.is_empty(), "schema validation errors: {errors:?}");
    assert_eq!(export["format"], "tranche.page-export");
    assert_eq!(export["schema_version"], 1);
    assert_eq!(export["repository"], "omacom/omarchy");
    assert_eq!(export["report_binding"].as_str().unwrap().len(), 64);
    let rows = export["pull_requests"].as_array().expect("PR rows");
    assert!(!rows.is_empty(), "the export carries PR-level data");
    let title = a_fixture_title();
    assert!(rows.iter().any(|row| row["title"] == title.as_str()));
    assert!(
        !export["groups"]["confirmed_groups"]
            .as_array()
            .expect("confirmed groups")
            .is_empty(),
        "the export carries grouping information"
    );
    assert_eq!(export["batches_available"], true);
    assert!(
        !export["batches"].as_array().expect("batches").is_empty(),
        "the fixture includes batch information"
    );
    assert!(
        !export["parked"]["members"]
            .as_array()
            .expect("parked members")
            .is_empty(),
        "the fixture includes park information"
    );
    assert!(!root.path().join("docs/data/report.xlsx").exists());
}

#[test]
fn excel_export_is_a_standalone_workbook_and_can_be_selected_alone() {
    let root = root();
    let output = run_with(root.path(), &["--export-xlsx"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let path = root.path().join("docs/data/report.xlsx");
    let bytes = fs::read(&path).expect("the workbook reads");
    assert!(
        bytes.starts_with(b"PK\x03\x04"),
        "an xlsx file is a ZIP archive"
    );
    assert!(bytes.len() > 1_000, "the workbook contains report data");

    let mut archive = zip::ZipArchive::new(fs::File::open(path).expect("the workbook opens"))
        .expect("the workbook ZIP is valid");
    let mut workbook_xml = String::new();
    archive
        .by_name("xl/workbook.xml")
        .expect("workbook metadata")
        .read_to_string(&mut workbook_xml)
        .expect("workbook metadata reads");
    for sheet in ["Report", "PRs", "Groups", "Batches", "Parked"] {
        assert!(
            workbook_xml.contains(&format!("name=\"{sheet}\"")),
            "sheet {sheet}"
        );
    }
    let mut shared_strings = String::new();
    archive
        .by_name("xl/sharedStrings.xml")
        .expect("workbook strings")
        .read_to_string(&mut shared_strings)
        .expect("workbook strings read");
    for value in [
        "report_binding",
        "number",
        "title",
        "kind",
        "review_prompt",
        "unblock",
        "B001",
        "C001",
        "same_change_hold",
    ] {
        assert!(shared_strings.contains(value), "workbook contains {value}");
    }
    for index in 1..=5 {
        let mut sheet_xml = String::new();
        archive
            .by_name(&format!("xl/worksheets/sheet{index}.xml"))
            .expect("worksheet XML")
            .read_to_string(&mut sheet_xml)
            .expect("worksheet XML reads");
        assert!(
            sheet_xml.contains("<autoFilter ref=\""),
            "sheet {index} is filterable"
        );
        assert!(
            sheet_xml.contains("ySplit=\"1\""),
            "sheet {index} freezes its header"
        );
    }
    assert!(!root.path().join("docs/data/report.json").exists());
}

#[test]
fn xlsx_export_is_byte_stable_for_identical_inputs() {
    let root = root();
    let first_output = run_with(root.path(), &["--export-xlsx"]);
    assert!(
        first_output.status.success(),
        "{}",
        String::from_utf8_lossy(&first_output.stderr)
    );
    let path = root.path().join("docs/data/report.xlsx");
    let first = fs::read(&path).expect("the first workbook reads");
    std::thread::sleep(std::time::Duration::from_secs(2));
    let repeated_output = run_with(root.path(), &["--export-xlsx"]);
    assert!(
        repeated_output.status.success(),
        "{}",
        String::from_utf8_lossy(&repeated_output.stderr)
    );
    let repeated = fs::read(&path).expect("the repeated workbook reads");
    assert_eq!(first, repeated, "identical inputs produce identical bytes");

    let file = fs::File::open(path).expect("the workbook opens");
    let mut archive = zip::ZipArchive::new(file).expect("the workbook ZIP is valid");
    let mut core_xml = String::new();
    archive
        .by_name("docProps/core.xml")
        .expect("document properties")
        .read_to_string(&mut core_xml)
        .expect("document properties read");
    assert!(
        core_xml.contains("2000-01-01T00:00:00Z"),
        "workbook metadata uses a fixed timestamp"
    );
}

#[test]
fn both_export_formats_can_be_selected_together() {
    let root = root();
    let output = run_with(root.path(), &["--export-json", "--export-xlsx"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(root.path().join("docs/data/report.json").is_file());
    assert!(root.path().join("docs/data/report.xlsx").is_file());
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

    let output = run_with(root.path(), &["--export-json", "--export-xlsx"]);
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
    assert!(!root.path().join("docs/data/report.json").exists());
    assert!(!root.path().join("docs/data/report.xlsx").exists());
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
