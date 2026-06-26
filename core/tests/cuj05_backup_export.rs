// CUJ-05 · backup subset — integration test for `phantom backup --to`.
//
// 對應 [`docs/cuj/05-export-and-uninstall.md`] 的 Happy path A (export +
// keep) — verify `phantom backup` produces a tar.gz that, when extracted,
// restores the entire ~/.phantom-mesh/ tree byte-identical.
//
// 命名規約: `cuj{NN}_{slug}_{scope}.rs`. CUJ-05 covers export + delete +
// reinstall; this file is the export+restore-roundtrip subset. The
// delete-all + broker-DELETE + identity-import subsets land in separate
// cuj05_delete_all.rs and cuj05_reinstall_import.rs.
//
// WHY THIS FILE EXISTS:
//   `phantom data export` (E004 Task 6, already shipped) emits events as
//   JSON/Markdown for portability, but does NOT include identity.key,
//   habits.sqlite chip_palette, coach reviews, or agents.toml. The new
//   `phantom backup` subcommand wraps the entire tree so the user can
//   actually leave the platform (GDPR Article 17) AND come back via
//   reinstall + tar -xzf. This test guards the roundtrip contract.
//
// VERIFIES (CUJ-05 Happy path A):
//   - tar.gz file is produced at the requested path
//   - extracting it yields .phantom-mesh/identity.key + events.sqlite +
//     events/* etc. with identical bytes (roundtrip-safe)
//   - destination size > 0 (not an empty archive)

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

