//! The per-capture writer lock.
//!
//! One writer per capture, with a bounded TTL so a crash cannot strand work. The
//! lock records the owning pid: it is released by its owner, reclaimed when that
//! process is gone, and refused while a live writer holds it. `break_lock` is
//! the explicit operator override for the remaining case — a live-looking pid
//! that no longer owns the capture.

use std::io::Write;
use std::path::PathBuf;

use super::{EvidenceError, LOCK_TTL, now, refuse};
use crate::evidence::paths::lock_path;
use crate::report::Root;

/// An advisory lock held by one process for one capture.
pub struct Lock {
    path: PathBuf,
    held: bool,
}

impl Lock {
    pub fn new(root: &Root, capture_id: &str) -> Result<Self, EvidenceError> {
        Ok(Self {
            path: lock_path(root, capture_id)?,
            held: false,
        })
    }

    /// Take the lock, reclaiming it only when the recorded owner is gone.
    pub fn acquire(&mut self, break_lock: bool) -> Result<(), EvidenceError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| refuse(format!("cannot create the lock directory: {error}")))?;
        }
        for attempt in 0..2 {
            match std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&self.path)
            {
                Ok(mut file) => {
                    let record = serde_json::json!({
                        "pid": std::process::id(),
                        "started": now(),
                        "started_at": std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|elapsed| elapsed.as_secs_f64())
                            .unwrap_or(0.0),
                    });
                    let _ = file.write_all(record.to_string().as_bytes());
                    let _ = file.sync_all();
                    self.held = true;
                    return Ok(());
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let owner = self.owner();
                    let owner_pid = owner
                        .as_ref()
                        .and_then(|owner| owner.get("pid"))
                        .and_then(serde_json::Value::as_i64);
                    // Age alone never grants permission to replace a live writer:
                    // the TTL only applies once the recorded process is gone.
                    let stale = match owner_pid {
                        Some(pid) => !process_alive(pid),
                        None => self.age_seconds() > LOCK_TTL,
                    };
                    if break_lock || stale {
                        self.release_file();
                        if attempt == 0 {
                            continue;
                        }
                        return Err(refuse(format!(
                            "could not replace the writer lock at {}",
                            self.path.display()
                        )));
                    }
                    let described = owner_pid
                        .map(|pid| pid.to_string())
                        .unwrap_or_else(|| "unknown".to_owned());
                    return Err(refuse(format!(
                        "another writer holds this capture (pid {described}); \
                         use --break-lock if it is gone"
                    )));
                }
                Err(error) => {
                    return Err(refuse(format!("cannot take the writer lock: {error}")));
                }
            }
        }
        Err(refuse(format!(
            "could not replace the writer lock at {}",
            self.path.display()
        )))
    }

    fn owner(&self) -> Option<serde_json::Value> {
        let text = std::fs::read_to_string(&self.path).ok()?;
        serde_json::from_str(&text).ok()
    }

    fn age_seconds(&self) -> f64 {
        std::fs::metadata(&self.path)
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .map(|elapsed| elapsed.as_secs_f64())
            .unwrap_or(0.0)
    }

    fn release_file(&self) {
        let _ = std::fs::remove_file(&self.path);
    }

    pub fn release(&mut self) {
        if self.held {
            self.release_file();
            self.held = false;
        }
    }

    /// Whether this handle currently holds the lock.
    pub fn held(&self) -> bool {
        self.held
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(unix)]
fn process_alive(pid: i64) -> bool {
    if pid <= 0 {
        return false;
    }
    // Signal 0 performs the permission and existence checks without delivering
    // anything, which is exactly the question being asked.
    unsafe {
        libc::kill(pid as libc::pid_t, 0) == 0
            || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}

#[cfg(not(unix))]
fn process_alive(pid: i64) -> bool {
    pid > 0
}
