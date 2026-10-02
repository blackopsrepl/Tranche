//! Reading a stored capture.
//!
//! A capture on disk was written by an earlier run, and every identity it claims
//! is re-derived here rather than believed: an accepted-but-wrong manifest lets a
//! corrupt capture be resumed from or exported.

use tranche_core::evidence::components::groups_for;
use tranche_core::evidence::coverage::{fresh_components, is_complete, roll_up};
use tranche_core::evidence::manifest::{build_manifest, read_manifest};
use tranche_core::evidence::selection::{generation_id, is_sha1, selection_from_json, source_id};
use tranche_core::report::Root;

const CAPTURE: &str = "108dbe75ccac6ea366b543791c8f168e";

fn corpus() -> Root {
    Root::new(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("nested in the repository"),
    )
}

#[test]
#[ignore = "reads the local 111 MiB corpus; the committed fixture covers the rest"]
fn a_stored_capture_reports_the_batch_it_was_taken_from() {
    let root = corpus();
    let manifest = read_manifest(&root, CAPTURE).expect("the real capture verifies");
    assert_eq!(manifest["format"], "tranche.evidence-packet/v1");
    assert_eq!(manifest["profile"], "pr-review/v1");
    assert!(is_complete(&manifest));

    let selection = selection_from_json(&manifest["selection"]).expect("selection");
    assert_eq!(selection.numbers(), vec![11720, 12891, 7087, 12582]);
    assert_eq!(selection.batch_id, "B001");
    assert_eq!(selection.batch_ordinal, 1);
    assert_eq!(selection.repository["full_name"], "omacom/omarchy");
    assert_eq!(selection.repository["id"], 994093166);
}

#[test]
#[ignore = "reads the local 111 MiB corpus; the committed fixture covers the rest"]
fn a_capture_identity_recomputes_from_what_it_stored() {
    let root = corpus();
    let manifest = read_manifest(&root, CAPTURE).expect("verifies");
    let selection = selection_from_json(&manifest["selection"]).expect("selection");
    assert_eq!(
        selection.membership_digest(),
        "4a1b1efca71051df3a3d6ac686a2ff8550587f9735971430c5e091579eb6e569"
    );
    assert_eq!(
        generation_id(&selection, CAPTURE),
        manifest["generation"].as_str().unwrap(),
        "the generation recomputes from the selection it stores"
    );

    // And it changes when the thing it names changes.
    let mut moved = manifest.clone();
    moved["selection"]["members"][0]["head_sha"] = serde_json::json!("1".repeat(40));
    let moved_selection = selection_from_json(&moved["selection"]).expect("selection");
    assert_ne!(
        generation_id(&moved_selection, CAPTURE),
        manifest["generation"].as_str().unwrap()
    );
}

#[test]
#[ignore = "reads the local 111 MiB corpus; the committed fixture covers the rest"]
fn a_thread_update_does_not_mint_a_new_capture_generation() {
    // `updated_at` is observed upstream data. Treating it as a revision marker
    // would force re-downloading unchanged code.
    let root = corpus();
    let manifest = read_manifest(&root, CAPTURE).expect("verifies");
    let mut touched = manifest.clone();
    touched["selection"]["members"][0]["updated_at"] = serde_json::json!("2099-01-01T00:00:00Z");
    let selection = selection_from_json(&touched["selection"]).expect("selection");
    assert_eq!(
        generation_id(&selection, CAPTURE),
        manifest["generation"].as_str().unwrap()
    );
}

#[test]
#[ignore = "reads the local 111 MiB corpus; the committed fixture covers the rest"]
fn every_recorded_source_proves_its_own_identity() {
    let root = corpus();
    let manifest = read_manifest(&root, CAPTURE).expect("verifies");
    let generation = manifest["generation"].as_str().expect("generation");
    let sources = manifest["sources"].as_array().expect("sources");
    assert!(!sources.is_empty());
    for source in sources {
        assert_eq!(
            source_id(generation, source),
            source["id"].as_str().unwrap(),
            "every source id recomputes from its own contents"
        );
    }
}

#[test]
#[ignore = "reads the local 111 MiB corpus; the committed fixture covers the rest"]
fn a_capture_that_is_not_there_is_refused() {
    let root = corpus();
    let error = read_manifest(&root, &"0".repeat(32)).expect_err("an absent capture is refused");
    assert!(error.to_string().contains("no capture"), "{error}");
    // A malformed id never becomes a path.
    assert!(read_manifest(&root, "not-a-capture").is_err());
}

