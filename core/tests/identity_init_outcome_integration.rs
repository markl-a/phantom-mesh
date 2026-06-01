//! Integration test — `identity_wire::build_init_outcome` Stage 3 / Stage 4 path.
//!
//! Phase G2 follow-up (2026-05-26): the legacy `From<identity::InitOutcome>`
//! bridge was deleted (see `docs/superpowers/phase-g-init-outcome-notes.md`);
//! the wire-side `build_init_outcome` is now the single canonical path
//! exposed to Tauri / RPC. This binary asserts:
//!
//!   1. `InitOutcome` round-trips through serde with the canonical camelCase
//!      field names — the wire contract UI / TS code keys off.
//!   2. All six `KeystoreBackend` slugs are non-empty, lower-kebab, distinct,
//!      and two anchor slugs (`linux-secret-service`, `file-chmod-0600`) are
//!      pinned to their exact strings.
//!   3. `fingerprint_short` is 12 lower-hex chars, deterministic, and
//!      distinct across distinct inputs.
//!   4. (Linux + Secret Service only, `#[ignore]`-gated) Full
//!      `build_init_outcome(force)` end-to-end cycle: first init creates,
//!      re-init without force is idempotent, re-init with force rotates,
//!      cleanup via `delete_from_keystore` succeeds and `read_from_keystore`
//!      then surfaces `MasterNotFound` per the §11 error catalog.
//!
//! The four native-keystore arms (macOS / iOS Keychain via `security-framework`,
//! Android via `jni`, Windows DPAPI via `windows-rs`) are still
//! `unimplemented!("Stage 4: …")`, so an end-to-end exercise can only run on
//! Linux today. The Linux test stays `#[ignore]`-gated so CI (no D-Bus session
//! bus) and headless servers (locked default collection would block on the
//! desktop secret-agent prompt) don't fail. Run manually on a Linux desktop
//! with an unlocked keyring:
//!
//!     cargo test --test identity_init_outcome_integration -- --ignored --nocapture

use phantom_mesh::identity_wire::{
    fingerprint_short, InitOutcome, KeystoreBackend,
};

// ─── 1/4 — InitOutcome serde shape ───────────────────────────────────────────

#[test]
fn init_outcome_round_trips_in_camelcase() {
    let original = InitOutcome {
        created: true,
        fingerprint: "abc123def456".to_string(),
        public_key_hex: "00".repeat(32),
        keystore_backend: KeystoreBackend::FileChmod0600.name().to_string(),
        initialized_at: "2026-05-25T00:00:00Z".to_string(),
    };
    let json = serde_json::to_string(&original).expect("serialise InitOutcome");

    // §6.2 wire contract: camelCase field names exposed to TS / Tauri.
    assert!(json.contains("\"publicKeyHex\""), "wire field name: {json}");
    assert!(json.contains("\"keystoreBackend\""), "wire field name: {json}");
    assert!(json.contains("\"initializedAt\""), "wire field name: {json}");
    assert!(
        !json.contains("\"public_key_hex\""),
        "wire MUST be camelCase, not snake_case: {json}"
    );

    let back: InitOutcome = serde_json::from_str(&json).expect("deserialise InitOutcome");
    assert_eq!(original.created, back.created);
    assert_eq!(original.fingerprint, back.fingerprint);
    assert_eq!(original.public_key_hex, back.public_key_hex);
    assert_eq!(original.keystore_backend, back.keystore_backend);
    assert_eq!(original.initialized_at, back.initialized_at);
}

// ─── 2/4 — KeystoreBackend slugs ─────────────────────────────────────────────

#[test]
fn keystore_backend_slugs_are_stable_lower_kebab_and_distinct() {
    // §7.3 wire contract: each backend has a stable lower-kebab slug carried
    // verbatim in `InitOutcome.keystore_backend`. Any rename is a wire break.
    let slugs = [
        KeystoreBackend::MacosKeychain.name(),
        KeystoreBackend::IosKeychain.name(),
        KeystoreBackend::AndroidEncryptedSharedPreferences.name(),
        KeystoreBackend::WindowsCredentialManager.name(),
        KeystoreBackend::LinuxSecretService.name(),
        KeystoreBackend::FileChmod0600.name(),
    ];
    for s in &slugs {
        assert!(!s.is_empty(), "slug must be non-empty");
        assert!(
            s.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
            "slug `{s}` not lower-kebab — wire contract break"
        );
    }
    // All distinct (no two backends collide on the same slug).
    let mut sorted: Vec<&str> = slugs.to_vec();
    sorted.sort();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        slugs.len(),
        "duplicate slug across keystore backends"
    );

    // Pin two anchor strings — UI / TS code matches against these literals.
    assert_eq!(
        KeystoreBackend::LinuxSecretService.name(),
        "linux-secret-service"
    );
    assert_eq!(KeystoreBackend::FileChmod0600.name(), "file-chmod-0600");
}

