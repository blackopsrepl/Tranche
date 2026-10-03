//! Command dispatch.
//!
//! Every operation resolves to one entry point. Dispatch itself decides nothing
//! about a report: it picks the handler and turns the result into an exit code.

use tranche_core::report::Root;

use crate::cli::{Cli, Command, Evidence};
use crate::report_files::read_json;
use crate::reports::{batches as build_batches, cluster_report};

/// What the operator sees, and the status the process exits with.
///
/// Exit codes are an interface — scripts branch on them — so each outcome is
/// constructed in one place rather than chosen ad hoc at the call site.
#[derive(Debug)]
pub struct Outcome {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Outcome {
    /// A completed operation.
    pub fn success(stdout: String) -> Self {
        Self {
            code: 0,
            stdout,
            stderr: String::new(),
        }
    }

    /// A refusal: a message for the operator and the status that reports it.
    pub fn refusal(message: impl Into<String>, code: i32) -> Self {
        Self {
            code,
            stdout: String::new(),
            stderr: format!("{}\n", message.into()),
        }
    }
}

pub fn run(cli: &Cli) -> Outcome {
    let root = Root::new(&cli.root);
    match &cli.command {
        Command::Evidence { command } => evidence(&root, command, cli.json),
        Command::Init { repository, force } => crate::init::init(&root, repository, *force),
        Command::Cluster(args) => cluster_report(&root, args.allow_unbound, cli.json),
        Command::Batches => build_batches(&root, cli.json),
        Command::Info => print_info(&root, cli.json),
        Command::Fetch(args) => run_fetch(&root, args.transport),
        Command::Judge(args) => run_judge(&root, args.resume, args.limit),
        Command::Dupes(args) => run_dupes(&root, args.max_pairs),
        Command::Refresh(args) => crate::refresh::refresh(
            &root,
            args.max_pairs,
            args.no_page,
            args.dry_run,
            &mut |line: &str| println!("{line}"),
        ),
        Command::All(args) => run_all(&root, args.limit, args.max_pairs),
        Command::Page(args) => crate::workbench::page(
            &root,
            args.export_json,
            args.export_xlsx,
            &mut |line: &str| println!("{line}"),
        ),
        Command::Mcp => match crate::mcp::serve(&root) {
            Ok(()) => Outcome::success(String::new()),
            Err(error) => Outcome::refusal(error, 1),
        },
    }
}

/// Judge, then compare, then rebuild the report and the batches.
///
/// The same steps `refresh` runs minus the fetch and the page, so it is the
/// command for a corpus that is already captured.
fn run_all(root: &Root, limit: Option<u64>, max_pairs: u64) -> Outcome {
    let mut say = |line: &str| println!("{line}");
    if let Err(error) =
        crate::checkpoint::pass_result(crate::judgment::judge(root, true, limit, &mut say))
    {
        return Outcome::refusal(format!("all stopped at judge: {error}"), 1);
    }
    if let Err(error) =
        crate::checkpoint::pass_result(crate::dupes::dupes(root, max_pairs, &mut say))
    {
        return Outcome::refusal(format!("all stopped at dupes: {error}"), 1);
    }
    let cluster = crate::reports::cluster_report(root, false, false);
    if cluster.code != 0 {
        return Outcome::refusal(
            format!("all stopped at cluster: {}", cluster.stderr.trim()),
            1,
        );
    }
    let batches = crate::reports::batches(root, false);
    if batches.code != 0 {
        return Outcome::refusal(
            format!("all stopped at batches: {}", batches.stderr.trim()),
            1,
        );
    }
    Outcome::success(String::new())
}

/// Compare the outstanding candidate pairs.
fn run_dupes(root: &Root, max_pairs: u64) -> Outcome {
    let mut progress = |line: &str| println!("{line}");
    match crate::dupes::dupes(root, max_pairs, &mut progress) {
        Ok((written, errors)) => {
            if errors > 0 {
                return Outcome::refusal(
                    format!("{written} verdicts appended; {errors} failed; re-run to continue"),
                    1,
                );
            }
            Outcome::success(String::new())
        }
        Err(error) => Outcome::refusal(error, 1),
    }
}

/// Judge every unjudged PR, newest first.
fn run_judge(root: &Root, resume: bool, limit: Option<u64>) -> Outcome {
    let mut progress = |line: &str| println!("{line}");
    match crate::judgment::judge(root, resume, limit, &mut progress) {
        Ok((written, errors)) => {
            if errors > 0 {
                // A partial pass is not a failure, but the operator must see it,
                // and re-running with `--resume` picks up where it stopped.
                return Outcome::refusal(
                    format!("{written} judgments appended; {errors} failed; re-run with --resume"),
                    1,
                );
            }
            Outcome::success(String::new())
        }
        Err(error) => Outcome::refusal(error, 1),
    }
}

/// Refresh the observed open-PR membership.
fn run_fetch(root: &Root, transport: crate::cli::Transport) -> Outcome {
    let transport = match transport {
        crate::cli::Transport::Gh => tranche_core::gh::Transport::Gh,
        crate::cli::Transport::Curl => tranche_core::gh::Transport::Curl,
        crate::cli::Transport::Urllib => tranche_core::gh::Transport::Urllib,
    };
    let mut progress = |line: &str| println!("{line}");
    match crate::pipeline::fetch(root, transport, &mut progress) {
        Ok(_) => Outcome::success(String::new()),
        Err(error) => Outcome::refusal(error, 1),
    }
}

/// Print the numbers from the current report.
///
/// Read from the summary on disk rather than recomputed, so this reports what
/// was published.
fn print_info(root: &Root, json: bool) -> Outcome {
    let summary = match read_json(&root.summary_path()) {
        Ok(summary) => summary,
        Err(error) => return Outcome::refusal(format!("summary.json: {error}"), 1),
    };
    if json {
        return Outcome::success(format!(
            "{}\n",
            tranche_core::util::indented_json(&summary).unwrap_or_default()
        ));
    }
    let mut lines = String::new();
    for (key, value) in summary.as_object().into_iter().flatten() {
        lines.push_str(&format!("{key:>22}  {value}\n"));
    }
    Outcome::success(lines)
}

fn evidence(root: &Root, verb: &Evidence, json: bool) -> Outcome {
    crate::evidence::run(root, verb, json)
}
