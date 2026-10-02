//! Reading a published report and refusing one that does not verify.
//!
//! A report is an observation, not a file: it is accepted only when its recorded
//! binding still matches the judgments and pairs behind it.

use std::path::Path;

use super::paths::{BoundReport, Limits, MODEL, REPOSITORY, ReportError, Root, refuse};

pub fn read_report(
    root: &Root,
    _limits: &Limits,
    allow_unbound: bool,
) -> Result<BoundReport, ReportError> {
    use crate::domain::{cluster::cluster, dupe::current_pairs, judge, pr::load_prs};

    let summary = read_json(&root.summary_path())?;
    let clusters = read_json(&root.clusters_path())?;
    let dupes = read_json(&root.dupes_path())?;
    let prs = load_prs(root, REPOSITORY).map_err(|error| Refuse::from(error.0))?;

    // A malformed record anywhere in the log makes the whole observation unsafe,
    // even though only the last record per PR is used.
    if root.judgments_path().exists() {
        check_records_are_objects(&root.judgments_path())?;
    }
    let judgments = judge::current_judgments(root, &prs, REPOSITORY, MODEL, allow_unbound)
        .map_err(Refuse::from)?;
    let pairs = current_pairs(root, &prs, &judgments, REPOSITORY, MODEL, allow_unbound)
        .map_err(Refuse::from)?;
    if root.pairs_path().exists() {
        check_records_are_objects(&root.pairs_path())?;
    }

    // Only the producer's current projection, which the report must bind.
    let expected_clusters = crate::util::digest(&clusters);
    let expected_dupes = crate::util::digest(&dupes);
    let rebuilt = cluster(&prs, &judgments, &pairs, REPOSITORY, MODEL, allow_unbound);
    let binding_matches = summary
        .get("report_binding")
        .and_then(serde_json::Value::as_str)
        == Some(rebuilt.summary["report_binding"].as_str().unwrap_or(""));
    let digests_match = summary.get("output_digests").is_some_and(|digests| {
        digests
            .get("clusters.json")
            .and_then(serde_json::Value::as_str)
            == Some(&expected_clusters)
            && digests
                .get("dupes.json")
                .and_then(serde_json::Value::as_str)
                == Some(&expected_dupes)
    });
    if summary
        .get("format_version")
        .and_then(serde_json::Value::as_u64)
        != Some(2)
        || summary.get("repo").and_then(serde_json::Value::as_str) != Some(REPOSITORY)
        || summary
            .get("allow_unbound")
            .and_then(serde_json::Value::as_bool)
            != Some(false)
        || !binding_matches
        || !digests_match
    {
        return Err(refuse(
            "Reports are stale, unbound, foreign or modified; rerun cluster",
        ));
    }

    let batches = read_optional(root, &root.batches_path())?;
    if let Some(batches) = &batches {
        let expected = crate::domain::batch::merge_batches(
            &dupes,
            &judgments,
            &prs,
            &expected_dupes,
            REPOSITORY,
        )
        .map_err(Refuse::from)?;
        if crate::util::digest(batches) != crate::util::digest(&expected) {
            return Err(refuse("batches.json is stale or modified; rerun batches"));
        }
    }

    // The park record is part of the same observation as the batches it gated.
    let parked = read_optional(root, &root.parked_path())?;
    if let Some(parked) = &parked {
        let parks = crate::domain::batch::park_state(&dupes, &judgments, &prs);
        let expected = crate::domain::batch::parked_payload(
            &parks,
            &prs,
            &judgments,
            &expected_dupes,
            REPOSITORY,
        );
        if crate::util::digest(parked) != crate::util::digest(&expected) {
            return Err(refuse("parked.json is stale or modified; rerun batches"));
        }
    }
    if batches
        .as_ref()
        .and_then(|batches| batches.get("parked_prs"))
        .and_then(serde_json::Value::as_u64)
        .is_some_and(|parked| parked > 0)
        && parked.is_none()
    {
        return Err(refuse(
            "batches.json parked PRs but parked.json is missing; rerun batches",
        ));
    }

    let mut identity = serde_json::Map::new();
    identity.insert(
        "report_binding".to_owned(),
        summary["report_binding"].clone(),
    );
    identity.insert(
        "batches.json".to_owned(),
        batches
            .as_ref()
            .map(crate::util::digest)
            .map(serde_json::Value::String)
            .unwrap_or(serde_json::Value::Null),
    );
    identity.insert(
        "parked.json".to_owned(),
        parked
            .as_ref()
            .map(crate::util::digest)
            .map(serde_json::Value::String)
            .unwrap_or(serde_json::Value::Null),
    );
    if let Some(digests) = summary
        .get("output_digests")
        .and_then(serde_json::Value::as_object)
    {
        for (name, value) in digests {
            identity.insert(name.clone(), value.clone());
        }
    }

    let latest_judgments = judge::load_done(root).map_err(Refuse::from)?;

    Ok(BoundReport {
        summary,
        clusters,
        dupes,
        batches,
        parked,
        prs,
        judgments,
        pairs,
        latest_judgments,
        identity: serde_json::Value::Object(identity),
    })
}
/// A refusal carrying a message, converted from a lower-level error's text.
struct Refuse(String);
impl From<String> for Refuse {
    fn from(message: String) -> Self {
        Self(message)
    }
}
impl From<Refuse> for ReportError {
    fn from(value: Refuse) -> Self {
        // Lower-level failures are never reported as the specific gate they are
        // not; an unreadable input is the same class as an unusable report.
        let message = if value.0.contains("Invalid or missing")
            || value.0.contains("must contain")
            || value.0.contains("checksum")
        {
            "Invalid or missing bound reports; rerun cluster and batches".to_owned()
        } else {
            value.0
        };
        ReportError(message)
    }
}
fn read_json(path: &Path) -> Result<serde_json::Value, ReportError> {
    let text = std::fs::read_to_string(path)
        .map_err(|_| refuse("Invalid or missing bound reports; rerun cluster and batches"))?;
    serde_json::from_str(&text)
        .map_err(|_| refuse("Invalid or missing bound reports; rerun cluster and batches"))
}
fn read_optional(root: &Root, path: &Path) -> Result<Option<serde_json::Value>, ReportError> {
    let _ = root;
    if !path.exists() {
        return Ok(None);
    }
    read_json(path).map(Some)
}
/// Every non-empty line must be a JSON object, or the log is not trustworthy.
fn check_records_are_objects(path: &Path) -> Result<(), ReportError> {
    let text = std::fs::read_to_string(path)
        .map_err(|_| refuse("Invalid or missing bound reports; rerun cluster and batches"))?;
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<serde_json::Value>(line) {
            Ok(serde_json::Value::Object(_)) => {}
            _ => {
                return Err(refuse(
                    "Invalid or missing bound reports; rerun cluster and batches",
                ));
            }
        }
    }
    Ok(())
}
