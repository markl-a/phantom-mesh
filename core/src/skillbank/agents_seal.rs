//! At-rest sealing for `agents.toml` provider API keys (`[providers.<name>].api_key`).
//!
//! Closes the apex P4「加密為先」gap: `agents.toml` holds provider **API keys**
//! (secrets) that are read at startup, but they were historically stored in
//! PLAINTEXT while `events/` + `memory.db` already encrypt at rest. This is a
//! thin sealing layer wrapped around the EXISTING load/save of `agents.toml` —
//! it does NOT rewrite the config parser and invents NO new crypto.
//!
//! Design (mirrors `skillbank::memory_seal` EXACTLY):
//!   * Reuses `memory_seal::{seal, open, is_sealed}` — i.e. the SAME age v1 /
//!     `EventKey` primitive (key derived from `~/.spectyn-mesh/identity.key` via
//!     `encryption_wire`) that the owned-memory store, the conversation seal, and
//!     the SPEC-16 event store already use. There is exactly ONE encryption
//!     implementation; this module adds none.
//!   * Opt-in kill-switch `SPECTYN_ENCRYPT_AGENTS` — **DEFAULT OFF**. Off ⇒
//!     load/save are byte-identical to today (plaintext exactly as before) and
//!     the new code paths are pure no-ops. A DEDICATED flag (not the memory flag
//!     `SPECTYN_ENCRYPT_MEMORY`) so the two stores ship independently.
//!   * Seal-on-save (when ON): [`seal_api_key_for_save`] seals the provider key
//!     immediately before it is written by [`crate::keys::set_api_key`] (the
//!     canonical provider-key writer). Empty / already-sealed values pass through
//!     unchanged (idempotent — a double save never double-seals).
//!   * Unseal-on-load (when ON): [`unseal_on_load`] runs right after
//!     `toml::from_str` in [`crate::config::AgentsConfig::load_path`]. Per field:
//!     `is_sealed` ⇒ `open` (decrypt) in place, else passthrough (a plaintext key
//!     written before the flag was flipped migrates smoothly).
//!   * **Fail CLOSED**: sealing with the flag ON but no `EventKey` returns `Err`
//!     so the write is REFUSED (never silently persist plaintext). A field that
//!     probes as sealed but won't decrypt (no key / wrong key / tampered) returns
//!     `Err` from load — its ciphertext is NEVER surfaced as the in-memory key.
//!
//! Scope: every API-key/token secret string in `agents.toml` is sealed via the
//! ONE seam above — the provider struct's `ProviderEntry.api_key` AND the other
//! secret-bearing fields outside it: `[tools].brave_search_api_key`,
//! `[tools].todoist_api_token`, and `[core].hub_api_key`. They all seal on save
//! ([`seal_api_key_for_save`], called by the `keys.rs` writers) and unseal on
//! load ([`unseal_on_load`]) under the SAME `SPECTYN_ENCRYPT_AGENTS` flag, with
//! the SAME age/`EventKey` primitive — no per-field mechanism. With the flag OFF
//! (the ship default) behaviour is 100% unchanged.

use crate::config::AgentsConfig;
use crate::skillbank::memory_seal::{self, MemSealError};