// ─── 3/4 — fingerprint_short structural invariant ────────────────────────────

#[test]
fn fingerprint_short_is_12_lowercase_hex_and_deterministic() {
    // §7.1 wire promise: every fingerprint exposed to UI is exactly 12
    // lower-case hex chars. Re-pinned at the integration level so a
    // regression to a different truncation (8 / 16 / …) trips here too.
    let fp = fingerprint_short(&[0u8; 32]);
    assert_eq!(fp.len(), 12, "fingerprint must be 12 hex chars per §7.1");
    assert!(
        fp.chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
        "fingerprint must be lower-case hex: got {fp}"
    );

    // Deterministic — same input always yields same output.
    assert_eq!(fp, fingerprint_short(&[0u8; 32]));

    // Distinct inputs → distinct fingerprints (collision-resistance smoke).
    assert_ne!(fp, fingerprint_short(&[1u8; 32]));
}

// ─── 4/4 — Live build_init_outcome cycle (Linux + Secret Service, ignored) ──
//
// Stage 3 → Stage 4 readiness check on the only host where the native
// keystore arm is live: `secret-service = "5"` + libsecret default
// collection. All non-Linux hosts have `unimplemented!()` in their
// dispatch arms today, so this block is `#[cfg(target_os = "linux")]`-gated
// at compile time and `#[ignore]`-gated at run time.
//
// WARNING for operators: `build_init_outcome` writes under the canonical
// account `"identity-master"` — there is no public API in Stage 3 to
// override the account. Running this test on a Linux box with a real
// identity will **clobber it** on the `force=true` branch. Back up first:
//     phantom keys backup --to /tmp/backup-pre-test.json

#[cfg(target_os = "linux")]
#[test]
#[ignore]
fn live_build_init_outcome_full_cycle_linux() {
    use phantom_mesh::identity_wire::{
        build_init_outcome, delete_from_keystore, read_from_keystore,
        KeyDerivationError,
    };

    eprintln!(
        "[isolation] build_init_outcome writes account=\"identity-master\"; \
         run on a dev machine or back up first."
    );

    // Step 1: force-init up front so the remainder of the test sees
    // deterministic state regardless of whether a prior identity exists.
    let first =
        build_init_outcome(true).expect("force init must succeed on Linux Secret Service");
    assert!(first.created, "force init must report created=true: {first:?}");
    assert_eq!(
        first.fingerprint.len(),
        12,
        "fingerprint must be 12 hex: {}",
        first.fingerprint
    );
    assert_eq!(
        first.public_key_hex.len(),
        64,
        "public_key_hex must be 64 hex: {}",
        first.public_key_hex
    );
    assert!(
        first.public_key_hex.chars().all(|c| c.is_ascii_hexdigit()),
        "public_key_hex must be all-hex: {}",
        first.public_key_hex
    );
    assert_eq!(
        first.keystore_backend, "linux-secret-service",
        "Linux host must pick Secret Service backend: {first:?}"
    );
    assert!(
        first.initialized_at.ends_with('Z'),
        "initialized_at must be UTC RFC 3339: {}",
        first.initialized_at
    );

    // Step 2: idempotent re-init without force. Same fingerprint + pubkey.
    let again = build_init_outcome(false).expect("re-init without force must succeed");
    assert!(
        !again.created,
        "re-init without force must be idempotent (created=false): {again:?}"
    );
    assert_eq!(again.fingerprint, first.fingerprint);
    assert_eq!(again.public_key_hex, first.public_key_hex);
    assert_eq!(again.keystore_backend, first.keystore_backend);

    // Step 3: force-rotate — fresh fingerprint + fresh pubkey.
    let rotated = build_init_outcome(true).expect("force re-init must succeed");
    assert!(rotated.created, "force re-init must report created=true");
    assert_ne!(
        rotated.fingerprint, first.fingerprint,
        "force must rotate fingerprint"
    );
    assert_ne!(
        rotated.public_key_hex, first.public_key_hex,
        "force must rotate public_key_hex"
    );

    // Step 4: cleanup via delete_from_keystore — confirms the §6.3
    // fingerprint-gate works AND that the wire surface is the canonical
    // delete path (not the deleted legacy bridge). Pass the rotated fp.
    delete_from_keystore(
        KeystoreBackend::LinuxSecretService,
        "identity-master",
        &rotated.fingerprint,
    )
    .expect("delete with matching fingerprint must succeed");

    // Read after delete must surface MasterNotFound per §11 catalog.
    match read_from_keystore(KeystoreBackend::LinuxSecretService, "identity-master") {
        Err(KeyDerivationError::MasterNotFound) => {} // expected
        other => panic!("read after delete must be MasterNotFound, got {other:?}"),
    }
}
