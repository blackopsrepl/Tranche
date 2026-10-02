//! URL policy for the read transport.
//!
//! A URL is checked before it is requested, and the checks are scope checks
//! rather than authenticity checks: staying inside the API origin and one
//! repository's namespace says nothing about whether the response is the
//! resource that was asked for. That is verified against the payload itself at
//! acquisition time.

use super::ReadError;

/// The only origin this transport will request.
pub const ORIGIN: &str = "https://api.github.com";

/// Query keys that look like a credential, refused wherever they appear.
///
/// The credential stays inside `gh`; a URL that carries one is a bug or an
/// attempt to leak it, and either way it is not sent.
const CREDENTIAL_KEYS: [&str; 10] = [
    "token",
    "access_token",
    "authorization",
    "api_key",
    "client_secret",
    "private_token",
    "password",
    "key",
    "auth",
    "credentials",
];

/// The pieces of a URL this policy cares about.
struct Parts<'a> {
    scheme: &'a str,
    netloc: &'a str,
    path: &'a str,
    query: &'a str,
    fragment: &'a str,
}

/// Split a URL without pulling in a URL crate.
///
/// The inputs are GitHub's own `Link` headers, so the shapes are known; anything
/// unparseable is refused rather than repaired.
fn parts(url: &str) -> Option<Parts<'_>> {
    let (scheme, rest) = url.split_once("://")?;
    let (authority, rest) = match rest.find('/') {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, ""),
    };
    let (rest, fragment) = match rest.split_once('#') {
        Some((rest, fragment)) => (rest, fragment),
        None => (rest, ""),
    };
    let (path, query) = match rest.split_once('?') {
        Some((path, query)) => (path, query),
        None => (rest, ""),
    };
    Some(Parts {
        scheme,
        netloc: authority,
        path,
        query,
        fragment,
    })
}

/// A URL this transport is willing to request.
///
/// `repo_prefix` is `owner/repo` when the request must stay inside one
/// repository's REST namespace.
pub fn validate(url: &str, repo_prefix: Option<&str>) -> Result<String, ReadError> {
    if url.is_empty() {
        return Err(ReadError::new("refusing empty URL"));
    }
    if url
        .chars()
        .any(|character| character.is_whitespace() || character.is_control())
    {
        return Err(ReadError::new(
            "refusing URL containing whitespace or control characters",
        ));
    }
    let parts = parts(url).ok_or_else(|| ReadError::new("refusing unparseable URL"))?;
    if parts.scheme != "https" || parts.netloc != "api.github.com" {
        return Err(ReadError::new(format!(
            "refusing URL outside {ORIGIN}: {url}"
        )));
    }
    if !parts.fragment.is_empty() {
        return Err(ReadError::new("refusing URL with a fragment"));
    }
    // A lookalike origin such as `https://api.github.com.evil.test` parses to a
    // different netloc, but a prefix check catches one the parser would accept.
    if !url.starts_with(ORIGIN) {
        return Err(ReadError::new(format!(
            "refusing URL that only resembles {ORIGIN}: {url}"
        )));
    }
    if query_keys(parts.query)
        .iter()
        .any(|key| CREDENTIAL_KEYS.contains(&key.as_str()))
    {
        return Err(ReadError::new(
            "refusing URL carrying credential-shaped query parameters",
        ));
    }
    if let Some(prefix) = repo_prefix {
        let namespace = format!("/repos/{prefix}");
        if parts.path != namespace && !parts.path.starts_with(&format!("{namespace}/")) {
            return Err(ReadError::new(format!(
                "refusing URL outside repository {prefix}: {url}"
            )));
        }
    }
    Ok(url.to_owned())
}

/// The lowercased query keys, for the credential check.
fn query_keys(query: &str) -> Vec<String> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| pair.split('=').next().unwrap_or("").to_ascii_lowercase())
        .collect()
}

/// The `rel="next"` continuation URL, validated. Other relations are ignored.
pub fn next_link(header: Option<&str>) -> Result<Option<String>, ReadError> {
    let Some(header) = header else {
        return Ok(None);
    };
    for entry in split_entries(header) {
        let Some((target, relations)) = entry.split_once('>') else {
            continue;
        };
        let target = target.trim_start_matches('<').trim();
        if !relations
            .split(',')
            .any(|relation| is_next(relation.trim()))
        {
            continue;
        }
        return validate(target, None).map(Some);
    }
    Ok(None)
}

/// Split a `Link` header on the commas that separate entries, not the ones inside
/// a URL.
fn split_entries(header: &str) -> Vec<&str> {
    let mut entries = Vec::new();
    let mut start = 0;
    for (index, character) in header.char_indices() {
        if character == ',' && header[index + 1..].trim_start().starts_with('<') {
            entries.push(&header[start..index]);
            start = index + 1;
        }
    }
    entries.push(&header[start..]);
    entries
}

/// Whether one `Link` entry's parameters declare `rel="next"`.
fn is_next(relation: &str) -> bool {
    // The parameter keeps its `;` separator from the entry, and GitHub sends no
    // space before it.
    let relation = relation.trim().trim_start_matches(';').trim();
    let Some((key, value)) = relation.split_once('=') else {
        return false;
    };
    key.trim().eq_ignore_ascii_case("rel")
        && value.trim().trim_matches('"').eq_ignore_ascii_case("next")
}

/// The `page` number a URL carries, defaulting to one.
pub fn page_number(url: &str) -> Result<u64, ReadError> {
    let parts = parts(url).ok_or_else(|| ReadError::new("refusing unparseable URL"))?;
    for pair in parts.query.split('&') {
        if let Some((key, value)) = pair.split_once('=')
            && key == "page"
        {
            return value
                .parse()
                .map_err(|_| ReadError::new(format!("unreadable page parameter in {url}")));
        }
    }
    Ok(1)
}

/// Resolve a redirect target against the URL that produced it.
pub fn absolute(location: &str, base: &str) -> Result<String, ReadError> {
    if location.starts_with("https://") {
        return Ok(location.to_owned());
    }
    if !location.starts_with('/') {
        return Err(ReadError::new(format!(
            "refusing relative redirect target {location:?}"
        )));
    }
    if base.is_empty() {
        return Err(ReadError::new("refusing redirect with no base"));
    }
    Ok(format!("{ORIGIN}{location}"))
}
