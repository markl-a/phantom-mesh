//! Unit tests for the JSON-Lines tracer.
//!
//! All tests use `TempDir` so we never touch the user's real
//! `~/.spectyn-mesh/traces/`.

use super::*;
use serde_json::Value;
use tempfile::TempDir;

#[test]
fn write_three_events_in_order() {
    let dir = TempDir::new().unwrap();
    let mut tracer = Tracer::new_in_dir("test-task-001", dir.path()).unwrap();

    tracer
        .record(Event::Plan {
            plan: "read README.md, summarise it".to_string(),
        })
        .unwrap();
    tracer
        .record(Event::Route {
            provider: "anthropic".to_string(),
            model: "claude-opus-4".to_string(),
            reason: "first in agent.master.providers".to_string(),
        })
        .unwrap();
    tracer
        .record(Event::Result {
            ok: true,
            summary: "summary done in 1 turn".to_string(),
        })
        .unwrap();
    drop(tracer);

    let path = dir.path().join("test-task-001.jsonl");
    let content = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 3, "should have written 3 lines");

    // each line is valid JSON
    for line in &lines {
        let _: Value = serde_json::from_str(line).expect("each line is valid JSON");
    }

    // sequence numbers ascend 0, 1, 2
    for (expected_seq, line) in lines.iter().enumerate() {
        let v: Value = serde_json::from_str(line).unwrap();
        assert_eq!(v["seq"].as_u64(), Some(expected_seq as u64));
    }

    // first line is Plan (per #[serde(tag = "kind", rename_all = "snake_case")])
    let first: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(first["kind"], "plan");
}

#[test]
fn append_mode_does_not_clobber_existing() {
    let dir = TempDir::new().unwrap();

    // first session writes 1 event
    {
        let mut t = Tracer::new_in_dir("resume-task", dir.path()).unwrap();
        t.record(Event::Result {
            ok: true,
            summary: "first".into(),
        })
        .unwrap();
    }

    // second session re-opens with same task_id, writes 1 more
    {
        let mut t = Tracer::new_in_dir("resume-task", dir.path()).unwrap();
        t.record(Event::Result {
            ok: true,
            summary: "second".into(),
        })
        .unwrap();
    }

    let content = std::fs::read_to_string(dir.path().join("resume-task.jsonl")).unwrap();
    assert_eq!(
        content.lines().count(),
        2,
        "append mode should preserve previous lines"
    );
}

#[test]
fn sanitize_filename_strips_path_separators() {
    // task IDs from untrusted sources (Telegram chat id, broker task id)
    // must not allow path traversal via "../" or absolute paths
    assert_eq!(sanitize_filename("normal-task-001"), "normal-task-001");
    assert_eq!(sanitize_filename("../../etc/passwd"), ".._.._etc_passwd");
    assert_eq!(sanitize_filename("foo/bar"), "foo_bar");
    assert_eq!(sanitize_filename("a b c"), "a_b_c");
    assert_eq!(sanitize_filename("name.with.dots"), "name.with.dots");
    assert_eq!(sanitize_filename("under_score-OK"), "under_score-OK");
}

#[test]
fn task_id_accessor_returns_original() {
    let dir = TempDir::new().unwrap();
    let tracer = Tracer::new_in_dir("my-task", dir.path()).unwrap();
    assert_eq!(tracer.task_id(), "my-task");
}

#[test]
fn tool_call_event_serializes_with_args() {
    let dir = TempDir::new().unwrap();
    let mut tracer = Tracer::new_in_dir("tool-task", dir.path()).unwrap();
    tracer
        .record(Event::ToolCall {
            name: "fs:read".to_string(),
            args: serde_json::json!({ "path": "/tmp/README.md", "max_bytes": 4096 }),
        })
        .unwrap();
    drop(tracer);

    let content = std::fs::read_to_string(dir.path().join("tool-task.jsonl")).unwrap();
    let v: Value = serde_json::from_str(content.trim()).unwrap();
    assert_eq!(v["kind"], "tool_call");
    assert_eq!(v["name"], "fs:read");
    assert_eq!(v["args"]["path"], "/tmp/README.md");
    assert_eq!(v["args"]["max_bytes"], 4096);
}

/// Documents and verifies the Tracer's concurrency contract:
/// when callers share a Tracer behind `Arc<Mutex<_>>` (the pattern
/// the module doc-comment recommends), every recorded event lands as
/// one whole JSON line — no torn or interleaved bytes.
///
/// 8 threads × 50 events = 400 lines. If the mutex were skipped, the
/// BufWriter would interleave writes byte-for-byte and `serde_json::
/// from_str` would fail on most lines. With the mutex, every line
/// parses and the seq numbers are a complete permutation of 0..400.
#[test]
fn atomic_writes_no_partial_lines() {
    use std::sync::{Arc, Mutex};
    use std::thread;

    let dir = TempDir::new().unwrap();
    let tracer = Arc::new(Mutex::new(
        Tracer::new_in_dir("concurrent-task", dir.path()).unwrap(),
    ));

    let n_threads = 8usize;
    let per_thread = 50usize;

    let handles: Vec<_> = (0..n_threads)
        .map(|tid| {
            let tracer = Arc::clone(&tracer);
            thread::spawn(move || {
                for i in 0..per_thread {
                    let mut t = tracer.lock().unwrap();
                    t.record(Event::Result {
                        ok: true,
                        summary: format!("t{}-w{}", tid, i),
                    })
                    .unwrap();
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }

    // Drop the tracer so the BufWriter flushes to disk before we read.
    let inner = Arc::try_unwrap(tracer)
        .ok()
        .expect("no other strong refs")
        .into_inner()
        .unwrap();
    drop(inner);

    let path = dir.path().join("concurrent-task.jsonl");
    let content = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(
        lines.len(),
        n_threads * per_thread,
        "expected {} whole lines",
        n_threads * per_thread
    );

    let mut seen_seqs = std::collections::HashSet::new();
    for (idx, line) in lines.iter().enumerate() {
        let v: Value = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("line {} is not valid JSON ({}): {:?}", idx, e, line));
        let seq = v["seq"].as_u64().expect("every line has a seq");
        assert!(
            seen_seqs.insert(seq),
            "seq {} appeared twice — atomic-write contract broken",
            seq
        );
    }
    assert_eq!(
        seen_seqs.len(),
        n_threads * per_thread,
        "every seq 0..{} appears exactly once",
        n_threads * per_thread
    );
}

#[test]
fn timestamp_is_recent() {
    let dir = TempDir::new().unwrap();
    let mut tracer = Tracer::new_in_dir("time-task", dir.path()).unwrap();
    tracer
        .record(Event::Result {
            ok: true,
            summary: "test".into(),
        })
        .unwrap();
    drop(tracer);

    let content = std::fs::read_to_string(dir.path().join("time-task.jsonl")).unwrap();
    let v: Value = serde_json::from_str(content.trim()).unwrap();
    let secs = v["timestamp_secs"].as_u64().unwrap();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    // timestamp should be within last 60 seconds
    assert!(secs <= now);
    assert!(
        now - secs < 60,
        "timestamp drift > 60s ({} vs {})",
        secs,
        now
    );
}
