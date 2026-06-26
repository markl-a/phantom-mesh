//! At-rest sealing for the owned-memory store (`hermes_memory` rows).
//!
//! Closes the P0-8 gap: the FTS5 owned-memory DB (`~/.phantom-mesh/hermes-runtime.db`)
//! historically stored `text`/`source` in the clear. This module seals those
//! columns with the SAME age v1 / `EventKey` primitive the conversation seal
//! (`crate::vault::conversation_seal`) and the SPEC-16 event store already use,
//! so there is exactly ONE encryption implementation.
//!
//! Design (P0-8, 2026-06-17):
//!   * Per-COLUMN seal of `text` + `source` (the PII-bearing owned-memory
//!     content). `kind`/`created_at`/`tags` stay plaintext so
//!     `list_by_kind`/`list_since` and tag-based verdict recall keep working.
//!     The FTS5 index is fed a de-PII'd TOKEN form (`fts_index_form`) so keyword
//!     recall survives while the verbatim sentence never enters the index pages.
//!   * Opt-in kill-switch `PHANTOM_ENCRYPT_MEMORY` — DEFAULT OFF. Off ⇒ callers
//!     write plaintext exactly as before (byte-identical, ships safe). Dedicated
//!     flag (not the conversation flag `PHANTOM_ENCRYPT_CONVERSATIONS`) so the
//!     two stores ship independently.
//!   * Auto-detecting reads: a stored value is treated as plaintext (legacy /
//!     flag-off) UNLESS the flag is ON *and* it base64-decodes to bytes that
//!     start with the age v1 magic line. This is stricter than the conversation
//!     seal's `starts_with('{')` probe because owned-memory `text` is free-form
//!     (a plaintext like `"rebase onto main"` does NOT start with `{` and must
//!     NOT be mistaken for a sealed blob — the migration-window correctness bug).
//!   * Fail CLOSED: sealing with the flag ON but no `EventKey` returns `Err` so
//!     the caller refuses the write. A value that LOOKS sealed (age-magic after
//!     b64) but won't decrypt returns `Err` — its ciphertext is NEVER surfaced
//!     as plaintext.
//!
//! v1 scope note (documented, not hidden): the tokenized-FTS form is lossy for
//! literal-*phrase* search over sealed rows (`MATCH "exact phrase"` degrades to
//! token recall). Exact-phrase recall over encrypted rows is a v2 follow-up
//! (would need a deterministic searchable-encryption scheme — out of scope for
//! "reuse existing crypto, default OFF"). With the flag OFF (the ship default)
//! FTS behaviour is 100% unchanged.

use base64::Engine as _;

/// Errors from the memory seal/open path. Deliberately coarse on the decrypt
/// side (mirrors `encryption_wire`'s oracle-leak-safe collapse): callers must
/// not branch on "wrong key" vs "tampered" vs "no key", and the error text
/// carries NEITHER plaintext NOR ciphertext.
#[derive(thiserror::Error, Debug)]
pub enum MemSealError {
    /// Flag is ON but no device `EventKey` is available, so we cannot encrypt.
    /// The caller MUST fail closed (refuse the write) — never write plaintext.
    #[error("memory seal: no EventKey available (encryption enabled but identity key missing)")]
    NoKey,
    /// age encryption failed.
    #[error("memory seal: encrypt failed: {0}")]
    Encrypt(String),
    /// A sealed value could not be decrypted (wrong key / no key / tampered).
    /// Collapsed on purpose. The plaintext is NOT recoverable and is NOT echoed.
    #[error("memory seal: open failed (undecryptable sealed value)")]
    Open,
}

