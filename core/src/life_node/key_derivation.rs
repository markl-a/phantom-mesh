//! Per-device event encryption key, derived from `~/.phantom-mesh/identity.key`
//! via HKDF-SHA256.
//!
//! `identity.key` is the existing 64-byte file phantom uses for per-device
//! identity. We treat its bytes as IKM (input keying material) and derive a
//! 32-byte event encryption key with HKDF-extract-then-expand, label
//! `"phantom-mesh.event-encryption-v1"`.
//!
//! The derived key is wrapped in `EventKey`, a struct that zeroes its bytes
//! on drop so a Drop-time panic can't accidentally leave the key on the stack.

use hkdf::Hkdf;
use sha2::Sha256;
use std::path::Path;
use zeroize::Zeroize;

const HKDF_LABEL: &[u8] = b"phantom-mesh.event-encryption-v1";
const EVENT_KEY_LEN: usize = 32;

/// 32-byte event encryption key. Zeroed on drop.
#[derive(Clone)]
pub struct EventKey {
    bytes: [u8; EVENT_KEY_LEN],
}

impl EventKey {
    pub fn as_bytes(&self) -> &[u8; EVENT_KEY_LEN] {
        &self.bytes
    }
}

impl Drop for EventKey {
    fn drop(&mut self) {
        self.bytes.zeroize();
    }
}

impl std::fmt::Debug for EventKey {
    /// Never print key bytes — only the length.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "EventKey {{ <{} bytes redacted> }}", EVENT_KEY_LEN)
    }
}

