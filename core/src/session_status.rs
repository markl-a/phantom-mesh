//! Dev-session status heartbeat (S2 of the multi-machine dev framework).
//!
//! The dev session running on a node (Claude Code / codex loop) writes one
//! small JSON file each routine tick: what it is doing, on which branch,
//! and how its last gate went. `phantom serve` exposes the file at
//! `GET /rpc/session-status` (HMAC-gated) so any machine can roll up the
//! whole cluster's sessions with `phantom status mesh` — without SSH.
//!
//! 中文: dev session 心跳檔 — routine 每 tick 寫一次,serve 對 tailnet 曝光,
//! 任一台 `phantom status mesh` 彙總全艦。

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::clock::{Clock, SystemClock};

/// One node's dev-session heartbeat. All fields are self-reported by the
/// local routine; freshness is judged via `updated_at`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStatus {
    /// Node name (peers.json identity), e.g. "mac-coordinator".
    pub node: String,
    /// Coarse activity: "working" | "idle" | "blocked" | whatever the
    /// routine reports. Free-form on purpose — display-only.
    pub state: String,
    /// What it is working on (spec id / short description).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    /// Current git branch, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Last gate outcome (e.g. "review:APPROVE", "verify:green").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_verdict: Option<String>,
    /// Free-form one-liner (truncated at write time).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Unix seconds of the last write.
    pub updated_at: u64,
}

/// Cap free-form fields so a runaway routine can't bloat the heartbeat.
const MAX_FIELD_BYTES: usize = 512;

pub fn status_path(home: &Path) -> PathBuf {
    crate::cli_config::phantom_dir_under(home).join("session-status.json")
}

fn now_unix() -> u64 {
    SystemClock.now_unix_secs()
}

fn clamp(s: &str) -> String {
    if s.len() <= MAX_FIELD_BYTES {
        s.to_string()
    } else {
        // truncate on a char boundary
        let mut end = MAX_FIELD_BYTES;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s[..end].to_string()
    }
}

