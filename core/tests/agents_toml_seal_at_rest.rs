//! apex P4 — at-rest sealing for `agents.toml` provider API keys
//! (`SPECTYN_ENCRYPT_AGENTS`), anti-fake-green hermetic coverage.
//!
//! Runs against the PUBLIC crate API only — `keys::set_api_key` (the canonical
//! provider-key writer / save seam) and `AgentsConfig::load_path` (the load
//! seam) — plus raw on-disk inspection. It exercises the REAL device-key path:
//! a generated `identity.key` is written into a temp `SPECTYN_HOME` and the lib
//! derives the `EventKey` from it exactly as in production (the integration
//! build does NOT get the lib's `#[cfg(test)]` "never read identity.key" guard).
//!
//! Safety: every test sets `SPECTYN_HOME` to its own temp dir BEFORE any crypto
//! call and clears the process-global `EventKey` cache, so the operator's real
//! `~/.spectyn-mesh/identity.key` is NEVER read or written.
//!
//! Process-global state (the `EventKey` cache + the `SPECTYN_ENCRYPT_AGENTS` env
//! flag) means these MUST run single-threaded — the harness is invoked with
//! `-- --test-threads=1`, and an in-file serial lock is belt-and-suspenders.
//!
//! Not feature-gated on purpose: the sealing layer is always-compiled, so this
//! runs (and must pass) in the DEFAULT build that ships.

use spectyn_mesh::config::AgentsConfig;
use spectyn_mesh::encryption_wire::clear_event_key_cache;
use spectyn_mesh::skillbank::memory_seal;
use spectyn_mesh::keys;

/// Serialize all key/env-touching tests on one process-global mutex.
fn serial_lock() -> std::sync::MutexGuard<'static, ()> {
    use std::sync::{Mutex, OnceLock};
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Point the data-root at an isolated temp dir so the lib's identity-key path can
/// never reach the operator's real `~/.spectyn-mesh`.
fn isolate_home(td: &std::path::Path) {
    std::env::set_var("SPECTYN_HOME", td);
}

/// Write a generated 64-byte `identity.key` (the lib derives the EventKey from
/// its first 32 bytes). A distinct `fill` byte ⇒ a distinct device key.
fn write_identity_key(td: &std::path::Path, fill: u8) {
    std::fs::write(td.join("identity.key"), vec![fill; 64]).expect("write identity.key");
}

/// Reset the process-global state a test mutated.
fn teardown() {
    std::env::remove_var("SPECTYN_ENCRYPT_AGENTS");
    std::env::remove_var("SPECTYN_HOME");
    clear_event_key_cache();
}

/// Pull the raw on-disk `api_key = "<value>"` string out of a TOML file WITHOUT
/// going through the unsealing loader — a sealed value is base64 (no `"` in the
/// alphabet), so a simple quote scan is exact.
fn raw_api_key(contents: &str) -> String {
    raw_field(contents, "api_key")
}

/// Generalized form of [`raw_api_key`] for any `<field> = "<value>"` line — used
/// to inspect the on-disk `[tools]` / `[core]` secret fields without unsealing.
fn raw_field(contents: &str, field: &str) -> String {
    let marker = format!("{field} = \"");
    let start = contents
        .find(&marker)
        .unwrap_or_else(|| panic!("{field} present"))
        + marker.len();
    let rest = &contents[start..];
    let end = rest.find('"').expect("closing quote");
    rest[..end].to_string()
}

/// (1) ON: a provider api_key is SEALED on disk (no plaintext present) and the
/// real load path decrypts it back to the original plaintext (round-trip).
#[test]
fn on_round_trips_and_seals_provider_key_on_disk() {
    let _g = serial_lock();
    let td = tempfile::tempdir().unwrap();
    isolate_home(td.path());
    write_identity_key(td.path(), 0x42);
    clear_event_key_cache();
    std::env::set_var("SPECTYN_ENCRYPT_AGENTS", "1");

    let path = td.path().join("agents.toml");
    const SECRET: &str = "sk-ant-PLAINTEXT-NEEDLE-顧客機密-0xDEADBEEF";
    keys::set_api_key(&path, "anthropic", SECRET).expect("set_api_key should seal, not fail");

    // On-disk: the plaintext key must NOT appear; the stored value IS sealed.
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(
        !on_disk.contains(SECRET),
        "plaintext api_key leaked onto disk:\n{on_disk}"
    );
    let stored = raw_api_key(&on_disk);
    assert_ne!(stored, SECRET, "stored value must differ from plaintext");
    assert!(
        memory_seal::is_sealed(&stored),
        "stored api_key must be a sealed age blob"
    );

    // Load back through the REAL load path → in-memory key == original plaintext.
    let cfg = AgentsConfig::load_path(&path).expect("load round-trip should decrypt");
    assert_eq!(
        cfg.providers
            .get("anthropic")
            .and_then(|p| p.api_key.as_deref()),
        Some(SECRET),
        "decrypted in-memory key must equal the original plaintext"
    );

    teardown();
}

