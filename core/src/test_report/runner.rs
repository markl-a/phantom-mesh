//! Check runners: spawn `cargo test` / scripts, parse output → [`TestRunResult`].
//!
//! Pure parsing (`parse_libtest_output`) is split from spawning so the unit tests
//! are hermetic (fed recorded fixtures); real spawning is exercised only by the
//! `#[ignore]` smoke test and the live CLI. No network here (constraint 5).

use std::path::Path;

use chrono::{DateTime, Utc};

use super::{
    resolve_check, Check, CheckKind, CheckOutcome, CheckResolution, CheckRun, GateId,
    TestRunResult, TestStatus,
};

/// Dispatch a single check by kind → its [`CheckRun`] (resolve, then run what can run).
///
/// - `Manual` ⇒ `Manual` (never auto-run, never auto-green).
/// - `CargoTest` ⇒ spawn `cargo test --test <target>`.
/// - `Script`/`DocLint` ⇒ spawn the script (interpreter by extension).
/// - `Scenario` ⇒ resolve catalog membership; a real (S1..S40) scenario has no
///   auto harness in P2-2 (the `core/tests/scenarios/S{NN}.rs` stubs are Phase-E),
///   so it is honestly `Unknown` — never `Green`. An out-of-range id ⇒ `Failed`.
pub fn run_check(check: &Check, gate_id: GateId, repo_root: &Path) -> CheckRun {
    match check.kind {
        CheckKind::Manual => CheckRun { outcome: CheckOutcome::Manual, results: vec![] },
        CheckKind::CargoTest => match resolve_check(check, repo_root) {
            CheckResolution::Resolved => run_cargo_test(check, gate_id, repo_root),
            CheckResolution::Unresolved(reason) => CheckRun {
                outcome: CheckOutcome::Failed(unresolved_blocking(check, reason)),
                results: vec![],
            },
            CheckResolution::Manual => CheckRun { outcome: CheckOutcome::Manual, results: vec![] },
        },
        CheckKind::Script | CheckKind::DocLint => match resolve_check(check, repo_root) {
            CheckResolution::Resolved => run_script(check, gate_id, repo_root),
            CheckResolution::Unresolved(reason) => CheckRun {
                outcome: CheckOutcome::Failed(unresolved_blocking(check, reason)),
                results: vec![],
            },
            CheckResolution::Manual => CheckRun { outcome: CheckOutcome::Manual, results: vec![] },
        },
        CheckKind::Scenario => match resolve_check(check, repo_root) {
            // Catalog scenario with no auto harness yet ⇒ Unknown (un-evidenced).
            CheckResolution::Resolved => CheckRun { outcome: CheckOutcome::Unknown, results: vec![] },
            CheckResolution::Unresolved(reason) => CheckRun {
                outcome: CheckOutcome::Failed(unresolved_blocking(check, reason)),
                results: vec![],
            },
            CheckResolution::Manual => CheckRun { outcome: CheckOutcome::Manual, results: vec![] },
        },
    }
}

fn unresolved_blocking(check: &Check, reason: String) -> Vec<String> {
    if check.blocking_test_ids.is_empty() {
        vec![format!("unresolved-check: {reason}")]
    } else {
        check.blocking_test_ids.clone()
    }
}

/// Static (no-run) outcome from resolution alone — used by `--no-run` reporting.
/// Green is NEVER produced here (nothing ran): Resolved ⇒ `Unknown`, Manual ⇒
/// `Manual`, Unresolved ⇒ `Failed`.
pub fn resolve_only(check: &Check, repo_root: &Path) -> CheckRun {
    match resolve_check(check, repo_root) {
        CheckResolution::Resolved => CheckRun { outcome: CheckOutcome::Unknown, results: vec![] },
        CheckResolution::Manual => CheckRun { outcome: CheckOutcome::Manual, results: vec![] },
        CheckResolution::Unresolved(reason) => CheckRun {
            outcome: CheckOutcome::Failed(unresolved_blocking(check, reason)),
            results: vec![],
        },
    }
}

/// One parsed line of stable `cargo test` human output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedTest {
    pub name: String,
    pub status: ParsedStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParsedStatus {
    Ok,
    Failed,
    Ignored,
}

