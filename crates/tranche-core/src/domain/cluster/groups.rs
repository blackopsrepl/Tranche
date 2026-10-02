//! Pair verdicts to duplicate groups.
//!
//! Connectivity proposes a group; every internal relationship stays visible, so
//! a group with weak or contradictory internal evidence is reported for review
//! rather than presented as a confirmed duplicate set.

use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};

use super::predicates::SECURITY_PRIORITY;
use crate::domain::dupe::p_same;

/// The probability band the pipeline treats as a confirmed same-change pair.
const ACCEPT_THRESHOLD: f64 = 0.65;
const REJECT_THRESHOLD: f64 = 0.35;

/// What a stored verdict means once its probability and label are read together.
///
/// A verdict and a probability can disagree — the model can label two PRs
/// `same_change` while scoring them below the reject band — and that
/// self-contradiction is a real state, not a missing one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Classification {
    Same,
    Different,
    Uncertain,
    Contradictory,
    Malformed,
}

impl Classification {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Same => "same",
            Self::Different => "different",
            Self::Uncertain => "uncertain",
            Self::Contradictory => "contradictory",
            Self::Malformed => "malformed",
        }
    }
}

/// Separate genuine contradictions from the human-review threshold band.
pub fn pair_classification(pair: &Value) -> Classification {
    let Some(probability) = p_same(pair) else {
        return Classification::Malformed;
    };
    match pair.get("verdict").and_then(Value::as_str) {
        Some("same_change") => {
            if probability >= ACCEPT_THRESHOLD {
                Classification::Same
            } else if probability < REJECT_THRESHOLD {
                Classification::Contradictory
            } else {
                Classification::Uncertain
            }
        }
        Some("related_but_different") | Some("unrelated") => {
            if probability < REJECT_THRESHOLD {
                Classification::Different
            } else if probability >= ACCEPT_THRESHOLD {
                Classification::Contradictory
            } else {
                Classification::Uncertain
            }
        }
        _ => Classification::Malformed,
    }
}

pub fn accepted_pair(pair: &Value) -> bool {
    pair_classification(pair) == Classification::Same
}

/// The pair identity a verdict belongs to, when both sides are usable.
fn pair_of(verdict: &Value) -> Option<(u64, u64)> {
    let a = verdict.get("a").and_then(Value::as_u64)?;
    let b = verdict.get("b").and_then(Value::as_u64)?;
    Some((a.min(b), a.max(b)))
}

/// Connectivity proposes groups; every internal relationship stays visible.
pub fn duplicate_groups(verdicts: &[Value]) -> (Vec<Vec<u64>>, Vec<Value>) {
    let mut indexed: HashMap<(u64, u64), &Value> = HashMap::new();
    // The published group order follows the verdict order, not the order of a
    // hash map: `dupes.json` is digested, so this is a compatibility surface.
    let mut ordered: Vec<(u64, u64)> = Vec::new();
    for verdict in verdicts {
        if let Some(pair) = pair_of(verdict)
            && indexed.insert(pair, verdict).is_none()
        {
            ordered.push(pair);
        }
    }
    let mut dsu = Dsu::default();
    for (a, b) in ordered {
        if indexed
            .get(&(a, b))
            .is_some_and(|verdict| accepted_pair(verdict))
        {
            dsu.union(a, b);
        }
    }

    let mut connected: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
    let mut roots: Vec<u64> = dsu.parent_keys();
    roots.sort_unstable();
    for number in roots {
        connected.entry(dsu.find(number)).or_default().push(number);
    }
    let mut candidates: Vec<Vec<u64>> = connected.into_values().filter(|g| g.len() >= 2).collect();
    // Largest first, then by member order, so the report is deterministic.
    candidates.sort_by(|left, right| right.len().cmp(&left.len()).then_with(|| left.cmp(right)));

    let mut consistent = Vec::new();
    let mut review = Vec::new();
    for members in candidates {
        let mut conflicts = Vec::new();
        let mut uncertain = Vec::new();
        let mut missing = Vec::new();
        let mut unbound = false;
        for (index, a) in members.iter().enumerate() {
            for b in &members[index + 1..] {
                let Some(pair) = indexed.get(&(*a.min(b), *a.max(b))) else {
                    missing.push(json!([a, b]));
                    continue;
                };
                if pair.get("freshness").and_then(Value::as_str) != Some("current") {
                    unbound = true;
                }
                if accepted_pair(pair) {
                    continue;
                }
                let classification = pair_classification(pair);
                let diagnostic = json!({
                    "a": a, "b": b,
                    "verdict": pair.get("verdict").cloned().unwrap_or(Value::Null),
                    "p_same": p_same(pair),
                    "classification": classification.as_str(),
                });
                // Strong difference evidence conflicts with the proposed group; a
                // self-contradictory model response also needs relationship review.
                match classification {
                    Classification::Different | Classification::Contradictory => {
                        conflicts.push(diagnostic)
                    }
                    _ => uncertain.push(diagnostic),
                }
            }
        }
        if !conflicts.is_empty() || !uncertain.is_empty() || !missing.is_empty() || unbound {
            review.push(json!({
                "members": members,
                "conflicting_pairs": conflicts,
                "uncertain_pairs": uncertain,
                "missing_pairs": missing,
                "unbound_evidence": unbound,
            }));
        } else {
            consistent.push(members);
        }
    }
    (consistent, review)
}

/// Disjoint-set union over PR numbers, kept as a sorted map so a group's
/// iteration order is the member order and not a hash order.
#[derive(Default)]
struct Dsu {
    parent: BTreeMap<u64, u64>,
}

impl Dsu {
    fn find(&mut self, node: u64) -> u64 {
        let entry = *self.parent.entry(node).or_insert(node);
        if entry == node {
            return node;
        }
        let root = self.find(entry);
        self.parent.insert(node, root);
        root
    }

    fn union(&mut self, a: u64, b: u64) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.parent.insert(rb, ra);
        }
    }

    fn parent_keys(&self) -> Vec<u64> {
        self.parent.keys().copied().collect()
    }
}

const _: f64 = SECURITY_PRIORITY;
