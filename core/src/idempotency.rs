//! Best-effort, process-local at-most-once dedup for partner ingress
//! (NORTH-STAR §5 ③/④ safety rail). This is NOT a strict or distributed
//! guarantee: the check→append is serialized only by a process [`Mutex`] (a
//! second process, or a fresh restart racing an in-flight request, can both
//! see "unseen") and it is fail-open on write errors (a transient FS failure
//! is treated as "not a duplicate" so a real request is never blocked). It
//! collapses the common retry storms; it does not provide exactly-once.
//!
//! The partner has multiple, independently-retrying front doors:
//!   - the iOS app re-sends a message when the network blips,
//!   - the swarm/squad dispatch re-posts a job on a peer timeout,
//!   - a human hits "send" twice.
//!
//! Before this gate every retry executed the agent turn again. That was merely
//! wasteful while the partner only *read* (search/recall) — but the MVP is about
//! to give it *write* tools (Todoist add-task, note capture, later email/calendar).
//! A re-sent "remind me to call mum" must NOT create two tasks. So we record a
//! dedup key the first time we see a request and short-circuit any later request
//! carrying the same key within a TTL window.
//!
//! Design (deliberately small + dependency-light):
//!   - A single JSONL ledger at `~/.phantom-mesh/idempotency.jsonl` — same
//!     convention as `partner-signals.jsonl`, no DB to provision, survives a
//!     restart (so an at-most-once guarantee holds across a serve bounce too).
//!   - One `{key, ts, kind}` record per accepted request, appended atomically.
//!     An optional `val` field carries a caller-associated value (e.g. the
//!     `job_id` minted for an accepted `/rpc/task/assign`) so a later duplicate
//!     can be answered with the ORIGINAL value instead of a bare marker.
//!   - [`check_and_record`] scans the recent (within-TTL) tail: if the key is
//!     already present it returns [`Decision::Duplicate`] WITHOUT appending;
//!     otherwise it appends and returns [`Decision::First`]. Expired records are
//!     ignored on read and pruned opportunistically so the file stays bounded.
//!   - A process-wide [`Mutex`] serializes the read-modify-append so two
//!     concurrent retries of the same key can't both see "not present" and both
//!     proceed (the file append alone is not enough — the check is the race).
//!
//! Keys are caller-supplied when available (an explicit `idempotency_key` /
//! request id from the client) and otherwise derived from a stable content hash
//! ([`content_key`]) so an identical body resent without a key still dedups.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// Default TTL: a dedup key is honored for this many seconds after first seen.
/// Long enough to cover realistic retry storms (a few minutes of flaky network
/// or a stuck-then-resumed job) without letting the ledger grow without bound or
/// blocking a legitimately-repeated request the next day.
pub const DEFAULT_TTL_SECS: u64 = 24 * 60 * 60;

/// Serializes the read-modify-append so the check itself is atomic across
/// threads in this process. The on-disk append handles cross-process safety at
/// the OS level; this guards the in-process check→append window which a bare
/// append can't.
static GUARD: Mutex<()> = Mutex::new(());

/// Outcome of [`check_and_record`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// This key has not been seen within the TTL window — the request was
    /// recorded and the caller SHOULD proceed (run the side-effecting work).
    First,
    /// This key was already recorded within the TTL window — the caller MUST
    /// NOT re-run the side effect. `first_seen` is the unix ts of the original.
    Duplicate { first_seen: u64 },
}

impl Decision {
    /// `true` when the caller should proceed (first time).
    pub fn is_first(&self) -> bool {
        matches!(self, Decision::First)
    }
    /// `true` when the request is a duplicate and must be skipped.
    pub fn is_duplicate(&self) -> bool {
        matches!(self, Decision::Duplicate { .. })
    }
}

/// Path of the dedup ledger. Override with `PHANTOM_IDEMPOTENCY_STORE` (used in
/// tests and to relocate the brain's home). Mirrors `partner::signals_path`.
pub fn store_path() -> PathBuf {
    if let Ok(p) = std::env::var("PHANTOM_IDEMPOTENCY_STORE") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    crate::cli_config::phantom_data_dir()
        .unwrap_or_else(|_| PathBuf::from(".").join(".phantom-mesh"))
        .join("idempotency.jsonl")
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Derive a stable dedup key from a request's content (and a `scope` namespace
/// so the same text on two different endpoints doesn't collide). Used when the
/// client supplies no explicit idempotency key. SHA-256 hex, truncated to 32
/// chars (128 bits — collision-safe for this scale, compact in the ledger).
pub fn content_key(scope: &str, content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(scope.as_bytes());
    hasher.update([0u8]); // domain separator so scope|content is unambiguous
    hasher.update(content.as_bytes());
    let digest = hasher.finalize();
    let hex = hex_lower(&digest);
    format!("{scope}:{}", &hex[..32])
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        s.push(char::from_digit((b & 0xf) as u32, 16).unwrap());
    }
    s
}