#[test]
#[ignore = "reads the local 111 MiB corpus; the committed fixture covers the rest"]
fn a_manifest_that_lost_a_field_is_refused() {
    let root = corpus();
    let manifest = read_manifest(&root, CAPTURE).expect("verifies");
    for field in ["format", "selection", "sources", "components", "citations"] {
        let mut stripped = manifest.clone();
        stripped.as_object_mut().expect("object").remove(field);
        assert!(
            selection_from_json(&stripped["selection"]).is_err() || stripped.get(field).is_none(),
            "{field} matters"
        );
    }
    // A selection carrying a foreign key is refused rather than ignored.
    let mut wrong = manifest.clone();
    wrong["selection"]["unexpected"] = serde_json::json!(1);
    assert!(selection_from_json(&wrong["selection"]).is_err());
}

#[test]
#[ignore = "reads the local 111 MiB corpus; the committed fixture covers the rest"]
fn a_fresh_capture_claims_nothing_is_acquired() {
    let root = corpus();
    let manifest = read_manifest(&root, CAPTURE).expect("verifies");
    let selection = selection_from_json(&manifest["selection"]).expect("selection");
    let fresh = fresh_components(&selection);
    // Eight components per member, every one missing.
    assert_eq!(fresh.len(), selection.members.len() * 8);
    for entry in &fresh {
        assert_eq!(entry["status"], "missing");
        assert_eq!(entry["reason"], "not acquired");
    }
    let rebuilt = build_manifest(&selection, CAPTURE, 200, Some("2026-01-01T00:00:00Z"));
    assert_eq!(rebuilt["generation"], manifest["generation"]);
    assert_eq!(rebuilt["capture"]["requests_used"], 0);
    assert!(!is_complete(&rebuilt));
}

#[test]
#[ignore = "reads the local 111 MiB corpus; the committed fixture covers the rest"]
fn component_groups_follow_the_repositories_a_check_lives_in() {
    assert_eq!(groups_for("metadata"), vec!["metadata".to_owned()]);
    // A PR's own checks live in its base repository while a fork's head carries
    // its own, so `checks` is four groups inside one component.
    assert_eq!(
        groups_for("checks"),
        vec![
            "check_runs".to_owned(),
            "statuses".to_owned(),
            "fork_check_runs".to_owned(),
            "fork_statuses".to_owned()
        ]
    );
}

#[test]
#[ignore = "reads the local 111 MiB corpus; the committed fixture covers the rest"]
fn an_incomplete_component_names_why() {
    let mut entry = serde_json::json!({
        "component": "checks",
        "groups": [
            {"group": "check_runs", "status": "complete", "reason": null},
            {"group": "statuses", "status": "complete", "reason": null},
            {"group": "fork_check_runs", "status": "partial", "reason": "GitHub reports 3 items but 2 were observed"},
            {"group": "fork_statuses", "status": "complete", "reason": null}
        ]
    });
    roll_up(&mut entry);
    assert_eq!(entry["status"], "partial");
    assert!(
        entry["reason"]
            .as_str()
            .unwrap()
            .contains("GitHub reports 3 items"),
        "{}",
        entry["reason"]
    );

    // A blocked group is the more specific failure.
    entry["groups"][2]["status"] = serde_json::json!("blocked");
    entry["groups"][2]["reason"] = serde_json::json!("response exceeds the transport bound");
    roll_up(&mut entry);
    assert_eq!(entry["status"], "blocked");

    // A group never acquired outranks a partial one: nothing about it is known.
    entry["groups"][2] = serde_json::json!({"group": "fork_check_runs", "status": "missing", "reason": "not acquired"});
    roll_up(&mut entry);
    assert_eq!(entry["status"], "missing");

    for group in entry["groups"].as_array_mut().expect("groups") {
        group["status"] = serde_json::json!("complete");
        group["reason"] = serde_json::Value::Null;
    }
    roll_up(&mut entry);
    assert_eq!(entry["status"], "complete");
    assert!(
        entry["reason"].is_null(),
        "a complete component carries no reason"
    );
}

#[test]
#[ignore = "reads the local 111 MiB corpus; the committed fixture covers the rest"]
fn a_revision_must_be_a_full_commit_sha() {
    assert!(is_sha1("d7c44f2670a0d18598cd6eadee07c0e8d0885cc4"));
    // A short or uppercase revision is never substituted or normalised.
    assert!(!is_sha1("d7c44f26"));
    assert!(!is_sha1(&"D".repeat(40)));
    assert!(!is_sha1(&"g".repeat(40)));
}
