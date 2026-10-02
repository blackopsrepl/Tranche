//! `evidence capture`: validate a batch and acquire or resume its evidence.
//!
//! The only evidence command that spends a GitHub budget. It refuses an unknown batch,
//! a stored capture bound to a different selection, and a completion check it cannot
//! afford, rather than producing evidence that describes the wrong work.

use serde_json::Value;
use tranche_core::domain::pr::load_prs;
use tranche_core::evidence::capture;
use tranche_core::evidence::report;
use tranche_core::evidence::{Plan, select};
use tranche_core::policy::Contract;
use tranche_core::report::Root;

use crate::commands::Outcome;

use super::reading::{print_coverage, resume_hint, resume_target};
use crate::cli::Capture;
use serde_json::json;
use tranche_core::evidence::EXIT_INCOMPLETE;
use tranche_core::evidence::capture::Stopped;

/// `evidence capture`: validate a batch and acquire or resume its evidence.
pub fn capture_evidence(root: &Root, args: &Capture, json_output: bool) -> Outcome {
    let say = |line: &str| println!("{line}");
    let contract = match Contract::load(root.path()) {
        Ok(contract) => contract,
        Err(error) => return Outcome::refusal(error, 1),
    };
    let corpus = match load_prs(root, contract.repository()) {
        Ok(corpus) => corpus,
        Err(error) => return Outcome::refusal(format!("corpus: {}", error.0), 1),
    };
    let plan = match Plan::read(root) {
        Ok(plan) => plan,
        Err(error) => return Outcome::refusal(error.0, 1),
    };
    let selection = match select(root, &args.batch, &corpus, &plan) {
        Ok(selection) => selection,
        Err(error) => return Outcome::refusal(error.0, 3),
    };

    // Resume an existing capture of this selection, or start a fresh generation.
    let existing = if args.fresh {
        None
    } else {
        match args.reuse_capture.as_deref() {
            Some(id) => Some(id.to_owned()),
            None => resume_target(root, &selection),
        }
    };
    let capture_id = existing.unwrap_or_else(capture::new_capture_id);

    // A stored capture of a different selection is refused rather than extended:
    // the evidence would describe two different batches under one identity.
    if let Ok(manifest) = capture::stored_manifest(root, &capture_id)
        && manifest["selection"] != selection.as_json()
    {
        return Outcome::refusal(
            format!(
                "capture {capture_id} belongs to a different selection; \
                 omit --reuse-capture to start a fresh one"
            ),
            3,
        );
    }

    let manifest = match capture::stored_manifest(root, &capture_id) {
        Ok(manifest) => manifest,
        Err(_) => {
            capture::fresh_manifest(&selection, &capture_id, args.request_budget, args.max_bytes)
        }
    };

    let mut lock = match capture::lock_for(root, &capture_id) {
        Ok(lock) => lock,
        Err(error) => return Outcome::refusal(error, 3),
    };
    if let Err(error) = lock.acquire(args.break_lock) {
        return Outcome::refusal(error.0, 3);
    }

    say(&format!(
        "capture {capture_id} for batch {} ({} members, {} components each, budget {})",
        args.batch,
        selection.members.len(),
        tranche_core::evidence::COMPONENTS.len(),
        args.request_budget
    ));
    let outcome = {
        let mut run = match capture::Capture::new(
            root,
            selection,
            capture_id.clone(),
            manifest,
            args.request_budget,
            args.max_bytes,
        ) {
            Ok(run) => run,
            Err(error) => {
                lock.release();
                return Outcome::refusal(error, 3);
            }
        };
        let stopped = run.run();
        for line in &run.log {
            say(&format!("  {line}"));
        }
        // Re-read the manifest the run wrote: the checkpoint is the record, and the
        // in-memory copy is only what the run last knew.
        let stored = match capture::stored_manifest(root, &capture_id) {
            Ok(stored) => stored,
            Err(_) => run.manifest().clone(),
        };
        (stopped, report::coverage(&stored))
    };
    lock.release();

    let (stopped, coverage) = outcome;
    if json_output {
        let payload = json!({
            "coverage": coverage,
            "next": if coverage["complete"].as_bool() == Some(true) {
                Value::Null
            } else {
                json!(resume_hint(&args.batch, &capture_id))
            },
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_owned())
        );
    } else {
        print_coverage(&coverage);
        if coverage["complete"].as_bool() != Some(true) {
            say(&format!("  {}", resume_hint(&args.batch, &capture_id)));
        }
    }
    // A stop reason is a fact about the run, reported in the same place the coverage
    // is, so a caller reads one block rather than two.
    match stopped {
        Stopped::Finished => {}
        Stopped::Budget => {
            say("  stopped: the request budget could not cover the completion checks")
        }
        Stopped::Transport => say("  stopped: the transport failed"),
        Stopped::RevisionDrift => say("  stopped: a member moved, so start a fresh capture"),
    }
    if coverage["complete"].as_bool() == Some(true) {
        Outcome::success(String::new())
    } else {
        // A usable but incomplete capture is its own outcome: the exit code tells a
        // caller to resume rather than that something failed.
        Outcome::refusal(
            format!("capture {capture_id} is incomplete"),
            EXIT_INCOMPLETE,
        )
    }
}