/// Derive the dedup key for a `/rpc/task/assign` request (#321 §5). Prefers the
/// caller's explicit `idempotency_key` (so a forwarded retry that preserves the
/// key dedups byte-for-byte); absent or blank, falls back to a stable content
/// hash of `agent\nprompt` so an identical body resent without a key still
/// dedups. Shared by both `/rpc/task/assign` handlers (the hardened `serve.rs`
/// router and the shipped `main.rs` daemon) so they cannot drift.
pub fn task_assign_idem_key(idempotency_key: Option<&str>, agent: &str, prompt: &str) -> String {
    idempotency_key
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| format!("task_assign:{s}"))
        .unwrap_or_else(|| content_key("task_assign", &format!("{agent}\n{prompt}")))
}

/// Check `key` against the ledger and, if unseen within `ttl_secs`, record it.
///
/// Returns [`Decision::First`] (and appends) the first time; [`Decision::Duplicate`]
/// (without appending) for a retry. Honors `PHANTOM_IDEMPOTENCY_STORE`.
///
/// Fail-open philosophy: an unreadable/corrupt ledger is treated as "key not
/// seen" so a transient FS error never *blocks* a legitimate request — the cost
/// of a false-negative dedup (a rare double-run) is lower than the cost of the
/// partner refusing to act at all. The dedup is a best-effort safety rail, not a
/// transactional guarantee.
pub fn check_and_record(key: &str, kind: &str, ttl_secs: u64) -> Decision {
    check_and_record_value(key, kind, None, ttl_secs).0
}

/// Convenience: dedup window of [`DEFAULT_TTL_SECS`].
pub fn check_and_record_default(key: &str, kind: &str) -> Decision {
    check_and_record(key, kind, DEFAULT_TTL_SECS)
}

/// Like [`check_and_record`], but associates `value` with the key on the first
/// sighting and, on a duplicate, returns the value stored with the ORIGINAL
/// sighting. Used by `/rpc/task/assign` to carry the accepted `job_id`: a
/// retried/forwarded assign that dedups must answer with the first job's id so
/// the caller polls the same job (a job_id-less success is treated as an error
/// by `mesh::assign_task_to_peer*`). The returned `Option<String>` is the
/// stored value (the original on Duplicate; the just-recorded `value` on First).
pub fn check_and_record_value(
    key: &str,
    kind: &str,
    value: Option<&str>,
    ttl_secs: u64,
) -> (Decision, Option<String>) {
    let path = store_path();
    let _g = GUARD.lock().unwrap_or_else(|p| p.into_inner());
    check_and_record_value_at(&path, key, kind, value, ttl_secs, now_unix())
}

/// Convenience: value-carrying check over [`DEFAULT_TTL_SECS`].
pub fn check_and_record_value_default(
    key: &str,
    kind: &str,
    value: Option<&str>,
) -> (Decision, Option<String>) {
    check_and_record_value(key, kind, value, DEFAULT_TTL_SECS)
}

/// Testable core: pure over (`path`, `now`) — no wall clock, no global env.
/// Reads the ledger, decides, and (on First) appends + opportunistically prunes.
pub fn check_and_record_at(
    path: &Path,
    key: &str,
    kind: &str,
    ttl_secs: u64,
    now: u64,
) -> Decision {
    check_and_record_value_at(path, key, kind, None, ttl_secs, now).0
}

