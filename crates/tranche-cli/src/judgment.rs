//! The judgment pass: ask the model about each unjudged PR and append the
//! answers.
//!
//! The log is append-only, so a re-run never rewrites an earlier answer and a
//! truncated tail costs only the record it truncated. Resume reuses a record only
//! when its binding still matches today's evidence, question set and model, which
//! is what stops a question-policy change from silently reusing stale answers.

use std::collections::HashMap;
use std::io::Write;
use std::sync::Mutex;

use serde_json::Value;
use tranche_core::domain::judge::{
    Judgment, current_judgments, judgment_binding, normalize_judgment, reusable_judgment,
};
use tranche_core::domain::pr::{Pr, Prs, load_prs};
use tranche_core::jev;
use tranche_core::policy::Contract;
use tranche_core::report::Root;

/// How many requests are in flight at once.
const WORKERS: usize = 6;

/// Record one answer for a PR.
struct Job {
    number: u64,
    state: Value,
    binding: String,
    title: String,
    evidence_digest: String,
    head_sha: Option<String>,
    updated: String,
}

/// Judge every unjudged PR, newest first.
///
/// Returns how many were asked and how many failed.
pub fn judge(
    root: &Root,
    resume: bool,
    limit: Option<u64>,
    report: &mut dyn FnMut(&str),
) -> Result<(usize, usize), String> {
    let contract = Contract::load(root.path())?;
    let repository = contract.repository().to_owned();
    let model = contract.model().to_owned();
    let questions = contract.judge_questions().clone();
    let corpus: Prs = load_prs(root, &repository).map_err(|error| error.0)?;
    if corpus.is_empty() {
        return Err(format!(
            "no captured PR membership under {}; fetch first",
            root.pages_dir().display()
        ));
    }

    // Reusable means current, complete and bound: a record that is merely present
    // is not enough, or a question change would go unnoticed.
    let already: HashMap<u64, Judgment> = if resume {
        current_judgments(root, &corpus, &repository, &model, &questions, false)?
            .into_iter()
            .filter(|(_, judgment)| reusable_judgment(judgment))
            .collect()
    } else {
        HashMap::new()
    };

    let mut ordered: Vec<&Pr> = corpus.iter().collect();
    // Newest first: the backlog is read from the top.
    ordered.sort_by_key(|pr| std::cmp::Reverse(pr.number));
    let mut todo: Vec<&Pr> = ordered
        .into_iter()
        .filter(|pr| !already.contains_key(&pr.number))
        .collect();
    if let Some(limit) = limit {
        todo.truncate(limit as usize);
    }

    report(&format!(
        "{} PRs, {} already judged, {} to go",
        corpus.len(),
        already.len(),
        todo.len()
    ));
    if todo.is_empty() {
        return Ok((0, 0));
    }

    let path = root.judgments_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    if !resume {
        // A fresh pass starts the log over. Resume appends to what is there.
        std::fs::write(&path, "").map_err(|error| format!("cannot reset the log: {error}"))?;
    }

    let jobs: Vec<Job> = todo
        .iter()
        .map(|pr| Job {
            number: pr.number,
            state: tranche_core::domain::pr::pr_state(pr),
            binding: judgment_binding(pr, &repository, &model, &questions),
            title: pr.title.clone(),
            evidence_digest: pr.evidence_digest.clone(),
            head_sha: pr.head_sha.clone(),
            updated: pr.updated.clone(),
        })
        .collect();

    let outcomes = ask_all(&jobs, &questions, &model);

    // Append in a deterministic order. The log's order is not digested, but an
    // unordered append makes a resumed pass hard to read back.
    let mut errors = Vec::new();
    let mut written = 0usize;
    let mut stream = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|error| format!("cannot open the log: {error}"))?;
    for (job, outcome) in jobs.iter().zip(outcomes) {
        match outcome {
            Ok(answer) => {
                let record = normalize_judgment(&record_for(job, &answer, &model), &questions);
                let line = serde_json::to_string(&record)
                    .map_err(|error| format!("cannot encode a judgment: {error}"))?;
                writeln!(stream, "{line}")
                    .map_err(|error| format!("cannot append to the log: {error}"))?;
                written += 1;
            }
            Err(error) => errors.push(format!("#{}: {error}", job.number)),
        }
    }
    stream
        .flush()
        .map_err(|error| format!("cannot flush the log: {error}"))?;

    report(&format!(
        "done: {written} judgments appended; errors: {}",
        errors.len()
    ));
    for error in errors.iter().take(10) {
        report(error);
    }
    Ok((written, errors.len()))
}

/// The record a judgment is stored as.
///
/// The field set is a compatibility surface: `normalize_judgment` reads these and
/// the binding is the cache key, so a missing field makes a record unusable
/// rather than merely incomplete.
fn record_for(job: &Job, answer: &Value, model: &str) -> Value {
    serde_json::json!({
        "number": job.number,
        "title": job.title,
        "answers": answer.get("answers"),
        "usage": answer.get("usage").cloned().unwrap_or_else(|| serde_json::json!({})),
        "binding": job.binding,
        "input": job.state,
        "source_digest": job.evidence_digest,
        "head_sha": job.head_sha,
        "updated_at": job.updated,
        "requested_model": model,
        "judged_at": judged_at(),
        "resolved_model": answer.get("model"),
        "request_id": answer.get("request_id"),
    })
}

/// Ask about every job, at most `WORKERS` at a time.
///
/// Each job's outcome is kept, so one failure does not lose the others. A
/// rejected key is fatal: it cannot succeed for any of the remaining jobs, so the
/// pass stops rather than spending five hundred failed requests.
fn ask_all(jobs: &[Job], questions: &Value, model: &str) -> Vec<Result<Value, String>> {
    let outcomes: Mutex<Vec<Option<Result<Value, String>>>> = Mutex::new(vec![None; jobs.len()]);
    let fatal: Mutex<Option<String>> = Mutex::new(None);
    let next = std::sync::atomic::AtomicUsize::new(0);

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
                    let index = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let Some(job) = jobs.get(index) else { return };
                    let outcome = runtime
                        .block_on(jev::ask(&job.state, questions, model))
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
        // Every unfinished job reports the same cause rather than a silent gap.
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

/// The judgment time, in the ISO-8601 form the log records.
fn judged_at() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| String::new())
}
