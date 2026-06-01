//! V9 — zh-TW / en string catalog parity gate (Phase E Wave 18 / SPEC-60 §8.9).
//!
//! Goal of this gate (extracted from
//! `docs/superpowers/PHASE-E-INTEGRATION-TEST-PLAN.md` §3.9):
//!   - (a) zh-TW / en string catalogs share the same key set (no orphan IDs).
//!   - (b) every key has a non-empty value in both locales.
//!
//! Why this matters
//! ----------------
//! `SPEC-05-FOUNDATION-i18n-locale` §3.1 G2 demands 100 % zh-TW + en string
//! completeness ("key parity check must pass"). A drifted catalog leaks one
//! of two bugs to end users: a Traditional-Chinese reader sees raw English
//! fallback text (breaks the "first-eye 繁中" promise in JS1), or an OSS
//! contributor running with `LC_ALL=en_US.UTF-8` sees raw `t('key.path')`
//! identifiers (breaks JS2). Either is a ship-blocker for the Wave 18 gate.
//!
//! Catalog source
//! --------------
//! SPEC-05 §6.1 specifies `app/src/i18n/strings/{zh-TW,en}.ts` as the
//! source-of-truth and PHASE-E-INTEGRATION-TEST-PLAN.md §3.9 points the
//! parity test at `app/src/lib/i18n/{zh-TW,en}.json`. Neither path exists
//! yet (Stage 4 of `tauri_wire` / extract-strings.ts not yet shipped), so
//! this file currently exercises the parity-lint *logic* against fixtures
//! under `core/tests/fixtures/i18n/`. Once a real catalog lands, swap
//! `fixture_path("zh-TW.json")` for the production path — the lint logic
//! does not change.
//!
//! No new Cargo deps: `serde_json` is already in `core/Cargo.toml`.

use std::collections::BTreeMap;
use std::path::PathBuf;

/// Resolve a fixture under `core/tests/fixtures/i18n/<name>` via
/// `CARGO_MANIFEST_DIR` so the test is invariant to the cwd at
/// `cargo test` invocation time.
fn fixture_path(name: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p.push("i18n");
    p.push(name);
    p
}

/// Load a flat string catalog (`{ "key.path": "value", ... }`) into a
/// deterministic `BTreeMap`. Using `BTreeMap` instead of `HashMap` keeps
/// any future failure messages stable across runs, which matters when
/// the parity diff is the assertion payload.
fn load_catalog(name: &str) -> BTreeMap<String, String> {
    let path = fixture_path(name);
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read fixture {}: {e}", path.display()));
    serde_json::from_str::<BTreeMap<String, String>>(&raw)
        .unwrap_or_else(|e| panic!("fixture {} is not a flat string catalog: {e}", path.display()))
}

/// Pure parity lint. Returns `Ok(())` if both catalogs share the same
/// key set AND every value (in both locales) is non-empty after trim;
/// otherwise returns a deterministic, sorted, human-readable diff so
/// the assertion message tells the operator exactly which key drifted.
fn check_parity(
    primary_name: &str,
    primary: &BTreeMap<String, String>,
    secondary_name: &str,
    secondary: &BTreeMap<String, String>,
) -> Result<(), String> {
    let only_in_primary: Vec<&String> =
        primary.keys().filter(|k| !secondary.contains_key(*k)).collect();
    let only_in_secondary: Vec<&String> =
        secondary.keys().filter(|k| !primary.contains_key(*k)).collect();

    let empty_in_primary: Vec<&String> = primary
        .iter()
        .filter(|(_, v)| v.trim().is_empty())
        .map(|(k, _)| k)
        .collect();
    let empty_in_secondary: Vec<&String> = secondary
        .iter()
        .filter(|(_, v)| v.trim().is_empty())
        .map(|(k, _)| k)
        .collect();

    if only_in_primary.is_empty()
        && only_in_secondary.is_empty()
        && empty_in_primary.is_empty()
        && empty_in_secondary.is_empty()
    {
        return Ok(());
    }

    let mut msg = String::from("i18n parity check failed:\n");
    if !only_in_primary.is_empty() {
        msg.push_str(&format!(
            "  keys only in {primary_name}: {only_in_primary:?}\n"
        ));
    }
    if !only_in_secondary.is_empty() {
        msg.push_str(&format!(
            "  keys only in {secondary_name}: {only_in_secondary:?}\n"
        ));
    }
    if !empty_in_primary.is_empty() {
        msg.push_str(&format!(
            "  empty values in {primary_name}: {empty_in_primary:?}\n"
        ));
    }
    if !empty_in_secondary.is_empty() {
        msg.push_str(&format!(
            "  empty values in {secondary_name}: {empty_in_secondary:?}\n"
        ));
    }
    Err(msg)
}