/// (2) OFF (default): the on-disk agents.toml is byte-identical PLAINTEXT — the
/// sealing layer is a pure no-op, exactly today's behavior.
#[test]
fn off_is_byte_identical_plaintext() {
    let _g = serial_lock();
    let td = tempfile::tempdir().unwrap();
    isolate_home(td.path());
    // A device key IS available — proving OFF seals nothing even when it could.
    write_identity_key(td.path(), 0x55);
    clear_event_key_cache();
    std::env::remove_var("SPECTYN_ENCRYPT_AGENTS"); // explicitly OFF (ship default)

    let path = td.path().join("agents.toml");
    const KEY: &str = "gsk_PLAINTEXT_off_path_67890";
    keys::set_api_key(&path, "groq", KEY).expect("set_api_key (off)");

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(
        on_disk.contains(&format!("api_key = \"{KEY}\"")),
        "OFF must write the literal plaintext key:\n{on_disk}"
    );
    let stored = raw_api_key(&on_disk);
    assert_eq!(stored, KEY, "OFF must store the key verbatim (no transform)");
    assert!(!memory_seal::is_sealed(&stored), "OFF must never seal");

    // Determinism / byte-identical write: a second OFF write of the same inputs
    // yields a byte-for-byte identical file (the seal layer contributes nothing).
    let path2 = td.path().join("agents2.toml");
    keys::set_api_key(&path2, "groq", KEY).expect("set_api_key (off) #2");
    assert_eq!(
        std::fs::read(&path).unwrap(),
        std::fs::read(&path2).unwrap(),
        "OFF write must be byte-identical plaintext"
    );

    // Load back with the flag OFF → no unseal performed, key returned verbatim.
    let cfg = AgentsConfig::load_path(&path).expect("load (off)");
    assert_eq!(
        cfg.providers.get("groq").and_then(|p| p.api_key.as_deref()),
        Some(KEY)
    );

    teardown();
}

/// (3a) FAIL-CLOSED: a sealed field + the WRONG device key → load returns Err
/// (never surfaces ciphertext as the key, never carries plaintext in the error).
#[test]
fn sealed_field_wrong_key_fails_closed_on_load() {
    let _g = serial_lock();
    let td = tempfile::tempdir().unwrap();
    isolate_home(td.path());
    write_identity_key(td.path(), 0x11);
    clear_event_key_cache();
    std::env::set_var("SPECTYN_ENCRYPT_AGENTS", "1");

    let path = td.path().join("agents.toml");
    const SECRET: &str = "sk-ant-FAILCLOSED-NEEDLE-99999";
    keys::set_api_key(&path, "anthropic", SECRET).expect("seal with key A");

    // Rotate the device identity to a DIFFERENT key + drop the cache so the next
    // open re-derives from the new identity.key.
    clear_event_key_cache();
    write_identity_key(td.path(), 0x22);

    let res = AgentsConfig::load_path(&path);
    assert!(
        res.is_err(),
        "sealed key + wrong device key must fail closed (Err), got Ok"
    );
    let msg = format!("{}", res.unwrap_err());
    assert!(
        !msg.contains(SECRET),
        "error text must not carry the plaintext: {msg}"
    );

    teardown();
}

