//! Literal GitHub repository identifiers, not URLs or paths.

/// Parse exactly `owner/name`, without normalizing the caller's identity.
/// Owners use GitHub's alphanumeric/single-hyphen grammar (39 characters);
/// repository names use ASCII alphanumerics, dots, underscores and hyphens.
pub fn parse_repository(repository: &str) -> Result<(&str, &str), String> {
    let invalid = || format!("repository must be a GitHub `owner/name`, got `{repository}`");
    let (owner, name) = repository.split_once('/').ok_or_else(invalid)?;
    let owner_valid = !owner.is_empty()
        && owner.len() <= 39
        && owner
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && !owner.starts_with('-')
        && !owner.ends_with('-')
        && !owner.contains("--");
    let name_valid = !name.is_empty()
        && name.len() <= 100
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
    if !owner_valid || !name_valid {
        return Err(invalid());
    }
    Ok((owner, name))
}
