//! Encrypted secrets management using ChaCha20-Poly1305 AEAD.
//! Secrets in config are stored as `enc2:<hex(nonce || ciphertext || tag)>`.

use anyhow::{anyhow, Result};
use chacha20poly1305::aead::{Aead, KeyInit, OsRng};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use rand::RngCore;
use std::path::Path;
use tracing::{debug, info};

const SECRET_KEY_FILE: &str = ".secret_key";
const PREFIX: &str = "enc2:";
const KEY_SIZE: usize = 32;
const NONCE_SIZE: usize = 12;

/// Manages encryption/decryption of secrets using a local key file.
pub struct SecretManager {
    cipher: ChaCha20Poly1305,
}

impl SecretManager {
    /// Load or create the secret key from `~/.phantom-mesh/.secret_key`
    pub fn new(phantom_mesh_dir: &str) -> Result<Self> {
        let key_path = Path::new(phantom_mesh_dir).join(SECRET_KEY_FILE);

        let key_bytes = if key_path.exists() {
            let hex_str = std::fs::read_to_string(&key_path)?;
            let hex_str = hex_str.trim();
            hex::decode(hex_str)
                .map_err(|e| anyhow!("Invalid secret key file: {}", e))?
        } else {
            // Generate a new key
            let mut key = vec![0u8; KEY_SIZE];
            OsRng.fill_bytes(&mut key);
            let hex_str = hex::encode(&key);

            // Ensure directory exists
            if let Some(parent) = key_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&key_path, &hex_str)?;

            // Set file permissions (best-effort on Windows)
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))?;
            }

            info!("Generated new secret key at {}", key_path.display());
            key
        };

        if key_bytes.len() != KEY_SIZE {
            return Err(anyhow!("Secret key must be {} bytes, got {}", KEY_SIZE, key_bytes.len()));
        }

        let key = chacha20poly1305::Key::from_slice(&key_bytes);
        let cipher = ChaCha20Poly1305::new(key);

        Ok(Self { cipher })
    }

    /// Create with an explicit key (for testing)
    pub fn with_key(key: &[u8; KEY_SIZE]) -> Self {
        let key = chacha20poly1305::Key::from_slice(key);
        Self { cipher: ChaCha20Poly1305::new(key) }
    }

    /// Encrypt a plaintext value, returning `enc2:<hex>` string
    pub fn encrypt(&self, plaintext: &str) -> Result<String> {
        let mut nonce_bytes = [0u8; NONCE_SIZE];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = self.cipher.encrypt(nonce, plaintext.as_bytes())
            .map_err(|e| anyhow!("Encryption failed: {}", e))?;

        // Format: nonce || ciphertext (tag is appended by AEAD)
        let mut combined = Vec::with_capacity(NONCE_SIZE + ciphertext.len());
        combined.extend_from_slice(&nonce_bytes);
        combined.extend_from_slice(&ciphertext);

        Ok(format!("{}{}", PREFIX, hex::encode(&combined)))
    }

    /// Decrypt an `enc2:<hex>` string, returning plaintext
    pub fn decrypt(&self, encrypted: &str) -> Result<String> {
        let hex_str = encrypted.strip_prefix(PREFIX)
            .ok_or_else(|| anyhow!("Not an encrypted value (missing '{}' prefix)", PREFIX))?;

        let combined = hex::decode(hex_str)
            .map_err(|e| anyhow!("Invalid hex in encrypted value: {}", e))?;

        if combined.len() < NONCE_SIZE + 1 {
            return Err(anyhow!("Encrypted value too short"));
        }

        let (nonce_bytes, ciphertext) = combined.split_at(NONCE_SIZE);
        let nonce = Nonce::from_slice(nonce_bytes);

        let plaintext = self.cipher.decrypt(nonce, ciphertext)
            .map_err(|_| anyhow!("Decryption failed (wrong key or corrupted data)"))?;

        String::from_utf8(plaintext)
            .map_err(|e| anyhow!("Decrypted value is not valid UTF-8: {}", e))
    }

    /// Check if a string is an encrypted value
    pub fn is_encrypted(value: &str) -> bool {
        value.starts_with(PREFIX)
    }

    /// Decrypt a value if it's encrypted, otherwise return as-is
    pub fn maybe_decrypt(&self, value: &str) -> Result<String> {
        if Self::is_encrypted(value) {
            self.decrypt(value)
        } else {
            Ok(value.to_string())
        }
    }

    /// Process a TOML config, decrypting any `enc2:` values in-place
    pub fn decrypt_config(&self, config: &mut serde_json::Value) {
        match config {
            serde_json::Value::String(s) => {
                if Self::is_encrypted(s) {
                    match self.decrypt(s) {
                        Ok(decrypted) => {
                            debug!("Decrypted config value (len={})", decrypted.len());
                            *s = decrypted;
                        }
                        Err(e) => {
                            tracing::error!("Failed to decrypt config value: {}", e);
                        }
                    }
                }
            }
            serde_json::Value::Object(map) => {
                for (_, v) in map.iter_mut() {
                    self.decrypt_config(v);
                }
            }
            serde_json::Value::Array(arr) => {
                for v in arr.iter_mut() {
                    self.decrypt_config(v);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_manager() -> SecretManager {
        SecretManager::with_key(&[42u8; KEY_SIZE])
    }

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let mgr = test_manager();
        let plaintext = "sk-ant-api03-1234567890";
        let encrypted = mgr.encrypt(plaintext).unwrap();
        assert!(encrypted.starts_with("enc2:"));
        let decrypted = mgr.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_encrypt_different_each_time() {
        let mgr = test_manager();
        let e1 = mgr.encrypt("test").unwrap();
        let e2 = mgr.encrypt("test").unwrap();
        assert_ne!(e1, e2); // Different nonces
    }

    #[test]
    fn test_decrypt_wrong_key() {
        let mgr1 = SecretManager::with_key(&[1u8; KEY_SIZE]);
        let mgr2 = SecretManager::with_key(&[2u8; KEY_SIZE]);
        let encrypted = mgr1.encrypt("secret").unwrap();
        assert!(mgr2.decrypt(&encrypted).is_err());
    }

    #[test]
    fn test_is_encrypted() {
        assert!(SecretManager::is_encrypted("enc2:abcdef1234"));
        assert!(!SecretManager::is_encrypted("plain-text"));
        assert!(!SecretManager::is_encrypted(""));
    }

    #[test]
    fn test_maybe_decrypt_plain() {
        let mgr = test_manager();
        let result = mgr.maybe_decrypt("plain-text").unwrap();
        assert_eq!(result, "plain-text");
    }

    #[test]
    fn test_maybe_decrypt_encrypted() {
        let mgr = test_manager();
        let encrypted = mgr.encrypt("my-secret").unwrap();
        let result = mgr.maybe_decrypt(&encrypted).unwrap();
        assert_eq!(result, "my-secret");
    }

    #[test]
    fn test_decrypt_invalid_hex() {
        let mgr = test_manager();
        assert!(mgr.decrypt("enc2:not-valid-hex").is_err());
    }

    #[test]
    fn test_decrypt_too_short() {
        let mgr = test_manager();
        assert!(mgr.decrypt("enc2:0011").is_err());
    }

    #[test]
    fn test_decrypt_no_prefix() {
        let mgr = test_manager();
        assert!(mgr.decrypt("plain-text").is_err());
    }

    #[test]
    fn test_decrypt_config() {
        let mgr = test_manager();
        let encrypted = mgr.encrypt("my-api-key").unwrap();
        let mut config = serde_json::json!({
            "api_key": encrypted,
            "name": "test",
            "nested": {
                "secret": mgr.encrypt("nested-secret").unwrap(),
            }
        });
        mgr.decrypt_config(&mut config);
        assert_eq!(config["api_key"], "my-api-key");
        assert_eq!(config["name"], "test");
        assert_eq!(config["nested"]["secret"], "nested-secret");
    }

    #[test]
    fn test_encrypt_empty_string() {
        let mgr = test_manager();
        let encrypted = mgr.encrypt("").unwrap();
        let decrypted = mgr.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, "");
    }

    #[test]
    fn test_encrypt_unicode() {
        let mgr = test_manager();
        let plaintext = "密碼：你好世界🔐";
        let encrypted = mgr.encrypt(plaintext).unwrap();
        let decrypted = mgr.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }
}