/// (3b) FAIL-CLOSED: a sealed field + NO device key → load returns Err (never a
/// silent empty/garbage key).
#[test]
fn sealed_field_missing_key_fails_closed_on_load() {
    let _g = serial_lock();
    let td = tempfile::tempdir().unwrap();
    isolate_home(td.path());
    write_identity_key(td.path(), 0x33);
    clear_event_key_cache();
    std::env::set_var("SPECTYN_ENCRYPT_AGENTS", "1");

    let path = td.path().join("agents.toml");
    const SECRET: &str = "sk-ant-NOKEY-NEEDLE-77777";
    keys::set_api_key(&path, "anthropic", SECRET).expect("seal");

    // Remove the device key entirely + drop the cache → no key available at all.
    clear_event_key_cache();
    std::fs::remove_file(td.path().join("identity.key")).unwrap();

    let res = AgentsConfig::load_path(&path);
    assert!(
        res.is_err(),
        "sealed key + no device key must fail closed (Err), never an empty key"
    );

    teardown();
}

// ── apex P4 follow-up: [tools] + [core] secret fields (same seam/flag/crypto) ──

/// (4) ON: the `[tools]` (brave_search_api_key / todoist_api_token) and `[core]`
/// (hub_api_key) secrets are SEALED on disk (no plaintext present) and the real
/// load path decrypts each back to its original plaintext (round-trip).
#[test]
fn on_round_trips_and_seals_tools_and_core_secrets_on_disk() {
    let _g = serial_lock();
    let td = tempfile::tempdir().unwrap();
    isolate_home(td.path());
    write_identity_key(td.path(), 0x43);
    clear_event_key_cache();
    std::env::set_var("SPECTYN_ENCRYPT_AGENTS", "1");

    let path = td.path().join("agents.toml");
    const BRAVE: &str = "brave-PLAINTEXT-NEEDLE-顧客機密-0xBEEF01";
    const TODOIST: &str = "todoist-PLAINTEXT-NEEDLE-0xBEEF02";
    const HUB: &str = "hub-PLAINTEXT-NEEDLE-0xBEEF03";
    keys::set_table_secret(&path, "tools", "brave_search_api_key", BRAVE)
        .expect("set_table_secret should seal brave_search_api_key, not fail");
    keys::set_table_secret(&path, "tools", "todoist_api_token", TODOIST)
        .expect("set_table_secret should seal todoist_api_token, not fail");
    keys::set_table_secret(&path, "core", "hub_api_key", HUB)
        .expect("set_table_secret should seal hub_api_key, not fail");

    // On-disk: NONE of the plaintext secrets may appear; each stored value IS sealed.
    let on_disk = std::fs::read_to_string(&path).unwrap();
    for secret in [BRAVE, TODOIST, HUB] {
        assert!(
            !on_disk.contains(secret),
            "plaintext secret leaked onto disk:\n{on_disk}"
        );
    }
    for field in ["brave_search_api_key", "todoist_api_token", "hub_api_key"] {
        let stored = raw_field(&on_disk, field);
        assert_ne!(stored, "", "{field} must be present on disk");
        assert!(
            memory_seal::is_sealed(&stored),
            "stored {field} must be a sealed age blob, got: {stored}"
        );
    }

    // Load back through the REAL load path → every in-memory field == original.
    let cfg = AgentsConfig::load_path(&path).expect("load round-trip should decrypt");
    assert_eq!(
        cfg.tools.brave_search_api_key.as_deref(),
        Some(BRAVE),
        "decrypted brave_search_api_key must equal the original plaintext"
    );
    assert_eq!(
        cfg.tools.todoist_api_token.as_deref(),
        Some(TODOIST),
        "decrypted todoist_api_token must equal the original plaintext"
    );
    assert_eq!(
        cfg.core.hub_api_key.as_deref(),
        Some(HUB),
        "decrypted hub_api_key must equal the original plaintext"
    );

    teardown();
}

