//! Stored pair verdicts, in the order they were written.
//!
//! Order is part of `dupes.json`, which is digested, and it decides which pairs
//! a budgeted `dupes` pass reaches first. A hash map here would silently reorder
//! both.

use serde_json::{Value, json};
use std::collections::HashMap;

use super::super::judge::Judgment;
use super::normalizing::normalize_pair;
use super::verdict::{pair_binding, pair_is_current};
use crate::report::Root;

/// Stored verdicts in the order they first appear in the log.
///
/// Order is part of `dupes.json`, which is digested, and it decides which pairs
/// a budgeted `dupes` pass reaches first. A hash map here would silently reorder
/// both.
#[derive(Debug, Default, Clone)]
pub struct Verdicts {
    pairs: Vec<((u64, u64), Value)>,
    index: HashMap<(u64, u64), usize>,
}

impl Verdicts {
    pub fn len(&self) -> usize {
        self.pairs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }

    /// Records in first-appearance order.
    pub fn records(&self) -> impl Iterator<Item = &Value> {
        self.pairs.iter().map(|(_, record)| record)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&(u64, u64), &Value)> {
        self.pairs.iter().map(|(pair, record)| (pair, record))
    }

    pub fn get(&self, pair: &(u64, u64)) -> Option<&Value> {
        self.index
            .get(pair)
            .map(|position| &self.pairs[*position].1)
    }

    fn insert(&mut self, pair: (u64, u64), record: Value) {
        match self.index.get(&pair) {
            // The log is append-only, so a pair repeats; the newest record wins
            // but keeps the position the pair first appeared at.
            Some(position) => self.pairs[*position].1 = record,
            None => {
                self.index.insert(pair, self.pairs.len());
                self.pairs.push((pair, record));
            }
        }
    }
}

/// The latest stored verdict per pair, for pairs whose members are both captured.
pub fn current_verdicts(root: &Root, prs: &crate::domain::pr::Prs) -> Result<Verdicts, String> {
    let path = root.pairs_path();
    if !path.exists() {
        return Ok(Verdicts::default());
    }
    let text = std::fs::read_to_string(&path).map_err(|error| error.to_string())?;
    let mut verdicts = Verdicts::default();
    for line in text.lines() {
        if !line.contains("\"a\"") {
            continue;
        }
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(object) = record.as_object() else {
            continue;
        };
        if object.contains_key("error") {
            continue;
        }
        let read = |key: &str| object.get(key).and_then(Value::as_u64).filter(|n| *n > 0);
        let (Some(a), Some(b)) = (read("a"), read("b")) else {
            continue;
        };
        let pair = (a.min(b), a.max(b));
        if pair.0 == pair.1 || prs.get(pair.0).is_none() || prs.get(pair.1).is_none() {
            continue;
        }
        verdicts.insert(pair, normalize_pair(&record));
    }
    Ok(verdicts)
}

/// Pair verdicts a report may publish.
///
/// A verdict is published when the pair's current descriptions still verify
/// against the record. A pair whose similarity has since fallen below the
/// discovery threshold is still published when the model actually judged it: the
/// threshold finds candidates, it does not get to hide a verified dupe.
pub fn current_pairs(
    root: &Root,
    prs: &crate::domain::pr::Prs,
    judgments: &HashMap<u64, Judgment>,
    repository: &str,
    model: &str,
    allow_unbound: bool,
) -> Result<Vec<Value>, String> {
    let mut published = Vec::new();
    for (pair, record) in current_verdicts(root, prs)?.iter() {
        let (a, b) = *pair;
        if !judgments.contains_key(&a) || !judgments.contains_key(&b) {
            continue;
        }
        let (Some(pr_a), Some(pr_b)) = (prs.get(a), prs.get(b)) else {
            continue;
        };
        let matches = pair_is_current(record, &pair_binding(pr_a, pr_b, repository, model));
        let legacy = record.get("binding").is_none() && allow_unbound;
        if matches || legacy {
            let mut published_record = record.clone();
            if let Some(object) = published_record.as_object_mut() {
                object.insert("a".to_owned(), json!(a));
                object.insert("b".to_owned(), json!(b));
                object.insert(
                    "freshness".to_owned(),
                    json!(if matches { "current" } else { "unbound" }),
                );
            }
            published.push(published_record);
        }
    }
    Ok(published)
}

/// Every captured pair with a stored verdict, stamped with its currency.
pub fn pair_cache(
    root: &Root,
    prs: &crate::domain::pr::Prs,
    repository: &str,
    model: &str,
) -> Result<HashMap<(u64, u64), Value>, String> {
    let mut cache = HashMap::new();
    for (pair, record) in current_verdicts(root, prs)?.iter() {
        let (a, b) = *pair;
        let (Some(pr_a), Some(pr_b)) = (prs.get(a), prs.get(b)) else {
            continue;
        };
        let matches = pair_is_current(record, &pair_binding(pr_a, pr_b, repository, model));
        let mut stamped = record.clone();
        if let Some(object) = stamped.as_object_mut() {
            object.insert("a".to_owned(), json!(a));
            object.insert("b".to_owned(), json!(b));
            object.insert(
                "freshness".to_owned(),
                json!(if matches { "current" } else { "stale" }),
            );
        }
        cache.insert((a, b), stamped);
    }
    Ok(cache)
}