#[derive(thiserror::Error, Debug)]
pub enum KeyDerivationError {
    #[error("identity.key not found at {0}")]
    IdentityKeyMissing(String),
    #[error("identity.key too short: got {got} bytes, need at least 16")]
    IdentityKeyTooShort { got: usize },
    #[error("HKDF expand failed: {0}")]
    HkdfExpand(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Derive the event encryption key from raw `identity.key` bytes.
/// Returns deterministic output for the same input.
pub fn derive_event_key(identity_bytes: &[u8]) -> Result<EventKey, KeyDerivationError> {
    if identity_bytes.len() < 16 {
        return Err(KeyDerivationError::IdentityKeyTooShort {
            got: identity_bytes.len(),
        });
    }
    let hk = Hkdf::<Sha256>::new(None, identity_bytes); // no salt; IKM only
    let mut okm = [0u8; EVENT_KEY_LEN];
    hk.expand(HKDF_LABEL, &mut okm)
        .map_err(|e| KeyDerivationError::HkdfExpand(e.to_string()))?;
    // `[u8; N]` is Copy, so this COPIES `okm` into the struct rather than
    // moving it — the derived key would otherwise linger in the `okm` stack
    // local, dropped without zeroizing. EventKey zeroizes its own copy on
    // drop; scrub the intermediate here so no un-zeroized key material is left
    // behind. (Determinism is unchanged — see the derive_event_key tests.)
    let key = EventKey { bytes: okm };
    okm.zeroize();
    Ok(key)
}

/// Legacy event-key derivation, kept as a `with_identity_file` decrypt fallback
/// for events written before the 2026-05-30 EventKey unification (commit
/// 3398a223). v0.6.0 keeps no on-disk record of which derivation produced an
/// event; this stub returns `KeyDerivationError::IdentityKeyMissing` so the
/// caller treats `legacy_key` as `None` — pre-unification events become
/// unreadable while canonical-derived events stay readable. Restore a real
/// legacy HKDF info string here if a user reports data loss (would require
/// git-archaeology of the pre-unification derivation).
pub fn load_event_key_legacy(_identity_path: &Path) -> Result<EventKey, KeyDerivationError> {
    Err(KeyDerivationError::IdentityKeyMissing(
        "legacy derivation disabled — see load_event_key_legacy doc".to_string(),
    ))
}

/// Read identity.key from disk and derive the event key. Convenience wrapper.
pub fn load_event_key(identity_path: &Path) -> Result<EventKey, KeyDerivationError> {
    if !identity_path.exists() {
        return Err(KeyDerivationError::IdentityKeyMissing(
            identity_path.display().to_string(),
        ));
    }
    // The raw identity.key bytes are the root IKM (more sensitive than the
    // derived key). std::fs::read hands back a heap Vec that would be freed —
    // not scrubbed — on return; zeroize it after derivation, on every path.
    let mut bytes = std::fs::read(identity_path)?;
    // W3: on Windows the root IKM is DPAPI-wrapped at rest. Unwrap BEFORE
    // derivation — derive_event_key accepts any >=16 bytes, so feeding it the
    // wrapped blob would silently derive a WRONG key. unprotect_at_rest is a
    // no-op (Ok(None)) on unix, so this is called unconditionally — NO
    // statement-level #[cfg], which the save-time formatter strips. `Ok(None)`
    // = legacy/unix plaintext (use bytes as-is); `Err` = unwrap failed.
    match crate::identity_wire::unprotect_at_rest(&bytes) {
        Ok(Some(mut seed)) => {
            bytes.zeroize();
            let result = derive_event_key(&seed);
            seed.zeroize();
            return result;
        }
        Ok(None) => {}
        Err(e) => {
            bytes.zeroize();
            return Err(KeyDerivationError::Io(e));
        }
    }
    let result = derive_event_key(&bytes);
    bytes.zeroize();
    result
}

/// Resolve the event key for a WRITE path under the no-silent-downgrade policy:
///
///   * `Ok(Some(key))` — a usable `identity.key` exists → encrypt at rest.
///   * `Ok(None)`      — NO `identity.key` at all → plaintext is the intended
///                       pre-encryption state (matches `EventStore::new`).
///   * `Err(_)`        — `identity.key` is PRESENT but unloadable (corrupt /
///                       too short / unreadable). The user configured
///                       encryption, so callers MUST refuse to write rather
///                       than silently downgrade to plaintext (D24).
///
/// This is the single source of truth for that three-way decision so the
/// capture write paths (note / focus / dispatch) can't drift apart on it.
pub fn event_key_for_write(identity_path: &Path) -> Result<Option<EventKey>, KeyDerivationError> {
    match load_event_key(identity_path) {
        Ok(k) => Ok(Some(k)),
        Err(KeyDerivationError::IdentityKeyMissing(_)) => Ok(None),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_event_key_is_deterministic() {
        let ikm = [0x42u8; 64];
        let k1 = derive_event_key(&ikm).unwrap();
        let k2 = derive_event_key(&ikm).unwrap();
        assert_eq!(
            k1.as_bytes(),
            k2.as_bytes(),
            "HKDF must be deterministic — same IKM, same key"
        );
    }

    #[test]
    fn derive_event_key_differs_from_input() {
        let ikm = [0x42u8; 64];
        let k = derive_event_key(&ikm).unwrap();
        // HKDF output should not equal the IKM bytes
        assert_ne!(
            &k.as_bytes()[..],
            &ikm[..32],
            "HKDF output must differ from the raw IKM"
        );
    }

    #[test]
    fn derive_event_key_different_ikm_different_key() {
        let k1 = derive_event_key(&[0x42u8; 64]).unwrap();
        let k2 = derive_event_key(&[0x43u8; 64]).unwrap();
        assert_ne!(
            k1.as_bytes(),
            k2.as_bytes(),
            "Different IKM must produce different keys"
        );
    }

    #[test]
    fn derive_event_key_rejects_too_short_input() {
        let r = derive_event_key(&[0u8; 8]);
        assert!(matches!(
            r,
            Err(KeyDerivationError::IdentityKeyTooShort { got: 8 })
        ));
    }

    #[test]
    fn debug_format_does_not_leak_bytes() {
        let k = derive_event_key(&[0x42u8; 64]).unwrap();
        let dbg = format!("{:?}", k);
        assert!(dbg.contains("redacted"), "Debug must redact: {}", dbg);
        // Ensure no hex of the actual bytes leaks
        let hex_first_byte = format!("{:02x}", k.as_bytes()[0]);
        assert!(
            !dbg.contains(&hex_first_byte),
            "Debug leaks first byte hex: {}",
            dbg
        );
    }

    #[test]
    fn load_event_key_from_temp_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("identity.key");
        std::fs::write(&path, [0x77u8; 64]).unwrap();
        let k = load_event_key(&path).unwrap();
        // sanity: matches direct derivation
        let direct = derive_event_key(&[0x77u8; 64]).unwrap();
        assert_eq!(k.as_bytes(), direct.as_bytes());
    }

    // ── D24: the write-path policy must distinguish absent / present-corrupt ──

    #[test]
    fn event_key_for_write_none_when_absent() {
        let tmp = tempfile::tempdir().unwrap();
        // No identity.key on disk → Ok(None) → plaintext is the intended state.
        let got = event_key_for_write(&tmp.path().join("identity.key")).unwrap();
        assert!(got.is_none(), "absent key must yield Ok(None), got {got:?}");
    }

    #[test]
    fn event_key_for_write_some_when_valid() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("identity.key");
        std::fs::write(&path, [0x77u8; 64]).unwrap();
        assert!(event_key_for_write(&path).unwrap().is_some());
    }

    #[test]
    fn event_key_for_write_errors_on_present_but_corrupt() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("identity.key");
        // Present but too short (<16 bytes): the user configured encryption, so
        // this MUST error — never silently fall back to plaintext (D24).
        std::fs::write(&path, [0x01u8; 5]).unwrap();
        let err = event_key_for_write(&path).unwrap_err();
        assert!(
            matches!(err, KeyDerivationError::IdentityKeyTooShort { .. }),
            "corrupt key must surface an error, got {err:?}"
        );
    }
}
