use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

/// The Tranche command line.
///
/// Acquisition and model calls only ever happen because a command asked for
/// them; there is no implicit work.
#[derive(Debug, Parser)]
#[command(
    name = "tranche",
    version,
    about = "Tranche · PR review pipeline",
    long_about = "Tranche · PR review pipeline\n\n\
                  Reads a checkout containing the captured corpus and the derived \
                  reports. `--root` selects it; the default is the current directory.",
    subcommand_required = true,
    arg_required_else_help = true,
    propagate_version = true
)]
pub struct Cli {
    /// Checkout holding the captured corpus and the derived reports.
    #[arg(long, global = true, default_value = ".", value_name = "DIR")]
    pub root: PathBuf,

    /// Emit machine-readable JSON instead of a formatted report.
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Write a starter deployment contract for a repository.
    Init {
        /// The repository to review, as `owner/name`.
        #[arg(value_name = "OWNER/REPO")]
        repository: String,

        /// Overwrite an existing contract.
        #[arg(long)]
        force: bool,
    },
    /// Refresh the observed open-PR membership from GitHub.
    Fetch(Fetch),
    /// Judge each PR with one batched model call.
    Judge(Judge),
    /// Compare candidate pairs for sameness.
    Dupes(Dupes),
    /// Build tranches, duplicate groups and escalation lists.
    Cluster(Cluster),
    /// Build cumulative pre-release batches and the park record.
    Batches,
    /// Print the numbers from the current report.
    Info,
    /// Run fetch, judge, dupes, cluster, batches and page in order.
    Refresh(Refresh),
    /// Judge, then dupes, cluster and batches.
    All(All),
    /// Serve the read-only MCP stdio surface over the bound report.
    Mcp,
    /// Render the workbench from the bound reports.
    Page(Page),
    /// Capture, inspect and export public PR evidence.
    #[command(subcommand_required = true, arg_required_else_help = true)]
    Evidence {
        #[command(subcommand)]
        command: Evidence,
    },
}

/// How GitHub is reached when membership is refreshed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Transport {
    /// The authenticated `gh` CLI.
    Gh,
    /// The standard library HTTP client.
    Urllib,
    /// The `curl` binary.
    Curl,
}

impl std::fmt::Display for Transport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Gh => "gh",
            Self::Urllib => "urllib",
            Self::Curl => "curl",
        };
        f.write_str(name)
    }
}

#[derive(Debug, Args)]
pub struct Fetch {
    /// How to reach GitHub.
    #[arg(long, value_enum, default_value_t = Transport::Gh)]
    pub transport: Transport,
}

#[derive(Debug, Args)]
pub struct Judge {
    /// Judge only the N newest unjudged PRs.
    #[arg(long, value_name = "N", value_parser = positive)]
    pub limit: Option<u64>,

    /// Reuse judgments bound to unchanged input, questions and model.
    #[arg(long)]
    pub resume: bool,
}

#[derive(Debug, Args)]
pub struct Dupes {
    /// Cap the number of pair comparisons this pass.
    #[arg(long, value_name = "N", default_value_t = 300)]
    pub max_pairs: u64,
}

#[derive(Debug, Args)]
pub struct Cluster {
    /// Inspect legacy judgments with freshness warnings.
    #[arg(long)]
    pub allow_unbound: bool,
}

#[derive(Debug, Args)]
#[command(group(clap::ArgGroup::new("exports").args(["export_json", "export_xlsx"]).multiple(true)))]
pub struct Page {
    /// Write only standalone exports, without rendering HTML or installing assets.
    #[arg(long, requires = "exports")]
    pub no_html: bool,

    /// Also write the standalone JSON report to docs/data/report.json.
    #[arg(long, group = "exports")]
    pub export_json: bool,

    /// Also write the filterable Excel workbook to docs/data/report.xlsx.
    #[arg(long, group = "exports")]
    pub export_xlsx: bool,
}

#[derive(Debug, Args)]
pub struct Refresh {
    /// Cap the number of pair comparisons this pass.
    #[arg(long, value_name = "N", default_value_t = 400)]
    pub max_pairs: u64,

    /// Skip rendering the workbench.
    #[arg(long)]
    pub no_page: bool,

    /// Report what would change without writing.
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Args)]
pub struct All {
    /// Judge only the N newest unjudged PRs.
    #[arg(long, value_name = "N", value_parser = positive)]
    pub limit: Option<u64>,

    /// Accepted for compatibility; `all` always resumes.
    #[arg(long)]
    pub resume: bool,

    /// Cap the number of pair comparisons this pass.
    #[arg(long, value_name = "N", default_value_t = 300)]
    pub max_pairs: u64,
}

#[derive(Debug, Subcommand)]
pub enum Evidence {
    /// Validate a batch and acquire or resume its public evidence.
    Capture(Capture),
    /// Inspect coverage, sources or citations offline.
    Show(Show),
    /// Write a self-contained packet after the public-sharing gate.
    Export(Export),
}

#[derive(Debug, Args)]
pub struct Capture {
    /// Batch to capture, such as B001.
    #[arg(long, value_name = "BATCH")]
    pub batch: String,

    /// Requests this run may attempt; the last few are held back for the
    /// completion checks.
    #[arg(long, value_name = "N", default_value_t = 200)]
    pub request_budget: u64,

    /// Bytes this capture may store.
    #[arg(long, value_name = "N", default_value_t = 67_108_864)]
    pub max_bytes: u64,

    /// Start a new capture generation even if one is stored.
    #[arg(long)]
    pub fresh: bool,

    /// Resume or extend one explicit capture.
    #[arg(long, value_name = "ID")]
    pub reuse_capture: Option<String>,

    /// Replace a writer lock whose process is gone.
    #[arg(long)]
    pub break_lock: bool,
}

#[derive(Debug, Args)]
pub struct Show {
    /// Inspect the evidence associated with the current batch.
    #[arg(long, value_name = "BATCH", conflicts_with = "capture")]
    pub batch: Option<String>,

    /// Inspect one capture historically, with no report consulted.
    #[arg(long, value_name = "ID")]
    pub capture: Option<String>,

    /// Read a bounded window from one stored source.
    #[arg(long, value_name = "ID", conflicts_with = "citation")]
    pub source: Option<String>,

    /// Resolve one citation against stored bytes.
    #[arg(long, value_name = "ID")]
    pub citation: Option<String>,

    /// First byte of the window.
    #[arg(long, value_name = "N", default_value_t = 0)]
    pub start_byte: u64,

    /// Window length in bytes.
    #[arg(long, value_name = "N", default_value_t = 16_384)]
    pub length: u64,
}

#[derive(Debug, Args)]
pub struct Export {
    /// Export the evidence associated with the current batch.
    #[arg(long, value_name = "BATCH", conflicts_with = "capture")]
    pub batch: Option<String>,

    /// Export one historical capture.
    #[arg(long, value_name = "ID")]
    pub capture: Option<String>,

    /// Destination file, or `-` for stdout.
    #[arg(long, value_name = "FILE")]
    pub output: String,

    /// Confirm an export of a capture that is not the current one.
    #[arg(long)]
    pub allow_historical: bool,

    /// Requests the sharing gate may spend.
    #[arg(long, value_name = "N", default_value_t = 8)]
    pub request_budget: u64,

    /// Bytes the sharing gate may read.
    #[arg(long, value_name = "N", default_value_t = 67_108_864)]
    pub max_bytes: u64,
}

fn positive(value: &str) -> Result<u64, String> {
    match value.parse::<u64>() {
        Ok(0) | Err(_) => Err("must be a positive integer".to_owned()),
        Ok(number) => Ok(number),
    }
}
