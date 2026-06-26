//! SPEC-61 §19 scenario-catalog meta-tests (P2-2 Task 7), now real.
//! Validates the shipped appendix/scenarios-S1-S40.csv: 40 contiguous rows,
//! auto-ratio >= 80%, manual rows justified, and every testId resolves to >= 1
//! SPEC-*.md under v060-deep-spec/.

use std::path::Path;

use phantom_mesh::scenarios::{
    auto_ratio, load_catalog_at, validate_count_contiguous, validate_manual_justified,
    validate_test_ids_resolve,
};

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
}

#[test]
fn catalog_complete_40_contiguous() {
    let cat = load_catalog_at(repo_root()).expect("load catalog");
    validate_count_contiguous(&cat).expect("40 contiguous S1..S40");
}

#[test]
fn catalog_auto_ratio_at_least_80pct() {
    let cat = load_catalog_at(repo_root()).expect("load catalog");
    assert!(auto_ratio(&cat) >= 0.80, "auto ratio below 0.80");
}

#[test]
fn catalog_manual_rows_justified() {
    let cat = load_catalog_at(repo_root()).expect("load catalog");
    validate_manual_justified(&cat).expect("manual rows justified");
}

#[test]
fn catalog_test_ids_resolve_to_specs() {
    let cat = load_catalog_at(repo_root()).expect("load catalog");
    let unresolved = validate_test_ids_resolve(&cat, repo_root());
    assert!(
        unresolved.is_empty(),
        "these testIds resolve to 0 SPEC-*.md files: {unresolved:?}"
    );
}