/// Parse stable-toolchain `cargo test` human output into per-test results.
///
/// Targets the STABLE per-test lines `test <name> ... ok|FAILED|ignored` (the
/// nightly `--format json` path is deliberately avoided — the repo builds on
/// stable; the recorded fixtures lock this format so a toolchain change is caught
/// by a failing parser test, not a silent miswire). The `test result:` summary
/// line is excluded.
pub fn parse_libtest_output(stdout: &str) -> Vec<ParsedTest> {
    let mut out = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("test ") else { continue };
        let Some((name, status_part)) = rest.split_once(" ... ") else { continue };
        if name == "result:" {
            continue; // the "test result: ok. N passed; ..." summary line
        }
        let status = status_part.trim();
        let parsed = if status == "ok" || status.starts_with("ok ") {
            ParsedStatus::Ok
        } else if status.starts_with("FAILED") {
            ParsedStatus::Failed
        } else if status.starts_with("ignored") {
            ParsedStatus::Ignored
        } else {
            // bench lines ("... bench: ...") and anything else → not a pass/fail test.
            continue;
        };
        out.push(ParsedTest { name: name.to_string(), status: parsed });
    }
    out
}

/// Pure classification of a cargo-test run (split from spawning for hermetic tests).
///
/// Honesty: zero parsed tests ⇒ `Failed(target-produced-no-tests)` — this catches
/// the `cargo test <substring>` "matched nothing, exit 0" fake-green. Any failed
/// test, or a non-zero process exit, ⇒ `Failed`; otherwise `Passed`.
pub fn classify_cargo_run(
    parsed: &[ParsedTest],
    exit_success: bool,
    check: &Check,
    gate_id: GateId,
    now: DateTime<Utc>,
) -> CheckRun {
    let results: Vec<TestRunResult> = parsed
        .iter()
        .map(|p| TestRunResult {
            test_id: p.name.clone(),
            spec_id: check.spec.clone(),
            gate_id,
            os: None,
            status: match p.status {
                ParsedStatus::Ok => TestStatus::Pass,
                ParsedStatus::Failed => TestStatus::Fail,
                ParsedStatus::Ignored => TestStatus::Skip,
            },
            duration_ms: 0,
            attempts: 1,
            reproducer_cmd: Some(format!("cargo test --test {} -- --exact {}", check.target, p.name)),
            failure_log: None,
            timestamp: now,
        })
        .collect();

    let blocking = |fallback: Vec<String>| -> Vec<String> {
        if check.blocking_test_ids.is_empty() {
            fallback
        } else {
            check.blocking_test_ids.clone()
        }
    };

    if parsed.is_empty() {
        // Either the test file matched no tests, or cargo never produced output.
        return CheckRun {
            outcome: CheckOutcome::Failed(blocking(vec![format!(
                "{}: target-produced-no-tests",
                check.target
            )])),
            results,
        };
    }

    let failed: Vec<String> = parsed
        .iter()
        .filter(|p| p.status == ParsedStatus::Failed)
        .map(|p| p.name.clone())
        .collect();

    if !failed.is_empty() || !exit_success {
        let fallback = if failed.is_empty() {
            vec![format!("{}: cargo exited non-zero", check.target)]
        } else {
            failed
        };
        CheckRun { outcome: CheckOutcome::Failed(blocking(fallback)), results }
    } else {
        CheckRun { outcome: CheckOutcome::Passed, results }
    }
}