/// Value-carrying core: like [`check_and_record_at`] but stores `value` with the
/// key on First and returns the original value on Duplicate. See
/// [`check_and_record_value`] for the contract.
pub fn check_and_record_value_at(
    path: &Path,
    key: &str,
    kind: &str,
    value: Option<&str>,
    ttl_secs: u64,
    now: u64,
) -> (Decision, Option<String>) {
    let cutoff = now.saturating_sub(ttl_secs);
    // key -> (earliest in-window ts, value recorded at that earliest sighting),
    // so a duplicate reports both the original first_seen and its stored value.
    let mut seen: HashMap<String, (u64, Option<String>)> = HashMap::new();
    let mut any_expired = false;

    if let Ok(content) = std::fs::read_to_string(path) {
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let rec: Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue, // skip a torn/partial line, don't abort
            };
            let ts = rec.get("ts").and_then(Value::as_u64).unwrap_or(0);
            let k = match rec.get("key").and_then(Value::as_str) {
                Some(k) => k,
                None => continue,
            };
            if ts <= cutoff {
                any_expired = true; // candidate for pruning
                continue;
            }
            // `val` is optional and absent on rows written before this field
            // existed (older binaries) — read back as None, never an error.
            let val = rec.get("val").and_then(Value::as_str).map(str::to_string);
            seen.entry(k.to_string())
                .and_modify(|e| {
                    // Keep the earliest sighting's ts AND its value together.
                    if ts < e.0 {
                        *e = (ts, val.clone());
                    }
                })
                .or_insert((ts, val));
        }
    }

    if let Some((first_seen, stored_val)) = seen.get(key) {
        return (Decision::Duplicate { first_seen: *first_seen }, stored_val.clone());
    }

    // First sighting: record it. If the file had expired rows, rewrite it
    // pruned (drop them) while we're appending so it can't grow unbounded.
    let value_owned = value.map(str::to_string);
    if any_expired {
        seen.insert(key.to_string(), (now, value_owned.clone()));
        rewrite_pruned(path, &seen, kind, key, now);
    } else {
        append(path, key, kind, value, now);
    }
    (Decision::First, value_owned)
}

/// Append one `{key, ts, kind}` record (creating parents). Best-effort: a write
/// failure leaves the decision as First (we already returned the intent to
/// proceed) — see fail-open note on [`check_and_record`].
fn append(path: &Path, key: &str, kind: &str, value: Option<&str>, ts: u64) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut rec = json!({ "key": key, "ts": ts, "kind": kind });
    if let Some(v) = value {
        rec["val"] = json!(v); // optional — only written when a value is supplied
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{rec}");
    }
}

