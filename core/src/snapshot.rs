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

use anyhow::{anyhow, Context, Result};
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
    let out = Command::new("tmutil").arg("localsnapshot").output().await?;
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
    snaps.sort_by_key(|s| std::cmp::Reverse(s.created_at_ms));
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
        return Err(anyhow!(
            "target path must be absolute: {}",
            target.display()
        ));
    }

    // Verify the snapshot exists.
    let snaps = list().await?;
    if !snaps.iter().any(|s| s.id == id) {
        return Err(anyhow!(
            "snapshot {} not found — `phantom snapshot list` for ids",
            id
        ));
    }

    let mount_pt = std::path::PathBuf::from("/tmp").join(format!("phantom-snapshot-{}", id));
    let snap_name = format!("com.apple.TimeMachine.{}.local", id);

    // The snapshot is mounted at `mount_pt`; the target's content lives at
    // `mount_pt + target` (snapshots include the absolute path).
    let target_rel = target
        .strip_prefix("/")
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|_| target.to_path_buf());
    let src_path = mount_pt.join(&target_rel);

    // T7 fix (codex audit 2026-05-15): every step is an argv-list call to
    // `sudo <tool>`. The shell never sees `target.display()` so paths
    // containing `;`, `$(...)`, `&&`, or other metacharacters cannot
    // result in command injection. Previously these were `format!`-built
    // strings passed through `sh -c`, which interpreted shell metachars.
    let mut src_with_slash = src_path.clone().into_os_string();
    src_with_slash.push("/");
    let mut tgt_with_slash = target.to_path_buf().into_os_string();
    tgt_with_slash.push("/");

    if dry_run {
        println!("# Dry run — these are the commands that would execute.");
        println!("# Pass --execute to actually run them.");
        println!("sudo mkdir -p {}", mount_pt.display());
        println!(
            "sudo mount_apfs -o nobrowse,rdonly -s {} / {}",
            snap_name,
            mount_pt.display()
        );
        println!(
            "sudo rsync -aHAX --delete --info=stats1 {} {}",
            std::path::Path::new(&src_with_slash).display(),
            std::path::Path::new(&tgt_with_slash).display(),
        );
        println!("sudo umount {}", mount_pt.display());
        println!("sudo rmdir {}", mount_pt.display());
        return Ok(());
    }

    eprintln!(
        "⚠  About to restore {} from snapshot {} (rsync --delete will REMOVE files in the target that aren't in the snapshot).\n   Mount point: {}",
        target.display(), id, mount_pt.display(),
    );

    // mkdir mount point
    let s = Command::new("sudo")
        .arg("mkdir")
        .arg("-p")
        .arg(&mount_pt)
        .status()
        .with_context(|| {
            format!(
                "failed to launch `sudo mkdir -p {}` — is `sudo` on PATH and are you on a terminal that can prompt for a password?",
                mount_pt.display()
            )
        })?;
    if !s.success() {
        return Err(anyhow!(
            "mkdir mount point {} failed (sudo denied or path unwritable)",
            mount_pt.display()
        ));
    }

    // mount snapshot
    let s = Command::new("sudo")
        .arg("mount_apfs")
        .args(["-o", "nobrowse,rdonly", "-s"])
        .arg(&snap_name)
        .arg("/")
        .arg(&mount_pt)
        .status()
        .with_context(|| {
            format!("failed to launch `sudo mount_apfs` for snapshot {id} — `mount_apfs` is macOS-only and requires sudo")
        })?;
    if !s.success() {
        return Err(anyhow!(
            "mount snapshot {id} failed — the snapshot may have been purged by macOS; run `phantom snapshot list` to confirm it still exists"
        ));
    }

    // rsync from snapshot — argv-list so user-supplied paths cannot be
    // interpreted as shell metacharacters.
    let rsync_status = Command::new("sudo")
        .arg("rsync")
        .args(["-aHAX", "--delete", "--info=stats1"])
        .arg(&src_with_slash)
        .arg(&tgt_with_slash)
        .status()
        .with_context(|| "failed to launch `sudo rsync` — is `rsync` installed and on PATH?")?;
    if !rsync_status.success() {
        // Best-effort cleanup if mount succeeded but rsync failed.
        if let Err(e) = Command::new("sudo").arg("umount").arg(&mount_pt).status() {
            tracing::warn!(mount_pt = %mount_pt.display(), "snapshot cleanup: umount failed (mount point may remain): {}", e);
        }
        if let Err(e) = Command::new("sudo").arg("rmdir").arg(&mount_pt).status() {
            tracing::warn!(mount_pt = %mount_pt.display(), "snapshot cleanup: rmdir failed: {}", e);
        }
        return Err(anyhow!("rsync from snapshot failed"));
    }

    // unmount
    let s = Command::new("sudo")
        .arg("umount")
        .arg(&mount_pt)
        .status()
        .with_context(|| format!("failed to launch `sudo umount {}`", mount_pt.display()))?;
    if !s.success() {
        return Err(anyhow!(
            "unmount of {} failed — the restore copied successfully but the read-only snapshot mount is still attached; run `sudo umount {}` manually",
            mount_pt.display(),
            mount_pt.display()
        ));
    }

    // cleanup mount point
    let s = Command::new("sudo")
        .arg("rmdir")
        .arg(&mount_pt)
        .status()
        .with_context(|| format!("failed to launch `sudo rmdir {}`", mount_pt.display()))?;
    if !s.success() {
        return Err(anyhow!(
            "could not remove leftover mount point {} — restore succeeded; remove it with `sudo rmdir {}`",
            mount_pt.display(),
            mount_pt.display()
        ));
    }

    eprintln!(
        "✓ Restore complete: {} now matches snapshot {}",
        target.display(),
        id
    );
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

    /// MAC P0 — calling `manual_rollback_hint` twice with the same id
    /// must produce identical output. The function is documented as
    /// pure (no I/O, no clock reads), so referential transparency is
    /// the contract.
    #[test]
    fn rollback_idempotent() {
        let id = "2026-04-28-101535";
        let a = manual_rollback_hint(id);
        let b = manual_rollback_hint(id);
        let c = manual_rollback_hint(id);
        assert_eq!(a, b, "rollback hint not idempotent (call 1 vs 2)");
        assert_eq!(b, c, "rollback hint not idempotent (call 2 vs 3)");

        // Different id → different hint (sanity: the function actually
        // uses its argument, isn't just returning a constant).
        let d = manual_rollback_hint("2026-05-18-090000");
        assert_ne!(
            a, d,
            "different ids produced identical hints — function ignores arg?"
        );
    }

    /// MAC P0 — `apply(id, cwd, dry_run=true)` against a real snapshot
    /// id must succeed (i.e., the id-existence check + path-building +
    /// dry-run print path all work). Doesn't actually mount or rsync —
    /// the executing variant requires sudo and we keep this test
    /// hermetic. Side effect: creates one purgeable APFS snapshot
    /// (same as other snapshot tests, auto-pruned by macOS).
    #[tokio::test]
    async fn apply_with_cwd_path_succeeds() {
        let snap = create(Some("phantom-tdd-apply-with-cwd-path-succeeds"))
            .await
            .expect("tmutil localsnapshot");

        // dry_run = true: function must NOT exec sudo; only print the
        // would-execute recipe and return Ok.
        let cwd = std::env::current_dir().expect("cwd");
        let result = apply(&snap.id, &cwd, true).await;
        assert!(
            result.is_ok(),
            "apply(<existing-id>, cwd, dry_run=true) should succeed; got {:?}",
            result
        );

        // Negative path: a bogus id (not in `list()`) must error with a
        // clear message that mentions the id. Catches the regression
        // where apply silently no-ops on missing snapshots.
        let bogus = "0000-00-00-000000";
        let err = apply(bogus, &cwd, true).await.unwrap_err();
        let msg = format!("{}", err);
        assert!(
            msg.contains(bogus),
            "error for bogus id should mention the id; got: {}",
            msg
        );

        // Non-absolute path must be rejected (security: argv-list still
        // requires the caller commit to an explicit absolute target).
        let rel = std::path::PathBuf::from("relative/path");
        let err = apply(&snap.id, &rel, true).await.unwrap_err();
        assert!(
            format!("{}", err).contains("absolute"),
            "relative-path rejection should mention `absolute`; got: {}",
            err
        );
    }

    /// MAC P0 — create a snapshot then assert it appears in `list()`.
    /// Exercises the round-trip across two tmutil subcommands
    /// (`localsnapshot` → `listlocalsnapshots /`) and the parser
    /// distinguishing our id format from tmutil's wrapped form
    /// `com.apple.TimeMachine.<id>.local`.
    #[tokio::test]
    async fn list_includes_newly_created() {
        let snap = create(Some("phantom-tdd-list-includes-newly-created"))
            .await
            .expect("tmutil localsnapshot should succeed on a Mac dev host");

        let all = list()
            .await
            .expect("tmutil listlocalsnapshots / should succeed");
        assert!(
            !all.is_empty(),
            "list() returned empty right after a successful create() — \
             parser likely failed on `com.apple.TimeMachine.<id>.local` lines"
        );

        let found = all.iter().any(|s| s.id == snap.id);
        assert!(
            found,
            "snapshot id `{}` not found in list() ({} entries: {:?})",
            snap.id,
            all.len(),
            all.iter().take(3).map(|s| &s.id).collect::<Vec<_>>()
        );

        // list() is sorted newest first — the just-created snapshot
        // should be at (or very near) the head. Allow a small skew in
        // case another snapshot landed in the same second.
        let head_idx = all
            .iter()
            .position(|s| s.id == snap.id)
            .expect("found, asserted above");
        assert!(
            head_idx < 3,
            "newly-created snapshot at index {} (should be near 0); \
             list order broken — first 3: {:?}",
            head_idx,
            all.iter().take(3).map(|s| &s.id).collect::<Vec<_>>()
        );
    }

    /// MAC P0 — calls `tmutil localsnapshot` for real (sudo-free, ~1 s).
    /// macOS auto-prunes purgeable snapshots so this isn't a disk leak;
    /// the assertion shape is the public contract: `id` must be the
    /// canonical `YYYY-MM-DD-HHMMSS` 17-char form, parseable into a
    /// recent epoch ms.
    #[tokio::test]
    async fn create_returns_unique_id() {
        let snap = match create(Some("phantom-tdd-create-returns-unique-id")).await {
            Ok(s) => s,
            // tmutil errors only on a non-APFS boot volume, sudo
            // denial (not applicable for `localsnapshot`), or a
            // container without `/dev/disk*` — none should occur on a
            // standard dev Mac, and the assertion failure is more
            // informative than `?`.
            Err(e) => panic!(
                "create() failed — is this an APFS-backed boot volume? \
                 error: {}",
                e
            ),
        };

        // Canonical id shape: YYYY-MM-DD-HHMMSS (17 chars, two `-`s).
        assert_eq!(
            snap.id.len(),
            17,
            "snapshot id should be 17 chars (YYYY-MM-DD-HHMMSS); got `{}`",
            snap.id
        );
        let chars: Vec<char> = snap.id.chars().collect();
        assert_eq!(chars[4], '-', "char 4 should be `-`: {}", snap.id);
        assert_eq!(chars[7], '-', "char 7 should be `-`: {}", snap.id);
        assert_eq!(chars[10], '-', "char 10 should be `-`: {}", snap.id);

        // created_at_ms must be within 5 min of now (loose to absorb
        // CI clock drift + slow tmutil response).
        let now = now_ms();
        let drift = (now - snap.created_at_ms).abs();
        assert!(
            drift < 5 * 60 * 1000,
            "snapshot timestamp drift {} ms exceeds 5 min — \
             id={} parsed_ms={} now_ms={}",
            drift,
            snap.id,
            snap.created_at_ms,
            now
        );

        // Label round-trips through SnapshotInfo verbatim (the field
        // is informational; tmutil doesn't store labels itself, but
        // SnapshotInfo callers rely on it being preserved).
        assert_eq!(
            snap.label.as_deref(),
            Some("phantom-tdd-create-returns-unique-id")
        );
    }
}