/// Spawn `cargo test --test <target>` (stable), parse + classify.
///
/// Honors an inherited `CARGO_TARGET_DIR`. Runs against `core/Cargo.toml`. Never
/// hits the network (constraint 5). Only exercised by the live CLI + the
/// `#[ignore]` spawn smoke; the unit tests drive [`classify_cargo_run`] on fixtures.
pub fn run_cargo_test(check: &Check, gate_id: GateId, repo_root: &Path) -> CheckRun {
    let now = Utc::now();
    let mut cmd = std::process::Command::new("cargo");
    cmd.arg("test")
        .arg("--manifest-path")
        .arg(repo_root.join("core/Cargo.toml"))
        .arg("--test")
        .arg(&check.target);
    // A target gated behind a non-default feature (e.g. the experimental-skillbank-
    // memory skill-store test) compiles to ZERO tests under default features —
    // which the classifier honestly treats as `target-produced-no-tests` (Failed).
    // Enable the gate-map-declared features so the REAL test actually runs.
    if !check.features.is_empty() {
        cmd.arg("--features").arg(check.features.join(","));
    }
    let output = cmd.current_dir(repo_root).output();

    match output {
        Ok(out) => {
            let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
            combined.push_str(&String::from_utf8_lossy(&out.stderr));
            let parsed = parse_libtest_output(&combined);
            classify_cargo_run(&parsed, out.status.success(), check, gate_id, now)
        }
        Err(e) => CheckRun {
            outcome: CheckOutcome::Failed(vec![format!("failed to spawn cargo: {e}")]),
            results: vec![],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
        std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {p:?}: {e}"))
    }

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-06-17T12:00:00Z").unwrap().with_timezone(&Utc)
    }

    fn cargo_check(target: &str) -> Check {
        Check {
            kind: super::super::CheckKind::CargoTest,
            target: target.into(),
            spec: "SPEC-23".into(),
            blocking_test_ids: vec![],
            manual_reason: None,
            features: vec![],
        }
    }

    #[test]
    fn parse_pass_fixture() {
        let parsed = parse_libtest_output(&fixture("libtest_pass.txt"));
        assert_eq!(parsed.len(), 3);
        assert!(parsed.iter().all(|p| p.status == ParsedStatus::Ok));
    }

    #[test]
    fn parse_fail_fixture_maps_each_status() {
        let parsed = parse_libtest_output(&fixture("libtest_fail.txt"));
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].status, ParsedStatus::Ok);
        assert_eq!(parsed[1].status, ParsedStatus::Failed);
        assert_eq!(parsed[2].status, ParsedStatus::Ignored);
        // the "test result: FAILED. ..." summary line is NOT counted as a test.
        assert!(!parsed.iter().any(|p| p.name == "result:"));
    }

    #[test]
    fn parse_empty_fixture_is_zero_tests() {
        let parsed = parse_libtest_output(&fixture("libtest_empty.txt"));
        assert!(parsed.is_empty());
    }

    #[test]
    fn classify_all_pass_is_passed() {
        let parsed = parse_libtest_output(&fixture("libtest_pass.txt"));
        let run = classify_cargo_run(&parsed, true, &cargo_check("daily_loop_golden"), GateId::V3, now());
        assert_eq!(run.outcome, CheckOutcome::Passed);
        assert_eq!(run.results.len(), 3);
    }

    #[test]
    fn classify_with_failure_is_failed() {
        let parsed = parse_libtest_output(&fixture("libtest_fail.txt"));
        let run = classify_cargo_run(&parsed, false, &cargo_check("daily_loop_golden"), GateId::V3, now());
        match run.outcome {
            CheckOutcome::Failed(b) => assert!(b.contains(&"tests::beta".to_string())),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    /// The fake-green catch: a target that produced zero tests is Failed, never Passed.
    #[test]
    fn classify_zero_tests_is_failed_not_passed() {
        let parsed = parse_libtest_output(&fixture("libtest_empty.txt"));
        // even with exit_success=true (the classic `cargo test <substring>` exit-0 trap):
        let run = classify_cargo_run(&parsed, true, &cargo_check("no_such_filter"), GateId::V3, now());
        match run.outcome {
            CheckOutcome::Failed(b) => assert!(b.iter().any(|s| s.contains("target-produced-no-tests"))),
            other => panic!("expected Failed(target-produced-no-tests), got {other:?}"),
        }
    }

    /// blocking_test_ids from the gate-map override the raw failed-fn names.
    #[test]
    fn classify_uses_gatemap_blocking_ids() {
        let parsed = parse_libtest_output(&fixture("libtest_fail.txt"));
        let mut check = cargo_check("daily_loop_golden");
        check.blocking_test_ids = vec!["T-daily-loop-golden".into()];
        let run = classify_cargo_run(&parsed, false, &check, GateId::V3, now());
        match run.outcome {
            CheckOutcome::Failed(b) => assert_eq!(b, vec!["T-daily-loop-golden".to_string()]),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    // ── Task 4: script runner ────────────────────────────────────────────────

    fn script_check(target: &str) -> Check {
        Check {
            kind: super::super::CheckKind::Script,
            target: target.into(),
            spec: "SPEC-10".into(),
            blocking_test_ids: vec![],
            manual_reason: None,
            features: vec![],
        }
    }

    fn have(interp: &str) -> bool {
        std::process::Command::new(interp)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    #[test]
    fn classify_script_run_exit_codes() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let pass = classify_script_run(Some(0), true, "ok", &script_check("scripts/x.sh"), GateId::V10, root, now());
        assert_eq!(pass.outcome, CheckOutcome::Passed);
        assert_eq!(pass.results[0].status, TestStatus::Pass);

        let fail = classify_script_run(Some(1), false, "boom", &script_check("scripts/x.sh"), GateId::V10, root, now());
        assert!(matches!(fail.outcome, CheckOutcome::Failed(_)));
        assert_eq!(fail.results[0].status, TestStatus::Fail);
        assert!(fail.results[0].failure_log.as_ref().unwrap().contains("exit 1"));
    }

    #[test]
    fn redact_strips_repo_and_home() {
        let root = Path::new("C:/Users/secret/pm-p22-wt");
        let text = "error at C:/Users/secret/pm-p22-wt/core/src/foo.rs line 1";
        let red = redact(text, root);
        assert!(red.contains("<repo>/core/src/foo.rs"), "got: {red}");
        assert!(!red.contains("secret/pm-p22-wt/core"), "abs repo path leaked: {red}");
    }

    #[test]
    fn run_script_fixtures_pass_and_fail_via_bash() {
        // Probe a USABLE bash (Git Bash, not the Windows WSL stub). run_script picks
        // the same candidate; skip cleanly only when no real bash exists at all.
        let usable = super::bash_candidates().into_iter().any(|b| {
            std::process::Command::new(&b)
                .args(["-c", "echo ok"])
                .output()
                .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == "ok")
                .unwrap_or(false)
        });
        if !usable {
            eprintln!("skip: no usable bash on this host");
            return;
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let ok = run_script(&script_check("core/tests/fixtures/exit0.sh"), GateId::V10, root);
        assert_eq!(ok.outcome, CheckOutcome::Passed, "exit0.sh should pass");
        let bad = run_script(&script_check("core/tests/fixtures/exit1.sh"), GateId::V10, root);
        assert!(matches!(bad.outcome, CheckOutcome::Failed(_)), "exit1.sh should fail");
    }

    #[test]
    #[cfg(windows)]
    fn run_script_ps1_fixture_via_powershell() {
        if !have("pwsh") && !have("powershell") {
            eprintln!("skip: no PowerShell on PATH");
            return;
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let ok = run_script(&script_check("core/tests/fixtures/exit0.ps1"), GateId::V10, root);
        assert_eq!(ok.outcome, CheckOutcome::Passed, "exit0.ps1 should pass");
    }
}

/// Ordered `bash` candidates. On Windows, prefer real Git Bash over the
/// `System32\bash.exe` WSL stub (which prints "install a distro" + exits non-zero
/// when no distro is present); fall back to PATH `bash` last.
#[cfg(windows)]
fn bash_candidates() -> Vec<String> {
    let mut v = Vec::new();
    let mut roots: Vec<String> = vec![
        "C:\\Program Files\\Git".into(),
        "C:\\Program Files (x86)\\Git".into(),
    ];
    if let Ok(pf) = std::env::var("ProgramFiles") {
        roots.push(pf);
    }
    for root in roots {
        for sub in ["bin\\bash.exe", "usr\\bin\\bash.exe"] {
            let p = format!("{root}\\{sub}");
            if std::path::Path::new(&p).is_file() && !v.contains(&p) {
                v.push(p);
            }
        }
    }
    v.push("bash".into());
    v
}
#[cfg(not(windows))]
fn bash_candidates() -> Vec<String> {
    vec!["bash".into()]
}

/// Pick the interpreter(s) for a script by extension. `.ps1` prefers `pwsh`
/// (PowerShell 7), falling back to Windows PowerShell `powershell`; `.sh` → bash
/// (Git Bash preferred on Windows). Returns the ordered candidates to try.
fn interpreters_for(target: &str) -> Vec<String> {
    let lower = target.to_ascii_lowercase();
    if lower.ends_with(".ps1") {
        vec!["pwsh".into(), "powershell".into()]
    } else if lower.ends_with(".sh") {
        bash_candidates()
    } else {
        vec![]
    }
}

fn interpreter_args(interp: &str, abs_target: &Path) -> Vec<String> {
    let lower = interp.to_ascii_lowercase();
    if lower.contains("powershell") || lower.ends_with("pwsh") || lower == "pwsh" {
        vec![
            "-NoProfile".into(),
            "-File".into(),
            abs_target.to_string_lossy().into_owned(),
        ]
    } else {
        vec![abs_target.to_string_lossy().into_owned()] // bash <file>
    }
}

/// Redact output to be OSS-safe: repo-relative paths only (strip the absolute repo
/// root + the user home dir), and truncate to ~200 lines (SPEC-60 §7.2 / constraint 6).
pub fn redact(text: &str, repo_root: &Path) -> String {
    let mut s = text.to_string();
    let root = repo_root.to_string_lossy().into_owned();
    s = s.replace(&root, "<repo>");
    s = s.replace(&root.replace('\\', "/"), "<repo>");
    if let Some(home) = dirs::home_dir() {
        let h = home.to_string_lossy().into_owned();
        s = s.replace(&h, "~");
        s = s.replace(&h.replace('\\', "/"), "~");
    }
    let truncated: Vec<&str> = s.lines().take(200).collect();
    let mut out = truncated.join("\n");
    if s.lines().count() > 200 {
        out.push_str("\n… (truncated to 200 lines)");
    }
    out
}

/// Pure classification of a script run (split from spawning for a hermetic exit-code test).
pub fn classify_script_run(
    exit_code: Option<i32>,
    success: bool,
    combined_output: &str,
    check: &Check,
    gate_id: GateId,
    repo_root: &Path,
    now: DateTime<Utc>,
) -> CheckRun {
    let test_id = if check.target.trim().is_empty() {
        format!("script:{}", gate_id.as_str())
    } else {
        check.target.trim().to_string()
    };
    let blocking = if check.blocking_test_ids.is_empty() {
        vec![test_id.clone()]
    } else {
        check.blocking_test_ids.clone()
    };

    if success {
        CheckRun {
            outcome: CheckOutcome::Passed,
            results: vec![TestRunResult {
                test_id,
                spec_id: check.spec.clone(),
                gate_id,
                os: None,
                status: TestStatus::Pass,
                duration_ms: 0,
                attempts: 1,
                reproducer_cmd: Some(check.target.clone()),
                failure_log: None,
                timestamp: now,
            }],
        }
    } else {
        let code = exit_code.map(|c| c.to_string()).unwrap_or_else(|| "signal".into());
        CheckRun {
            outcome: CheckOutcome::Failed(blocking.clone()),
            results: vec![TestRunResult {
                test_id,
                spec_id: check.spec.clone(),
                gate_id,
                os: None,
                status: TestStatus::Fail,
                duration_ms: 0,
                attempts: 1,
                reproducer_cmd: Some(check.target.clone()),
                failure_log: Some(format!(
                    "exit {code}\n{}",
                    redact(combined_output, repo_root)
                )),
                timestamp: now,
            }],
        }
    }
}

/// Spawn a `.sh`/`.ps1` script (interpreter by extension), map exit → status,
/// redact output. `doc_lint` checks route here too (they just name a `.ps1`/`.sh`
/// lint script, e.g. `scripts/check-doc-tree.ps1` — reuse, no new lint).
pub fn run_script(check: &Check, gate_id: GateId, repo_root: &Path) -> CheckRun {
    let now = Utc::now();
    let target = check.target.trim();
    let abs = repo_root.join(target);
    let candidates = interpreters_for(target);
    if candidates.is_empty() {
        return CheckRun {
            outcome: CheckOutcome::Failed(vec![format!("unknown script type: {target}")]),
            results: vec![],
        };
    }

    let mut last_err = String::new();
    for interp in &candidates {
        let res = std::process::Command::new(interp)
            .args(interpreter_args(interp, &abs))
            .current_dir(repo_root)
            .output();
        match res {
            Ok(out) => {
                let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
                combined.push_str(&String::from_utf8_lossy(&out.stderr));
                return classify_script_run(
                    out.status.code(),
                    out.status.success(),
                    &combined,
                    check,
                    gate_id,
                    repo_root,
                    now,
                );
            }
            Err(e) => last_err = format!("{interp}: {e}"),
        }
    }
    CheckRun {
        outcome: CheckOutcome::Failed(vec![format!("no interpreter available for {target}: {last_err}")]),
        results: vec![],
    }
}