/// Rewrite the ledger with only in-window keys (prunes expired rows). Writes to
/// a sibling temp file then renames, so a crash mid-write can't truncate the
/// ledger. `kind` is recorded for the just-added `key`; other rows are written
/// with a generic kind (their original kind isn't tracked in the in-memory map,
/// which is fine — kind is diagnostic metadata, the key/ts is what dedups).
fn rewrite_pruned(
    path: &Path,
    seen: &HashMap<String, (u64, Option<String>)>,
    kind: &str,
    new_key: &str,
    now: u64,
) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut buf = String::new();
    for (k, (ts, val)) in seen {
        let row_kind = if k == new_key { kind } else { "retained" };
        // The new key gets `now` as its ts (it's the just-recorded sighting).
        let row_ts = if k == new_key { now } else { *ts };
        let mut rec = json!({ "key": k, "ts": row_ts, "kind": row_kind });
        if let Some(v) = val {
            rec["val"] = json!(v); // preserve any associated value across pruning
        }
        buf.push_str(&rec.to_string());
        buf.push('\n');
    }
    let tmp = path.with_extension("jsonl.tmp");
    if std::fs::write(&tmp, &buf).is_ok() {
        if std::fs::rename(&tmp, path).is_err() {
            // Rename failed (e.g. cross-device) — fall back to direct write so
            // the new key is still persisted; pruning is best-effort.
            let _ = std::fs::write(path, &buf);
            let _ = std::fs::remove_file(&tmp);
        }
    } else {
        // Couldn't stage the pruned file — at least append the new key (with
        // its value) so the dedup guarantee for THIS request still holds.
        let v = seen.get(new_key).and_then(|(_, v)| v.as_deref());
        append(path, new_key, kind, v, now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_store() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("idempotency.jsonl");
        (dir, path)
    }

    #[test]
    fn first_then_duplicate() {
        let (_d, path) = tmp_store();
        let now = 1_000_000;
        let d1 = check_and_record_at(&path, "k1", "partner_message", 3600, now);
        assert_eq!(d1, Decision::First, "first sighting proceeds");
        let d2 = check_and_record_at(&path, "k1", "partner_message", 3600, now + 5);
        assert_eq!(
            d2,
            Decision::Duplicate { first_seen: now },
            "a retry within TTL is a duplicate reporting the original ts"
        );
    }

    #[test]
    fn value_is_stored_on_first_and_returned_on_duplicate() {
        // The job_id (or any associated value) recorded with the first sighting
        // must come back on a later duplicate, so /rpc/task/assign can answer a
        // retry with the ORIGINAL job_id instead of a bare marker.
        let (_d, path) = tmp_store();
        let now = 1_500_000;
        let (d1, v1) =
            check_and_record_value_at(&path, "k", "task_assign", Some("job-abc"), 3600, now);
        assert_eq!(d1, Decision::First);
        assert_eq!(v1.as_deref(), Some("job-abc"), "first returns the value it stored");
        // A retry within TTL: duplicate, and the stored value is echoed back even
        // though the caller passed a different candidate value.
        let (d2, v2) =
            check_and_record_value_at(&path, "k", "task_assign", Some("job-xyz"), 3600, now + 2);
        assert_eq!(d2, Decision::Duplicate { first_seen: now });
        assert_eq!(
            v2.as_deref(),
            Some("job-abc"),
            "duplicate returns the ORIGINAL stored value, not the retry's candidate"
        );
    }

    #[test]
    fn value_survives_pruning_rewrite() {
        // When a First sighting triggers a prune-rewrite (expired rows present),
        // the value associated with a still-fresh key must be preserved.
        let (_d, path) = tmp_store();
        let old = json!({ "key": "old", "ts": 10u64, "kind": "x" });
        let fresh = json!({ "key": "fresh", "ts": 9_000u64, "kind": "task_assign", "val": "job-fresh" });
        std::fs::write(&path, format!("{old}\n{fresh}\n")).unwrap();
        let now = 10_000u64;
        // Record a new key with a TTL that expires `old` but keeps `fresh` → rewrite.
        let (d, _) = check_and_record_value_at(&path, "new", "task_assign", Some("job-new"), 5_000, now);
        assert_eq!(d, Decision::First);
        // `fresh` is still a duplicate AND its stored value survived the rewrite.
        let (d2, v2) = check_and_record_value_at(&path, "fresh", "task_assign", None, 5_000, now);
        assert_eq!(d2, Decision::Duplicate { first_seen: 9_000 });
        assert_eq!(v2.as_deref(), Some("job-fresh"), "value preserved across prune");
    }

    #[test]
    fn distinct_keys_both_first() {
        let (_d, path) = tmp_store();
        let now = 2_000_000;
        assert!(check_and_record_at(&path, "a", "k", 3600, now).is_first());
        assert!(check_and_record_at(&path, "b", "k", 3600, now).is_first());
        // Re-checking each is now a duplicate.
        assert!(check_and_record_at(&path, "a", "k", 3600, now).is_duplicate());
        assert!(check_and_record_at(&path, "b", "k", 3600, now).is_duplicate());
    }

    #[test]
    fn expired_key_is_first_again() {
        let (_d, path) = tmp_store();
        let now = 3_000_000;
        assert!(check_and_record_at(&path, "k", "x", 100, now).is_first());
        // Well past the 100s TTL: the old record is expired, so it's First again.
        let later = now + 1_000;
        assert_eq!(
            check_and_record_at(&path, "k", "x", 100, later),
            Decision::First,
            "a key past its TTL is treated as new"
        );
    }

    #[test]
    fn pruning_drops_expired_rows() {
        let (_d, path) = tmp_store();
        // Seed an old (expired) row and a still-fresh row directly.
        let old = json!({ "key": "old", "ts": 10u64, "kind": "x" });
        let fresh = json!({ "key": "fresh", "ts": 9_999u64, "kind": "x" });
        std::fs::write(&path, format!("{old}\n{fresh}\n")).unwrap();
        // Now record a new key with a TTL that expires `old` but keeps `fresh`.
        let now = 10_000u64;
        assert!(check_and_record_at(&path, "new", "x", 5_000, now).is_first());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(!content.contains("\"old\""), "expired row pruned: {content}");
        assert!(content.contains("\"fresh\""), "fresh row kept: {content}");
        assert!(content.contains("\"new\""), "new row added: {content}");
    }

    #[test]
    fn corrupt_lines_are_skipped_not_fatal() {
        let (_d, path) = tmp_store();
        std::fs::write(&path, "{not json\n{\"key\":\"good\",\"ts\":50,\"kind\":\"x\"}\n").unwrap();
        // `good` is within TTL → duplicate; the torn line above must not abort.
        let d = check_and_record_at(&path, "good", "x", 1000, 100);
        assert_eq!(d, Decision::Duplicate { first_seen: 50 });
    }

    #[test]
    fn content_key_is_stable_and_scoped() {
        let a = content_key("partner_message", "remind me to call mum");
        let b = content_key("partner_message", "remind me to call mum");
        assert_eq!(a, b, "same scope+content → same key");
        let c = content_key("dispatch", "remind me to call mum");
        assert_ne!(a, c, "different scope → different key");
        let d = content_key("partner_message", "something else");
        assert_ne!(a, d, "different content → different key");
        assert!(a.starts_with("partner_message:"), "key is scope-prefixed: {a}");
    }

    #[test]
    fn missing_store_is_first() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.jsonl");
        assert_eq!(check_and_record_at(&path, "k", "x", 100, 1), Decision::First);
    }
}
