//! `tranche init`: write the starter deployment contract.
//!
//! One command turns an empty directory into a Tranche deployment root. The
//! starter policy is deliberately generic — fix/chore/docs-style categories, the
//! same seven answer shapes the engine reads — and the operator is expected to
//! edit it: the policy is a frozen contract, so its wording deserves the same
//! care the repository's own review standards get.

use std::io::Write;

use tranche_core::policy::Contract;
use tranche_core::report::Root;
use tranche_core::util::indented_json;

use crate::commands::Outcome;

/// The starter policy: generic categories, the engine's answer schema.
fn starter(repository: &str) -> serde_json::Value {
    serde_json::json!({
        "version": 1,
        "repository": repository,
        "model": "jev-latest",
        "display": {
            "title": format!("TRIAGE with Tranche — {repository}"),
            "repo_url": format!("https://github.com/{repository}"),
            "category_labels": {
                "fix": "Fixes",
                "feature": "Features",
                "config": "Config & Build",
                "docs": "Docs",
                "chore": "Chores",
                "unclear": "Unclear"
            }
        },
        "policy": {
            "version": 1,
            "judge": {
                "category": {
                    "type": "choice",
                    "instructions": {
                        "question": "Which area does this pull request mainly touch? Read `pr.title` and `pr.body`; `pr.diffstat` shows the size. Pick exactly one; use `unclear` only when the text is too thin to tell."
                    },
                    "criteria": {
                        "fix": "repairs a bug or regression in existing behavior",
                        "feature": "adds new capability or changes existing behavior by design",
                        "config": "build files, dependency bumps, CI, packaging",
                        "docs": "documentation only",
                        "chore": "maintenance that fits none of the above",
                        "unclear": "cannot be placed from title and body alone"
                    }
                },
                "risk": {
                    "type": "score",
                    "instructions": {
                        "question": "How risky is merging this pull request for existing installations? Judge from `pr.title`, `pr.body` and `pr.diffstat`."
                    },
                    "criteria": [
                        "Text-only: docs and comments; nothing executes",
                        "Config or script change that is scoped and revertible",
                        "Changes system-wide defaults or runs with elevated privileges",
                        "Touches networking, permissions, security posture, or drivers/firmware",
                        "Could break existing installs outright: data loss, boot failure, or lockout"
                    ]
                },
                "is_fix": {
                    "type": "noul",
                    "instructions": "Is this pull request primarily a fix for a bug or regression, rather than a new feature or a refactor? Judge from `pr.title` and `pr.body`."
                },
                "dupe_signal": {
                    "type": "noul",
                    "instructions": "Do `pr.title` or `pr.body` indicate this pull request duplicates another change, or is superseded by / supersedes one?"
                },
                "finished_form": {
                    "type": "score",
                    "instructions": {
                        "question": "Is this pull request in finished, reviewable form as described by `pr.body` (with `pr.title`)?"
                    },
                    "criteria": [
                        "Empty or near-empty body; no description of what or why",
                        "Says what it does but not why, or shows no evidence it was tried",
                        "Clear what and why; states that it was tested on a real system",
                        "Clear what and why plus concrete QA evidence; small and focused"
                    ]
                },
                "review_effort": {
                    "type": "score",
                    "instructions": {
                        "question": "How much reviewer effort does this pull request need, judging by `pr.diffstat` and the change described?"
                    },
                    "criteria": [
                        "Trivial and mechanical: a typo, version number, or one-line constant",
                        "Small: one focused change a reviewer can hold in their head",
                        "Moderate: several related edits that must be checked together",
                        "Substantial: architectural or many-part change needing deep review"
                    ]
                },
                "security_flag": {
                    "type": "noul",
                    "instructions": "Does this change touch credentials or secrets, download-and-execute remote code, permission changes, network exposure, or crypto material? Judge from `pr.title` and `pr.body`."
                }
            },
            "pair": {
                "sameness": {
                    "type": "choice",
                    "instructions": {
                        "question": "Do `pr_a` and `pr_b` propose the same underlying change? Judge by what they modify and the outcome, not by wording."
                    },
                    "criteria": {
                        "same_change": "two attempts at the same change; merging one makes the other redundant",
                        "related_but_different": "same area or theme but distinct outcomes; both could merge",
                        "unrelated": "different changes that merely share words"
                    }
                }
            }
        }
    })
}

/// Write `tranche.json` for `repository`, refusing an existing contract.
pub fn init(root: &Root, repository: &str, force: bool) -> Outcome {
    if let Err(error) = tranche_core::policy::parse_repository(repository) {
        return Outcome::refusal(error, 1);
    }
    let path = root.path().join(Contract::FILE_NAME);
    if path.exists() && !force {
        return Outcome::refusal(
            format!(
                "{} already exists; pass --force to overwrite (this re-binds every judgment)",
                path.display()
            ),
            1,
        );
    }
    let contract = starter(repository);
    let mut encoded = indented_json(&contract).unwrap_or_default();
    encoded.push('\n');
    match std::fs::File::create(&path).and_then(|mut file| file.write_all(encoded.as_bytes())) {
        Ok(()) => Outcome::success(format!(
            "wrote {} — edit the policy before judging: any wording change re-asks the corpus\n",
            path.display()
        )),
        Err(error) => Outcome::refusal(format!("cannot write {}: {error}", path.display()), 1),
    }
}
