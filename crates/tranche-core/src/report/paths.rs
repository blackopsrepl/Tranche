//! Where everything lives, and the values a report is bounded by.
//!
//! One place decides the path vocabulary, so a relocated checkout (tests,
//! `--root`, a copied report directory) cannot end up with two modules
//! disagreeing about which file they are reading.

use std::path::{Path, PathBuf};

use super::inputs::read_page_paths;

/// Kept for provenance readers: the alias this deployment historically bound
/// under. New code reads the model from the deployment contract.
#[allow(dead_code)]
pub const MODEL: &str = "jev-latest";
/// Byte and file-count bounds for fingerprinting the bound inputs.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
    pub max_input_files: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_file_bytes: 128 * 1024 * 1024,
            max_total_bytes: 256 * 1024 * 1024,
            max_input_files: 512,
        }
    }
}
/// The local observation cannot safely be used.
#[derive(Debug)]
pub struct ReportError(pub String);
impl std::fmt::Display for ReportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ReportError {}
pub(crate) fn refuse(message: &str) -> ReportError {
    ReportError(message.to_owned())
}
/// The validated report projection: inputs, derived records and identity.
///
/// Obtained only from [`load`], which applies every gate. Nothing else may
/// construct one, so a consumer cannot read a stale, foreign or modified report
/// by forgetting to check — the type is the check.
#[derive(Debug)]
pub struct BoundReport {
    pub summary: serde_json::Value,
    pub clusters: serde_json::Value,
    pub dupes: serde_json::Value,
    pub batches: Option<serde_json::Value>,
    pub parked: Option<serde_json::Value>,
    pub prs: crate::domain::pr::Prs,
    pub judgments: std::collections::HashMap<u64, crate::domain::judge::Judgment>,
    pub pairs: Vec<serde_json::Value>,
    /// Every stored judgment, current or not, for provenance questions.
    pub latest_judgments: std::collections::HashMap<u64, serde_json::Value>,
    /// The digests a reader quotes to prove which report it was served from.
    pub identity: serde_json::Value,
}
/// The checkout holding the captured corpus and the derived reports.
#[derive(Clone, Debug)]
pub struct Root {
    path: PathBuf,
}
impl Root {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Captured PR membership pages.
    pub fn pages_dir(&self) -> PathBuf {
        self.path.join("data").join("pages")
    }

    /// Every derived report and cache file.
    pub fn out_dir(&self) -> PathBuf {
        self.path.join("out")
    }

    pub fn judgments_path(&self) -> PathBuf {
        self.out_dir().join("judgments.jsonl")
    }

    pub fn pairs_path(&self) -> PathBuf {
        self.out_dir().join("pair_verdicts.jsonl")
    }

    pub fn summary_path(&self) -> PathBuf {
        self.out_dir().join("summary.json")
    }

    pub fn clusters_path(&self) -> PathBuf {
        self.out_dir().join("clusters.json")
    }

    pub fn dupes_path(&self) -> PathBuf {
        self.out_dir().join("dupes.json")
    }

    pub fn batches_path(&self) -> PathBuf {
        self.out_dir().join("batches.json")
    }

    pub fn parked_path(&self) -> PathBuf {
        self.out_dir().join("parked.json")
    }

    pub fn tranches_path(&self) -> PathBuf {
        self.out_dir().join("tranches.md")
    }

    /// Captured evidence: manifests, content-addressed bodies and writer locks.
    pub fn evidence_dir(&self) -> PathBuf {
        self.out_dir().join("evidence")
    }

    pub fn docs_dir(&self) -> PathBuf {
        self.path.join("docs")
    }

    pub fn template_path(&self) -> PathBuf {
        self.path.join("page").join("template.html")
    }

    /// The single-file membership snapshot, when the corpus was captured whole.
    pub fn snapshot_path(&self) -> PathBuf {
        self.pages_dir().join("snapshot.json")
    }

    /// The ordered bound inputs, matching `report_loader.source_paths()`.
    ///
    /// The snapshot and the page shards are alternatives, not a pair: when the
    /// corpus was captured whole, the shards are stale leftovers and are not
    /// part of the observation. Fingerprinting both would report a changed input
    /// set that the pipeline never reads.
    pub fn source_paths(&self) -> Vec<PathBuf> {
        let snapshot = self.snapshot_path();
        let mut paths = vec![snapshot.clone()];
        if !snapshot.exists() {
            paths.extend(read_page_paths(&self.pages_dir()));
        }
        paths.push(self.judgments_path());
        paths.push(self.pairs_path());
        for name in [
            "summary.json",
            "clusters.json",
            "dupes.json",
            "batches.json",
            "parked.json",
        ] {
            paths.push(self.out_dir().join(name));
        }
        paths
    }
}
