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
    assert!(run(temp.path(), &["page"]).status.success());
    let loaded = tranche_core::report::load(&root, &Default::default()).unwrap();
    assert_eq!(loaded.assignments.as_ref(), Some(&proposal));
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