/// Write the heartbeat atomically (tmp + rename), stamping `updated_at`.
pub fn write_status(
    home: &Path,
    node: &str,
    state: &str,
    task: Option<&str>,
    branch: Option<&str>,
    last_verdict: Option<&str>,
    detail: Option<&str>,
) -> anyhow::Result<()> {
    if state.trim().is_empty() {
        anyhow::bail!("session status state is empty");
    }
    let dir = crate::cli_config::phantom_dir_under(home);
    fs::create_dir_all(&dir)?;
    let status = SessionStatus {
        node: clamp(node),
        state: clamp(state),
        task: task.map(clamp),
        branch: branch.map(clamp),
        last_verdict: last_verdict.map(clamp),
        detail: detail.map(clamp),
        updated_at: now_unix(),
    };
    let tmp = dir.join(".session-status.json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(&status)?)?;
    fs::rename(&tmp, status_path(home))?;
    Ok(())
}

/// Read the heartbeat; `None` when the node has never written one (a fresh
/// box is not an error) or the file is unreadable/garbled.
pub fn read_status(home: &Path) -> Option<SessionStatus> {
    let raw = fs::read(status_path(home)).ok()?;
    serde_json::from_slice(&raw).ok()
}

/// Seconds since the heartbeat was written (saturating).
pub fn age_secs(status: &SessionStatus) -> u64 {
    age_secs_on(&SystemClock, status)
}

/// Clock-injected core of [`age_secs`]: heartbeat age judged against `clock`'s
/// "now" so node-liveness / staleness checks (the gateway loop's dead-or-idle
/// detection) are deterministically testable with a `MockClock`. Saturates to 0
/// when the clock reads before `updated_at` (clock skew / future stamp).
pub fn age_secs_on(clock: &dyn Clock, status: &SessionStatus) -> u64 {
    clock.now_unix_secs().saturating_sub(status.updated_at)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status_at(updated_at: u64) -> SessionStatus {
        SessionStatus {
            node: "m1".into(),
            state: "working".into(),
            task: None,
            branch: None,
            last_verdict: None,
            detail: None,
            updated_at,
        }
    }

    #[test]
    fn age_secs_is_deterministic_under_a_pinned_clock() {
        use crate::clock::MockClock;
        let status = status_at(1_000); // heartbeat at unix-second 1_000

        // clock 90s after the heartbeat → age exactly 90 (no wall-clock dependence).
        let clock = MockClock::new(1_090_000); // 1_090_000 ms = 1_090 s
        assert_eq!(age_secs_on(&clock, &status), 90);

        clock.advance_ms(10_000); // +10s → 1_100 s
        assert_eq!(age_secs_on(&clock, &status), 100);

        // clock BEFORE the heartbeat (skew / future stamp) → saturates to 0, no underflow.
        let past = MockClock::new(500_000); // 500 s < 1_000 s
        assert_eq!(age_secs_on(&past, &status), 0);
    }

    #[test]
    fn write_then_read_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        write_status(
            tmp.path(),
            "m1",
            "working",
            Some("S2 heartbeat"),
            Some("step3"),
            Some("review:APPROVE"),
            None,
        )
        .unwrap();
        let s = read_status(tmp.path()).unwrap();
        assert_eq!(s.node, "m1");
        assert_eq!(s.state, "working");
        assert_eq!(s.task.as_deref(), Some("S2 heartbeat"));
        assert_eq!(s.branch.as_deref(), Some("step3"));
        assert_eq!(s.last_verdict.as_deref(), Some("review:APPROVE"));
        assert!(s.updated_at > 0);
        assert!(age_secs(&s) < 5);
    }

    #[test]
    fn overwrite_replaces_previous() {
        let tmp = tempfile::tempdir().unwrap();
        write_status(tmp.path(), "m1", "working", Some("a"), None, None, None).unwrap();
        write_status(tmp.path(), "m1", "idle", None, None, None, None).unwrap();
        let s = read_status(tmp.path()).unwrap();
        assert_eq!(s.state, "idle");
        assert!(s.task.is_none(), "stale fields must not survive overwrite");
    }

    #[test]
    fn missing_or_garbled_file_reads_none() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(read_status(tmp.path()).is_none());
        let dir = tmp.path().join(".phantom-mesh");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("session-status.json"), b"{garbage").unwrap();
        assert!(read_status(tmp.path()).is_none());
    }

    #[test]
    fn long_fields_are_clamped_not_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let long = "x".repeat(MAX_FIELD_BYTES * 3);
        write_status(
            tmp.path(),
            "m1",
            "working",
            Some(&long),
            None,
            None,
            Some(&long),
        )
        .unwrap();
        let s = read_status(tmp.path()).unwrap();
        assert_eq!(s.task.unwrap().len(), MAX_FIELD_BYTES);
        assert_eq!(s.detail.unwrap().len(), MAX_FIELD_BYTES);
    }

    #[test]
    fn empty_state_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(write_status(tmp.path(), "m1", "  ", None, None, None, None).is_err());
    }

    #[test]
    fn clamp_truncates_multibyte_on_char_boundary() {
        // 512 (the cap) is not a multiple of 3, so a naive byte-cut at
        // MAX_FIELD_BYTES would land mid-char and panic on `s[..512]`. clamp
        // must back up to the nearest char boundary instead.
        let s = "中".repeat(MAX_FIELD_BYTES); // each '中' is 3 bytes
        let out = clamp(&s);
        assert!(out.len() <= MAX_FIELD_BYTES, "stays within the byte cap");
        assert!(
            s.starts_with(&out),
            "result is a clean prefix, not corrupted"
        );
        // largest char boundary <= 512 is 510 bytes = 170 full 3-byte chars
        assert_eq!(out.chars().count(), MAX_FIELD_BYTES / 3);
    }
}
