//! P2-2 Task 12 — anti-fake-green meta-tests (SPEC-60 §19 `T-testing-*`).
//! The load-bearing proof that the collector itself is HONEST: it cannot be
//! tricked into green by an unresolved/unknown gate, red stays red across an
//! override on a different gate, and the Rust resolver agrees with the existing
//! `scripts/check-test-citations.sh`.

use std::path::Path;

use spectyn_mesh::test_report::{
    build_report, collect_report, load_gate_map, lint_gate_map, resolve_citation, Check, CheckKind,
    CheckResolution, GateId, GateMap, GateSpec, GateStatus, Override, OverallStatus, RunContext,
    ShipGate, TriggeredBy,
};

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
}
fn now() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339("2026-06-17T12:00:00Z").unwrap().with_timezone(&chrono::Utc)
}
fn ctx() -> RunContext {
    RunContext { trigger: TriggeredBy::WorkflowDispatch }
}
fn gate(id: GateId, checks: Vec<Check>) -> GateSpec {
    GateSpec {
        id,
        name: format!("gate {}", id.as_str()),
        entry_criteria: vec![],
        exit_criteria: vec![],
        pr_required: false,
        checks,
    }
}
fn cargo_check(target: &str) -> Check {
    Check { kind: CheckKind::CargoTest, target: target.into(), spec: "SPEC-08".into(), blocking_test_ids: vec![], manual_reason: None, features: vec![] }
}

/// T-testing-gate-map-resolves — every non-manual check in the SHIPPED map resolves.
#[test]
fn t_testing_gate_map_resolves() {
    let path = repo_root().join("docs/superpowers/specs/v060-deep-spec/appendix/ship-gate-map.toml");
    let map = load_gate_map(&path).expect("shipped map parses");
    let unresolved = lint_gate_map(&map, repo_root());
    assert!(unresolved.is_empty(), "shipped map has unresolved checks: {unresolved:#?}");
}

/// T-testing-no-fake-green — a map whose only check points at a nonexistent test
/// produces overall_status != Green (the unresolved check cannot be greened).
#[test]
fn t_testing_no_fake_green() {
    let map = GateMap { gates: vec![gate(GateId::V8, vec![cargo_check("no_such_test_xyz")])] };
    let report = collect_report(&map, repo_root(), &ctx(), "test", "deadbeef", now(), false);
    assert_ne!(report.overall_status, OverallStatus::Green, "unresolved check must not be green");
    assert_eq!(report.gates[0].status, GateStatus::Red);
}

/// T-testing-unknown-gate-not-green — checks=[] ⇒ Unknown, and that makes
/// overall_status Red without an override.
#[test]
fn t_testing_unknown_gate_not_green() {
    let map = GateMap { gates: vec![gate(GateId::V6, vec![])] };
    let report = collect_report(&map, repo_root(), &ctx(), "test", "deadbeef", now(), false);
    assert_eq!(report.gates[0].status, GateStatus::Unknown);
    assert_eq!(report.overall_status, OverallStatus::Red);
}

/// T-testing-citations-agreement — the Rust resolver mirrors
/// `scripts/check-test-citations.sh`. Asserted two ways: (a) a shared fixture of
/// citations resolves as the bash script's rules dictate, and (b) the real script
/// exits 0 on the repo (its own ✅ citations resolve), proving shared semantics.
#[test]
fn t_testing_citations_agreement() {
    let root = repo_root();
    let cases: &[(&str, bool)] = &[
        ("--test coach_shame_free_fixture", true),  // core/tests/<name>.rs exists
        ("--test definitely_not_a_real_test", false),
        ("--lib classify_gate", true),              // real fn in core/src
        ("--lib resolve_check", true),              // real fn in core/src
        ("--lib zzz_no_such_symbol_zzz", false),
    ];
    for (cite, want_resolved) in cases {
        let got = matches!(resolve_citation(cite, root), CheckResolution::Resolved);
        assert_eq!(got, *want_resolved, "resolver disagreed on `{cite}`");
    }
    // (b) the real bash resolver agrees that the repo's own citations resolve.
    // Use a USABLE bash (Git Bash), not the Windows System32\bash.exe WSL stub
    // (which spawns OK but exits non-zero with an "install a distro" message).
    match usable_bash() {
        Some(bash) => {
            let out = std::process::Command::new(&bash)
                .arg("scripts/check-test-citations.sh")
                .current_dir(root)
                .output()
                .expect("spawn check-test-citations.sh");
            assert!(
                out.status.success(),
                "check-test-citations.sh failed (exit {:?}); resolvers disagree:\n{}",
                out.status.code(),
                String::from_utf8_lossy(&out.stdout)
            );
        }
        None => eprintln!("skip bash cross-check: no usable bash on this host"),
    }
}

/// First bash that actually runs (`bash -c 'echo SPECTYN_BASH_OK'` → exact match),
/// preferring Git Bash over the WSL stub on Windows.
fn usable_bash() -> Option<String> {
    let mut candidates: Vec<String> = Vec::new();
    if cfg!(windows) {
        for p in [
            "C:\\Program Files\\Git\\bin\\bash.exe",
            "C:\\Program Files\\Git\\usr\\bin\\bash.exe",
        ] {
            if Path::new(p).is_file() {
                candidates.push(p.to_string());
            }
        }
    }
    candidates.push("bash".to_string());
    for c in candidates {
        if let Ok(out) = std::process::Command::new(&c).args(["-c", "echo SPECTYN_BASH_OK"]).output() {
            if out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == "SPECTYN_BASH_OK" {
                return Some(c);
            }
        }
    }
    None
}

/// T-testing-red-stays-red — a check that exits 1 ⇒ gate Red, and an override on a
/// DIFFERENT gate does NOT flip it green/red_with_override.
#[test]
fn t_testing_red_stays_red() {
    // V8 has a script that exits 1 (real spawn); V6 is Unknown (checks=[]).
    let exit1 = Check {
        kind: CheckKind::Script,
        target: "core/tests/fixtures/exit1.sh".into(),
        spec: "SPEC-08".into(),
        blocking_test_ids: vec![],
        manual_reason: None,
        features: vec![],
    };
    let map = GateMap { gates: vec![gate(GateId::V8, vec![exit1]), gate(GateId::V6, vec![])] };
    let report = collect_report(&map, repo_root(), &ctx(), "test", "deadbeef", now(), true);

    let v8 = report.gates.iter().find(|g| g.gate_id == GateId::V8).unwrap();
    // exit1.sh needs bash; if unavailable run_script yields Failed too (no interpreter).
    assert_eq!(v8.status, GateStatus::Red, "exit-1 script ⇒ Red");

    // Now acknowledge a DIFFERENT gate (V6) with an override — V8 must stay red.
    let gates_overridden: Vec<ShipGate> = report
        .gates
        .iter()
        .cloned()
        .map(|mut g| {
            if g.gate_id == GateId::V6 {
                g.override_by = Some(Override {
                    operator: "op".into(),
                    reason: "accepted".into(),
                    timestamp: now(),
                });
            }
            g
        })
        .collect();
    let rebuilt = build_report(gates_overridden, vec![], TriggeredBy::WorkflowDispatch, "t", "d", now(), now());
    assert_eq!(rebuilt.overall_status, OverallStatus::Red, "override on V6 must NOT flip the V8 red");
}
