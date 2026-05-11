use std::process::Command;

/// Verifies `phantom evolve goals next --file <path>` emits ONLY the goal text
/// on stdout (no eprintln decorations) so it composes cleanly with shell pipelines.
#[test]
fn test_evolve_goals_next_stdout_only() {
    let td = tempfile::tempdir().unwrap();
    let goals_file = td.path().join("EVOLVE-GOALS.md");
    std::fs::write(&goals_file, "\
# Evolve Goals

## Pending
- [ ] Test goal: verify stdout isolation

## Done
").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_phantom"))
        .args(["evolve", "goals", "next", "--file", &goals_file.to_string_lossy()])
        .output()
        .expect("phantom binary must be available via CARGO_BIN_EXE_phantom");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // stdout must contain only the goal text (no ANSI codes, no prefix).
    assert!(
        stdout.trim() == "Test goal: verify stdout isolation",
        "stdout should be bare goal text, got: {stdout:?}\nstderr was: {stderr:?}"
    );

    // stderr gets the decorations — verify it mentions the file path.
    assert!(
        stderr.contains("EVOLVE-GOALS.md"),
        "stderr should contain file decoration, got: {stderr:?}"
    );
}

/// Verifies `phantom evolve goals list --json` outputs valid JSON.
#[test]
fn test_evolve_goals_list_json() {
    let td = tempfile::tempdir().unwrap();
    let goals_file = td.path().join("EVOLVE-GOALS.md");
    std::fs::write(&goals_file, "\
# Evolve Goals

## Pending
- [ ] First pending goal
- [ ] Second pending goal

## Done
- [x] (2026-05-01 sha=feedb33) A completed goal
").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_phantom"))
        .args(["evolve", "goals", "list", "--json", "--file", &goals_file.to_string_lossy()])
        .output()
        .expect("phantom binary must be available via CARGO_BIN_EXE_phantom");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "command should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .expect("stdout must be valid JSON");

    assert!(parsed["pending"].is_array(), "pending must be an array");
    assert!(parsed["done"].is_array(), "done must be an array");
    assert_eq!(parsed["pending"].as_array().unwrap().len(), 2);
    assert_eq!(parsed["done"].as_array().unwrap().len(), 1);
    assert_eq!(parsed["pending"][0], "First pending goal");
}

/// Verifies `phantom evolve goals add "<text>"` appends a new unchecked goal
/// under `## Pending`, saves the file, and the next `goals next` call
/// returns exactly that text — i.e. load → add → save → load → next_pending
/// matches the new entry.
#[test]
fn test_evolve_goals_add_round_trip() {
    let td = tempfile::tempdir().unwrap();
    let goals_file = td.path().join("EVOLVE-GOALS.md");

    // Start with an empty file (no sections yet).
    std::fs::write(&goals_file, "").unwrap();

    let goal_text = "Add JSON output to goals list";

    // ── Step 1: add the goal via CLI ───────────────────────────────────
    let add_output = Command::new(env!("CARGO_BIN_EXE_phantom"))
        .args(["evolve", "goals", "add", goal_text, "--file", &goals_file.to_string_lossy()])
        .output()
        .expect("phantom binary must be available via CARGO_BIN_EXE_phantom");

    assert!(
        add_output.status.success(),
        "add command failed: {}",
        String::from_utf8_lossy(&add_output.stderr)
    );

    // ── Step 2: verify `goals next` returns the newly added goal ─────────
    let next_output = Command::new(env!("CARGO_BIN_EXE_phantom"))
        .args(["evolve", "goals", "next", "--file", &goals_file.to_string_lossy()])
        .output()
        .expect("phantom binary must be available via CARGO_BIN_EXE_phantom");

    let next_stdout = String::from_utf8_lossy(&next_output.stdout).trim().to_string();
    assert_eq!(
        next_stdout, goal_text,
        "`goals next` stdout should be the newly added goal text; got: {next_stdout:?}"
    );

    // ── Step 3: verify file contents on disk ────────────────────────────
    let file_text = std::fs::read_to_string(&goals_file).unwrap();
    assert!(
        file_text.contains("- [ ] Add JSON output to goals list"),
        "saved file should contain the new unchecked goal, got:\n{file_text}"
    );
    assert!(
        file_text.contains("## Pending"),
        "saved file should contain the ## Pending section header"
    );
}
