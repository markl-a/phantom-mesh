//! age-format encryption/decryption wrappers for `EventStore` files.
//!
//! Uses age v1 x25519 recipient mode. The age x25519 identity is
//! **deterministically derived** from the 32-byte `EventKey` by encoding it
//! as an `AGE-SECRET-KEY-...` bech32 string (age's standard wire format),
//! then parsing with `age::x25519::Identity::from_str`. This gives us:
//!
//! 1. Standard age binary format on disk (interoperable with the `age` CLI
//!    if the user ever needs emergency recovery; they can recover the
//!    secret-key string from `identity.key` + the HKDF label).
//! 2. No scrypt slowdown (recipient mode skips the passphrase KDF).
//! 3. Deterministic encryption identity per device (key never leaves the
//!    machine; same `identity.key` = same encryption key forever).

use crate::life_node::key_derivation::EventKey;
use age::{Decryptor, Encryptor};
use bech32::Hrp;
use std::io::{Read, Write};

const AGE_HRP: &str = "age-secret-key-";

#[derive(thiserror::Error, Debug)]
pub enum CryptoError {
    #[error("bech32 encode: {0}")]
    Bech32(String),
    #[error("age identity parse: {0}")]
    IdentityParse(String),
    #[error("age encrypt: {0}")]
    Encrypt(String),
    #[error("age decrypt: {0}")]
    Decrypt(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Build a deterministic age x25519 identity from a 32-byte EventKey.
fn key_to_age_identity(key: &EventKey) -> Result<age::x25519::Identity, CryptoError> {
    // age's secret-key wire format: "AGE-SECRET-KEY-<bech32 over 32 bytes>"
    // The bech32 HRP is "age-secret-key-" (lowercase); the encoding is then
    // uppercased to match the public format.
    let hrp = Hrp::parse(AGE_HRP).map_err(|e| CryptoError::Bech32(e.to_string()))?;
    let encoded = bech32::encode::<bech32::Bech32>(hrp, key.as_bytes())
        .map_err(|e| CryptoError::Bech32(e.to_string()))?;
    // age expects uppercase
    let s = encoded.to_uppercase();
    s.parse::<age::x25519::Identity>()
        .map_err(|e| CryptoError::IdentityParse(e.to_string()))
}

/// Encrypt plaintext bytes for the holder of `key` using age x25519.
pub fn encrypt(plaintext: &[u8], key: &EventKey) -> Result<Vec<u8>, CryptoError> {
    let identity = key_to_age_identity(key)?;
    let recipient = identity.to_public();
    // age 0.10: `Encryptor::with_recipients` returns `Option<Self>` (None if
    // recipient list is empty). We always pass exactly one recipient, so the
    // None branch should be unreachable in practice — but handle it cleanly.
    let encryptor = Encryptor::with_recipients(vec![Box::new(recipient)])
        .ok_or_else(|| CryptoError::Encrypt("no recipients".into()))?;
    let mut buf = Vec::with_capacity(plaintext.len() + 256);
    let mut writer = encryptor
        .wrap_output(&mut buf)
        .map_err(|e| CryptoError::Encrypt(e.to_string()))?;
    writer.write_all(plaintext)?;
    writer
        .finish()
        .map_err(|e| CryptoError::Encrypt(e.to_string()))?;
    Ok(buf)
}

/// Decrypt age-format ciphertext using `key`. Returns the recovered plaintext.
pub fn decrypt(ciphertext: &[u8], key: &EventKey) -> Result<Vec<u8>, CryptoError> {
    let identity = key_to_age_identity(key)?;
    // age 0.10: `Decryptor::new` returns the `Decryptor` enum. For x25519
    // recipient-mode files we expect the `Recipients(_)` variant; passphrase
    // mode would be a misuse here.
    let decryptor = Decryptor::new(ciphertext).map_err(|e| CryptoError::Decrypt(e.to_string()))?;
    let recipients_decryptor = match decryptor {
        Decryptor::Recipients(r) => r,
        Decryptor::Passphrase(_) => {
            return Err(CryptoError::Decrypt(
                "passphrase-encrypted age file; expected x25519 recipient".into(),
            ))
        }
    };
    let mut reader = recipients_decryptor
        .decrypt(std::iter::once(&identity as &dyn age::Identity))
        .map_err(|e| CryptoError::Decrypt(e.to_string()))?;
    let mut out = Vec::new();
    reader.read_to_end(&mut out)?;
    Ok(out)
}

/// Decrypt age ciphertext trying EACH key in turn (age selects the matching
/// recipient internally). Used for the canonical+legacy fallback after the
/// 2026-05-30 EventKey unification: pass `[canonical, legacy]` so an event
/// written under EITHER derivation decrypts — zero data loss across the
/// migration. Fails only if NO supplied key matches.
pub fn decrypt_any(ciphertext: &[u8], keys: &[&EventKey]) -> Result<Vec<u8>, CryptoError> {
    let identities: Vec<age::x25519::Identity> = keys
        .iter()
        .map(|k| key_to_age_identity(k))
        .collect::<Result<_, _>>()?;
    let decryptor =
        Decryptor::new(ciphertext).map_err(|e| CryptoError::Decrypt(e.to_string()))?;
    let recipients_decryptor = match decryptor {
        Decryptor::Recipients(r) => r,
        Decryptor::Passphrase(_) => {
            return Err(CryptoError::Decrypt(
                "passphrase-encrypted age file; expected x25519 recipient".into(),
            ))
        }
    };
    let id_refs: Vec<&dyn age::Identity> =
        identities.iter().map(|i| i as &dyn age::Identity).collect();
    let mut reader = recipients_decryptor
        .decrypt(id_refs.into_iter())
        .map_err(|e| CryptoError::Decrypt(e.to_string()))?;
    let mut out = Vec::new();
    reader.read_to_end(&mut out)?;
    Ok(out)
}

/// Cheap probe: does this byte slice start with age v1 binary magic?
/// Used by `EventStore` to decide between encrypted vs legacy plaintext.
pub fn looks_like_age(bytes: &[u8]) -> bool {
    // age v1 binary magic is the ASCII line "age-encryption.org/v1\n"
    const MAGIC: &[u8] = b"age-encryption.org/v1\n";
    bytes.starts_with(MAGIC)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::life_node::key_derivation::derive_event_key;

    fn fixed_key() -> EventKey {
        derive_event_key(&[0x42u8; 64]).unwrap()
    }

    #[test]
    fn age_encrypt_decrypt_round_trip() {
        let k = fixed_key();
        let pt = b"hello, life node \xe9\xa4\x90\xe6\xa1\x8c"; // "餐桌" in UTF-8
        let ct = encrypt(pt, &k).expect("encrypt");
        let recovered = decrypt(&ct, &k).expect("decrypt");
        assert_eq!(recovered, pt);
    }

    #[test]
    fn encrypted_output_starts_with_age_magic() {
        let k = fixed_key();
        let ct = encrypt(b"x", &k).unwrap();
        assert!(
            looks_like_age(&ct),
            "ct must start with age magic; got first 32 bytes = {:?}",
            &ct[..ct.len().min(32)]
        );
    }

    #[test]
    fn looks_like_age_returns_false_for_plaintext_json() {
        let pt_json = br#"{"event_id":"abc"}"#;
        assert!(!looks_like_age(pt_json));
    }

    #[test]
    fn decrypt_fails_on_tampered_ciphertext() {
        let k = fixed_key();
        let mut ct = encrypt(b"top secret", &k).unwrap();
        // flip a byte in the encrypted payload area (after the header).
        // age header is ~150 bytes; flipping at offset near the end is in body.
        let idx = ct.len().saturating_sub(8);
        ct[idx] ^= 0xff;
        let r = decrypt(&ct, &k);
        assert!(r.is_err(), "tampered ciphertext must fail decryption");
    }

    #[test]
    fn decrypt_with_wrong_key_fails() {
        let k1 = fixed_key();
        let k2 = derive_event_key(&[0x43u8; 64]).unwrap();
        let ct = encrypt(b"data", &k1).unwrap();
        let r = decrypt(&ct, &k2);
        assert!(r.is_err(), "wrong key must fail decryption");
    }

    #[test]
    fn encrypts_empty_input() {
        let k = fixed_key();
        let ct = encrypt(b"", &k).unwrap();
        let pt = decrypt(&ct, &k).unwrap();
        assert_eq!(pt, b"");
    }

    #[test]
    fn decrypt_any_bridges_multiple_keys_no_data_loss() {
        // Updated for the 2026-05-30 EventKey unification (commit 3398a223):
        // `derive_event_key_legacy(&[u8])` no longer exists — legacy derivation
        // is disabled and now lives behind `load_event_key_legacy(&Path)` which
        // returns an error (see key_derivation.rs). This test still pins the
        // `decrypt_any` multi-key fallback contract using two distinct CANONICAL
        // keys standing in for the "current + alternate" key set.
        use crate::life_node::key_derivation::derive_event_key;
        let canon = derive_event_key(&[0x42u8; 64]).unwrap();
        let alt = derive_event_key(&[0x7eu8; 64]).unwrap();
        // Data encrypted under the alternate key decrypts via the fallback set.
        let ct_alt = encrypt(b"old life-node note", &alt).unwrap();
        assert_eq!(
            decrypt_any(&ct_alt, &[&canon, &alt]).unwrap(),
            b"old life-node note"
        );
        // Data encrypted under the canonical key decrypts via the same call.
        let ct_canon = encrypt(b"new capture event", &canon).unwrap();
        assert_eq!(
            decrypt_any(&ct_canon, &[&canon, &alt]).unwrap(),
            b"new capture event"
        );
        // A key outside the set still fails (no oracle weakening).
        let other = derive_event_key(&[0x99u8; 64]).unwrap();
        let ct_other = encrypt(b"x", &other).unwrap();
        assert!(decrypt_any(&ct_other, &[&canon, &alt]).is_err());
    }
}
