//! V4 chaos integration tests — graceful degradation when external deps fail.
//!
//! Each test exercises ONE wire module's public API with a deliberately
//! broken external dependency (missing D-Bus, unset env var, garbage JWT,
//! unreachable URL, missing on-disk DB, etc.) and asserts the call returns
//! a typed `Err` variant from the wire's `*Error` catalog instead of
//! panicking. Per `docs/superpowers/PHASE-E-INTEGRATION-TEST-PLAN.md` §V4,
//! these are the "system survives when the world misbehaves" sanity checks
//! that gate the v0.6.0 GA cut — a panic here means an end-user-visible
//! crash in the same scenario.
//!
//! Style:
//!   * Real wire APIs only (no `_pseudo` helpers reached directly).
//!   * Tests whose underlying wire still has a Stage 4 `unimplemented!()`
//!     inner helper are marked `#[ignore = "pending Stage 4"]` so they
//!     can be flipped on as the modules land their real impls without
//!     editing this file's structure.
//!   * Network-dependent tests use `#[tokio::test]` (tokio is already in
//!     `core/Cargo.toml` `[dependencies]` as `tokio = { version = "1",
//!     features = ["full"] }`, so it's available to integration tests
//!     without a dev-dep entry).

use std::time::Duration;

// ─── 1/8 — identity_wire Linux libsecret w/o D-Bus ───────────────────────────
//
// SPEC-12: `write_to_keystore(LinuxSecretService, ...)` MUST return
// `KeyDerivationError::KeystoreUnavailable` when D-Bus / Secret Service is
// absent (sandboxed CI, headless container) instead of panicking. The Linux
// gate keeps this test from running on macOS / Windows where the same call
// hits the cross-platform stub variant.

#[cfg(target_os = "linux")]
#[test]
fn v4_identity_linux_no_dbus_returns_keystore_unavailable() {
    use phantom_mesh::identity_wire::{
        write_to_keystore, KeyDerivationError, KeystoreBackend,
    };

    // CI runners typically have no DBUS_SESSION_BUS_ADDRESS set; that drives
    // `secret_service::SecretService::connect` straight to a `connect` error
    // mapped to `KeystoreUnavailable`. We don't unset env here on purpose —
    // a developer running this on a Linux desktop with a real keyring would
    // see `Ok(())` (the secret really got written), which is ALSO a valid
    // chaos outcome (the system is healthy). So we accept either: the test
    // only fails on panic / wrong error variant.
    match write_to_keystore(
        KeystoreBackend::LinuxSecretService,
        "phantom-mesh-v4-chaos-test",
        b"not-a-real-seed-just-bytes-for-test",
    ) {
        Ok(()) => { /* keyring is actually up; degraded path not exercised */ }
        Err(KeyDerivationError::KeystoreUnavailable { detail }) => {
            assert!(!detail.is_empty(), "detail must be human-readable");
        }
        Err(other) => panic!("unexpected error variant: {other:?}"),
    }
}

// ─── 2/8 — providers_wire validate_config with empty api_key_ref ─────────────
//
// SPEC-14: `validate_config` MUST return `ProviderError::AuthError` when the
// caller passes a `ProviderConfig` whose `api_key_ref` is blank (the legacy
// path is "user forgot to set the vault ref in agents.toml"). No env var
// manipulation needed — the field is the test input.

#[test]
fn v4_providers_validate_config_missing_api_key_ref() {
    use phantom_mesh::providers_wire::{validate_config, ProviderConfig, ProviderError};

    let cfg = ProviderConfig {
        slug: "groq".to_string(),
        api_key_ref: "   ".to_string(), // whitespace-only — must fail
        default_model: "llama-3.1-8b-instant".to_string(),
        base_url: None,
        timeout_ms: 30_000,
    };

    match validate_config(&cfg) {
        Err(ProviderError::AuthError { detail }) => {
            assert!(detail.contains("api_key_ref"), "detail names the field");
        }
        Ok(()) => panic!("validate_config must reject empty api_key_ref"),
        Err(other) => panic!("expected AuthError, got {other:?}"),
    }
}

// ─── 3/8 — broker_vault_wire verify_broker_jwt with garbage ──────────────────
//
// SPEC-15: `verify_broker_jwt` MUST return `BrokerError::Unauthorized` (or
// `JwtExpired` for the expired-signature ErrorKind) on any malformed token
// — never panic, never trust an unsigned blob.

