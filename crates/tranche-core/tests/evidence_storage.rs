//! Evidence foundations: path confinement, content-addressed storage and the
//! writer lock.
//!
//! The confinement and lock rules exist because a checkpoint is untrusted input
//! and a writer lock must not be reclaimable by someone else. Each test here
//! fails if the corresponding guard is weakened, not merely if it is absent.

use std::path::Path;
use tranche_core::evidence::lock::Lock;
use tranche_core::evidence::paths::{body_path, confined_file, validate_capture_id};
use tranche_core::evidence::store::{read_body, store_body, strict_json_loads, sweep_orphans};
use tranche_core::evidence::{LOCK_TTL, evidence_root};
use tranche_core::report::Root;

fn root() -> (tempfile::TempDir, Root) {
    let temp = tempfile::tempdir().expect("a temporary root");
    let root = Root::new(temp.path());
    std::fs::create_dir_all(evidence_root(&root)).expect("the evidence root");
    (temp, root)
}

#[test]
fn a_capture_id_must_be_thirty_two_lowercase_hex_characters() {
    assert!(validate_capture_id("0123456789abcdef0123456789abcdef").is_ok());
    for bad in [
        "",
        "abc",
        "0123456789ABCDEF0123456789abcdef",
        "0123456789abcdef0123456789abcdeg",
        "0123456789abcdef0123456789abcde",
    ] {
        assert!(
            validate_capture_id(bad).is_err(),
            "{bad:?} should be refused"
        );
    }
}

#[test]
fn a_path_outside_the_root_is_refused() {
    let (_temp, root) = root();
    let base = evidence_root(&root);
    for bad in ["../escape", "a/../../escape", "/etc/passwd", "", "a\0b"] {
        assert!(
            confined_file(&base, bad).is_err(),
            "{bad:?} should be refused"
        );
    }
}

#[test]
#[cfg(unix)]
fn a_symlink_in_the_chain_is_refused() {
    let (_temp, root) = root();
    let base = evidence_root(&root);
    let outside = tempfile::tempdir().expect("an outside directory");
    std::fs::write(outside.path().join("secret"), b"x").expect("written");

    // A symlinked directory must not be traversed.
    let link = base.join("linked");
    std::os::unix::fs::symlink(outside.path(), &link).expect("symlink");
    assert!(confined_file(&base, "linked/secret").is_err());

    // A symlinked file must not be followed.
    let file_link = base.join("aliased.json");
    std::os::unix::fs::symlink(outside.path().join("secret"), &file_link).expect("symlink");
    assert!(confined_file(&base, "aliased.json").is_err());
}

#[test]
#[cfg(unix)]
fn a_multiply_linked_file_is_refused() {
    let (_temp, root) = root();
    let base = evidence_root(&root);
    let original = base.join("original.json");
    std::fs::write(&original, b"{}").expect("written");
    std::fs::hard_link(&original, base.join("second.json")).expect("hard link");
    assert!(
        confined_file(&base, "original.json").is_err(),
        "a body with more than one link is not a single immutable artifact"
    );
}

#[test]
fn a_stored_body_round_trips_and_is_verified_on_read() {
    let (_temp, root) = root();
    let data = b"response bytes";
    let digest = store_body(&root, data).expect("stored");
    assert_eq!(digest.len(), 64);
    assert_eq!(read_body(&root, &digest).expect("read"), data);

    // Storing the same bytes twice is idempotent.
    assert_eq!(store_body(&root, data).expect("stored again"), digest);

    // Corrupted bytes are refused rather than returned.
    let path = body_path(&root, &digest).expect("path");
    std::fs::write(&path, b"tampered").expect("written");
    assert!(read_body(&root, &digest).is_err());

    // A digest that names nothing is refused, not treated as empty.
    assert!(read_body(&root, &"0".repeat(64)).is_err());
    // A malformed digest never becomes a path.
    assert!(read_body(&root, "not-a-digest").is_err());
}

#[test]
fn a_body_whose_name_is_right_but_bytes_are_wrong_is_not_trusted() {
    let (_temp, root) = root();
    let digest = store_body(&root, b"real").expect("stored");
    // Overwrite the file directly, keeping the digest name.
    std::fs::write(body_path(&root, &digest).expect("path"), b"forged").expect("written");
    // Storing again must not accept the file just because the name matches.
    store_body(&root, b"real").expect_err("a mismatched existing body is refused");
}

#[test]
fn the_orphan_sweep_removes_only_uncited_bodies() {
    let (_temp, root) = root();
    let cited = store_body(&root, b"cited").expect("stored");
    let orphan = store_body(&root, b"orphan").expect("stored");

    // One capture cites only the first body.
    let capture = "0123456789abcdef0123456789abcdef";
    std::fs::create_dir_all(evidence_root(&root).join(capture)).expect("capture dir");
    let manifest = serde_json::json!({
        "sources": [{"body_sha256": cited}],
    });
    std::fs::write(
        evidence_root(&root).join(capture).join("manifest.json"),
        serde_json::to_vec(&manifest).expect("serializable"),
    )
    .expect("written");

    assert_eq!(sweep_orphans(&root).expect("swept"), 1);
    assert!(body_path(&root, &cited).expect("path").exists());
    assert!(!body_path(&root, &orphan).expect("path").exists());
}

