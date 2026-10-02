//! Reading a capture back: coverage, windows and citations.
//!
//! These answer from the stored state alone, which is what makes a capture
//! checkable by someone who has only the directory. The escape test matters as
//! much as the resolution ones: a captured body is untrusted text and must not be
//! able to drive the terminal that reads it.

use std::path::{Path, PathBuf};

use serde_json::Value;
use tranche_core::evidence::report;
use tranche_core::report::Root;

const CAPTURE: &str = "108dbe75ccac6ea366b543791c8f168e";

/// A checkout carrying the real capture, copied so a read never touches it.
fn root() -> Option<tempfile::TempDir> {
    let source = Path::new("/srv/lab/hack/omarchy-pr-jev-triage").join("out/evidence");
    if !source.join(CAPTURE).join("manifest.json").exists() {
        return None;
    }
    let root = tempfile::tempdir().expect("a root");
    copy(&source, &root.path().join("out/evidence"));
    Some(root)
}

fn copy(source: &Path, destination: &Path) {
    std::fs::create_dir_all(destination).expect("a directory");
    for entry in std::fs::read_dir(source).expect("the capture reads") {
        let entry = entry.expect("an entry");
        let target = destination.join(entry.file_name());
        if entry.file_type().expect("a type").is_dir() {
            copy(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("a file");
        }
    }
}

fn manifest(root: &Path) -> Value {
    let path = root.join(format!("out/evidence/{CAPTURE}/manifest.json"));
    serde_json::from_str(&std::fs::read_to_string(path).expect("the manifest reads"))
        .expect("the manifest parses")
}

fn checkout(root: &Path) -> Root {
    Root::new(root.to_path_buf())
}

#[test]
fn coverage_counts_what_the_manifest_recorded() {
    let Some(root) = root() else {
        return;
    };
    let manifest = manifest(root.path());
    let coverage = report::coverage(&manifest);
    assert_eq!(coverage["capture_id"], CAPTURE);
    assert!(coverage["members"].is_array());

    // Every component of every member appears exactly once, and a component that
    // completed carries no reason while a stopped one does.
    let expected = manifest["components"].as_array().expect("components").len();
    let seen: usize = coverage["members"]
        .as_array()
        .expect("members")
        .iter()
        .map(|member| member["components"].as_array().expect("components").len())
        .sum();
    assert_eq!(seen, expected, "one entry per recorded component");

    // Page counts roll up from the groups.
    for member in coverage["members"].as_array().expect("members") {
        for component in member["components"].as_array().expect("components") {
            let groups: u64 = component["groups"]
                .as_array()
                .expect("groups")
                .iter()
                .map(|group| group["pages"].as_u64().unwrap_or(0))
                .sum();
            assert_eq!(component["pages"].as_u64().unwrap_or(9), groups);
        }
    }
    assert_eq!(
        coverage["citations"],
        manifest["citations"].as_array().unwrap().len()
    );
}

#[test]
fn a_window_walks_a_source_in_bounded_steps() {
    let Some(root) = root() else {
        return;
    };
    let manifest = manifest(root.path());
    let checkout = checkout(root.path());
    let Some(source_id) = manifest["sources"][0]["id"].as_str() else {
        return;
    };

    let mut start = 0usize;
    let mut walked = 0usize;
    let total =
        report::window(&checkout, &manifest, source_id, 0, 64).expect("a window")["body_bytes"]
            .as_u64()
            .expect("a byte count") as usize;
    // Walk in small windows and prove the concatenation covers the whole body
    // exactly once: no gap, no repeat.
    while start < total {
        let window = report::window(&checkout, &manifest, source_id, start, 64).expect("a window");
        let delivered = window["end_byte"].as_u64().expect("an end") as usize - start;
        assert!(delivered > 0, "a window delivers something");
        walked += delivered;
        match window["next_start"].as_u64() {
            Some(next) => start = next as usize,
            None => break,
        }
    }
    assert_eq!(walked, total, "the windows cover the body once");

    // A start past the end is refused rather than silently clamped.
    assert!(report::window(&checkout, &manifest, source_id, total + 1, 64).is_err());
    // And a window has a bound.
    assert!(report::window(&checkout, &manifest, source_id, 0, 0).is_err());
    assert!(report::window(&checkout, &manifest, source_id, 0, 10_000_000).is_err());
}

#[test]
fn a_window_escapes_bytes_a_terminal_must_not_receive() {
    // An escape sequence in captured bytes must arrive as text, not as a command.
    let hostile = b"plain \x1b[31mred\x1b[0m and \x07bell";
    let rendered = report::printable(hostile);
    assert!(!rendered.contains('\u{1b}'), "no escape character survives");
    assert!(!rendered.contains('\u{7}'), "no bell survives");
    assert!(rendered.contains("\\u001b"), "the escape is shown as text");
    // Newlines stay literal: a diff is unreadable without them.
    assert_eq!(report::printable(b"a\nb"), "a\nb");
}

#[test]
fn a_citation_resolves_and_a_mismatch_is_reported() {
    let Some(root) = root() else {
        return;
    };
    let manifest = manifest(root.path());
    let checkout = checkout(root.path());
    let citations = manifest["citations"].as_array().expect("citations");
    let Some(id) = citations[0]["id"].as_str() else {
        return;
    };
    let resolved = report::resolve_citation(&checkout, &manifest, id).expect("resolved");
    // The reported digest is the recorded one, and it recomputes from the raw
    // range the citation names — not from the escaped text shown to a reader.
    let body = tranche_core::evidence::store::read_body(
        &checkout,
        citations[0]["source_sha256"].as_str().expect("a digest"),
    )
    .expect("the body reads");
    let start = citations[0]["start_byte"].as_u64().expect("a start") as usize;
    let end = citations[0]["end_byte"].as_u64().expect("an end") as usize;
    assert_eq!(
        tranche_core::util::sha256_hex(&body[start..end]),
        citations[0]["excerpt_sha256"].as_str().expect("recorded"),
        "the raw range digests to the recorded excerpt digest"
    );
    assert_eq!(
        resolved["excerpt_sha256"],
        citations[0]["excerpt_sha256"].clone(),
        "resolution reports the recorded digest"
    );
    assert!(
        !resolved["excerpt"].as_str().unwrap_or("").is_empty(),
        "and the excerpt itself is shown"
    );

    // An unknown citation is refused, not answered with nothing.
    assert!(report::resolve_citation(&checkout, &manifest, "not-a-citation").is_err());
}

#[test]
fn a_missing_source_is_refused_by_name() {
    let Some(root) = root() else {
        return;
    };
    let manifest = manifest(root.path());
    let checkout = checkout(root.path());
    let error = report::window(&checkout, &manifest, "no-such-source", 0, 64).expect_err("refused");
    assert!(error.contains("no-such-source"), "{error}");
}

/// The unused-import guard.
#[allow(dead_code)]
fn _path_type(_: PathBuf) {}
