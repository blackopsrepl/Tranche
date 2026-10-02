//! Judgment and pair bindings, pinned against the real corpus.
//!
//! These are cache keys. If a binding differs by one byte, resume stops matching
//! and the whole backlog is re-judged — thousands of paid model calls. The pins
//! are read from the committed corpus, so this is the check that a change did not
//! silently invalidate the cache.

use tranche_core::domain::dupe::{current_verdicts, pair_binding};
use tranche_core::domain::judge::{
    current_judgments, judgment_binding, load_done, reusable_judgment,
};
use tranche_core::domain::pr::load_prs;
use tranche_core::report::{REPOSITORY, Root};

const MODEL: &str = "jev-latest";

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
fn judgment_bindings_match_the_stored_corpus() {
    let root = corpus();
    let prs = load_prs(&root, REPOSITORY).expect("the capture loads");
    let expected = [
        (
            3507,
            "1be30deeecee233121b52ba1d7b7569d07f4431248891ef76bdcefc786aa53ca",
        ),
        (
            4593,
            "78e61a00c5582274c0971d0f137fcace684f5497b943dcc6b87106b885fbd990",
        ),
        (
            10660,
            "d5064af500a7c994a777bc66a2db92700f0374f9400a17d4e9e2413519a37fa4",
        ),
        (
            13971,
            "b1b01c132675eedce5b14a82e383d3814b973018e7d18d3fbdc7338e11b665bb",
        ),
    ];
    for (number, binding) in expected {
        let pr = prs.get(number).unwrap_or_else(|| panic!("PR {number}"));
        assert_eq!(
            judgment_binding(pr, REPOSITORY, MODEL),
            binding,
            "judgment binding for {number}"
        );
    }
}

#[test]
#[ignore = "reads the local 111 MiB corpus; the committed fixture covers the rest"]
fn every_stored_judgment_is_still_current() {
    let root = corpus();
    let prs = load_prs(&root, REPOSITORY).expect("the capture loads");
    let stored = load_done(&root).expect("the log reads");
    assert_eq!(stored.len(), 2903, "records in the append-only log");

    // The whole corpus is judged under the current policy; a binding drift shows
    // up here as a collapse to zero, which is exactly the failure that would
    // re-bill the backlog.
    let current = current_judgments(&root, &prs, REPOSITORY, MODEL, false).expect("judgments");
    assert_eq!(
        current.len(),
        2817,
        "every captured PR still has its judgment"
    );

    let reusable = current.values().filter(|j| reusable_judgment(j)).count();
    assert_eq!(reusable, 2817, "every current judgment is reusable");
}

#[test]
#[ignore = "reads the local 111 MiB corpus; the committed fixture covers the rest"]
fn pair_bindings_match_the_stored_corpus() {
    let root = corpus();
    let prs = load_prs(&root, REPOSITORY).expect("the capture loads");
    let verdicts = current_verdicts(&root, &prs).expect("the verdict log reads");
    assert_eq!(verdicts.len(), 909, "distinct pairs with a stored verdict");

    let expected = [
        (
            (4750, 7168),
            "8a355122758ab93e09eded7e9b18c27f9fc2d9658003bdcb7ccf454d2c749904",
        ),
        (
            (4829, 7882),
            "ee5f92ae6c87f308b076d9b78e13310471c6ccbcc0e5e733820ca562ebce493b",
        ),
        (
            (4928, 7700),
            "3344cf1c44285120c7bab3834f1e443f321784bc28dfcec28d53bc96730cc040",
        ),
    ];
    for ((a, b), binding) in expected {
        let pr_a = prs.get(a).unwrap_or_else(|| panic!("PR {a}"));
        let pr_b = prs.get(b).unwrap_or_else(|| panic!("PR {b}"));
        assert_eq!(
            pair_binding(pr_a, pr_b, REPOSITORY, MODEL),
            binding,
            "pair binding for ({a}, {b})"
        );
        // The binding orders the pair, so a verdict recorded the other way round
        // still verifies.
        assert_eq!(pair_binding(pr_b, pr_a, REPOSITORY, MODEL), binding);
    }
}

#[test]
#[ignore = "reads the local 111 MiB corpus; the committed fixture covers the rest"]
fn a_moved_policy_would_invalidate_every_judgment() {
    // The binding digests the question policy, which is why the policy is data
    // here rather than scattered through the code.
    let root = corpus();
    let prs = load_prs(&root, REPOSITORY).expect("the capture loads");
    let pr = prs.get(13971).expect("PR 13971");
    let baseline = judgment_binding(pr, REPOSITORY, MODEL);
    assert_ne!(
        judgment_binding(pr, REPOSITORY, "some-other-model"),
        baseline
    );
    assert_ne!(judgment_binding(pr, "other/repo", MODEL), baseline);
}

#[test]
#[ignore = "reads the local 111 MiB corpus; the committed fixture covers the rest"]
fn normalization_keeps_unknowns_unknown() {
    let root = corpus();
    let stored = load_done(&root).expect("the log reads");
    let record = stored.get(&13971).expect("PR 13971 was judged");
    let answers = record
        .get("answers")
        .and_then(|a| a.as_object())
        .expect("answers");
    // Seven questions, each normalized to exactly one answer field.
    assert_eq!(answers.len(), 7, "one answer per question");
    for (name, answer) in answers {
        let answer = answer
            .as_object()
            .unwrap_or_else(|| panic!("{name} answer"));
        assert!(
            answer.contains_key("choice")
                || answer.contains_key("score")
                || answer.contains_key("noul"),
            "{name} carries its field"
        );
    }
}