/// Build the `phantom` CLI bin path. Tries `target/release/phantom` first
/// (matches what `cargo build --release` produces in CI), then `target/
/// debug/phantom` (matches `cargo test` defaults). Skips the test if
/// neither exists — running `cargo test --test ...` does NOT cargo build
/// the bin target, so a fresh tree without a prior `cargo build` would
/// otherwise fail spuriously.
fn phantom_bin() -> Option<PathBuf> {
    // Allow CI / dev to point at a specific binary explicitly.
    if let Ok(p) = std::env::var("PHANTOM_TEST_BIN") {
        return Some(PathBuf::from(p));
    }
    // Target-specific paths first — a stale generic `target/release/phantom`
    // from a previous `cargo build --release` (without --target) can shadow
    // the freshly-built target-triple binary and run an old binary that
    // doesn't have the feature under test. Prefer the explicit target-triple
    // paths, fall back to the generic ones.
    let candidates = [
        "target/aarch64-apple-darwin/release/phantom",
        "target/aarch64-apple-darwin/debug/phantom",
        "target/release/phantom",
        "target/debug/phantom",
    ];
    for rel in candidates {
        // Manifest dir is core/, so resolve relative to it.
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

#[test]
fn cuj05_backup_roundtrip_preserves_phantom_mesh_tree() {
    let bin = match phantom_bin() {
        Some(p) => p,
        None => {
            eprintln!(
                "SKIPPED: cuj05_backup_roundtrip_preserves_phantom_mesh_tree — no built \
                 phantom bin found (run `cargo build --release --bin phantom`)"
            );
            return;
        }
    };

    // ───────────────────────────────────────────────────────────────────────
    // Setup: isolated $HOME with a populated ~/.phantom-mesh/ tree.
    // ───────────────────────────────────────────────────────────────────────
    let home_dir = TempDir::new().expect("tempdir for HOME");
    let phantom_dir = home_dir.path().join(".phantom-mesh");
    fs::create_dir_all(&phantom_dir).expect("create phantom-mesh");

    // Plant the four file shapes a real install carries: identity raw bytes,
    // a sqlite-shaped blob, an event JSON, and a markdown review.
    fs::write(phantom_dir.join("identity.key"), b"\x00\x01\x02\x03identitybytes")
        .expect("write identity.key");
    fs::write(phantom_dir.join("events.sqlite"), b"SQLite format 3\x00<placeholder>")
        .expect("write events.sqlite");
    fs::create_dir_all(phantom_dir.join("events/sample-uuid-001")).expect("mkdir event");
    fs::write(
        phantom_dir.join("events/sample-uuid-001/meta.json"),
        b"{\"kind\":\"habit\",\"ts_ms\":1700000000000}",
    )
    .expect("write event meta");
    fs::create_dir_all(phantom_dir.join("reviews")).expect("mkdir reviews");
    fs::write(
        phantom_dir.join("reviews/2026-05-31.md"),
        "# Daily review\n\n- 水 250ml\n- 咖啡 1 杯\n".as_bytes(),
    )
    .expect("write review");

    // Snapshot all paths + their bytes so we can compare after roundtrip.
    fn collect_files(root: &std::path::Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut out = Vec::new();
        for entry in walkdir(root) {
            if entry.is_file() {
                let bytes = fs::read(&entry).unwrap_or_default();
                let rel = entry.strip_prefix(root).unwrap().to_path_buf();
                out.push((rel, bytes));
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }
    // Snapshot BEFORE phantom runs. `phantom backup` itself triggers
    // diag::init which creates `events.jsonl` + may bump file mtimes, so we
    // can only compare the EXPLICITLY PLANTED files for byte-identity. Any
    // extra files (diag log) the restored tree picks up are tolerated.
    let before = collect_files(&phantom_dir);
    assert!(
        before.len() >= 4,
        "test fixture should plant ≥ 4 files, got {}",
        before.len()
    );
    let before_paths: std::collections::HashSet<_> =
        before.iter().map(|(p, _)| p.clone()).collect();

    // ───────────────────────────────────────────────────────────────────────
    // Act: run `phantom backup --to <tar.gz>` with HOME pointing at our temp.
    // ───────────────────────────────────────────────────────────────────────
    let out_path = home_dir.path().join("backup.tar.gz");
    let status = Command::new(&bin)
        .env("HOME", home_dir.path())
        .args(["backup", "--to"])
        .arg(&out_path)
        .status()
        .expect("spawn phantom backup");
    assert!(
        status.success(),
        "phantom backup should exit 0; got {}",
        status
    );
    assert!(out_path.exists(), "backup.tar.gz should be created");
    let size = fs::metadata(&out_path).unwrap().len();
    assert!(size > 50, "backup.tar.gz should not be empty, got {} bytes", size);

    // ───────────────────────────────────────────────────────────────────────
    // Verify: extract the tarball into a fresh location and diff trees.
    // ───────────────────────────────────────────────────────────────────────
    let restore_dir = TempDir::new().expect("tempdir for restore");
    let extract_status = Command::new("tar")
        .arg("-xzf")
        .arg(&out_path)
        .arg("-C")
        .arg(restore_dir.path())
        .status()
        .expect("spawn tar -xzf");
    assert!(
        extract_status.success(),
        "tar -xzf should exit 0; got {}",
        extract_status
    );
    let restored_phantom = restore_dir.path().join(".phantom-mesh");
    assert!(
        restored_phantom.exists(),
        ".phantom-mesh should restore at extract root"
    );
    let after = collect_files(&restored_phantom);
    let after_map: std::collections::HashMap<_, _> = after.into_iter().collect();
    // Every PLANTED file must survive byte-identical. Extra files added by
    // phantom's diag::init (events.jsonl etc.) are ignored — they're a
    // legitimate side-effect of running the binary, not a backup defect.
    for (rel, bytes) in &before {
        assert!(
            before_paths.contains(rel), // sanity: rel came from before
            "internal: rel {:?} should be in before_paths",
            rel
        );
        let restored_bytes = after_map.get(rel).unwrap_or_else(|| {
            panic!("planted file {:?} missing from restored tarball", rel)
        });
        assert_eq!(
            bytes, restored_bytes,
            "planted file {:?} bytes differ after roundtrip",
            rel
        );
    }
}

#[test]
fn cuj05_backup_missing_to_flag_errors_with_helpful_msg() {
    let bin = match phantom_bin() {
        Some(p) => p,
        None => {
            eprintln!(
                "SKIPPED: cuj05_backup_missing_to_flag_errors_with_helpful_msg — no \
                 built phantom bin found"
            );
            return;
        }
    };

    let home_dir = TempDir::new().expect("tempdir");
    fs::create_dir_all(home_dir.path().join(".phantom-mesh")).expect("mkdir");

    let output = Command::new(&bin)
        .env("HOME", home_dir.path())
        .args(["backup"])
        .output()
        .expect("spawn phantom backup");
    assert!(
        !output.status.success(),
        "missing --to should fail; got {}",
        output.status
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--to") || stderr.contains("phantom backup"),
        "stderr should mention --to or backup usage; got: {}",
        stderr
    );
}

#[test]
#[ignore = "phantom always auto-creates ~/.phantom-mesh on startup via diag::init, \
            so the 'missing dir' branch is unreachable from a normal CLI invocation. \
            Kept as #[ignore] documentation of the original intent — re-enable if \
            phantom ever gains a `--no-auto-init` flag that suppresses diag::init."]
fn cuj05_backup_no_phantom_mesh_dir_errors_friendly() {
    let bin = match phantom_bin() {
        Some(p) => p,
        None => {
            eprintln!(
                "SKIPPED: cuj05_backup_no_phantom_mesh_dir_errors_friendly — no built \
                 phantom bin found"
            );
            return;
        }
    };

    let home_dir = TempDir::new().expect("tempdir");
    let out_path = home_dir.path().join("backup.tar.gz");
    let output = Command::new(&bin)
        .env("HOME", home_dir.path())
        .args(["backup", "--to"])
        .arg(&out_path)
        .output()
        .expect("spawn phantom backup");
    assert!(
        !output.status.success(),
        "backup without ~/.phantom-mesh/ should fail loudly, not silently produce empty tar"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("phantom-mesh"),
        "stderr should mention the missing dir; got: {}",
        stderr
    );
}

/// Tiny recursive directory walker; kept inline to avoid a `walkdir` dep
/// when the test only needs a few-dozen-file traversal.
fn walkdir(root: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if root.is_file() {
        out.push(root.to_path_buf());
        return out;
    }
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(walkdir(&path));
            } else if path.is_file() {
                out.push(path);
            }
        }
    }
    out
}
