//! A captured pull request, and the corpus they make up.
//!
//! A PR is read from captured pages, never fetched here, so the corpus is a fixed
//! statement rather than a moving target: every digest downstream is computed
//! against these bytes.

use crate::util::digest;
use std::collections::HashMap;

/// Characters of a cleaned description kept for judging.
pub const BODY_CHARS: usize = 1200;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pr {
    pub number: u64,
    pub title: String,
    pub body: String,
    pub author: String,
    pub created: String,
    pub updated: String,
    pub draft: bool,
    pub files: Option<i64>,
    pub additions: Option<i64>,
    pub deletions: Option<i64>,
    pub labels: Vec<String>,
    pub refs: Vec<u64>,
    pub head_sha: Option<String>,
    pub url: String,
    pub source_digest: String,
    pub evidence_digest: String,
    pub ref_digest: String,
    pub body_truncated: bool,
}
/// The membership, in capture order.
///
/// Order is preserved because the membership digest is over an ordered list of
/// PR numbers; a set would silently reorder it.
#[derive(Debug, Clone, Default)]
pub struct Prs {
    order: Vec<u64>,
    pub(crate) by_number: HashMap<u64, Pr>,
}
impl Prs {
    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    pub fn get(&self, number: u64) -> Option<&Pr> {
        self.by_number.get(&number)
    }

    /// PR numbers in capture order.
    pub fn numbers(&self) -> &[u64] {
        &self.order
    }

    pub fn iter(&self) -> impl Iterator<Item = &Pr> {
        self.order.iter().filter_map(|n| self.by_number.get(n))
    }

    /// SHA-256 over the ordered PR-number list, as the pipeline records it.
    pub fn membership_digest(&self) -> String {
        digest(&serde_json::json!(self.order))
    }

    pub(crate) fn insert(&mut self, pr: Pr) {
        self.order.push(pr.number);
        self.by_number.insert(pr.number, pr);
    }
}
