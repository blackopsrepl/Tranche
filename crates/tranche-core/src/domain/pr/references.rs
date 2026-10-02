//! Literal body references: which PRs a description mentions.
//!
//! A mention selects a comparison candidate only when it names a pull request of
//! the repository under review: a bare `#N` can only mean this repository, while
//! a GitHub link or `owner/repo#N` names its own repository and must not be read
//! as ours.

use regex::Regex;
use std::sync::OnceLock;

pub fn reference_numbers(raw_body: &str, repository: &str, own_number: Option<u64>) -> Vec<u64> {
    let mut spans: Vec<(usize, usize)> = Vec::new();
    let mut numbers: Vec<u64> = Vec::new();
    let add = |value: u64, numbers: &mut Vec<u64>| {
        if !numbers.contains(&value) {
            numbers.push(value);
        }
    };
    for pattern in [link_reference(), qualified_reference()] {
        for capture in pattern.captures_iter(raw_body) {
            let whole = capture.get(0).expect("a capture has a whole match");
            spans.push((whole.start(), whole.end()));
            let repo = capture.name("repo").map(|m| m.as_str()).unwrap_or("");
            if repo.eq_ignore_ascii_case(repository)
                && let Some(number) = capture
                    .name("number")
                    .and_then(|m| m.as_str().parse::<u64>().ok())
            {
                add(number, &mut numbers);
            }
        }
    }
    for capture in bare_reference().captures_iter(raw_body) {
        let whole = capture.get(0).expect("a capture has a whole match");
        // The intent is a negative look-behind on `[\w#]`; the `regex` crate has no
        // look-around, so the preceding character is checked directly. Both
        // forms reject the `#123` inside `abc#123` or `#123#456`.
        if raw_body[..whole.start()]
            .chars()
            .next_back()
            .is_some_and(|previous| {
                previous.is_alphanumeric() || previous == '_' || previous == '#'
            })
        {
            continue;
        }
        if spans
            .iter()
            .any(|(start, end)| *start <= whole.start() && whole.start() < *end)
        {
            continue;
        }
        if let Some(number) = capture
            .name("number")
            .and_then(|m| m.as_str().parse::<u64>().ok())
        {
            add(number, &mut numbers);
        }
    }
    if let Some(own) = own_number {
        numbers.retain(|number| *number != own);
    }
    numbers.sort_unstable();
    numbers
}

fn link_reference() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(
            r"https?://(?:www\.)?github\.com/(?P<repo>[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+)/(?:issues|pull)/(?P<number>\d+)",
        )
        .expect("static pattern")
    })
}

fn qualified_reference() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r"(?P<repo>[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+)#(?P<number>\d+)")
            .expect("static pattern")
    })
}

fn bare_reference() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r"#(?P<number>\d+)\b").expect("static pattern"))
}