// ─── 1/5 — happy path: balanced catalog passes ─────────────────────────────

/// The shipped baseline fixture (`zh-TW.json` + `en.json`) must satisfy
/// the lint. If this test fails, either (a) a maintainer edited one
/// catalog without the other, or (b) the lint logic regressed. This is
/// the gate we will flip to point at the production catalog once
/// `app/src/lib/i18n/` ships.
#[test]
fn v9_baseline_catalogs_have_full_parity() {
    let zh = load_catalog("zh-TW.json");
    let en = load_catalog("en.json");

    assert!(
        !zh.is_empty(),
        "baseline zh-TW fixture must contain at least one key — found 0"
    );
    assert_eq!(
        zh.len(),
        en.len(),
        "baseline fixtures must have equal key counts: zh-TW={} en={}",
        zh.len(),
        en.len()
    );

    check_parity("zh-TW.json", &zh, "en.json", &en)
        .expect("baseline fixtures should pass the parity lint");
}

// ─── 2/5 — detection: missing keys are caught both directions ──────────────

/// If `en.json` has a key absent from `zh-TW.json`, the lint must
/// flag it. This guards the "no orphan string IDs" half of the gate
/// (SPEC-05 §3.1 G2) regardless of which catalog the orphan lives in.
#[test]
fn v9_parity_catches_missing_keys_in_both_directions() {
    let zh = load_catalog("zh-TW_missing_key.json");
    let en = load_catalog("en_missing_key.json");

    let err = check_parity("zh-TW", &zh, "en", &en)
        .expect_err("missing-key fixtures must fail the parity lint");

    assert!(
        err.contains("only in en"),
        "diff must call out the en-only key, got: {err}"
    );
    assert!(
        err.contains("chat.placeholder"),
        "diff must name the orphan key `chat.placeholder`, got: {err}"
    );
}

// ─── 3/5 — detection: empty values are treated as missing ──────────────────

/// An empty (or whitespace-only) translation is functionally identical
/// to a missing key — the user sees blank UI. SPEC-05 §3.1 G2 demands
/// 100 % completeness, so the lint must treat `""` as a failure rather
/// than rubber-stamping it.
#[test]
fn v9_parity_catches_empty_values() {
    let zh = load_catalog("zh-TW_empty_value.json");
    let en = load_catalog("en_empty_value.json");

    let err = check_parity("zh-TW", &zh, "en", &en)
        .expect_err("empty-value fixtures must fail the parity lint");

    assert!(
        err.contains("empty values in zh-TW"),
        "diff must surface the empty-value error, got: {err}"
    );
    assert!(
        err.contains("chat.placeholder"),
        "diff must name `chat.placeholder` as the empty key, got: {err}"
    );
}

// ─── 4/5 — diff output is deterministic (BTreeMap, sorted output) ──────────

/// Two consecutive parity checks against the same drifted catalog must
/// produce byte-identical error messages, otherwise CI would log noise
/// on retries and operator triage would chase a moving target. This
/// pins the `BTreeMap` + sorted-output choice as a load-bearing
/// invariant.
#[test]
fn v9_parity_diff_output_is_deterministic() {
    let zh = load_catalog("zh-TW_missing_key.json");
    let en = load_catalog("en_missing_key.json");

    let err_1 = check_parity("zh-TW", &zh, "en", &en).unwrap_err();
    let err_2 = check_parity("zh-TW", &zh, "en", &en).unwrap_err();

    assert_eq!(
        err_1, err_2,
        "parity diff must be deterministic across runs (BTreeMap ordering)"
    );
}

// ─── 5/5 — catalog schema: flat string→string (rejects nested objects) ─────

/// SPEC-05 §6.1 specifies catalogs as flat `key.path → string` maps;
/// nesting (`{ chat: { send: "Send" } }`) is explicitly out of scope
/// for v0.6.0 because the Rust `phf::Map` codegen step (SPEC-05 §6.1
/// Build node) assumes a flat layout. This test pins that assumption
/// — if a future PR ships a nested catalog, the schema parse fails
/// loudly rather than silently accepting half the strings.
#[test]
fn v9_catalog_schema_rejects_nested_objects() {
    let nested = r#"{ "chat": { "send": "Send" } }"#;
    let parsed = serde_json::from_str::<BTreeMap<String, String>>(nested);
    assert!(
        parsed.is_err(),
        "flat catalog schema must reject nested objects, but accepted: {parsed:?}"
    );
}
