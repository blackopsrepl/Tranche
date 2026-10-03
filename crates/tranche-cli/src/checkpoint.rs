//! Durable JSONL boundaries. Never append a response onto a torn record.
use std::io::Write;
use std::path::Path;

fn repair(path: &Path) -> Result<(), String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
    };
    if bytes.is_empty() || bytes.ends_with(b"\n") {
        return Ok(());
    }
    let boundary = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |i| i + 1);
    let tail = &bytes[boundary..];
    if serde_json::from_slice::<serde_json::Value>(tail).is_ok() {
        let mut stream = std::fs::OpenOptions::new()
            .append(true)
            .open(path)
            .map_err(|error| error.to_string())?;
        stream
            .write_all(b"\n")
            .and_then(|_| stream.sync_all())
            .map_err(|error| error.to_string())?;
    } else {
        // Preserve the damaged bytes before truncating. Unique names never overwrite evidence.
        let mut suffix = 0;
        loop {
            let quarantine = path.with_extension(format!("jsonl.torn-{suffix}"));
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(quarantine)
            {
                Ok(mut file) => {
                    file.write_all(tail)
                        .and_then(|_| file.sync_all())
                        .map_err(|error| error.to_string())?;
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => suffix += 1,
                Err(error) => return Err(error.to_string()),
            }
        }
        let stream = std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .map_err(|error| error.to_string())?;
        stream
            .set_len(boundary as u64)
            .and_then(|_| stream.sync_all())
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// Responses are synced as they arrive; publication is sorted by job index.
/// A fresh generation never touches the usable log until the pass succeeds.
pub struct Checkpoint<'a> {
    _pass: &'a Pass,
    path: std::path::PathBuf,
    journal: std::path::PathBuf,
    stream: std::sync::Mutex<std::fs::File>,
    fresh: bool,
}

/// A composite pass may publish only after every request succeeded.
pub fn pass_result(result: Result<(usize, usize), String>) -> Result<(), String> {
    let (written, errors) = result?;
    if errors == 0 {
        Ok(())
    } else {
        Err(format!(
            "{written} responses checkpointed; {errors} failed; re-run to continue"
        ))
    }
}

/// An OS lock covers recovery, job selection, checkpointing and publication.
/// Keep the lock inode in place: deleting it permits two independent holders.
/// The OS releases the lock on process exit, including an interrupted pass.
pub struct Pass {
    path: std::path::PathBuf,
    _lock: std::fs::File,
}

impl Pass {
    pub fn acquire(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path.with_extension("jsonl.lock"))
            .map_err(|error| error.to_string())?;
        lock.try_lock().map_err(|error| {
            format!(
                "cannot acquire model-pass lock for {}: {error}",
                path.display()
            )
        })?;
        Ok(Self {
            path: path.to_owned(),
            _lock: lock,
        })
    }

    pub fn repair(&self) -> Result<(), String> {
        repair(&self.path)
    }

    pub fn recover(&self) -> Result<(), String> {
        self.repair()?;
        publish(
            &self.path,
            &self.path.with_extension("jsonl.checkpoint"),
            false,
        )
    }

    pub fn checkpoint(&self, fresh: bool) -> Result<Checkpoint<'_>, String> {
        Checkpoint::new(self, fresh)
    }
}

impl<'a> Checkpoint<'a> {
    fn new(pass: &'a Pass, fresh: bool) -> Result<Self, String> {
        let path = &pass.path;
        let journal = path.with_extension("jsonl.checkpoint");
        let stream = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&journal)
            .map_err(|error| error.to_string())?;
        stream.sync_all().map_err(|error| error.to_string())?;
        sync_parent(path)?;
        Ok(Self {
            _pass: pass,
            path: path.to_owned(),
            journal,
            stream: std::sync::Mutex::new(stream),
            fresh,
        })
    }

    pub fn append(&self, index: usize, record: &serde_json::Value) -> Result<(), String> {
        let line = serde_json::to_vec(&serde_json::json!({"index": index, "record": record}))
            .map_err(|error| error.to_string())?;
        let mut stream = self.stream.lock().map_err(|error| error.to_string())?;
        stream
            .write_all(&line)
            .and_then(|_| stream.write_all(b"\n"))
            .and_then(|_| stream.sync_all())
            .map_err(|error| format!("cannot checkpoint response: {error}"))
    }

    pub fn finish(&self, successful: bool) -> Result<(), String> {
        if self.fresh && !successful {
            return Ok(());
        }
        publish(&self.path, &self.journal, self.fresh)
    }
}

fn publish(path: &Path, journal: &Path, fresh: bool) -> Result<(), String> {
    repair(journal)?;
    let text = match std::fs::read_to_string(journal) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.to_string()),
    };
    let mut records = Vec::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let value: serde_json::Value =
            serde_json::from_str(line).map_err(|error| error.to_string())?;
        let index = value["index"].as_u64().ok_or("checkpoint missing index")?;
        let record = value.get("record").ok_or("checkpoint missing record")?;
        records.push((
            index,
            serde_json::to_string(record).map_err(|error| error.to_string())?,
        ));
    }
    records.sort_by_key(|(index, _)| *index);
    let mut prior = if fresh {
        String::new()
    } else {
        match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(error.to_string()),
        }
    };
    // Exact-record deduplication makes recovery idempotent if rename succeeded
    // but the process died before removing its journal. New answers still win.
    let existing: std::collections::HashSet<String> = prior.lines().map(str::to_owned).collect();
    for (_, line) in records {
        if !existing.contains(&line) {
            prior.push_str(&line);
            prior.push('\n');
        }
    }
    let replacement = path.with_extension("jsonl.replacement");
    let mut file = std::fs::File::create(&replacement).map_err(|error| error.to_string())?;
    file.write_all(prior.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|error| error.to_string())?;
    std::fs::rename(replacement, path).map_err(|error| error.to_string())?;
    sync_parent(path)?;
    std::fs::remove_file(journal).map_err(|error| error.to_string())?;
    sync_parent(path)
}

fn sync_parent(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::File::open(parent)
            .and_then(|file| file.sync_all())
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}
