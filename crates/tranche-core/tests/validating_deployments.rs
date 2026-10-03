use serde_json::{Value, json};
use tranche_core::policy::Contract;

fn valid() -> Value {
    serde_json::from_str(include_str!("../../../tranche.json")).unwrap()
}

#[test]
fn additional_questions_are_refused_in_v1() {
    for half in ["judge", "pair"] {
        let mut value = valid();
        value["policy"][half]["extra"] = json!({"type": "noul", "instructions": "Required check"});
        assert!(
            Contract::from_value(value, "contract.json").is_err(),
            "unsupported {half} question"
        );
    }
}

#[test]
fn versions_and_model_are_validated_at_the_boundary() {
    for pointer in ["/version", "/policy/version", "/model"] {
        for bad in [Value::Null, json!(2), json!(""), json!("   ")] {
            let mut value = valid();
            *value.pointer_mut(pointer).unwrap() = bad;
            assert!(
                Contract::from_value(value, "contract.json").is_err(),
                "{pointer}"
            );
        }
    }
}

#[test]
fn malformed_repository_identifiers_are_refused() {
    for repository in [
        "a/b/c", "a/b?x", "a/b#x", "a/b%2fc", "a/b c", "a/..", "a/.", "-a/b", "a-/b", "a--b/c",
        "a_b/c", "a/é", "a\\b/c",
    ] {
        let mut value = valid();
        value["repository"] = json!(repository);
        assert!(
            Contract::from_value(value, "contract.json").is_err(),
            "{repository}"
        );
    }
    for repository in ["Owner-1/repo.name_2", "a/b"] {
        let mut value = valid();
        value["repository"] = json!(repository);
        assert_eq!(
            Contract::from_value(value, "contract.json")
                .unwrap()
                .repository(),
            repository
        );
    }
}

#[test]
fn required_questions_have_v1_shapes() {
    for (half, name, kind, length) in [
        ("judge", "category", "choice", 0),
        ("judge", "risk", "score", 5),
        ("judge", "is_fix", "noul", 0),
        ("judge", "dupe_signal", "noul", 0),
        ("judge", "finished_form", "score", 4),
        ("judge", "review_effort", "score", 4),
        ("judge", "security_flag", "noul", 0),
        ("pair", "sameness", "choice", 0),
    ] {
        let pointer = format!("/policy/{half}/{name}");
        for bad in [
            Value::Null,
            json!({"type": kind}),
            json!({"type": "other", "instructions": "q"}),
        ] {
            let mut value = valid();
            *value.pointer_mut(&pointer).unwrap() = bad;
            assert!(
                Contract::from_value(value, "contract.json").is_err(),
                "{pointer}"
            );
        }
        for bad in [
            Value::Null,
            json!(""),
            json!({}),
            json!({"question": " "}),
            json!(7),
        ] {
            let mut value = valid();
            value["policy"][half][name]["instructions"] = bad;
            assert!(
                Contract::from_value(value, "contract.json").is_err(),
                "{pointer} instructions"
            );
        }
        if kind != "noul" {
            for bad in [
                Value::Null,
                json!([]),
                json!({}),
                json!({"x": 2}),
                json!(["q"]),
            ] {
                let mut value = valid();
                value["policy"][half][name]["criteria"] = bad;
                assert!(
                    Contract::from_value(value, "contract.json").is_err(),
                    "{pointer} criteria"
                );
            }
            if kind == "score" {
                let mut value = valid();
                value["policy"][half][name]["criteria"] = json!(vec![""; length]);
                assert!(Contract::from_value(value, "contract.json").is_err());
            }
        }
    }
    let mut value = valid();
    value["policy"]["pair"]["sameness"]["criteria"] = json!({"arbitrary": "description"});
    assert!(Contract::from_value(value, "contract.json").is_err());
}

#[test]
fn historical_policy_values_are_not_rewritten() {
    let value = valid();
    let contract = Contract::from_value(value.clone(), "contract.json").unwrap();
    assert_eq!(contract.judge_questions(), &value["policy"]["judge"]);
    assert_eq!(contract.pair_questions(), &value["policy"]["pair"]);
}
