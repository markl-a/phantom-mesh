//! Secure API key pairing bundle for cluster node onboarding.
//!
//! Creates a passphrase-encrypted bundle containing cluster secret,
//! coordinator address, and API keys. Designed for QR code / manual
//! entry pairing with a 10-minute TTL.
//!
//! # Encryption Protocol
//!
//! 1. Generate random 16-byte salt
//! 2. Derive key: Argon2id(passphrase, salt) → 32-byte AES key
//! 3. Generate random 12-byte nonce
//! 4. Payload = JSON serialize { cluster_secret, coordinator_addr, api_keys }
//! 5. Encrypt: AES-256-GCM(key, nonce, payload)
//! 6. Set expires_at = now() + 600 (10 minutes)

use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::Argon2;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

const PAIRING_TTL_SECS: u64 = 600; // 10 minutes
const SALT_SIZE: usize = 16;
const NONCE_SIZE: usize = 12;
const KEY_SIZE: usize = 32;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum PairingError {
    #[error("wrong passphrase or corrupted bundle")]
    DecryptionFailed,
    #[error("pairing bundle expired")]
    Expired,
    #[error("invalid bundle format")]
    InvalidFormat,
    #[error("serialization error: {0}")]
    Serialization(String),
}

// ---------------------------------------------------------------------------
// PairingBundle — the encrypted, serializable bundle
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
pub struct PairingBundle {
    pub cluster_secret: String,
    pub coordinator_addr: String,
    pub encrypted_keys: Vec<u8>,
    pub nonce: Vec<u8>,
    pub salt: Vec<u8>,
    pub expires_at: u64,
}

// ---------------------------------------------------------------------------
// PairingData — the decrypted plaintext contents
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct PairingData {
    pub cluster_secret: String,
    pub coordinator_addr: String,
    pub api_keys: HashMap<String, String>, // provider_name → api_key
}

