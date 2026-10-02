//! Evidence paths and confinement.
//!
//! Everything the service reads or writes is derived from one root, and every
//! path that came from a stored record is refused unless it is a regular,
//! single-link file beneath that root with no symlink anywhere in it. A
//! checkpoint is untrusted input: it is a file on disk that an attacker with
//! write access to `out/` could have edited.

use std::path::{Component, Path, PathBuf};

use super::refuse;
use crate::evidence::EvidenceError;
use crate::report::Root;

/// A capture id must be exactly 32 lowercase hex characters.
pub fn validate_capture_id(capture_id: &str) -> Result<(), EvidenceError> {
    let valid = capture_id.len() == 32
        && capture_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if valid {
        Ok(())
    } else {
        Err(refuse("capture id must be 32 lowercase hex characters"))
    }
}

/// A body digest must be exactly 64 lowercase hex characters.
pub fn validate_body_digest(digest: &str) -> Result<(), EvidenceError> {
    let valid = digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if valid {
        Ok(())
    } else {
        Err(refuse(
            "source body digest must be 64 lowercase hex characters",
        ))
    }
}

/// The directory holding one capture's manifest.
pub fn capture_dir(root: &Root, capture_id: &str) -> Result<PathBuf, EvidenceError> {
    validate_capture_id(capture_id)?;
    Ok(super::evidence_root(root).join(capture_id))
}

/// The checkpoint path for one capture.
pub fn manifest_path(root: &Root, capture_id: &str) -> Result<PathBuf, EvidenceError> {
    Ok(capture_dir(root, capture_id)?.join("manifest.json"))
}

/// The advisory writer lock for one capture.
pub fn lock_path(root: &Root, capture_id: &str) -> Result<PathBuf, EvidenceError> {
    validate_capture_id(capture_id)?;
    Ok(super::evidence_root(root).join(format!("{capture_id}.lock")))
}

/// One content-addressed body, shared by every capture that recorded it.
///
/// Keying by digest rather than by capture is what lets a new association reuse
/// unchanged bytes without copying them. The manifest carries only the digest;
/// the location is derived, never recorded, so there is no second identity for
/// the same bytes.
pub fn body_path(root: &Root, body_digest: &str) -> Result<PathBuf, EvidenceError> {
    validate_body_digest(body_digest)?;
    Ok(super::evidence_root(root)
        .join("bodies")
        .join(format!("{body_digest}.bin")))
}

/// Resolve a stored relative path beneath `base`, refusing anything unsafe.
///
/// Rejects an absolute path, a `..` or empty component, a NUL byte, any symlink
/// in the chain, a multiply-linked file and anything that resolves outside the
/// base. The path is checked as it exists, not as it is written down.
pub fn confined_file(base: &Path, relative: &str) -> Result<PathBuf, EvidenceError> {
    if relative.is_empty() {
        return Err(refuse("empty evidence path"));
    }
    if relative.contains('\0') {
        return Err(refuse("evidence path contains a NUL byte"));
    }
    let candidate = Path::new(relative);
    if candidate.is_absolute() {
        return Err(refuse(format!("refusing unconfined path {relative:?}")));
    }
    let mut parts: Vec<&std::ffi::OsStr> = Vec::new();
    for component in candidate.components() {
        match component {
            Component::Normal(part) => parts.push(part),
            Component::CurDir => {}
            _ => return Err(refuse(format!("refusing unconfined path {relative:?}"))),
        }
    }
    if parts.is_empty() {
        return Err(refuse(format!("refusing empty path {relative:?}")));
    }

    let base = base
        .canonicalize()
        .map_err(|error| refuse(format!("evidence root is unusable: {error}")))?;
    let mut current = base.clone();
    for part in &parts {
        current.push(part);
        // `symlink_metadata` does not follow the link, which is the point: a
        // symlinked directory must be refused rather than traversed.
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(refuse(format!(
                    "refusing symlink in evidence path: {relative:?}"
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(refuse(format!("evidence path is unusable: {error}"))),
        }
    }
    let resolved = base.join(parts.iter().collect::<PathBuf>().as_path());
    if resolved != base && !resolved.starts_with(&base) {
        return Err(refuse(format!(
            "refusing path outside the evidence root: {relative:?}"
        )));
    }
    let metadata = match std::fs::metadata(&resolved) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(refuse(format!("evidence path is unusable: {error}"))),
    };
    if let Some(metadata) = metadata {
        if !metadata.is_file() {
            return Err(refuse(format!(
                "refusing non-regular evidence file: {relative:?}"
            )));
        }
        if hard_link_count(&metadata) > 1 {
            return Err(refuse(format!(
                "refusing multiply-linked evidence file: {relative:?}"
            )));
        }
    }
    Ok(resolved)
}

#[cfg(unix)]
fn hard_link_count(metadata: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    metadata.nlink()
}

#[cfg(not(unix))]
fn hard_link_count(_metadata: &std::fs::Metadata) -> u64 {
    1
}

/// Publish bytes atomically: temporary sibling, fsync, rename, mode 0600.
///
/// A partial checkpoint must never be observable, so every write that becomes a
/// record goes through here.
pub fn atomic_bytes(path: &Path, data: &[u8]) -> Result<(), EvidenceError> {
    use std::io::Write;
    let parent = path
        .parent()
        .ok_or_else(|| refuse("evidence path has no parent directory"))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| refuse(format!("cannot create the evidence directory: {error}")))?;
    let temporary = parent.join(format!(
        ".pending-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis())
            .unwrap_or(0)
    ));
    let result = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(data)?;
        file.sync_all()?;
        set_private_mode(&temporary)?;
        std::fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result.map_err(|error| refuse(format!("cannot publish evidence record: {error}")))
}

#[cfg(unix)]
fn set_private_mode(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn set_private_mode(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

/// Publish a manifest atomically, in the encoding the format specifies.
pub fn atomic_manifest(path: &Path, value: &serde_json::Value) -> Result<(), EvidenceError> {
    // Sorted keys and no whitespace, because a manifest is digested by the
    // export.
    let text = crate::util::canonical_json(value);
    atomic_bytes(path, text.as_bytes())
}
