// core/src/vault/conversation_seal.rs
//
// At-rest sealing for conversation history (E004 gap). Conversation history
// lives at `~/.spectyn-mesh/conversations/<id>.jsonl` (see `crate::session`),
// historically written in the clear. This module seals each JSONL line with the
// existing age v1 / EventKey primitive so the file is unreadable without the
// device identity key — while staying fully backward-compatible.
//
// Design (per the corrected vault-encrypt-at-rest spec, 2026-06-16):
//
//   * Per-LINE seal, not whole-file: each line is sealed independently so the
//     append-only O(1) write path, the atomic `.tmp`+rename rewrite, and `fork`
//     (a raw file copy) all keep working unchanged. A sealed line is the
//     base64 age blob emitted by `encryption_wire::encrypt_event`.
//
//   * Opt-in kill-switch `SPECTYN_ENCRYPT_CONVERSATIONS` — DEFAULT OFF. With it
//     off, callers write plaintext exactly as before (byte-identical, ships
//     safe). This is a dedicated flag (not the SPEC-15 broker `SPECTYN_VAULT_E2EE`,
//     which is a different subsystem) because conversation history must never
//     silently change format for existing users.
//
//   * Auto-detecting reads: a line that starts with `{` is plaintext JSON and is
//     passed through verbatim, so legacy history (and kill-switch-off history)
//     still loads. Any other line is treated as a sealed blob and decrypted.
//
//   * Fail CLOSED: sealing with the kill-switch ON but no EventKey available
//     returns `Err` so the caller can refuse the write rather than silently
//     persist plaintext. A sealed line that fails to decrypt returns `Err` and
//     its ciphertext is NEVER surfaced as plaintext content.
//
// The crypto itself is delegated — this module is a thin, security-reviewed
// adapter over `crate::encryption_wire` (the same age v1 / EventKey path the
// SPEC-16 event store already uses), so there is one encryption implementation.

use base64::Engine as _;

/// Errors from the conversation seal/open path. Deliberately coarse on the
/// decrypt side (mirrors `encryption_wire`'s oracle-leak-safe collapse): callers
/// must not branch on "wrong key" vs "tampered" vs "no key".
#[derive(thiserror::Error, Debug)]
pub enum SealError {
    /// Kill-switch is ON but no device EventKey is available, so we cannot
    /// encrypt. The caller MUST fail closed (refuse the write) — never fall
    /// back to writing plaintext.
    #[error("conversation seal: no EventKey available (encryption enabled but identity key missing)")]
    NoKey,
    /// age encryption failed.
    #[error("conversation seal: encrypt failed: {0}")]
    Encrypt(String),
    /// A sealed line could not be decrypted (wrong key / no key / tampered /
    /// not valid base64). Collapsed on purpose. The plaintext is NOT recoverable.
    #[error("conversation seal: open failed (undecryptable sealed line)")]
    Open,
}

/// The opt-in kill-switch. **Default OFF.** Only `SPECTYN_ENCRYPT_CONVERSATIONS`
/// set to `1` / `true` (case-insensitive) enables at-rest sealing of new writes.
/// With it off, the conversation store behaves byte-identically to before.
pub fn conversations_e2ee_enabled() -> bool {
    std::env::var("SPECTYN_ENCRYPT_CONVERSATIONS")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Seal one JSONL line (a serialized `ChatMessage`) into a base64 age blob.
///
/// Uses the per-process EventKey cache (`lookup_or_derive_event_key`). Returns
/// [`SealError::NoKey`] if no key is available so the caller can fail closed.
pub fn seal_line(plaintext_json: &str) -> Result<String, SealError> {
    let key = crate::encryption_wire::lookup_or_derive_event_key().ok_or(SealError::NoKey)?;
    let identity = crate::encryption_wire::event_key_to_age_identity(&key)
        .map_err(|e| SealError::Encrypt(format!("{e:?}")))?;
    let recipient = crate::encryption_wire::derive_recipient_from_identity(&identity);
    let envelope = crate::encryption_wire::encrypt_event(plaintext_json.as_bytes(), &recipient)
        .map_err(|e| SealError::Encrypt(format!("{e:?}")))?;
    // `encrypt_event` already base64-encodes the raw age blob; that base64 is the
    // on-disk line form (never starts with `{`, so reads can distinguish it from
    // a plaintext JSON line).
    Ok(envelope.ciphertext_b64)
}

/// Open one on-disk line back to its plaintext JSONL form.
///
/// A line starting with `{` is plaintext JSON (legacy or kill-switch-off) and is
/// returned verbatim. Otherwise the line is a sealed base64 age blob and is
/// decrypted with the cached EventKey; any failure returns [`SealError::Open`]
/// (fail closed — the ciphertext is never returned as if it were plaintext).
pub fn open_line(line: &str) -> Result<String, SealError> {
    let trimmed = line.trim_start();
    if trimmed.starts_with('{') {
        // Plaintext JSON — pass through unchanged (back-compat / kill-switch off).
        return Ok(line.to_string());
    }
    // Sealed line: base64 → raw age blob → decrypt with the cached key.
    let raw = base64::engine::general_purpose::STANDARD
        .decode(trimmed.trim_end())
        .map_err(|_| SealError::Open)?;
    let plaintext = crate::encryption_wire::decrypt_raw_age_blob(&raw).map_err(|_| SealError::Open)?;
    String::from_utf8(plaintext).map_err(|_| SealError::Open)
}

/// `true` if `line` is a sealed (non-plaintext) conversation line. Cheap probe
/// used by the store to decide whether a decrypt is even needed.
pub fn is_sealed_line(line: &str) -> bool {
    !line.trim_start().starts_with('{') && !line.trim().is_empty()
}