#[test]
fn v4_broker_verify_jwt_garbage_returns_unauthorized() {
    use phantom_mesh::broker_vault_wire::{verify_broker_jwt, BrokerError};

    let garbage = "this.is.definitely-not-a-jwt";
    let secret = b"any-secret-bytes-here-for-hs256";

    match verify_broker_jwt(garbage, secret) {
        Err(BrokerError::Unauthorized { detail }) => {
            assert!(!detail.is_empty(), "detail must carry parse cause");
        }
        Err(BrokerError::JwtExpired) => { /* also acceptable per §11 catalog */ }
        Ok(()) => panic!("garbage JWT must NOT verify"),
        Err(other) => panic!("expected Unauthorized/JwtExpired, got {other:?}"),
    }
}

// ─── 4/8 — cluster_dispatch_wire refresh_capabilities w/ unreachable URL ─────
//
// SPEC-26: `refresh_capabilities` is the public RPC entry point that wraps
// the private HMAC-signed `rpc_get`. We redirect the per-peer URL via the
// documented `PHANTOM_PEER_<ID>_URL` env-var hook to `127.0.0.1:1` (almost
// guaranteed connection-refused). The call MUST return a typed
// `DispatchError` (timeout / busy / auth-failed) within reasonable wall
// time — not hang and not panic.

#[tokio::test]
async fn v4_dispatch_refresh_capabilities_unreachable_url() {
    use phantom_mesh::cluster_dispatch_wire::{refresh_capabilities, DispatchError};

    // SPEC-26's `peer_base_url` reads these vars on each call. Edition 2021
    // keeps `set_var` safe; the same writes would need an `unsafe` block on
    // edition 2024 — flagged here so the test moves forward cleanly.
    std::env::set_var("PHANTOM_CLUSTER_SECRET", "test-secret-not-used-for-real-auth");
    std::env::set_var("PHANTOM_PEER_V4CHAOS_URL", "http://127.0.0.1:1");

    // Bound the wall clock so a regression to "hang forever" fails loudly.
    let res = tokio::time::timeout(
        Duration::from_secs(15),
        refresh_capabilities("v4chaos"),
    )
    .await
    .expect("call must complete within 15s, not hang");

    match res {
        Err(DispatchError::RouteTimeout)
        | Err(DispatchError::AllPeersBusy)
        | Err(DispatchError::DispatchAuthFailed) => { /* all acceptable */ }
        Ok(_) => panic!("unreachable URL must not yield Ok capabilities"),
        Err(other) => panic!("unexpected dispatch error: {other:?}"),
    }
}

// ─── 5/8 — mdns_wire start_browser with no-match cluster filter ──────────────
//
// SPEC-11: `start_browser` MUST not panic when the provided cluster-hash
// matches no advertisement on the LAN — the discovery loop is supposed to
// silently filter mismatches per §8. Acceptable outcomes: `Ok(())` (browser
// spun up, just emits nothing) or `MdnsError::BindFail` / `DaemonMissing`
// in headless CI where UDP-5353 isn't bind-able.

#[test]
fn v4_mdns_start_browser_no_match_does_not_panic() {
    use phantom_mesh::mdns_wire::{start_browser, MdnsError};

    // 16-hex cluster id hash that's vanishingly unlikely to match anything.
    let cluster_hash = "deadbeefcafef00d";

    match start_browser(cluster_hash) {
        Ok(()) => { /* browser registered; happy path */ }
        Err(MdnsError::BindFail { .. }) => { /* UDP-5353 already in use */ }
        Err(MdnsError::DaemonMissing { .. }) => { /* no avahi in CI */ }
        Err(MdnsError::PermissionDenied { .. }) => { /* sandboxed runner */ }
        Err(other) => panic!("unexpected mdns error: {other:?}"),
    }
}

// ─── 6/8 — event_storage_wire query_events on absent events dir ──────────────
//
// SPEC-16 §6.1: the data layout is `~/.phantom-mesh/events/`; when that
// directory is missing (fresh install before first capture), `query_events`
// MUST return `Ok(vec![])` per the §11 catalog "STORE-001 OpenFailed is
// reserved for unreadable, not absent." We point `HOME` at a fresh temp
// dir so the events root provably does NOT exist.

