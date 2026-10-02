//! Build a small, committed fixture from the local corpus.
//!
//! The full corpus is 111 MiB and must stay local, so the tests that compare
//! against the published artifacts cannot read it in CI. This cuts a slice
//! small enough to commit: a handful of PRs, the judgments and pair verdicts
//! that name them, and the artifacts rebuilt from them.
//!
//! Run from the repository root; the output is written under `tests/fixture/`
//! and is meant to be committed. Nothing here is run by `cargo test`.
//!
//! ```text
//! cargo run --release -p tranche-core --example slice_fixture -- 12
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tranche_core::domain::batch::{merge_batches, park_state, parked_payload};
use tranche_core::domain::cluster::{cluster, render};
use tranche_core::domain::dupe::current_pairs;
use tranche_core::domain::judge::{Judgment, current_judgments};
use tranche_core::domain::pr::load_prs;
use tranche_core::report::{MODEL, REPOSITORY, Root};
use tranche_core::util::{atomic_json, digest};

/// How many PRs the slice keeps, and how many of those must be in a group.
const DEFAULT_SIZE: usize = 12;

fn main() {
    let size: usize = std::env::args()
        .nth(1)
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_SIZE);
    let root = Root::new(Path::new("."));
    let out = PathBuf::from("crates/tranche-core/tests/fixture");
    std::fs::create_dir_all(out.join("data/pages")).expect("fixture directories");
    std::fs::create_dir_all(out.join("out")).expect("fixture directories");

    let corpus = load_prs(&root, REPOSITORY).expect("the local corpus loads");
    let judgments = current_judgments(&root, &corpus, REPOSITORY, MODEL, false).expect("judgments");
    let verdicts =
        current_pairs(&root, &corpus, &judgments, REPOSITORY, MODEL, false).expect("verdicts");
    let full = cluster(&corpus, &judgments, &verdicts, REPOSITORY, MODEL, false);

    // Members of confirmed groups first: a slice of unrelated PRs would leave
    // the group and park paths untested.
    let mut wanted: Vec<u64> = Vec::new();
    for group in full.dupes["confirmed_groups"]
        .as_array()
        .into_iter()
        .flatten()
    {
        for number in group
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_u64())
        {
            if wanted.len() < size && !wanted.contains(&number) {
                wanted.push(number);
            }
        }
    }
    let mut ordered: Vec<u64> = corpus.numbers().to_vec();
    ordered.sort_unstable();
    for number in ordered {
        if wanted.len() >= size {
            break;
        }
        if !wanted.contains(&number) {
            wanted.push(number);
        }
    }
    wanted.sort_unstable();

    // The captured pages, keeping only the chosen PRs. The snapshot is the
    // single-file form of membership, so slicing it is enough.
    let page = slice_membership(&root, &wanted);
    atomic_json(&out.join("data/pages/snapshot.json"), &page).expect("snapshot");

    // The judgment and verdict logs, keeping only records about chosen PRs.
    write_lines(
        &root.judgments_path(),
        &out.join("out/judgments.jsonl"),
        &wanted,
        true,
    );
    write_lines(
        &root.pairs_path(),
        &out.join("out/pair_verdicts.jsonl"),
        &wanted,
        false,
    );

    // Rebuild every artifact from the slice, so the fixture's `out/` is exactly
    // what this code produces for its inputs rather than copied bytes.
    let sliced = Root::new(&out);
    let prs = load_prs(&sliced, REPOSITORY).expect("the slice loads");
    let judgments: HashMap<u64, Judgment> =
        current_judgments(&sliced, &prs, REPOSITORY, MODEL, false).expect("slice judgments");
    let verdicts =
        current_pairs(&sliced, &prs, &judgments, REPOSITORY, MODEL, false).expect("slice verdicts");
    let built = cluster(&prs, &judgments, &verdicts, REPOSITORY, MODEL, false);

    atomic_json(&out.join("out/clusters.json"), &built.clusters).expect("clusters");
    atomic_json(&out.join("out/dupes.json"), &built.dupes).expect("dupes");
    std::fs::write(out.join("out/tranches.md"), render(&built, REPOSITORY)).expect("report");
    atomic_json(&out.join("out/summary.json"), &built.summary).expect("summary");

    let dupes_digest = digest(&built.dupes);
    let batches = merge_batches(&built.dupes, &judgments, &prs, &dupes_digest, REPOSITORY)
        .expect("the slice packs");
    let parks = park_state(&built.dupes, &judgments, &prs);
    atomic_json(&out.join("out/batches.json"), &batches).expect("batches");
    atomic_json(
        &out.join("out/parked.json"),
        &parked_payload(&parks, &prs, &judgments, &dupes_digest, REPOSITORY),
    )
    .expect("parked");

    // Batches append to the report, as the command does.
    let mut report = std::fs::read_to_string(out.join("out/tranches.md")).expect("report");
    report.push_str(&tranche_core::domain::batch::batch_plan_section(&batches));
    if !parks.is_empty() {
        report.push_str(&tranche_core::domain::batch::park_section(&parks, &prs));
    }
    std::fs::write(out.join("out/tranches.md"), report).expect("report");

    println!(
        "slice of {} PRs: {} judgments, {} verdicts, {} clusters bytes",
        prs.len(),
        judgments.len(),
        verdicts.len(),
        digest(&built.clusters).len()
    );
    println!("wrote {}", out.display());
}

/// The membership snapshot reduced to the chosen PRs.
fn slice_membership(root: &Root, wanted: &[u64]) -> serde_json::Value {
    let path = root.snapshot_path();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "{}: {error}. The corpus keeps its pages as shards; join them first.",
            path.display()
        )
    });
    let value: serde_json::Value = serde_json::from_str(&text).expect("snapshot parses");
    let items: Vec<serde_json::Value> = value["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter(|item| {
            item["number"]
                .as_u64()
                .is_some_and(|number| wanted.contains(&number))
        })
        .cloned()
        .collect();
    // The snapshot is self-verifying: its digest covers exactly the items.
    serde_json::json!({
        "version": value["version"],
        "repo": value["repo"],
        "items": items,
        "digest": digest(&serde_json::Value::Array(items.clone())),
    })
}

/// Reduce a JSON Lines log to records about the chosen PRs.
fn write_lines(source: &Path, destination: &Path, wanted: &[u64], single: bool) {
    let text = std::fs::read_to_string(source)
        .unwrap_or_else(|error| panic!("{}: {error}", source.display()));
    let mut kept = String::new();
    for line in text.lines() {
        let Ok(record) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let numbers: Vec<u64> = if single {
            record["number"].as_u64().into_iter().collect()
        } else {
            [record["a"].as_u64(), record["b"].as_u64()]
                .into_iter()
                .flatten()
                .collect()
        };
        if numbers.is_empty() || !numbers.iter().all(|number| wanted.contains(number)) {
            continue;
        }
        kept.push_str(line);
        kept.push('\n');
    }
    std::fs::write(destination, kept).expect("write");
}
