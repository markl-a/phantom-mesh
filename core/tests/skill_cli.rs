//! apex ② owned-memory `phantom skill` CLI — real-path integration test.
//!
//! Spawns the actually-built binary (`env!("CARGO_BIN_EXE_phantom")`) against a
//! temp `PHANTOM_DB_PATH` skill store seeded in-process via the public lib
//! `store_skill`, then drives the UNGATED inspect/drive arms a user hits on a
//! default build — no mocks:
//!
//!   * `skill list`   enumerates the seeded skill (its id is in stdout),
//!   * `skill stats`  reports the bank total,
//!   * `skill recall "<q>"` emits the FTS5 `<recalled_skills>` hot-path block,
//!   * `skill schedule` prints the OS scheduler unit (+ `--at` overrides time).
//!
//! Gated `#![cfg(feature = "experimental-memory")]` so
//! `cargo test --no-default-features` compiles this file empty and skips it
//! cleanly.

#![cfg(feature = "experimental-memory")]

use std::process::Command;

use phantom_mesh::skill_wire::{store_skill, Skill};

// `phantom_mesh::env_lock` is `#[cfg(test)]`-gated (unreachable from an
// integration test, which links the non-test lib build). Serialize on a
// file-local mutex instead — PHANTOM_DB_PATH is a process-global and the seed
// step mutates this process's env, so a file-local lock keeps it hermetic.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn phantom_bin() -> &'static str {
    env!("CARGO_BIN_EXE_phantom")
}

/// Seed one skill into a fresh temp DB via the in-process public store path,
/// returning the temp DB path string. Sets PHANTOM_DB_PATH in THIS process so
/// `store_skill`'s `resolve_db_path()` writes to the temp file; the caller is
/// responsible for restoring the env (it holds ENV_LOCK).
fn seed_skill(db_path: &std::path::Path, skill: Skill) {
    std::env::set_var("PHANTOM_DB_PATH", db_path);
    store_skill(&skill).expect("seed store_skill");
}

/// Run `phantom skill <args...>` with PHANTOM_DB_PATH + PHANTOM_OWNED_MEMORY set
/// so the child reads the seeded store and the loop is enabled.
fn run_skill(db_path: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(phantom_bin())
        .arg("skill")
        .args(args)
        .env("PHANTOM_DB_PATH", db_path)
        .env("PHANTOM_OWNED_MEMORY", "1")
        .output()
        .expect("phantom must spawn")
}

fn deploy_skill() -> Skill {
    Skill {
        id: "sk-cli-deploy".into(),
        name: "deploy the staging cluster".into(),
        trigger_pattern: "deploy staging".into(),
        steps: vec!["ssh staging".into(), "run deploy.sh".into()],
        examples: vec![],
        version: 1,
        quality_score: 0.9,
        last_applied_at: 0,
        source_event_count: 3,
    }
}

#[test]
fn skill_list_stdout_contains_seeded_id() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let saved = std::env::var_os("PHANTOM_DB_PATH");
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("skills.db");

    seed_skill(&db_path, deploy_skill());
    let out = run_skill(&db_path, &["list"]);

    match &saved {
        Some(v) => std::env::set_var("PHANTOM_DB_PATH", v),
        None => std::env::remove_var("PHANTOM_DB_PATH"),
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "skill list exit: {:?}\n{stdout}", out.status);
    assert!(
        stdout.contains("sk-cli-deploy"),
        "skill list must show the seeded id: {stdout:?}"
    );
}

#[test]
fn skill_stats_stdout_shows_total() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let saved = std::env::var_os("PHANTOM_DB_PATH");
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("skills.db");

    seed_skill(&db_path, deploy_skill());
    let out = run_skill(&db_path, &["stats"]);

    match &saved {
        Some(v) => std::env::set_var("PHANTOM_DB_PATH", v),
        None => std::env::remove_var("PHANTOM_DB_PATH"),
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "skill stats exit: {:?}\n{stdout}", out.status);
    assert!(
        stdout.contains("total=1"),
        "skill stats must report the bank total: {stdout:?}"
    );
}

#[test]
fn skill_recall_stdout_contains_recall_block() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let saved = std::env::var_os("PHANTOM_DB_PATH");
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("skills.db");

    // Seed a "deploy staging" skill so the FTS5 query has a match.
    seed_skill(&db_path, deploy_skill());
    let out = run_skill(&db_path, &["recall", "deploy the staging cluster"]);

    match &saved {
        Some(v) => std::env::set_var("PHANTOM_DB_PATH", v),
        None => std::env::remove_var("PHANTOM_DB_PATH"),
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "skill recall exit: {:?}\n{stdout}", out.status);
    assert!(
        stdout.contains("<recalled_skills>"),
        "skill recall must emit the FTS5 hot-path block: {stdout:?}"
    );
}

#[test]
fn skill_schedule_prints_unit_and_respects_at_override() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let saved = std::env::var_os("PHANTOM_DB_PATH");
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("skills.db");

    // `schedule` does not need a seeded DB; it only renders the unit text.
    let out = run_skill(&db_path, &["schedule"]);
    let out_at = run_skill(&db_path, &["schedule", "--at", "04:30"]);

    match &saved {
        Some(v) => std::env::set_var("PHANTOM_DB_PATH", v),
        None => std::env::remove_var("PHANTOM_DB_PATH"),
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "skill schedule exit: {:?}\n{stdout}", out.status);
    assert!(stdout.contains("skill"), "unit must reference the skill command: {stdout:?}");
    assert!(stdout.contains("learn"), "unit must reference the learn subcommand: {stdout:?}");
    // The label stem `skill-learn` appears in every platform render: the launchd
    // plist + schtasks /TN carry the full `ai.phantommesh.skill-learn` label, and
    // the systemd render carries the `phantom-skill-learn.{service,timer}` unit
    // stem — `skill-learn` is the common substring across all three.
    assert!(
        stdout.contains("skill-learn"),
        "unit must carry the skill-learn label/stem: {stdout:?}"
    );
    assert!(
        stdout.contains("does NOT auto-install"),
        "unit must carry the no-auto-install disclaimer: {stdout:?}"
    );

    let stdout_at = String::from_utf8_lossy(&out_at.stdout);
    assert!(out_at.status.success(), "skill schedule --at exit: {:?}\n{stdout_at}", out_at.status);
    assert!(
        stdout_at.contains("04:30"),
        "--at 04:30 must override the default time: {stdout_at:?}"
    );
}
