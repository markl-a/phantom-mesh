//! Lightweight JSON-Lines tracer for phantom agent task execution.
//!
//! Records agent plan / route / tool_call / result events as one-line JSON
//! to `~/.phantom-mesh/traces/<task-id>.jsonl`. Used by L2 of the testing
//! framework (see goal_plan/docs/29-phantom-mesh-testing-framework).
//!
//! Intentionally minimal: no OpenTelemetry, no async runtime dep, no
//! thread-safety beyond what `BufWriter<File>` gives. One Tracer per task.
//! For multi-threaded recording, share via `Arc<Mutex<Tracer>>` at the
//! call site.

mod events;
#[cfg(test)]
mod tests;

pub use events::{Event, TimestampedEvent};

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub struct Tracer {
    task_id: String,
    file: BufWriter<File>,
    seq: AtomicU64,
}

impl Tracer {
    /// Create a tracer writing to the default location:
    /// `~/.phantom-mesh/traces/<task_id>.jsonl`.
    pub fn new(task_id: impl Into<String>) -> io::Result<Self> {
        let dir = default_trace_dir()?;
        Self::new_in_dir(task_id, dir)
    }

    /// Create a tracer writing into a custom directory.
    /// Used by tests to avoid touching the real home dir.
    pub fn new_in_dir(task_id: impl Into<String>, dir: impl Into<PathBuf>) -> io::Result<Self> {
        let task_id = task_id.into();
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{}.jsonl", sanitize_filename(&task_id)));
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Tracer {
            task_id,
            file: BufWriter::new(file),
            seq: AtomicU64::new(0),
        })
    }

    /// Append one event to the trace file. Returns immediately after
    /// writing to BufWriter; call `flush()` to guarantee on-disk or
    /// rely on `Drop`.
    pub fn record(&mut self, event: Event) -> io::Result<()> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let timestamped = TimestampedEvent {
            task_id: self.task_id.clone(),
            seq: self.seq.fetch_add(1, Ordering::SeqCst),
            timestamp_secs: now.as_secs(),
            timestamp_nanos: now.subsec_nanos(),
            event,
        };
        let line = serde_json::to_string(&timestamped)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        writeln!(self.file, "{}", line)?;
        Ok(())
    }

    /// Force flush to disk.
    pub fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }

    pub fn task_id(&self) -> &str {
        &self.task_id
    }
}

impl Drop for Tracer {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

/// Returns `~/.phantom-mesh/traces/`.
pub fn default_trace_dir() -> io::Result<PathBuf> {
    let home = dirs::home_dir()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "home dir not found"))?;
    Ok(home.join(".phantom-mesh").join("traces"))
}

/// Strip path separators and characters that could escape the trace dir.
/// Task IDs may come from untrusted sources (Telegram chat ids, broker
/// task ids) — `../../etc/passwd` must not produce a path traversal.
fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect()
}
