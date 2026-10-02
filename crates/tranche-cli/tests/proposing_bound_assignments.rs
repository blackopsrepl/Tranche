use serde_json::{Value, json};
use std::{fs, path::Path};
use tranche_core::{
    domain::assignment::preprocessing::{Record, Taxonomy, binding},
    report::Root,
};

fn copy(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            copy(&entry.path(), &to.join(entry.file_name()));
        } else {
            fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
        }
    }
}
fn fixture() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    copy(&repo.join("crates/tranche-core/tests/fixture"), temp.path());
    copy(&repo.join("page"), &temp.path().join("page"));
    fs::create_dir_all(temp.path().join("input/team")).unwrap();
    fs::write(temp.path().join("skills.toml"), "[skills.docs]\ndescription = 'Documentation'\n[skills.security-review]\ndescription = 'Security'\n").unwrap();
    fs::write(
        temp.path().join("input/team/demo.md"),
        "Capacity: 3\nPRIVATE_RESUME_MARKER\nProduction documentation/security review.",
    )
    .unwrap();
    temp
}
fn run(root: &Path, args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_tranche"))
        .arg("--root")
        .arg(root)
        .args(args)
        .output()
        .unwrap()
}
#[test]
fn proposals_are_bound_private_capacity_limited_and_shared_with_page_and_mcp() {
    let temp = fixture();
    let root = Root::new(temp.path());
    let taxonomy = Taxonomy::load(&root).unwrap();
    let mut calls = 0;
    let mut stub = |_state: &Value, questions: &Value| {
        calls += 1;
        assert_eq!(questions.as_object().unwrap().len(), 2);
        assert!(
            questions
                .as_object()
                .unwrap()
                .values()
                .all(|q| q["type"] == "noul")
        );
        Ok(json!({"answers":{"docs":{"noul":0.9}, "security-review":{"noul":0.9}}}))
    };
    tranche_cli::assignment::qualify::preprocess_with(&root, false, None, &mut stub, &mut |_| {})
        .unwrap();
    assert_eq!(calls, 1);
    tranche_cli::assignment::qualify::preprocess_with(
        &root,
        false,
        None,
        &mut |_s, _q| panic!("cached"),
        &mut |_| {},
    )
    .unwrap();
    // Requirements are independent of the original seven judgments.
    let prs = tranche_core::domain::pr::load_prs(&root, "omacom/omarchy").unwrap();
    let mut rows = String::new();
    for pr in prs.iter() {
        let state = tranche_core::domain::assignment::preprocessing::requirement_state(pr);
        let r = Record {
            id: pr.number.to_string(),
            binding: binding(&state, &taxonomy, false),
            judged_at: "offline stub".into(),
            answers: json!({"answers":{"docs":{"noul":0.9},"security-review":{"noul":0.0}}}),
        };
        rows.push_str(&serde_json::to_string(&r).unwrap());
        rows.push('\n');
    }
    fs::write(root.out_dir().join("requirements.jsonl"), rows).unwrap();
    let out = run(temp.path(), &["assign"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let raw = fs::read_to_string(root.assignments_path()).unwrap();
    assert!(!raw.contains("PRIVATE_RESUME_MARKER"));
    let proposal: Value = serde_json::from_str(&raw).unwrap();
    let rows = proposal["assignments"].as_array().unwrap();
    assert!(rows.iter().filter(|r| r["member_id"].is_string()).count() <= 3);
    assert!(rows.iter().any(|r| r["member_id"].is_string()));
    assert!(run(temp.path(), &["assign"]).status.success());
    let markdown = fs::read_to_string(root.tranches_path()).unwrap();
    assert_eq!(markdown.matches("<!-- tranche-assignments -->").count(), 1);
    assert!(
        run(temp.path(), &["page", "--export-json", "--export-xlsx"])
            .status
            .success()
    );
    let export = fs::read_to_string(temp.path().join("docs/data/report.json")).unwrap();
    assert!(!export.contains("PRIVATE_RESUME_MARKER"));
    // Re-read after solving again; proposals need not be optimization-unique.
    let proposal: Value =
        serde_json::from_str(&fs::read_to_string(root.assignments_path()).unwrap()).unwrap();
    let rows = proposal["assignments"].as_array().unwrap();

    let loaded = tranche_core::report::load(&root, &Default::default()).unwrap();
    assert_eq!(loaded.assignments.as_ref(), Some(&proposal));
    let surface = mcp(temp.path(), "surface", json!({}));
    assert_eq!(
        surface["queues"]["assigned"]["count"].as_u64(),
        Some(rows.iter().filter(|r| r["member_id"].is_string()).count() as u64)
    );
    let query = mcp(
        temp.path(),
        "query",
        json!({"queue":"assigned", "limit":100}),
    );
    assert_eq!(query["total"], surface["queues"]["assigned"]["count"]);
    let page: Value = serde_json::from_str(
        &fs::read_to_string(temp.path().join("docs/data/workbench.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        page["prs"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["assignment"]["member_id"].is_string())
            .count(),
        query["total"].as_u64().unwrap() as usize
    );
    let mut modified = proposal.clone();
    modified["meaning"] = json!("tampered");
    fs::write(
        root.assignments_path(),
        serde_json::to_string(&modified).unwrap(),
    )
    .unwrap();
    assert!(tranche_core::report::load(&root, &Default::default()).is_err());
    assert!(!run(temp.path(), &["page"]).status.success());
    fs::write(
        root.assignments_path(),
        serde_json::to_string(&proposal).unwrap(),
    )
    .unwrap();
    fs::write(
        temp.path().join("input/team/demo.md"),
        "Capacity: 4\nchanged",
    )
    .unwrap();
    assert!(tranche_core::report::load(&root, &Default::default()).is_err());
    assert!(!run(temp.path(), &["page"]).status.success());
}

#[test]
fn preprocessing_dry_runs_do_not_create_caches_or_contact_a_model() {
    let temp = fixture();
    for command in ["qualify", "requirements"] {
        let out = run(temp.path(), &[command, "--dry-run", "--limit", "1"]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(String::from_utf8_lossy(&out.stdout).contains("no calls or writes"));
    }
    assert!(!temp.path().join("out/qualifications.jsonl").exists());
    assert!(!temp.path().join("out/requirements.jsonl").exists());
}

fn mcp(root: &Path, tool: &str, arguments: Value) -> Value {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new(env!("CARGO_BIN_EXE_tranche"))
        .arg("--root")
        .arg(root)
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let request = json!({"jsonrpc":"2.0", "id":1, "method":"tools/call", "params":{"name":tool,"arguments":arguments}});
    writeln!(child.stdin.take().unwrap(), "{request}").unwrap();
    let out = child.wait_with_output().unwrap();
    let response: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_ne!(response["result"]["isError"], true, "{response}");
    serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[test]
fn unknown_requirements_hold_every_pr_without_treating_absence_as_empty_skills() {
    let temp = fixture();
    assert!(run(temp.path(), &["assign"]).status.success());
    let payload: Value = serde_json::from_str(
        &fs::read_to_string(temp.path().join("out/assignments.json")).unwrap(),
    )
    .unwrap();
    assert!(
        payload["assignments"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["member_id"].is_null())
    );
    assert!(
        payload["assignments"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["evidence_known"] == false)
    );
    fs::write(
        temp.path().join("skills.toml"),
        "[skills.docs]\ndescription = 'Changed production evidence standard'\n",
    )
    .unwrap();
    assert!(!run(temp.path(), &["page"]).status.success());
    assert!(run(temp.path(), &["assign"]).status.success());
    assert!(run(temp.path(), &["page"]).status.success());
}
