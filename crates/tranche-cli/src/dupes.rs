//! The duplicate pass: compare candidate pairs and append the verdicts.
//!
//! Candidate selection and the stored-verdict cache are both already decided by
//! `tranche-core`; this decides which of that work to do now, asks about it, and
//! appends what comes back. The log is append-only, so a re-run never rewrites an
//! earlier verdict.

use std::io::Write;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;
use tranche_core::domain::dupe::{normalize_pair, pair_binding, reusable_pair};
use tranche_core::domain::judge::usage as token_usage;
use tranche_core::domain::pr::{Pr, Prs, load_prs};
use tranche_core::domain::questions::pair_questions;
use tranche_core::jev;
use tranche_core::report::{MODEL, REPOSITORY, Root};

use crate::pairing::{brief, outstanding, ref_index};

/// How many requests are in flight at once.
const WORKERS: usize = 6;

/// Past this many unjudged PRs, say so before comparing.
const WARN_MISSING: usize = 100;

/// One comparison to make.
struct Job {
    a: u64,
    b: u64,
    similarity: f64,
    state: Value,
    binding: String,
}

/// Compare the outstanding candidate pairs and append the verdicts.
///
/// Returns how many were compared and how many failed.
pub fn dupes(
    root: &Root,
    max_pairs: u64,
    report: &mut dyn FnMut(&str),
) -> Result<(usize, usize), String> {
    let corpus: Prs = load_prs(root, REPOSITORY).map_err(|error| error.0)?;
    let judgments = crate::pairing::judgments(root, &corpus)?;

    let missing = corpus.len().saturating_sub(judgments.len());
    if missing > WARN_MISSING {
        report(&format!(
            "warning: {missing} PRs not judged yet; the dupe pass runs on the judged subset"
        ));
    }

    let all = outstanding(root, &corpus, &judgments)?;
    let selected: Vec<(f64, u64, u64)> = all.iter().take(max_pairs as usize).copied().collect();
    report(&format!(
        "{} candidate pairs to compare ({} already current)",
        all.len().min(max_pairs as usize),
        cache_size(root, &corpus)
    ));
    if selected.is_empty() {
        return Ok((0, 0));
    }

    let jobs: Vec<Job> = selected
        .iter()
        .filter_map(|(score, a, b)| {
            // A candidate can go stale between selection and the call: a PR
            // closed and was refetched out of the corpus.
            let (left, right) = (corpus.get(*a)?, corpus.get(*b)?);
            Some(job(left, right, *score))
        })
        .collect();

    let questions = pair_questions();
    let outcomes = ask_all(&jobs, &questions);

    let mut errors = Vec::new();
    let mut written = 0usize;
    let mut tokens = (0i64, 0i64);
    let path = root.pairs_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    let mut stream = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|error| format!("cannot open the verdict log: {error}"))?;
    for (job, outcome) in jobs.iter().zip(outcomes) {
        match outcome {
            Ok(answer) => {
                let record = normalize_pair(&record_for(job, &answer));
                let line = serde_json::to_string(&record)
                    .map_err(|error| format!("cannot encode a verdict: {error}"))?;
                writeln!(stream, "{line}")
                    .map_err(|error| format!("cannot append to the verdict log: {error}"))?;
                let (input, output) = token_usage(&record);
                tokens.0 += input;
                tokens.1 += output;
                written += 1;
            }
            Err(error) => errors.push(format!("#{} ↔ #{}: {error}", job.a, job.b)),
        }
    }
    stream
        .flush()
        .map_err(|error| format!("cannot flush the verdict log: {error}"))?;

    report("candidate pair comparison complete");
    report(&format!("tokens: in={} out={}", tokens.0, tokens.1));
    for error in errors.iter().take(10) {
        report(error);
    }
    Ok((written, errors.len()))
}