// ---------------------------------------------------------------------------
// Internal: plaintext payload for JSON serialization
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct Payload {
    cluster_secret: String,
    coordinator_addr: String,
    api_keys: HashMap<String, String>,
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

impl PairingBundle {
    /// Create a pairing bundle encrypted with a passphrase.
    pub fn create(
        cluster_secret: &str,
        coordinator_addr: &str,
        api_keys: &HashMap<String, String>,
        passphrase: &str,
    ) -> Result<Self, PairingError> {
        // 1. Generate random 16-byte salt
        let mut salt = [0u8; SALT_SIZE];
        OsRng.fill_bytes(&mut salt);

        // 2. Derive 32-byte AES key via Argon2id
        let aes_key = Self::derive_key(passphrase, &salt)?;

        // 3. Generate random 12-byte nonce
        let mut nonce_bytes = [0u8; NONCE_SIZE];
        OsRng.fill_bytes(&mut nonce_bytes);

        // 4. Serialize payload to JSON
        let payload = Payload {
            cluster_secret: cluster_secret.to_string(),
            coordinator_addr: coordinator_addr.to_string(),
            api_keys: api_keys.clone(),
        };
        let plaintext = serde_json::to_vec(&payload)
            .map_err(|e| PairingError::Serialization(e.to_string()))?;

        // 5. Encrypt with AES-256-GCM
        let key = aes_gcm::Key::<Aes256Gcm>::from_slice(&aes_key);
        let cipher = Aes256Gcm::new(key);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(nonce, plaintext.as_ref())
            .map_err(|_| PairingError::DecryptionFailed)?;

        // 6. Set expiration
        let expires_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            + PAIRING_TTL_SECS;

        Ok(Self {
            cluster_secret: cluster_secret.to_string(),
            coordinator_addr: coordinator_addr.to_string(),
            encrypted_keys: ciphertext,
            nonce: nonce_bytes.to_vec(),
            salt: salt.to_vec(),
            expires_at,
        })
    }

    /// Decrypt a pairing bundle with the passphrase.
    pub fn open(&self, passphrase: &str) -> Result<PairingData, PairingError> {
        // Check expiration first
        if self.is_expired() {
            return Err(PairingError::Expired);
        }

        // Derive the same AES key from passphrase + salt
        let aes_key = Self::derive_key(passphrase, &self.salt)?;

        // Decrypt
        let key = aes_gcm::Key::<Aes256Gcm>::from_slice(&aes_key);
        let cipher = Aes256Gcm::new(key);

        if self.nonce.len() != NONCE_SIZE {
            return Err(PairingError::InvalidFormat);
        }
        let nonce = Nonce::from_slice(&self.nonce);

        let plaintext = cipher
            .decrypt(nonce, self.encrypted_keys.as_ref())
            .map_err(|_| PairingError::DecryptionFailed)?;

        // Deserialize payload
        let payload: Payload = serde_json::from_slice(&plaintext)
            .map_err(|e| PairingError::Serialization(e.to_string()))?;

        Ok(PairingData {
            cluster_secret: payload.cluster_secret,
            coordinator_addr: payload.coordinator_addr,
            api_keys: payload.api_keys,
        })
    }

    /// Encode to base64 string (for QR code or manual entry).
    pub fn to_base64(&self) -> Result<String, PairingError> {
        let json = serde_json::to_vec(self)
            .map_err(|e| PairingError::Serialization(e.to_string()))?;
        Ok(BASE64.encode(&json))
    }

    /// Decode from base64 string.
    pub fn from_base64(encoded: &str) -> Result<Self, PairingError> {
        let bytes = BASE64
            .decode(encoded)
            .map_err(|_| PairingError::InvalidFormat)?;
        serde_json::from_slice(&bytes)
            .map_err(|_| PairingError::InvalidFormat)
    }

    /// Check if bundle is expired.
    pub fn is_expired(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        now >= self.expires_at
    }

    /// Derive a 32-byte AES key from passphrase + salt using Argon2id.
    fn derive_key(passphrase: &str, salt: &[u8]) -> Result<[u8; KEY_SIZE], PairingError> {
        let argon2 = Argon2::default();
        let mut key = [0u8; KEY_SIZE];
        argon2
            .hash_password_into(passphrase.as_bytes(), salt, &mut key)
            .map_err(|_| PairingError::DecryptionFailed)?;
        Ok(key)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_keys() -> HashMap<String, String> {
        let mut keys = HashMap::new();
        keys.insert("openai".to_string(), "sk-abc123".to_string());
        keys.insert("anthropic".to_string(), "sk-ant-xyz789".to_string());
        keys
    }

    #[test]
    fn test_create_and_open() {
        let keys = sample_keys();
        let bundle = PairingBundle::create(
            "cluster-secret-42",
            "192.168.1.10:7878",
            &keys,
            "my-passphrase",
        )
        .unwrap();

        let data = bundle.open("my-passphrase").unwrap();
        assert_eq!(data.cluster_secret, "cluster-secret-42");
        assert_eq!(data.coordinator_addr, "192.168.1.10:7878");
        assert_eq!(data.api_keys.get("openai").unwrap(), "sk-abc123");
        assert_eq!(data.api_keys.get("anthropic").unwrap(), "sk-ant-xyz789");
    }

    #[test]
    fn test_wrong_passphrase() {
        let keys = sample_keys();
        let bundle = PairingBundle::create(
            "cluster-secret",
            "192.168.1.10:7878",
            &keys,
            "correct-passphrase",
        )
        .unwrap();

        let err = bundle.open("wrong-passphrase").unwrap_err();
        assert!(matches!(err, PairingError::DecryptionFailed));
    }

    #[test]
    fn test_expired_bundle() {
        let keys = sample_keys();
        let mut bundle = PairingBundle::create(
            "cluster-secret",
            "192.168.1.10:7878",
            &keys,
            "passphrase",
        )
        .unwrap();

        // Force expiration to the past
        bundle.expires_at = 0;

        assert!(bundle.is_expired());
        let err = bundle.open("passphrase").unwrap_err();
        assert!(matches!(err, PairingError::Expired));
    }

    #[test]
    fn test_base64_roundtrip() {
        let keys = sample_keys();
        let bundle = PairingBundle::create(
            "cluster-secret-b64",
            "10.0.0.1:9000",
            &keys,
            "roundtrip-pass",
        )
        .unwrap();

        let encoded = bundle.to_base64().unwrap();
        let decoded = PairingBundle::from_base64(&encoded).unwrap();
        let data = decoded.open("roundtrip-pass").unwrap();

        assert_eq!(data.cluster_secret, "cluster-secret-b64");
        assert_eq!(data.coordinator_addr, "10.0.0.1:9000");
        assert_eq!(data.api_keys.get("openai").unwrap(), "sk-abc123");
        assert_eq!(data.api_keys.get("anthropic").unwrap(), "sk-ant-xyz789");
    }

    #[test]
    fn test_multiple_keys() {
        let mut keys = HashMap::new();
        keys.insert("openai".to_string(), "sk-openai-key".to_string());
        keys.insert("anthropic".to_string(), "sk-ant-key".to_string());
        keys.insert("groq".to_string(), "gsk-groq-key".to_string());
        keys.insert("gemini".to_string(), "AIza-gemini-key".to_string());
        keys.insert("mistral".to_string(), "mist-key-12345".to_string());

        let bundle = PairingBundle::create(
            "multi-key-cluster",
            "172.16.0.1:7878",
            &keys,
            "five-key-pass",
        )
        .unwrap();

        let data = bundle.open("five-key-pass").unwrap();
        assert_eq!(data.api_keys.len(), 5);
        assert_eq!(data.api_keys.get("openai").unwrap(), "sk-openai-key");
        assert_eq!(data.api_keys.get("anthropic").unwrap(), "sk-ant-key");
        assert_eq!(data.api_keys.get("groq").unwrap(), "gsk-groq-key");
        assert_eq!(data.api_keys.get("gemini").unwrap(), "AIza-gemini-key");
        assert_eq!(data.api_keys.get("mistral").unwrap(), "mist-key-12345");
    }
}