/// Opt-in kill-switch. **Default OFF.** Only `SPECTYN_ENCRYPT_AGENTS` = `1` /
/// `true` (case-insensitive) enables at-rest sealing of `agents.toml` provider
/// API keys. Deliberately mirrors `memory_seal::memory_e2ee_enabled` exactly.
pub fn agents_e2ee_enabled() -> bool {
    std::env::var("SPECTYN_ENCRYPT_AGENTS")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Transform a provider `api_key` value into its at-rest form, just before it is
/// written to disk.
///
/// * Flag OFF ⇒ returns `plaintext` unchanged (byte-identical to today).
/// * Empty ⇒ unchanged (nothing to protect; never seal `""`).
/// * Already sealed ⇒ unchanged (idempotent — re-saving a value that is still
///   sealed, or a double save, never double-seals).
/// * Otherwise (flag ON, non-empty plaintext) ⇒ sealed via `memory_seal::seal`.
///   Returns `Err(MemSealError::NoKey)` when no `EventKey` is available — the
///   caller MUST fail closed (refuse the write), never persist plaintext.
pub fn seal_api_key_for_save(plaintext: &str) -> Result<String, MemSealError> {
    if !agents_e2ee_enabled() || plaintext.is_empty() || memory_seal::is_sealed(plaintext) {
        return Ok(plaintext.to_string());
    }
    memory_seal::seal(plaintext)
}

/// Decrypt one optional secret field in place when (and only when) it is sealed.
///
/// * `None` ⇒ untouched (nothing to decrypt).
/// * Plaintext ⇒ untouched (smooth migration of a value written before the flag
///   was flipped — `is_sealed` is false for it).
/// * Sealed ⇒ `open`ed (decrypted) in place. A sealed value that won't decrypt
///   (no key / wrong key / tampered) propagates the fail-closed `Err` WITHOUT
///   leaving a half-updated field (the `take` is only restored as plaintext).
fn unseal_field(field: &mut Option<String>) -> Result<(), MemSealError> {
    let sealed = field
        .as_deref()
        .map(memory_seal::is_sealed)
        .unwrap_or(false);
    if sealed {
        // `sealed` implies `Some`; take to satisfy the borrow checker, then
        // replace with the decrypted plaintext (or propagate the fail-closed
        // error without leaving ciphertext behind).
        let stored = field.take().expect("field present when sealed");
        *field = Some(memory_seal::open(&stored)?);
    }
    Ok(())
}

/// Unseal every sealed secret field in `agents.toml` in place, right after the
/// config is parsed from disk: provider `api_key`s AND the `[tools]` / `[core]`
/// secret fields (`brave_search_api_key`, `todoist_api_token`, `hub_api_key`).
///
/// * Flag OFF ⇒ no-op (byte-identical to today; the parsed struct is untouched).
/// * Flag ON ⇒ each sealed field `is_sealed` is `open`ed (decrypted) in place; a
///   plaintext value passes through (smooth migration of a value written before
///   the flag was flipped).
/// * **Fail closed**: a sealed value that won't decrypt (no key / wrong key /
///   tampered) returns `Err`, so the load surfaces an error instead of handing
///   back ciphertext as if it were the secret.
pub fn unseal_on_load(cfg: &mut AgentsConfig) -> Result<(), MemSealError> {
    if !agents_e2ee_enabled() {
        return Ok(());
    }
    for entry in cfg.providers.values_mut() {
        unseal_field(&mut entry.api_key)?;
    }
    // apex P4 follow-up: the remaining secret-bearing fields seal via the SAME
    // seam/flag/primitive — no new mechanism, just more fields through one path.
    unseal_field(&mut cfg.tools.brave_search_api_key)?;
    unseal_field(&mut cfg.tools.todoist_api_token)?;
    unseal_field(&mut cfg.core.hub_api_key)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // IMPORTANT: these lib unit tests deliberately NEVER set
    // `SPECTYN_ENCRYPT_AGENTS` to ON. The flag is process-global and is read by
    // `config::AgentsConfig::load_path` + `keys::set_api_key` on EVERY call, so
    // flipping it ON here could make a *concurrent* keys.rs/config.rs unit test
    // try to seal with no key (`#[cfg(test)]` has no identity.key) and panic on
    // its `.unwrap()`. All ON / crypto-round-trip / fail-closed coverage lives in
    // the isolated, single-threaded `core/tests/agents_toml_seal_at_rest.rs`,
    // which owns its own process + env. Here we only pin the default-OFF /
    // no-op-when-off contract, which needs no key and never sets the flag.

    fn cfg_with_key(provider: &str, api_key: &str) -> AgentsConfig {
        let toml_str =
            format!("[providers.{provider}]\ntype = \"{provider}\"\napi_key = \"{api_key}\"\n");
        toml::from_str::<AgentsConfig>(&toml_str).unwrap()
    }

    #[test]
    fn enabled_is_false_when_unset() {
        // Default OFF when the var is absent (the ship default). We do NOT assert
        // the ON case here — see the module-level note on cross-test env safety.
        if std::env::var_os("SPECTYN_ENCRYPT_AGENTS").is_none() {
            assert!(!agents_e2ee_enabled());
        }
    }

    #[test]
    fn save_is_passthrough_when_off() {
        // OFF ⇒ identity transform, no key needed, value never sealed.
        if !agents_e2ee_enabled() {
            let pt = "sk-plaintext-OFF-12345";
            assert_eq!(seal_api_key_for_save(pt).unwrap(), pt);
            assert!(!memory_seal::is_sealed(&seal_api_key_for_save(pt).unwrap()));
        }
    }

    #[test]
    fn unseal_is_noop_when_off() {
        // A value left in the struct stays verbatim when the flag is off — no
        // decrypt attempt, no key needed, byte-identical to today.
        if !agents_e2ee_enabled() {
            let mut cfg = cfg_with_key("anthropic", "sk-untouched-OFF");
            unseal_on_load(&mut cfg).unwrap();
            assert_eq!(
                cfg.providers.get("anthropic").unwrap().api_key.as_deref(),
                Some("sk-untouched-OFF")
            );
        }
    }
}