/// Opt-in kill-switch. **Default OFF.** Only `PHANTOM_ENCRYPT_MEMORY` = `1`/`true`
/// (case-insensitive) enables at-rest sealing of new owned-memory writes.
pub fn memory_e2ee_enabled() -> bool {
    std::env::var("PHANTOM_ENCRYPT_MEMORY")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Seal one column value into a base64 age blob. `NoKey` ⇒ caller fails closed.
pub fn seal(plaintext: &str) -> Result<String, MemSealError> {
    let key = crate::encryption_wire::lookup_or_derive_event_key().ok_or(MemSealError::NoKey)?;
    let identity = crate::encryption_wire::event_key_to_age_identity(&key)
        .map_err(|e| MemSealError::Encrypt(format!("{e:?}")))?;
    let recipient = crate::encryption_wire::derive_recipient_from_identity(&identity);
    let envelope = crate::encryption_wire::encrypt_event(plaintext.as_bytes(), &recipient)
        .map_err(|e| MemSealError::Encrypt(format!("{e:?}")))?;
    // `encrypt_event` already base64-encodes the raw age blob; that base64 is the
    // on-disk column form.
    Ok(envelope.ciphertext_b64)
}

/// Open one stored column value back to plaintext.
///
/// Reads are migration-safe. A stored value is decrypted ONLY when the flag is
/// ON *and* `is_sealed(stored)` (it base64-decodes to age-magic bytes).
/// Otherwise it is treated as plaintext (flag-off, legacy, or a free-form
/// plaintext row written before the flag was flipped) and returned verbatim.
/// A value that probes as sealed but won't decrypt returns [`MemSealError::Open`]
/// (fail closed — the ciphertext is never returned as if it were plaintext).
pub fn open(stored: &str) -> Result<String, MemSealError> {
    if !is_sealed(stored) {
        // Plaintext (flag-off / legacy / pre-flip free-form row) — passthrough.
        return Ok(stored.to_string());
    }
    let raw = base64::engine::general_purpose::STANDARD
        .decode(stored.trim())
        .map_err(|_| MemSealError::Open)?;
    let pt = crate::encryption_wire::decrypt_raw_age_blob(&raw).map_err(|_| MemSealError::Open)?;
    String::from_utf8(pt).map_err(|_| MemSealError::Open)
}

/// `true` if `stored` is a sealed column value: it base64-decodes to a byte
/// stream beginning with the age v1 magic line. This is intentionally stricter
/// than a `!starts_with('{')` probe so a free-form plaintext memory body is
/// never mis-classified as ciphertext (the migration-window correctness bug).
pub fn is_sealed(stored: &str) -> bool {
    let t = stored.trim();
    if t.is_empty() {
        return false;
    }
    match base64::engine::general_purpose::STANDARD.decode(t) {
        Ok(raw) => crate::life_node::crypto::looks_like_age(&raw),
        Err(_) => false,
    }
}

/// De-PII'd, lowercased token stream fed to the FTS5 index when sealing is ON.
/// Keeps keyword recall working WITHOUT putting the verbatim sentence (or its
/// ciphertext) into the index pages.
pub fn fts_index_form(text: &str) -> String {
    let redacted = redact_for_index(text);
    redacted
        .split_whitespace()
        .map(|t| t.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Minimal PII scrub for the index form. Lives here (not `skill_wire`) to avoid
/// a cross-module `pub` leak; kept private. Strips emails / IPv4 / unix+windows
/// paths / @-mentions before tokenization so the FTS index never holds raw PII.
fn redact_for_index(s: &str) -> String {
    use regex::Regex;
    let patterns: &[(&str, &str)] = &[
        (r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}", " "),
        (r"\b(?:\d{1,3}\.){3}\d{1,3}\b", " "),
        (r"/(?:Users|home|opt|etc|var|tmp|usr)/[^\s]+", " "),
        (r"[A-Z]:\\[^\s]+", " "),
        (r"@[A-Za-z0-9_]{2,}", " "),
    ];
    let mut out = s.to_string();
    for (pat, repl) in patterns {
        if let Ok(re) = Regex::new(pat) {
            out = re.replace_all(&out, *repl).into_owned();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encryption_wire::{clear_event_key_cache, install_event_key_from_seed};

    // EVENT_KEY_CACHE + PHANTOM_ENCRYPT_MEMORY are process-global; serialize
    // every key/env-touching test on this mutex.
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn enabled_reads_env_default_off() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Ensure unset → default OFF.
        std::env::remove_var("PHANTOM_ENCRYPT_MEMORY");
        assert!(!memory_e2ee_enabled());
        std::env::set_var("PHANTOM_ENCRYPT_MEMORY", "1");
        assert!(memory_e2ee_enabled());
        std::env::set_var("PHANTOM_ENCRYPT_MEMORY", "TRUE");
        assert!(memory_e2ee_enabled());
        std::env::set_var("PHANTOM_ENCRYPT_MEMORY", "0");
        assert!(!memory_e2ee_enabled());
        std::env::remove_var("PHANTOM_ENCRYPT_MEMORY");
    }

    #[test]
    fn seal_open_round_trips_with_installed_key() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        install_event_key_from_seed(&[0x42u8; 32]).expect("install");
        let pt = "private owned-memory: lunch was chicken bento 雞肉便當";
        let sealed = seal(pt).expect("seal");
        assert!(is_sealed(&sealed), "sealed value must probe as sealed");
        assert_ne!(sealed, pt, "ciphertext must differ from plaintext");
        assert!(!sealed.contains("chicken"), "plaintext token leaked into blob");
        assert_eq!(open(&sealed).expect("open"), pt);
        clear_event_key_cache();
    }

    #[test]
    fn seal_fails_closed_without_key() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_event_key_cache();
        // In #[cfg(test)], lookup_or_derive never reads the real identity.key,
        // so an empty cache yields NoKey — fail closed.
        assert!(matches!(seal("x"), Err(MemSealError::NoKey)));
    }

    #[test]
    fn open_passes_through_plaintext() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // JSON-ish legacy line.
        let json = r#"{"legacy":"row"}"#;
        assert_eq!(open(json).unwrap(), json);
        assert!(!is_sealed(json));
        // Empty.
        assert_eq!(open("").unwrap(), "");
        assert!(!is_sealed(""));
        // Free-form plaintext that does NOT start with '{' — the migration-window
        // edge case. Must passthrough, never be treated as ciphertext.
        let freeform = "rebase onto main feature branch";
        assert!(!is_sealed(freeform), "plaintext must not probe as sealed");
        assert_eq!(open(freeform).unwrap(), freeform);
    }

    #[test]
    fn open_wrong_key_fails_closed() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        install_event_key_from_seed(&[0x11u8; 32]).unwrap();
        let sealed = seal("secret").unwrap();
        clear_event_key_cache();
        install_event_key_from_seed(&[0x22u8; 32]).unwrap(); // different key
        let err = open(&sealed);
        assert!(
            matches!(err, Err(MemSealError::Open)),
            "wrong key must fail closed, never return ciphertext as plaintext"
        );
        // The error text must carry neither plaintext nor ciphertext.
        let msg = format!("{}", err.unwrap_err());
        assert!(!msg.contains("secret"));
        assert!(!msg.contains(&sealed));
        clear_event_key_cache();
    }

    #[test]
    fn fts_index_form_strips_pii_and_keeps_keywords() {
        let s = "email me at a@b.com about rebase onto main, ip 10.0.0.1";
        let f = fts_index_form(s);
        assert!(f.contains("rebase") && f.contains("main"));
        assert!(!f.contains("a@b.com") && !f.contains("10.0.0.1"));
    }
}
