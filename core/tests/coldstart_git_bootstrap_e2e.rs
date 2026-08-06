//! E2E proof for `spectyn memory bootstrap` (MAIN-REPO P1.4 cold-start).
//!
//! This is the anti-fake-green proof the feature actually ships: it drives the
//! REAL `spectyn` CLI binary (`env!("CARGO_BIN_EXE_spectyn")` — Cargo builds it
//! before running this test) end to end:
//!
//!   temp git repo (known commits) → `spectyn memory bootstrap --repo <tmprepo>`
//!     → `spectyn recall <kw>` returns the matching commit
//!     → re-run bootstrap is idempotent (no duplication)
//!
//! It is NOT a library-only unit test: every step goes through the compiled
//! `spectyn` binary over a subprocess, with `SPECTYN_HOME` pointed at a throwaway
//! data root so the REAL `~/.spectyn-mesh` store is never touched. The store the
//! bootstrap writes (`<SPECTYN_HOME>/events.sqlite` + `<SPECTYN_HOME>/events/`) is
//! the SAME store `spectyn recall` reads — that round-trip is the whole point.

use std::path::Path;
use std::process::Command;

/// The compiled `spectyn` binary under test. Cargo sets `CARGO_BIN_EXE_spectyn`
/// for integration tests and (re)builds the bin first, so this is always the
/// just-built default-build binary — the e2e can never silently skip.
fn spectyn_bin() -> &'static str {
    env!("CARGO_BIN_EXE_spectyn")
}

/// Run a `git` subcommand inside `repo`, asserting success.
fn git(repo: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn git {args:?}: {e}"));
    assert!(
        out.status.success(),
        "git {:?} failed: status={:?}\nstdout={}\nstderr={}",
        args,
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

/// Create a fresh git repo at `repo` with three known commits (newest last):
///   "alpha widget refactor", "beta cache fix", "gamma docs update".
fn make_repo_with_commits(repo: &Path) {
    std::fs::create_dir_all(repo).expect("mkdir repo");
    git(repo, &["init", "-q"]);
    // Local identity so commits succeed regardless of the machine's global git
    // config; disable signing so a configured gpg key can't block the commit.
    git(repo, &["config", "user.email", "coldstart-e2e@example.com"]);
    git(repo, &["config", "user.name", "Coldstart E2E"]);
    git(repo, &["config", "commit.gpgsign", "false"]);

    for (i, subject) in [
        "alpha widget refactor",
        "beta cache fix",
        "gamma docs update",
    ]
    .iter()
    .enumerate()
    {
        let f = repo.join(format!("file{i}.txt"));
        std::fs::write(&f, format!("change {i}\n")).expect("write repo file");
        git(repo, &["add", "."]);
        git(repo, &["commit", "-q", "-m", subject]);
    }
}

/// Run `spectyn <args>` with `SPECTYN_HOME` pointed at `data_root`. Returns
/// (success, stdout, stderr).
fn spectyn(data_root: &Path, args: &[&str]) -> (bool, String, String) {
    let out = Command::new(spectyn_bin())
        .args(args)
        .env("SPECTYN_HOME", data_root)
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn spectyn {args:?}: {e}"));
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

/// Parse `spectyn recall <q> --json` stdout into the hit array. The `--json`
/// surface is a JSON array of EventHit objects (`summary`, `kind`, ...).
fn recall_json(data_root: &Path, query: &str) -> Vec<serde_json::Value> {
    // `--mode keyword` forces the pure lexical (FTS5 + file-store) legs so the
    // result is deterministic and never depends on a local Ollama embedder.
    let (ok, stdout, stderr) =
        spectyn(data_root, &["recall", query, "--mode", "keyword", "--json"]);
    assert!(ok, "spectyn recall {query} failed: stderr=\n{stderr}");
    serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("recall {query} stdout not JSON ({e}): {stdout}"))
}

#[test]
fn bootstrap_git_commits_then_recall_returns_them_and_is_idempotent() {
    let bin = spectyn_bin();
    assert!(
        Path::new(bin).exists(),
        "spectyn binary missing at {bin} — `cargo test` should have built it",
    );

    let tmp = tempfile::tempdir().expect("tempdir");
    let repo = tmp.path().join("repo");
    // SPECTYN_HOME is the data root used VERBATIM (cli_config::spectyn_data_dir),
    // so this isolates BOTH events.sqlite and events/ — the real ~/.spectyn-mesh
    // is never written.
    let data_root = tmp.path().join("pmhome");
    std::fs::create_dir_all(&data_root).expect("mkdir data root");

    make_repo_with_commits(&repo);
    let repo_str = repo.to_str().unwrap();

    // ── Pre-condition: a fresh data root recalls NOTHING for "widget" ─────────
    // (Proves the hit we see after bootstrap genuinely came from the ingest, not
    // from pre-seeded content.)
    let before = recall_json(&data_root, "widget");
    assert!(
        before.is_empty(),
        "expected empty recall before bootstrap, got: {before:?}"
    );

    // ── Run 1: real CLI bootstrap of the temp repo's commits ─────────────────
    let (ok, stdout, stderr) =
        spectyn(&data_root, &["memory", "bootstrap", "--repo", repo_str]);
    assert!(ok, "bootstrap run 1 failed: stderr=\n{stderr}");
    assert!(
        stdout.contains("3 commit(s) ingested") && stdout.contains("0 already present"),
        "bootstrap run 1 should ingest all 3 commits, got: {stdout}"
    );

    // ── Real CLI recall returns the matching commit ──────────────────────────
    let widget = recall_json(&data_root, "widget");
    assert_eq!(
        widget.len(),
        1,
        "recall 'widget' should return exactly the alpha commit, got: {widget:?}"
    );
    let summary = widget[0]["summary"].as_str().unwrap_or("");
    assert!(
        summary.contains("alpha widget refactor"),
        "recall 'widget' summary should be the alpha commit, got: {summary:?}"
    );
    assert_eq!(
        widget[0]["kind"].as_str(),
        Some("text"),
        "git-commit events are recorded as kind=text"
    );

    // A different keyword surfaces a DIFFERENT commit — proves several distinct
    // commits were ingested (not one blob), and that retrieval is real keyword
    // search, not "return everything".
    let cache = recall_json(&data_root, "cache");
    assert_eq!(cache.len(), 1, "recall 'cache' should return the beta commit");
    assert!(
        cache[0]["summary"]
            .as_str()
            .unwrap_or("")
            .contains("beta cache fix"),
        "recall 'cache' should be the beta commit, got: {:?}",
        cache[0]["summary"]
    );

    // ── Run 2: idempotent — re-running ingests nothing new, no duplicates ────
    let (ok2, stdout2, stderr2) =
        spectyn(&data_root, &["memory", "bootstrap", "--repo", repo_str]);
    assert!(ok2, "bootstrap run 2 failed: stderr=\n{stderr2}");
    assert!(
        stdout2.contains("0 commit(s) ingested") && stdout2.contains("3 already present"),
        "bootstrap run 2 should be a no-op (all already present), got: {stdout2}"
    );

    // Recall is unchanged: still exactly ONE 'widget' hit (no duplicate row).
    let widget_again = recall_json(&data_root, "widget");
    assert_eq!(
        widget_again.len(),
        1,
        "after re-bootstrap, recall 'widget' must still be exactly 1 (no dup), got: {widget_again:?}"
    );
    assert_eq!(
        widget_again[0]["event_id"], widget[0]["event_id"],
        "same commit must keep the same event_id across re-runs (hash-keyed idempotency)"
    );
}
