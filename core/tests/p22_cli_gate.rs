//! P2-2 Task 9 — `phantom test` CLI integration.
//! Asserts `gate map --check` exits 0 on the real shipped gate-map, and
//! `report --json --no-run` emits a parseable ShipGateReport (12 gates, honest
//! non-green overall_status). `--no-run` keeps it fast + hermetic (no heavy spawns).

use std::path::Path;
use std::process::Command;

use phantom_mesh::test_report::{OverallStatus, ShipGateReport};

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
}

fn phantom() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_phantom"));
    c.current_dir(repo_root()); // so find_repo_root() locates the gate-map
    c
}

#[test]
fn gate_map_check_exits_zero() {
    let out = phantom().args(["test", "gate", "map", "--check"]).output().expect("spawn phantom");
    assert!(
        out.status.success(),
        "gate map --check should exit 0; got {:?}\nstdout: {}\nstderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn report_json_emits_valid_ship_gate_report() {
    let out = phantom()
        .args(["test", "report", "--json", "--no-run"])
        .output()
        .expect("spawn phantom");
    assert!(out.status.success(), "report should exit 0");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let report: ShipGateReport =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("parse ShipGateReport: {e}\n{stdout}"));
    assert_eq!(report.gates.len(), 12, "12 gates");
    // Honest: a static report (nothing ran) cannot be green — un-evidenced gates exist.
    assert_ne!(report.overall_status, OverallStatus::Green, "must NOT fake green");
    assert_eq!(report.overall_status, OverallStatus::Red);
}
