use serde_json::json;
use tranche_core::domain::assignment::preprocessing::{
    Taxonomy, binding, probabilities, questions,
};

#[test]
fn independent_supported_nouls_share_one_taxonomy() {
    let taxonomy =
        Taxonomy::parse("[skills.docs]\ndescription = 'Documentation review'\n").unwrap();
    let q = questions(&taxonomy, true);
    assert_eq!(q["docs"]["type"], "noul");
    assert!(
        q["docs"]["instructions"]
            .to_string()
            .contains("Documentation review")
    );
    let other =
        Taxonomy::parse("[skills.docs]\ndescription = 'Changed evidence standard'\n").unwrap();
    assert_ne!(
        binding(&json!({"resume_text":"same"}), &taxonomy, true),
        binding(&json!({"resume_text":"same"}), &other, true)
    );
    let p = probabilities(&json!({"answers":{"docs":{"noul":0.9}}}), &taxonomy);
    assert_eq!(p["docs"], Some(0.9));
    assert_eq!(
        probabilities(&json!({"answers":{"docs":{"noul":2}}}), &taxonomy)["docs"],
        None
    );
    assert_eq!(probabilities(&json!({}), &taxonomy)["docs"], None);
}

#[test]
fn declared_skills_never_substitute_for_a_current_qualification() {
    use tranche_core::{
        domain::assignment::{
            preprocessing::{Record, binding},
            team,
        },
        report::Root,
    };
    let temp = tempfile::tempdir().unwrap();
    let root = Root::new(temp.path());
    std::fs::create_dir_all(temp.path().join("input/team")).unwrap();
    std::fs::create_dir_all(temp.path().join("out")).unwrap();
    std::fs::write(
        temp.path().join("input/team/a.md"),
        "Skills: docs
Capacity: 2
Production docs work",
    )
    .unwrap();
    let t = Taxonomy::parse(
        "[skills.docs]
description = 'Docs'
",
    )
    .unwrap();
    let m = team::members(&root, &t).unwrap();
    assert!(!m[0].evidence_known);
    assert!(m[0].qualified_skills.is_empty());
    let state = team::resumes(&root).unwrap()[0].state();
    let record = Record {
        id: "synthetic-a".into(),
        binding: binding(&state, &t, true),
        answers: json!({"answers":{"docs":{"noul":0.9}}}),
        judged_at: "test".into(),
    };
    std::fs::write(
        temp.path().join("out/qualifications.jsonl"),
        serde_json::to_string(&record).unwrap(),
    )
    .unwrap();
    assert!(team::members(&root, &t).unwrap()[0].evidence_known);
    std::fs::write(
        temp.path().join("input/team/a.md"),
        "Skills: docs
Capacity: 2
Changed evidence",
    )
    .unwrap();
    assert!(!team::members(&root, &t).unwrap()[0].evidence_known);
}
