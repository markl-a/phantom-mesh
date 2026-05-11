//! APFS local snapshots — phantom's safety net on macOS.
//!
//! `tmutil localsnapshot` creates a volume-wide point-in-time snapshot in
//! ~1 s and does **not** require sudo. We use it the way Time Machine does:
//! pin a known-good state right before a subagent starts touching the
//! filesystem, then offer the user a clean way to roll back if things go
//! sideways. Combined with subagent's `auto_snapshot: true` flag, every
//! mesh-spawned task gets a free undo button.
//!
//! v1 (this module) covers create / list / delete / prune. The "rollback"
//! action emits a manual `mount_apfs` + `rsync` command — selective
//! in-place restoration of just-the-cwd is non-trivial because snapshots
//! are volume-scoped, and shipping a one-shot mount-and-rsync helper
//! requires sudo. Plan calls that out as a Sprint 2 follow-up.
//!
//! Gated `#[cfg(target_os = "macos")]`. Non-Mac callers get a friendly
//! error from the subcommand wrapper.

#![cfg(target_os = "macos")]

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use tokio::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotInfo {
    /// `YYYY-MM-DD-HHMMSS` — the part after `com.apple.TimeMachine.` and
    /// before `.local` in tmutil's listing.
    pub id: String,
    /// Unix milliseconds — derived from the id, or 0 if parse fails.
    pub created_at_ms: i64,
    /// Optional caller-supplied tag — phantom doesn't store this in the
    /// system snapshot itself, only in our in-memory log when relevant.
    pub label: Option<String>,
}

