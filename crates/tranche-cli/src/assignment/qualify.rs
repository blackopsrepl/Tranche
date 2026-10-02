//! Incremental preprocessing; no model calls occur in assign or the solver.
use serde_json::Value;
use std::io::Write;
use tranche_core::{
    domain::{
        assignment::{
            preprocessing::{Record, Taxonomy, binding, current, questions, records},
            team::resumes,
        },
        pr::load_prs,
    },
    jev,
    report::{MODEL, REPOSITORY, Root},
};

pub fn preprocess_options(
    root: &Root,
    requirements: bool,
    limit: Option<u64>,
    dry_run: bool,
    force: bool,
    report: &mut dyn FnMut(&str),
) -> Result<(usize, usize), String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    process(
        root,
        requirements,
        limit,
        dry_run,
        force,
        &mut |state, questions| {
            runtime
                .block_on(jev::ask(state, questions, MODEL))
                .map_err(|e| e.to_string())
        },
        report,
    )
}
pub fn preprocess_with(
    root: &Root,
    requirements: bool,
    limit: Option<u64>,
    ask: &mut dyn FnMut(&Value, &Value) -> Result<Value, String>,
    report: &mut dyn FnMut(&str),
) -> Result<(usize, usize), String> {
    process(root, requirements, limit, false, false, ask, report)
}
#[allow(clippy::too_many_arguments)]
fn process(
    root: &Root,
    requirements: bool,
    limit: Option<u64>,
    dry_run: bool,
    force: bool,
    ask: &mut dyn FnMut(&Value, &Value) -> Result<Value, String>,
    report: &mut dyn FnMut(&str),
) -> Result<(usize, usize), String> {
    let taxonomy = Taxonomy::load(root)?;
    let qualification = !requirements;
    let path = root.out_dir().join(if requirements {
        "requirements.jsonl"
    } else {
        "qualifications.jsonl"
    });
    let cache = records(&path)?;
    let mut sources: Vec<(String, Value)> = if requirements {
        load_prs(root, REPOSITORY)
            .map_err(|e| e.to_string())?
            .iter()
            .map(|p| {
                (
                    p.number.to_string(),
                    tranche_core::domain::assignment::preprocessing::requirement_state(p),
                )
            })
            .collect()
    } else {
        resumes(root)?
            .iter()
            .map(|r| (r.id.clone(), r.state()))
            .collect()
    };
    if sources.is_empty() {
        return Err("no preprocessing sources; add input/team resumes or capture PRs".into());
    }
    sources.sort_by(|a, b| a.0.cmp(&b.0));
    let todo: Vec<_> = sources
        .into_iter()
        .filter(|(id, state)| {
            force || current(&cache, id, state, &taxonomy, qualification).is_none()
        })
        .take(limit.unwrap_or(u64::MAX) as usize)
        .collect();
    report(&format!(
        "{} sources need preprocessing (limit applied)",
        todo.len()
    ));
    if dry_run {
        report(&format!(
            "{} model calls planned; no calls or writes",
            todo.len()
        ));
        return Ok((0, 0));
    }
    if todo.is_empty() {
        return Ok((0, 0));
    }
    std::fs::create_dir_all(root.out_dir()).map_err(|e| e.to_string())?;
    let mut stream = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    let questions = questions(&taxonomy, qualification);
    let mut written = 0;
    let mut errors = 0;
    for (id, state) in todo {
        match ask(&state, &questions) {
            Ok(answers) => {
                let row = Record {
                    id,
                    binding: binding(&state, &taxonomy, qualification),
                    answers,
                    judged_at: tranche_core::evidence::now(),
                };
                let value = serde_json::to_value(row).map_err(|e| e.to_string())?;
                writeln!(
                    stream,
                    "{}",
                    tranche_core::util::spaced_json(&value, false).map_err(|e| e.to_string())?
                )
                .map_err(|e| e.to_string())?;
                stream.flush().map_err(|e| e.to_string())?;
                written += 1;
            }
            // Do not print a provider body that could echo private resume text.
            Err(_) => {
                errors += 1;
                report(&format!(
                    "{id}: preprocessing request failed; source remains unknown"
                ));
            }
        }
    }
    Ok((written, errors))
}
