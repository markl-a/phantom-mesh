// T-EVAL-01 (2026-05-29) — insta snapshot regression for a stable wire struct.
//
// Goal: fast-feedback guard that the JSON serialization of a simple,
// frozen `ProviderConfig` instance does not silently change shape (field
// rename / reorder / drop). If the wire surface changes intentionally,
// run `cargo insta review` (cargo-insta binary) to accept the new snapshot,
// or delete tests/snapshots/eval_insta_snapshot__provider_config_wire.snap
// and re-run to regenerate.
//
// This is a dev/test-only artifact: no production code is touched. The
// instance uses OSS-safe placeholder data (user42 / example.com) only.
//
// Run directly with plain cargo:
//   cargo test --test eval_insta_snapshot
// (No cargo-nextest or cargo-insta binary required for the test to pass —
//  the accepted .snap file is checked in.)

use insta::assert_json_snapshot;
use spectyn_mesh::providers_wire::ProviderConfig;

/// A fixed, deterministic `ProviderConfig` with placeholder-only values so the
/// JSON snapshot is byte-stable across machines and CI runs.
fn fixed_provider_config() -> ProviderConfig {
    ProviderConfig {
        slug: "groq".to_string(),
        // Pointer into the SPEC-13 age vault — never a raw key. Placeholder ref.
        api_key_ref: "secrets.age#providers.user42.api_key".to_string(),
        default_model: "llama-3.1-8b-instant".to_string(),
        base_url: Some("https://api.example.com/v1".to_string()),
        timeout_ms: 30_000,
    }
}

#[test]
fn provider_config_wire_json_snapshot_is_stable() {
    let cfg = fixed_provider_config();
    assert_json_snapshot!("provider_config_wire", cfg);
}

#[test]
fn provider_config_json_round_trips() {
    // Belt-and-braces: independent of the snapshot, confirm serialize ->
    // deserialize is lossless so a green snapshot can't mask a broken wire.
    let cfg = fixed_provider_config();
    let json = serde_json::to_string(&cfg).expect("serialize");
    let back: ProviderConfig = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.slug, cfg.slug);
    assert_eq!(back.api_key_ref, cfg.api_key_ref);
    assert_eq!(back.default_model, cfg.default_model);
    assert_eq!(back.base_url, cfg.base_url);
    assert_eq!(back.timeout_ms, cfg.timeout_ms);
}