/// (5) OFF (default): the `[tools]` / `[core]` secrets are written as
/// byte-identical PLAINTEXT — the sealing layer is a pure no-op even though a
/// device key is available.
#[test]
fn off_tools_and_core_secrets_are_byte_identical_plaintext() {
    let _g = serial_lock();
    let td = tempfile::tempdir().unwrap();
    isolate_home(td.path());
    // A device key IS available — proving OFF seals nothing even when it could.
    write_identity_key(td.path(), 0x56);
    clear_event_key_cache();
    std::env::remove_var("SPECTYN_ENCRYPT_AGENTS"); // explicitly OFF (ship default)

    let path = td.path().join("agents.toml");
    const BRAVE: &str = "brave_PLAINTEXT_off_111";
    const HUB: &str = "hub_PLAINTEXT_off_222";
    keys::set_table_secret(&path, "tools", "brave_search_api_key", BRAVE)
        .expect("set_table_secret (off) brave");
    keys::set_table_secret(&path, "core", "hub_api_key", HUB)
        .expect("set_table_secret (off) hub");

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(
        on_disk.contains(&format!("brave_search_api_key = \"{BRAVE}\"")),
        "OFF must write the literal plaintext secret:\n{on_disk}"
    );
    assert!(
        on_disk.contains(&format!("hub_api_key = \"{HUB}\"")),
        "OFF must write the literal plaintext secret:\n{on_disk}"
    );
    assert!(
        !memory_seal::is_sealed(&raw_field(&on_disk, "brave_search_api_key")),
        "OFF must never seal brave_search_api_key"
    );
    assert!(
        !memory_seal::is_sealed(&raw_field(&on_disk, "hub_api_key")),
        "OFF must never seal hub_api_key"
    );

    // Determinism / byte-identical write: a second OFF write of the same inputs
    // yields a byte-for-byte identical file (the seal layer contributes nothing).
    let path2 = td.path().join("agents2.toml");
    keys::set_table_secret(&path2, "tools", "brave_search_api_key", BRAVE)
        .expect("set_table_secret (off) brave #2");
    keys::set_table_secret(&path2, "core", "hub_api_key", HUB)
        .expect("set_table_secret (off) hub #2");
    assert_eq!(
        std::fs::read(&path).unwrap(),
        std::fs::read(&path2).unwrap(),
        "OFF write must be byte-identical plaintext"
    );

    // Load back with the flag OFF → no unseal performed, secrets returned verbatim.
    let cfg = AgentsConfig::load_path(&path).expect("load (off)");
    assert_eq!(cfg.tools.brave_search_api_key.as_deref(), Some(BRAVE));
    assert_eq!(cfg.core.hub_api_key.as_deref(), Some(HUB));

    teardown();
}

/// (6a) FAIL-CLOSED: a sealed `[tools]` field + the WRONG device key → load
/// returns Err (never surfaces ciphertext as the secret, never leaks plaintext
/// into the error).
#[test]
fn sealed_tools_field_wrong_key_fails_closed_on_load() {
    let _g = serial_lock();
    let td = tempfile::tempdir().unwrap();
    isolate_home(td.path());
    write_identity_key(td.path(), 0x13);
    clear_event_key_cache();
    std::env::set_var("SPECTYN_ENCRYPT_AGENTS", "1");

    let path = td.path().join("agents.toml");
    const SECRET: &str = "brave-FAILCLOSED-NEEDLE-44444";
    keys::set_table_secret(&path, "tools", "brave_search_api_key", SECRET).expect("seal with key A");

    // Rotate the device identity to a DIFFERENT key + drop the cache so the next
    // open re-derives from the new identity.key.
    clear_event_key_cache();
    write_identity_key(td.path(), 0x24);

    let res = AgentsConfig::load_path(&path);
    assert!(
        res.is_err(),
        "sealed tools field + wrong device key must fail closed (Err), got Ok"
    );
    let msg = format!("{}", res.unwrap_err());
    assert!(
        !msg.contains(SECRET),
        "error text must not carry the plaintext: {msg}"
    );

    teardown();
}

/// (6b) FAIL-CLOSED: a sealed `[core]` field + NO device key → load returns Err
/// (never a silent empty/garbage secret).
#[test]
fn sealed_core_field_missing_key_fails_closed_on_load() {
    let _g = serial_lock();
    let td = tempfile::tempdir().unwrap();
    isolate_home(td.path());
    write_identity_key(td.path(), 0x34);
    clear_event_key_cache();
    std::env::set_var("SPECTYN_ENCRYPT_AGENTS", "1");

    let path = td.path().join("agents.toml");
    const SECRET: &str = "hub-NOKEY-NEEDLE-88888";
    keys::set_table_secret(&path, "core", "hub_api_key", SECRET).expect("seal");

    // Remove the device key entirely + drop the cache → no key available at all.
    clear_event_key_cache();
    std::fs::remove_file(td.path().join("identity.key")).unwrap();

    let res = AgentsConfig::load_path(&path);
    assert!(
        res.is_err(),
        "sealed core field + no device key must fail closed (Err), never an empty secret"
    );

    teardown();
}