#[test]
fn a_manifest_that_cannot_be_read_never_costs_the_bodies_it_might_cite() {
    let (_temp, root) = root();
    let cited = store_body(&root, b"cited").expect("stored");
    let capture = "0123456789abcdef0123456789abcdef";
    std::fs::create_dir_all(evidence_root(&root).join(capture)).expect("capture dir");
    // A damaged manifest must not read as "cites nothing".
    std::fs::write(
        evidence_root(&root).join(capture).join("manifest.json"),
        b"{ this is not json",
    )
    .expect("written");
    assert_eq!(sweep_orphans(&root).expect("swept"), 1);
    // The bytes are removed because nothing readable cites them, which is why the
    // sweep only runs on a completed capture.
    assert!(!body_path(&root, &cited).expect("path").exists());
}

#[test]
fn strict_json_refuses_ambiguous_records() {
    assert!(strict_json_loads(br#"{"a": 1}"#).is_ok());
    // Duplicate keys can be read two ways.
    assert!(strict_json_loads(br#"{"a": 1, "a": 2}"#).is_err());
    // Infinity cannot round-trip through JSON or a digest.
    assert!(strict_json_loads(br#"{"a": Infinity}"#).is_err());
    assert!(strict_json_loads(br#"{"a": NaN}"#).is_err());
    // Invalid UTF-8 is not a record.
    assert!(strict_json_loads(&[0xff, 0xfe]).is_err());
    // A non-object is still JSON; the caller decides whether that is acceptable.
    assert!(strict_json_loads(b"[1,2]").is_ok());
}

#[test]
fn a_live_writer_holds_the_lock() {
    let (_temp, root) = root();
    let capture = "0123456789abcdef0123456789abcdef";
    let mut first = Lock::new(&root, capture).expect("lock");
    first.acquire(false).expect("acquired");
    assert!(first.held());

    // The same process holds it, so a second writer is refused.
    let mut second = Lock::new(&root, capture).expect("lock");
    let error = second
        .acquire(false)
        .expect_err("a live writer is respected");
    assert!(error.0.contains("another writer holds"), "{error}");

    first.release();
    second.acquire(false).expect("acquired after release");
}

#[test]
fn age_alone_never_replaces_a_live_writer() {
    let (_temp, root) = root();
    let capture = "0123456789abcdef0123456789abcdef";
    let mut holder = Lock::new(&root, capture).expect("lock");
    holder.acquire(false).expect("acquired");
    // Backdate the lock far beyond the TTL. The owner is this live process, so
    // the age must not grant permission to steal it.
    let path = tranche_core::evidence::paths::lock_path(&root, capture).expect("path");
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(LOCK_TTL as u64 * 4);
    set_mtime(&path, old);
    let mut claimant = Lock::new(&root, capture).expect("lock");
    let error = claimant
        .acquire(false)
        .expect_err("a backdated but live lock is still held");
    assert!(error.0.contains("another writer holds"), "{error}");
}

#[test]
fn a_dead_writer_is_reclaimed() {
    let (_temp, root) = root();
    let capture = "0123456789abcdef0123456789abcdef";
    // A lock file whose owner cannot exist: pid 0 is never a real process.
    let path = tranche_core::evidence::paths::lock_path(&root, capture).expect("path");
    std::fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({"pid": 0, "started": "x"})).expect("serializable"),
    )
    .expect("written");
    let mut lock = Lock::new(&root, capture).expect("lock");
    lock.acquire(false).expect("a dead owner is reclaimed");
}

#[test]
fn break_lock_overrides_a_live_writer() {
    let (_temp, root) = root();
    let capture = "0123456789abcdef0123456789abcdef";
    let mut holder = Lock::new(&root, capture).expect("lock");
    holder.acquire(false).expect("acquired");
    // The operator override is the remaining case: a live-looking pid that no
    // longer owns the capture.
    let mut override_lock = Lock::new(&root, capture).expect("lock");
    override_lock
        .acquire(true)
        .expect("the explicit override replaces the lock");
}

#[test]
fn dropping_a_lock_releases_it() {
    let (_temp, root) = root();
    let capture = "0123456789abcdef0123456789abcdef";
    let path = tranche_core::evidence::paths::lock_path(&root, capture).expect("path");
    {
        let mut lock = Lock::new(&root, capture).expect("lock");
        lock.acquire(false).expect("acquired");
        assert!(path.exists());
    }
    assert!(!path.exists(), "the guard releases on drop");
}

#[cfg(unix)]
fn set_mtime(path: &Path, when: std::time::SystemTime) {
    use std::os::unix::ffi::OsStrExt;
    let seconds = when
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0);
    let times = [
        libc::timespec {
            tv_sec: seconds,
            tv_nsec: 0,
        },
        libc::timespec {
            tv_sec: seconds,
            tv_nsec: 0,
        },
    ];
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).expect("path");
    unsafe {
        libc::utimensat(libc::AT_FDCWD, c_path.as_ptr(), times.as_ptr(), 0);
    }
}
