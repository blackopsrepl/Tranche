//! Bounded byte fingerprints of the report inputs.
//!
//! Absent files are **omitted**, not recorded as null: the loader skips a path that
//! does not exist, and a client reading this map must see the same set of keys.
//! Keys are the path strings exactly as the pipeline builds them, because the
//! MCP `digests()` tool quotes this map back.

use std::path::{Path, PathBuf};

use super::paths::{Limits, ReportError, Root, refuse};

/// Sorted `page_*.json` membership shards, when there is no snapshot.
pub fn read_page_paths(pages: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(pages) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            name.starts_with("page_") && name.ends_with(".json")
        })
        .collect();
    paths.sort();
    paths
}
/// Bounded byte fingerprints include optional files and corpus membership.
///
/// Absent files are **omitted**, not recorded as null: the loader skips a path that
/// does not exist, and a client reading this map must see the same set of keys.
/// Keys are the path strings exactly as the pipeline builds them, because the
/// MCP `digests()` tool quotes this map back.
pub fn input_digests(root: &Root, limits: &Limits) -> Result<serde_json::Value, ReportError> {
    let paths = root.source_paths();
    let unique: Vec<&PathBuf> = {
        let mut seen = std::collections::HashSet::new();
        paths
            .iter()
            .filter(|path| path.exists())
            .filter(|path| seen.insert(*path))
            .collect()
    };
    if unique.len() > limits.max_input_files {
        return Err(refuse("Input file count exceeds limit"));
    }
    let mut result = serde_json::Map::new();
    let mut total = 0u64;
    for path in unique {
        match crate::util::file_digest(path, limits.max_file_bytes) {
            Ok(Some((digest, size))) => {
                total += size;
                if total > limits.max_total_bytes {
                    return Err(refuse("Input bytes exceed limit"));
                }
                result.insert(
                    path.to_string_lossy().into_owned(),
                    serde_json::json!(digest),
                );
            }
            Ok(None) => {}
            Err(_) => return Err(refuse("Input bytes exceed limit")),
        }
    }
    Ok(serde_json::Value::Object(result))
}