/// Take a fresh local snapshot. Returns the new snapshot's id.
pub async fn create(label: Option<&str>) -> Result<SnapshotInfo> {
    let out = Command::new("tmutil")
        .arg("localsnapshot")
        .output()
        .await?;
    if !out.status.success() {
        return Err(anyhow!(
            "tmutil localsnapshot failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Output looks like:
    //   NOTE: local snapshots are considered purgeable...
    //   Created local snapshot with date: 2026-04-28-101535
    let id = stdout
        .lines()
        .find_map(|l| {
            l.split_once("date: ")
                .map(|(_, rest)| rest.trim().to_string())
        })
        .ok_or_else(|| {
            anyhow!(
                "could not parse snapshot id from tmutil output:\n{}",
                stdout.trim()
            )
        })?;
    Ok(SnapshotInfo {
        id: id.clone(),
        created_at_ms: parse_id_to_ms(&id).unwrap_or_else(now_ms),
        label: label.map(String::from),
    })
}

/// List every local snapshot of the boot volume, newest first.
pub async fn list() -> Result<Vec<SnapshotInfo>> {
    let out = Command::new("tmutil")
        .args(["listlocalsnapshots", "/"])
        .output()
        .await?;
    if !out.status.success() {
        return Err(anyhow!(
            "tmutil listlocalsnapshots failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut snaps: Vec<SnapshotInfo> = stdout
        .lines()
        .filter_map(|line| {
            // com.apple.TimeMachine.2026-04-28-101535.local
            let mid = line.trim().strip_prefix("com.apple.TimeMachine.")?;
            let id = mid.strip_suffix(".local")?;
            Some(SnapshotInfo {
                id: id.to_string(),
                created_at_ms: parse_id_to_ms(id).unwrap_or(0),
                label: None,
            })
        })
        .collect();
    // Newest first.
    snaps.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms));
    Ok(snaps)
}

/// Delete a snapshot by its id (the `YYYY-MM-DD-HHMMSS` part).
pub async fn delete(id: &str) -> Result<()> {
    let out = Command::new("tmutil")
        .args(["deletelocalsnapshots", id])
        .output()
        .await?;
    if !out.status.success() {
        return Err(anyhow!(
            "tmutil deletelocalsnapshots {} failed: {}",
            id,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

/// Delete every snapshot older than `seconds` from now. Returns the count
/// of snapshots that were actually removed.
pub async fn prune_older_than(seconds: u64) -> Result<usize> {
    let now = now_ms();
    let cutoff = now - (seconds as i64 * 1000);
    let snaps = list().await?;
    let mut removed = 0usize;
    for s in snaps {
        if s.created_at_ms != 0 && s.created_at_ms < cutoff {
            if delete(&s.id).await.is_ok() {
                removed += 1;
            }
        }
    }
    Ok(removed)
}

/// Mount a snapshot read-only, rsync the requested path back, unmount.
/// Each step shells out to `sudo` separately so the user is prompted for
/// their password exactly once (per session, courtesy of sudo's
/// timestamp_timeout). Returns on first failure with a clear hint.
///
/// `target` must be an absolute path that lives under the snapshot's
/// volume. We compute the corresponding source path inside the mount.
///
/// `dry_run = true` prints the recipe instead of executing — same shape
/// as `manual_rollback_hint`, but checked against the real existence of
/// the snapshot id.
pub async fn apply(id: &str, target: &std::path::Path, dry_run: bool) -> Result<()> {
    use std::process::Command;

    if !target.is_absolute() {
        return Err(anyhow!("target path must be absolute: {}", target.display()));
    }

    // Verify the snapshot exists.
    let snaps = list().await?;
    if !snaps.iter().any(|s| s.id == id) {
        return Err(anyhow!(
            "snapshot {} not found — `phantom snapshot list` for ids", id
        ));
    }

    let mount_pt = std::path::PathBuf::from("/tmp")
        .join(format!("phantom-snapshot-{}", id));
    let snap_name = format!("com.apple.TimeMachine.{}.local", id);

    // The snapshot is mounted at `mount_pt`; the target's content lives at
    // `mount_pt + target` (snapshots include the absolute path).
    let target_rel = target
        .strip_prefix("/")
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|_| target.to_path_buf());
    let src_path = mount_pt.join(&target_rel);

    let mkdir = format!("sudo mkdir -p {}", mount_pt.display());
    let mount = format!(
        "sudo mount_apfs -o nobrowse,rdonly -s {} / {}",
        snap_name,
        mount_pt.display()
    );
    let rsync = format!(
        "sudo rsync -aHAX --delete --info=stats1 {}/ {}/",
        src_path.display(),
        target.display()
    );
    let umount = format!("sudo umount {}", mount_pt.display());
    let rmdir = format!("sudo rmdir {}", mount_pt.display());

    if dry_run {
        println!("# Dry run — these are the commands that would execute.");
        println!("# Pass --execute to actually run them.");
        println!("{}", mkdir);
        println!("{}", mount);
        println!("{}", rsync);
        println!("{}", umount);
        println!("{}", rmdir);
        return Ok(());
    }

    eprintln!(
        "⚠  About to restore {} from snapshot {} (rsync --delete will REMOVE files in the target that aren't in the snapshot).\n   Mount point: {}",
        target.display(), id, mount_pt.display(),
    );

    let steps: &[(&str, &str)] = &[
        ("mkdir mount point", &mkdir),
        ("mount snapshot",    &mount),
        ("rsync from snapshot", &rsync),
        ("unmount",            &umount),
        ("cleanup mount point", &rmdir),
    ];
    for (label, cmd) in steps {
        eprintln!("→ {}", cmd);
        let s = Command::new("sh").args(["-c", cmd]).status()?;
        if !s.success() {
            // Try to clean up if mount succeeded but a later step failed.
            if *label == "rsync from snapshot" {
                let _ = Command::new("sh").args(["-c", &umount]).status();
                let _ = Command::new("sh").args(["-c", &rmdir]).status();
            }
            return Err(anyhow!("{} failed", label));
        }
    }
    eprintln!("✓ Restore complete: {} now matches snapshot {}", target.display(), id);
    Ok(())
}

/// Print a copy-pasteable manual rollback procedure for the given snapshot
/// id. Used by `phantom snapshot rollback <id>`.
pub fn manual_rollback_hint(id: &str) -> String {
    format!(
        "\
# To inspect the snapshot:
sudo mkdir -p /tmp/phantom-snapshot-{id}
sudo mount_apfs -o nobrowse,rdonly \\
  -s com.apple.TimeMachine.{id} / /tmp/phantom-snapshot-{id}

# Restore a single file (replace <relpath>, no leading slash):
sudo cp /tmp/phantom-snapshot-{id}/<relpath> /<relpath>

# Restore everything under your current working dir:
sudo rsync -aHAX --delete \\
  /tmp/phantom-snapshot-{id}$(pwd)/ \\
  $(pwd)/

# When done:
sudo umount /tmp/phantom-snapshot-{id}
sudo rmdir /tmp/phantom-snapshot-{id}
"
    )
}

/// Convert a `YYYY-MM-DD-HHMMSS` snapshot id to unix milliseconds, honoring
/// the host's local timezone (tmutil records snapshot ids in local clock,
/// so naively treating them as UTC produces hours-off "X minutes ago"
/// labels). We shell out to BSD `date -j -f` instead of pulling in chrono
/// or time just for this — it's the macOS-native idiom and we are already
/// macOS-only here.
fn parse_id_to_ms(id: &str) -> Option<i64> {
    // Sanity check shape before paying for a fork+exec.
    let parts: Vec<&str> = id.split('-').collect();
    if parts.len() != 4 || parts[3].len() != 6 {
        return None;
    }
    let out = std::process::Command::new("date")
        .args(["-j", "-f", "%Y-%m-%d-%H%M%S", id, "+%s"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let secs: i64 = s.trim().parse().ok()?;
    Some(secs * 1000)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_id_round_trips_through_date() {
        // Reflexive: parsing back the formatted now() matches itself.
        let now_secs = now_ms() / 1000;
        let id_str = std::process::Command::new("date")
            .args(["-r", &now_secs.to_string(), "+%Y-%m-%d-%H%M%S"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string());
        if let Some(id) = id_str {
            let parsed = parse_id_to_ms(&id).unwrap();
            // Allow a 1-second drift (date truncates sub-second).
            assert!((parsed / 1000 - now_secs).abs() <= 1);
        }
    }

    #[test]
    fn parse_id_rejects_garbage() {
        assert!(parse_id_to_ms("not-a-snapshot").is_none());
        assert!(parse_id_to_ms("2026-04-28-12345").is_none()); // wrong time len
        assert!(parse_id_to_ms("2026-04").is_none());
    }

    #[test]
    fn rollback_hint_mentions_id() {
        let h = manual_rollback_hint("2026-04-28-101535");
        assert!(h.contains("2026-04-28-101535"));
        assert!(h.contains("mount_apfs"));
        assert!(h.contains("rsync"));
    }
}
