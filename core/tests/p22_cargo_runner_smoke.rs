//! P2-2 Task 3 spawn smoke: the cargo_test runner actually drives a real
//! `cargo test --test skip_marker_convention` (a tiny existing fast test) and
//! reports >= 1 pass. Gated behind `#[ignore = "p22-spawn"]` so it never slows the
//! default suite (it spawns a nested cargo build). Run on demand:
//!   cargo test --test p22_cargo_runner_smoke -- --ignored

use std::path::Path;

use spectyn_mesh::test_report::{Check, CheckKind, CheckOutcome};
use spectyn_mesh::test_report::runner::run_cargo_test;

#[test]
#[ignore = "p22-spawn"]
fn cargo_runner_drives_a_real_test() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    // The nested `cargo test` must use a SEPARATE target dir: this outer test
    // process already holds the build lock on the ambient CARGO_TARGET_DIR, so an
    // inherited dir would deadlock/abort. A dedicated sibling dir is cache-friendly
    // across on-demand runs. (run_cargo_test inherits CARGO_TARGET_DIR from env.)
    let nested_target = std::env::temp_dir().join("spectyn-p22-smoke-target");
    std::env::set_var("CARGO_TARGET_DIR", &nested_target);
    let check = Check {
        kind: CheckKind::CargoTest,
        target: "skip_marker_convention".into(),
        spec: "SPEC-60".into(),
        blocking_test_ids: vec![],
        manual_reason: None,
        features: vec![],
    };
    let run = run_cargo_test(&check, spectyn_mesh::test_report::GateId::V2, repo_root);
    assert!(
        matches!(run.outcome, CheckOutcome::Passed),
        "expected Passed, got {:?}",
        run.outcome
    );
    assert!(
        run.results.iter().any(|r| matches!(r.status, spectyn_mesh::test_report::TestStatus::Pass)),
        "expected >= 1 pass among {} results",
        run.results.len()
    );
}