#[test]
fn v4_event_storage_query_missing_dir_returns_ok_empty() {
    use phantom_mesh::event_storage_wire::{query_events, EventStoreQuery};

    let tmp = tempfile::tempdir().expect("tempdir");
    let prev_home = std::env::var("HOME").ok();
    // Edition 2021 keeps `set_var`/`remove_var` safe; see test 4 note.
    std::env::set_var("HOME", tmp.path());

    let query = EventStoreQuery::default();
    let result = query_events(&query);

    // Restore HOME before asserting so an assertion failure doesn't leak the
    // override into sibling tests in the same process.
    match prev_home {
        Some(h) => std::env::set_var("HOME", h),
        None => std::env::remove_var("HOME"),
    }

    match result {
        Ok(rows) => assert!(rows.is_empty(), "missing dir must yield empty vec"),
        Err(e) => panic!("missing dir must NOT be Err, got {e:?}"),
    }
}

// ─── 7/8 — coach_delivery_wire send_email with invalid SMTP host ─────────────
//
// SPEC-24 §11.1: `send_email` calls `vault_read_pseudo` first to resolve
// the SMTP password, and that helper is currently a Stage 4
// `unimplemented!()`. Until SPEC-15 vault-read lands, calling `send_email`
// would panic instead of exercising the chaos path. Marked `#[ignore]` so
// the test is wired up and ready to flip on as soon as Stage 4 lands.

#[test]
#[ignore = "pending Stage 4: coach_delivery_wire::vault_read_pseudo unimplemented"]
fn v4_coach_send_email_invalid_smtp_host_returns_smtp_failed() {
    use phantom_mesh::coach_delivery_wire::{send_email, DeliveryError, EmailConfig};

    let cfg = EmailConfig {
        smtp_host: "smtp.invalid".to_string(),
        smtp_port: 1, // refused
        smtp_user: "user@example.invalid".to_string(),
        smtp_password_ref: "vault://test/none".to_string(),
        from_address: "from@example.invalid".to_string(),
        to_address: "to@example.invalid".to_string(),
        use_tls: false,
    };

    match send_email(&cfg, "subject", "body") {
        Err(DeliveryError::EmailSmtpFailed { .. }) => { /* expected */ }
        Err(DeliveryError::ConfigMissing { .. }) => { /* vault unresolvable */ }
        Ok(()) => panic!("invalid SMTP host must not succeed"),
        Err(other) => panic!("expected EmailSmtpFailed, got {other:?}"),
    }
}

// ─── 8/8 — skill_wire recall_skills on absent FTS5 DB ────────────────────────
//
// SPEC-25: `recall_skills` is the public entry to the skill recall stack.
// `fts5_search` (its sqlite FTS5 leg) MUST degrade to an empty hit set
// when the DB file is absent (fresh install before SPEC-16 migration).
// We point `HOME` + `PHANTOM_DB_PATH` at a tempdir to guarantee absence.
// `recall_k = 0` skips the still-Stage-4 `embedding_search` panic path.

#[test]
fn v4_skill_recall_missing_db_does_not_panic() {
    use phantom_mesh::coach_wire::RecallPolicy;
    use phantom_mesh::skill_wire::recall_skills;

    let tmp = tempfile::tempdir().expect("tempdir");
    let prev_home = std::env::var("HOME").ok();
    let prev_db = std::env::var("PHANTOM_DB_PATH").ok();
    // Edition 2021 keeps `set_var` safe; see test 4 note.
    std::env::set_var("HOME", tmp.path());
    std::env::set_var(
        "PHANTOM_DB_PATH",
        tmp.path().join("does-not-exist.db"),
    );

    // recall_k = 0 keeps the call on the pure FTS5 leg (embedding_search is
    // Stage 4 unimplemented and would panic if recall_k > 0).
    let policy = RecallPolicy {
        core_all: false,
        recall_k: 0,
        archival_k: 0,
    };
    let result = recall_skills("anything", policy);

    // Restore env before asserting (see test 6 rationale).
    match prev_home {
        Some(h) => std::env::set_var("HOME", h),
        None => std::env::remove_var("HOME"),
    }
    match prev_db {
        Some(p) => std::env::set_var("PHANTOM_DB_PATH", p),
        None => std::env::remove_var("PHANTOM_DB_PATH"),
    }

    match result {
        Ok(recall) => {
            assert!(
                recall.skills.is_empty(),
                "missing DB must yield empty skill set"
            );
        }
        // A typed SkillError is also fine — the contract is "no panic."
        Err(_) => { /* graceful typed degrade */ }
    }
}
