//! Deciding which pairs are worth a paid model comparison.
//!
//! Candidate selection is mechanical and generous: title similarity, shared
//! content tokens and literal body references all nominate a pair. Nothing here
//! decides a duplicate — it decides what is worth asking about.

use std::collections::{HashMap, HashSet};

use crate::domain::judge::Judgment;

/// Candidate duplicate pairs: high title similarity within one judged category,
/// plus the body-reference edges that titles cannot express.
pub fn lexical_pairs(
    prs: &crate::domain::pr::Prs,
    judgments: &HashMap<u64, Judgment>,
    refs: Option<&HashMap<u64, Vec<u64>>>,
    threshold: f64,
    jaccard_threshold: f64,
    categories: &serde_json::Value,
) -> Vec<(f64, u64, u64)> {
    let mut by_category: HashMap<String, Vec<u64>> = HashMap::new();
    for (number, judgment) in judgments {
        by_category
            .entry(judgment.category(categories))
            .or_default()
            .push(*number);
    }
    let mut pairs: Vec<(f64, u64, u64)> = Vec::new();
    for numbers in by_category.values() {
        let mut numbers = numbers.clone();
        numbers.sort_unstable();
        for (index, a) in numbers.iter().enumerate() {
            let Some(pr_a) = prs.get(*a) else { continue };
            let title_a = pr_a.title.to_lowercase();
            for b in &numbers[index + 1..] {
                let Some(pr_b) = prs.get(*b) else { continue };
                let title_b = pr_b.title.to_lowercase();
                let ratio = similarity(&title_a, &title_b);
                if ratio >= threshold {
                    pairs.push((ratio, *a, *b));
                    continue;
                }
                let a_tokens = tokens(&pr_a.title);
                let b_tokens = tokens(&pr_b.title);
                if !a_tokens.is_empty() && !b_tokens.is_empty() {
                    let intersection = a_tokens.intersection(&b_tokens).count() as f64;
                    let union = a_tokens.union(&b_tokens).count() as f64;
                    let jaccard = intersection / union;
                    if jaccard >= jaccard_threshold {
                        pairs.push((jaccard, *a, *b));
                    }
                }
            }
        }
    }
    pairs.sort_by(|left, right| right.0.total_cmp(&left.0));
    let mut seen: HashSet<(u64, u64)> = pairs.iter().map(|(_, a, b)| (*a, *b)).collect();

    // Cross-referenced PRs are candidate duplicates even when titles differ.
    for pr in prs.iter() {
        let number = pr.number;
        if !judgments.contains_key(&number) || refs.is_some_and(|refs| !refs.contains_key(&number))
        {
            continue;
        }
        let references = refs.and_then(|refs| refs.get(&number)).unwrap_or(&pr.refs);
        for reference in references {
            let key = number.min(*reference);
            let other = number.max(*reference);
            if key != other
                && prs.get(key).is_some()
                && prs.get(other).is_some()
                && judgments.contains_key(&key)
                && judgments.contains_key(&other)
                && seen.insert((key, other))
            {
                pairs.push((1.0, key, other));
            }
        }
    }
    pairs.sort_by(|left, right| right.0.total_cmp(&left.0));
    pairs
}

/// Similarity of two lowercased titles, as the pipeline measures it.
///
/// `difflib.SequenceMatcher.ratio()` is 2M/T over the longest common
/// subsequence; reproducing it matters because the threshold decides which pairs
/// are worth a paid comparison.
fn similarity(left: &str, right: &str) -> f64 {
    let a: Vec<char> = left.chars().collect();
    let b: Vec<char> = right.chars().collect();
    let total = a.len() + b.len();
    if total == 0 {
        return 1.0;
    }
    let matches = lcs_length(&a, &b);
    (2.0 * matches as f64) / total as f64
}

fn lcs_length(a: &[char], b: &[char]) -> usize {
    if a.is_empty() || b.is_empty() {
        return 0;
    }
    let mut previous = vec![0usize; b.len() + 1];
    let mut current = vec![0usize; b.len() + 1];
    for left in a {
        for (index, right) in b.iter().enumerate() {
            current[index + 1] = if left == right {
                previous[index] + 1
            } else {
                current[index].max(previous[index + 1])
            };
        }
        std::mem::swap(&mut previous, &mut current);
        current.fill(0);
    }
    previous[b.len()]
}

/// Content tokens of a title, with the stop words the pipeline drops.
fn tokens(title: &str) -> HashSet<String> {
    const STOP: [&str; 14] = [
        "the", "a", "an", "and", "or", "to", "of", "for", "in", "on", "with", "fix", "add", "an",
    ];
    let mut found = HashSet::new();
    let mut word = String::new();
    for character in title.to_lowercase().chars() {
        if character.is_ascii_alphanumeric() {
            word.push(character);
        } else if !word.is_empty() {
            if !STOP.contains(&word.as_str()) {
                found.insert(std::mem::take(&mut word));
            } else {
                word.clear();
            }
        }
    }
    if !word.is_empty() && !STOP.contains(&word.as_str()) {
        found.insert(word);
    }
    found
}

/// The pairs worth a model verdict, as `(a, b) -> similarity`.
///
/// `categories` is the deployment's category set from its judge policy; pairs
/// are only nominated inside one category.
pub fn candidate_pairs(
    prs: &crate::domain::pr::Prs,
    judgments: &HashMap<u64, Judgment>,
    refs: Option<&HashMap<u64, Vec<u64>>>,
    categories: &serde_json::Value,
) -> HashMap<(u64, u64), f64> {
    lexical_pairs(prs, judgments, refs, 0.72, 0.62, categories)
        .into_iter()
        .map(|(score, a, b)| ((a, b), score))
        .collect()
}