/// How many stored verdicts are still current, for the progress line.
fn cache_size(root: &Root, corpus: &Prs) -> usize {
    tranche_core::domain::dupe::pair_cache(root, corpus, REPOSITORY, MODEL)
        .map(|cache| {
            cache
                .values()
                .filter(|record| {
                    reusable_pair(record) && record["freshness"].as_str() == Some("current")
                })
                .count()
        })
        .unwrap_or(0)
}

/// One comparison, ready to ask.
fn job(left: &Pr, right: &Pr, similarity: f64) -> Job {
    let (a, b) = (left.number, right.number);
    Job {
        a,
        b,
        similarity,
        state: serde_json::json!({"pr_a": brief(left), "pr_b": brief(right)}),
        binding: pair_binding(left, right, REPOSITORY, MODEL),
    }
}

/// The verdict a comparison is stored as.
///
/// The field set is a compatibility surface: `normalize_pair` reads these and the
/// binding is the cache key.
fn record_for(job: &Job, answer: &Value) -> Value {
    let sameness = answer
        .get("answers")
        .and_then(|answers| answers.get("sameness"))
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    serde_json::json!({
        "a": job.a,
        "b": job.b,
        // Three places, as the pipeline has always rounded it.
        "similarity": (job.similarity * 1000.0).round() / 1000.0,
        "verdict": sameness.get("choice"),
        "probabilities": sameness.get("probabilities"),
        "usage": answer.get("usage").cloned().unwrap_or_else(|| serde_json::json!({})),
        "binding": job.binding,
        "requested_model": MODEL,
        "resolved_model": answer.get("model"),
        "request_id": answer.get("request_id"),
        "judged_at": judged_at(),
        "input": job.state,
    })
}

/// Ask about every job, at most `WORKERS` at a time.
///
/// Each job's outcome is kept, so one failure does not lose the others. A
/// rejected key stops the pass rather than spending it on requests that cannot
/// succeed.
fn ask_all(jobs: &[Job], questions: &Value) -> Vec<Result<Value, String>> {
    let outcomes: Mutex<Vec<Option<Result<Value, String>>>> = Mutex::new(vec![None; jobs.len()]);
    let fatal: Mutex<Option<String>> = Mutex::new(None);
    let next = AtomicUsize::new(0);

    std::thread::scope(|scope| {
        for _ in 0..WORKERS.min(jobs.len()) {
            scope.spawn(|| {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        *fatal.lock().expect("lock") = Some(format!("no runtime: {error}"));
                        return;
                    }
                };
                loop {
                    if fatal.lock().expect("lock").is_some() {
                        return;
                    }
                    let index = next.fetch_add(1, Ordering::SeqCst);
                    let Some(job) = jobs.get(index) else { return };
                    let outcome = runtime
                        .block_on(jev::ask(&job.state, questions, MODEL))
                        .map_err(|error| error.to_string());
                    if let Err(message) = &outcome
                        && message.contains("401")
                    {
                        *fatal.lock().expect("lock") = Some(message.clone());
                    }
                    outcomes.lock().expect("lock")[index] = Some(outcome);
                }
            });
        }
    });

    if let Some(message) = fatal.lock().expect("lock").take() {
        return jobs
            .iter()
            .enumerate()
            .map(
                |(index, _)| match outcomes.lock().expect("lock")[index].take() {
                    Some(outcome) => outcome,
                    None => Err(message.clone()),
                },
            )
            .collect();
    }
    outcomes
        .into_inner()
        .expect("lock")
        .into_iter()
        .enumerate()
        .map(|(index, outcome)| {
            outcome.unwrap_or_else(|| Err(format!("job {index} never completed")))
        })
        .collect()
}

/// The reference index the corpus contributes, so a pair can be nominated by a
/// literal mention rather than only by prose resemblance.
pub fn references(corpus: &Prs) -> std::collections::HashMap<u64, Vec<u64>> {
    ref_index(corpus)
}
/// The judgment time, in the ISO-8601 form the log records.
fn judged_at() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| String::new())
}
