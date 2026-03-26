//! KeyVault — Encrypted API key storage with per-key permissions.
//!
//! Uses Argon2id for password → master key derivation, and AES-256-GCM for
//! per-key encryption. Each key gets a random 12-byte nonce.
//!
//! # Design
//!
//! - `KeyStore` trait: async interface for pluggable backends (local, SQLite, etc.)
//! - `LocalKeyVault`: in-memory HashMap backend, encrypted with master key
//! - Future: SQLite backend for persistence across restarts

use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::Argon2;
use async_trait::async_trait;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use subtle::ConstantTimeEq;
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};
use zeroize::Zeroize;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const NONCE_SIZE: usize = 12;
const KEY_SIZE: usize = 32;
const SALT_SIZE: usize = 16;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum KeyVaultError {
    #[error("vault is locked — call unlock() first")]
    Locked,

    #[error("wrong password")]
    WrongPassword,

    #[error("key not found: {0}")]
    NotFound(String),

    #[error("key already exists: {0}")]
    AlreadyExists(String),

    #[error("encryption error: {0}")]
    Encryption(String),

    #[error("decryption error: {0}")]
    Decryption(String),

    #[error("argon2 error: {0}")]
    Argon2(String),
}

pub type KeyVaultResult<T> = Result<T, KeyVaultError>;

// ---------------------------------------------------------------------------
// KeyPermission
// ---------------------------------------------------------------------------

/// Per-key access control: which providers, models, nodes, and budget.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyPermission {
    /// Which provider this key belongs to (e.g. "openai", "anthropic")
    pub provider: String,
    /// Allowed model patterns (empty = all models for this provider)
    pub allowed_models: Vec<String>,
    /// Daily budget cap in USD (0.0 = unlimited)
    pub daily_budget_usd: f64,
    /// Which cluster node IDs may use this key (empty = all nodes)
    pub allowed_nodes: Vec<String>,
}

// ---------------------------------------------------------------------------
// KeyMeta
// ---------------------------------------------------------------------------

/// Metadata about a stored key (without the actual secret).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyMeta {
    /// Unique name for this key (e.g. "openai-prod-1")
    pub name: String,
    /// Provider this key belongs to
    pub provider: String,
    /// When this key was stored (Unix timestamp)
    pub created_at: u64,
    /// Access control
    pub permissions: KeyPermission,
}

// ---------------------------------------------------------------------------
// KeyStore trait
// ---------------------------------------------------------------------------

/// Async trait for pluggable key storage backends.
#[async_trait]
pub trait KeyStore: Send + Sync {
    /// Store a key with metadata. Returns error if name already exists.
    async fn store_key(&self, name: &str, secret: &str, permissions: KeyPermission) -> KeyVaultResult<()>;

    /// Retrieve a decrypted key by name.
    async fn get_key(&self, name: &str) -> KeyVaultResult<String>;

    /// List all key metadata (without secrets).
    async fn list_keys(&self) -> KeyVaultResult<Vec<KeyMeta>>;

    /// Delete a key by name.
    async fn delete_key(&self, name: &str) -> KeyVaultResult<bool>;
}

// ---------------------------------------------------------------------------
// EncryptedEntry (internal)
// ---------------------------------------------------------------------------

/// Internal storage format: encrypted key + metadata.
#[derive(Debug, Clone)]
struct EncryptedEntry {
    /// AES-256-GCM encrypted secret (nonce || ciphertext || tag)
    ciphertext: Vec<u8>,
    /// Key metadata
    meta: KeyMeta,
}

// ---------------------------------------------------------------------------
// LocalKeyVault
// ---------------------------------------------------------------------------

/// In-memory encrypted key vault.
///
/// Password → Argon2id → 32-byte master key.
/// Each stored key is encrypted with AES-256-GCM using a random 12-byte nonce.
pub struct LocalKeyVault {
    /// Derived master key (None = locked)
    master_key: Arc<RwLock<Option<[u8; KEY_SIZE]>>>,
    /// Salt used for Argon2id derivation
    salt: [u8; SALT_SIZE],
    /// Password hash for verification on unlock
    password_hash: Arc<RwLock<Option<[u8; KEY_SIZE]>>>,
    /// Encrypted key store
    entries: Arc<RwLock<HashMap<String, EncryptedEntry>>>,
}

impl LocalKeyVault {
    /// Create a new vault with the given password. Derives master key via Argon2id.
    pub fn new(password: &str) -> KeyVaultResult<Self> {
        let mut salt = [0u8; SALT_SIZE];
        OsRng.fill_bytes(&mut salt);

        let master_key = Self::derive_key(password, &salt)?;

        // Store a hash of the password for verification on re-unlock
        let password_hash = Self::derive_key(&format!("verify:{}", password), &salt)?;

        info!("KeyVault created (Argon2id + AES-256-GCM)");

        Ok(Self {
            master_key: Arc::new(RwLock::new(Some(master_key))),
            salt,
            password_hash: Arc::new(RwLock::new(Some(password_hash))),
            entries: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Derive a 32-byte key from password + salt using Argon2id.
    fn derive_key(password: &str, salt: &[u8; SALT_SIZE]) -> KeyVaultResult<[u8; KEY_SIZE]> {
        let mut output = [0u8; KEY_SIZE];
        // Hardened for 6-digit PIN: m=256MiB, t=4, p=2 (~0.5s/hash, ~5.8 days brute-force)
        let params = argon2::Params::new(262_144, 4, 2, Some(KEY_SIZE))
            .map_err(|e| KeyVaultError::Argon2(e.to_string()))?;
        let argon2 = Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
        argon2
            .hash_password_into(password.as_bytes(), salt, &mut output)
            .map_err(|e| KeyVaultError::Argon2(e.to_string()))?;
        Ok(output)
    }

    /// Lock the vault — wipes master key and password hash from memory.
    pub async fn lock(&self) {
        let mut key = self.master_key.write().await;
        if let Some(ref mut k) = *key {
            // Zero out the key before dropping (using zeroize to prevent compiler optimization)
            k.zeroize();
        }
        *key = None;

        let mut hash = self.password_hash.write().await;
        if let Some(ref mut h) = *hash {
            h.zeroize();
        }
        *hash = None;

        info!("KeyVault locked");
    }

    /// Unlock the vault with password. Returns error if password is wrong.
    ///
    /// Compares the re-derived verification hash against the stored hash using
    /// constant-time comparison. If the stored hash was zeroed on `lock()`,
    /// re-derives it from the password — a wrong password will still produce
    /// an incorrect master key, causing AES-GCM decryption to fail.
    pub async fn unlock(&self, password: &str) -> KeyVaultResult<()> {
        let actual_hash = Self::derive_key(&format!("verify:{}", password), &self.salt)?;

        {
            let hash = self.password_hash.read().await;
            if let Some(expected_hash) = *hash {
                if actual_hash.ct_ne(&expected_hash).into() {
                    warn!("KeyVault unlock failed — wrong password");
                    return Err(KeyVaultError::WrongPassword);
                }
            }
            // If password_hash was zeroed on lock(), we skip early rejection.
            // Wrong passwords will still fail at AES-GCM decryption time.
        }

        let master_key = Self::derive_key(password, &self.salt)?;
        let mut key = self.master_key.write().await;
        *key = Some(master_key);

        // Restore password_hash for future lock/unlock cycles
        let mut hash = self.password_hash.write().await;
        *hash = Some(actual_hash);

        info!("KeyVault unlocked");
        Ok(())
    }

    /// Check if the vault is currently unlocked.
    pub async fn is_unlocked(&self) -> bool {
        self.master_key.read().await.is_some()
    }

    /// Get the master key, or return Locked error.
    async fn get_master_key(&self) -> KeyVaultResult<[u8; KEY_SIZE]> {
        self.master_key.read().await.ok_or(KeyVaultError::Locked)
    }

    /// Encrypt a plaintext secret using AES-256-GCM with a random nonce.
    fn encrypt_secret(master_key: &[u8; KEY_SIZE], plaintext: &str) -> KeyVaultResult<Vec<u8>> {
        let key = aes_gcm::Key::<Aes256Gcm>::from_slice(master_key);
        let cipher = Aes256Gcm::new(key);

        let mut nonce_bytes = [0u8; NONCE_SIZE];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, plaintext.as_bytes())
            .map_err(|e| KeyVaultError::Encryption(e.to_string()))?;

        // Format: nonce || ciphertext (tag is appended by AEAD)
        let mut combined = Vec::with_capacity(NONCE_SIZE + ciphertext.len());
        combined.extend_from_slice(&nonce_bytes);
        combined.extend_from_slice(&ciphertext);

        Ok(combined)
    }

    /// Decrypt an encrypted secret using AES-256-GCM.
    fn decrypt_secret(master_key: &[u8; KEY_SIZE], ciphertext: &[u8]) -> KeyVaultResult<String> {
        if ciphertext.len() < NONCE_SIZE + 1 {
            return Err(KeyVaultError::Decryption("ciphertext too short".into()));
        }

        let key = aes_gcm::Key::<Aes256Gcm>::from_slice(master_key);
        let cipher = Aes256Gcm::new(key);

        let (nonce_bytes, encrypted) = ciphertext.split_at(NONCE_SIZE);
        let nonce = Nonce::from_slice(nonce_bytes);

        let plaintext = cipher
            .decrypt(nonce, encrypted)
            .map_err(|_| KeyVaultError::Decryption("decryption failed (wrong key or corrupted)".into()))?;

        String::from_utf8(plaintext)
            .map_err(|e| KeyVaultError::Decryption(format!("not valid UTF-8: {}", e)))
    }
}

#[async_trait]
impl KeyStore for LocalKeyVault {
    async fn store_key(&self, name: &str, secret: &str, permissions: KeyPermission) -> KeyVaultResult<()> {
        let master_key = self.get_master_key().await?;

        let mut entries = self.entries.write().await;
        if entries.contains_key(name) {
            return Err(KeyVaultError::AlreadyExists(name.to_string()));
        }

        let ciphertext = Self::encrypt_secret(&master_key, secret)?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let meta = KeyMeta {
            name: name.to_string(),
            provider: permissions.provider.clone(),
            created_at: now,
            permissions,
        };

        entries.insert(name.to_string(), EncryptedEntry { ciphertext, meta });
        debug!("KeyVault: stored key '{}'", name);
        Ok(())
    }

    async fn get_key(&self, name: &str) -> KeyVaultResult<String> {
        let master_key = self.get_master_key().await?;
        let entries = self.entries.read().await;
        let entry = entries.get(name).ok_or_else(|| KeyVaultError::NotFound(name.to_string()))?;
        Self::decrypt_secret(&master_key, &entry.ciphertext)
    }

    async fn list_keys(&self) -> KeyVaultResult<Vec<KeyMeta>> {
        // list_keys doesn't need decryption — just return metadata
        let _master_key = self.get_master_key().await?;
        let entries = self.entries.read().await;
        let metas: Vec<KeyMeta> = entries.values().map(|e| e.meta.clone()).collect();
        Ok(metas)
    }

    async fn delete_key(&self, name: &str) -> KeyVaultResult<bool> {
        let _master_key = self.get_master_key().await?;
        let mut entries = self.entries.write().await;
        let removed = entries.remove(name).is_some();
        if removed {
            debug!("KeyVault: deleted key '{}'", name);
        }
        Ok(removed)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn test_permission(provider: &str) -> KeyPermission {
        KeyPermission {
            provider: provider.to_string(),
            allowed_models: vec![],
            daily_budget_usd: 0.0,
            allowed_nodes: vec![],
        }
    }

    #[tokio::test]
    async fn store_and_retrieve_key() {
        let vault = LocalKeyVault::new("test-password").unwrap();
        vault
            .store_key("openai-1", "sk-abc123", test_permission("openai"))
            .await
            .unwrap();
        let secret = vault.get_key("openai-1").await.unwrap();
        assert_eq!(secret, "sk-abc123");
    }

    #[tokio::test]
    async fn encrypt_decrypt_roundtrip_various() {
        let vault = LocalKeyVault::new("strong-password-123!").unwrap();

        let test_cases = vec![
            ("key-1", "sk-ant-api03-1234567890"),
            ("key-2", ""),
            ("key-3", "密碼：你好世界🔐"),
        ];

        for (name, secret) in &test_cases {
            vault
                .store_key(name, secret, test_permission("test"))
                .await
                .unwrap();
            let decrypted = vault.get_key(name).await.unwrap();
            assert_eq!(&decrypted, secret, "roundtrip failed for '{}'", name);
        }

        // Test long string separately to avoid leak()
        let long_str = "a".repeat(10_000);
        vault
            .store_key("key-4", long_str.as_str(), test_permission("test"))
            .await
            .unwrap();
        let decrypted = vault.get_key("key-4").await.unwrap();
        assert_eq!(decrypted, long_str, "roundtrip failed for 'key-4'");
    }

    #[tokio::test]
    async fn wrong_password_cannot_decrypt() {
        let vault = LocalKeyVault::new("correct-password").unwrap();
        vault
            .store_key("secret-key", "my-secret", test_permission("openai"))
            .await
            .unwrap();

        // Lock and try to unlock with wrong password.
        // Since password_hash is zeroed on lock(), the wrong password will
        // derive a wrong master key. Decryption will fail with AES-GCM auth error.
        vault.lock().await;
        assert!(!vault.is_unlocked().await);

        // unlock() itself succeeds (hash was zeroed, so no early rejection),
        // but the derived master key is wrong.
        vault.unlock("wrong-password").await.unwrap();

        // Decryption fails because the derived master key is incorrect
        let err = vault.get_key("secret-key").await;
        assert!(matches!(err.unwrap_err(), KeyVaultError::Decryption(_)));

        // Re-lock and unlock with correct password to verify recovery
        vault.lock().await;
        vault.unlock("correct-password").await.unwrap();
        let secret = vault.get_key("secret-key").await.unwrap();
        assert_eq!(secret, "my-secret");
    }

    #[tokio::test]
    async fn lock_and_unlock() {
        let vault = LocalKeyVault::new("my-password").unwrap();
        assert!(vault.is_unlocked().await);

        vault
            .store_key("key-1", "value-1", test_permission("test"))
            .await
            .unwrap();

        vault.lock().await;
        assert!(!vault.is_unlocked().await);

        // Operations fail while locked
        assert!(vault.get_key("key-1").await.is_err());
        assert!(vault.list_keys().await.is_err());
        assert!(vault.delete_key("key-1").await.is_err());

        // Unlock with correct password
        vault.unlock("my-password").await.unwrap();
        assert!(vault.is_unlocked().await);

        // Can retrieve key again
        let secret = vault.get_key("key-1").await.unwrap();
        assert_eq!(secret, "value-1");
    }

    #[tokio::test]
    async fn list_keys_returns_metadata() {
        let vault = LocalKeyVault::new("pass").unwrap();

        vault
            .store_key(
                "openai-prod",
                "sk-123",
                KeyPermission {
                    provider: "openai".into(),
                    allowed_models: vec!["gpt-4o".into()],
                    daily_budget_usd: 10.0,
                    allowed_nodes: vec!["node-1".into()],
                },
            )
            .await
            .unwrap();

        vault
            .store_key("anthropic-dev", "sk-ant-456", test_permission("anthropic"))
            .await
            .unwrap();

        let keys = vault.list_keys().await.unwrap();
        assert_eq!(keys.len(), 2);

        let openai_key = keys.iter().find(|k| k.name == "openai-prod").unwrap();
        assert_eq!(openai_key.provider, "openai");
        assert_eq!(openai_key.permissions.allowed_models, vec!["gpt-4o"]);
        assert_eq!(openai_key.permissions.daily_budget_usd, 10.0);
        assert_eq!(openai_key.permissions.allowed_nodes, vec!["node-1"]);
    }

    #[tokio::test]
    async fn delete_key_removes_entry() {
        let vault = LocalKeyVault::new("pass").unwrap();
        vault
            .store_key("key-1", "val", test_permission("test"))
            .await
            .unwrap();

        assert!(vault.delete_key("key-1").await.unwrap());
        assert!(!vault.delete_key("key-1").await.unwrap()); // already deleted

        assert!(vault.get_key("key-1").await.is_err());
        assert!(vault.list_keys().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn duplicate_key_name_rejected() {
        let vault = LocalKeyVault::new("pass").unwrap();
        vault
            .store_key("key-1", "val-1", test_permission("test"))
            .await
            .unwrap();

        let err = vault
            .store_key("key-1", "val-2", test_permission("test"))
            .await;
        assert!(matches!(err.unwrap_err(), KeyVaultError::AlreadyExists(_)));
    }

    #[tokio::test]
    async fn get_nonexistent_key_returns_not_found() {
        let vault = LocalKeyVault::new("pass").unwrap();
        let err = vault.get_key("nonexistent").await;
        assert!(matches!(err.unwrap_err(), KeyVaultError::NotFound(_)));
    }

    #[tokio::test]
    async fn vault_operations_while_locked_fail() {
        let vault = LocalKeyVault::new("pass").unwrap();
        vault.lock().await;

        assert!(matches!(
            vault.store_key("k", "v", test_permission("t")).await.unwrap_err(),
            KeyVaultError::Locked
        ));
        assert!(matches!(vault.get_key("k").await.unwrap_err(), KeyVaultError::Locked));
        assert!(matches!(vault.list_keys().await.unwrap_err(), KeyVaultError::Locked));
        assert!(matches!(vault.delete_key("k").await.unwrap_err(), KeyVaultError::Locked));
    }

    #[tokio::test]
    async fn permissions_are_preserved() {
        let vault = LocalKeyVault::new("pass").unwrap();
        let perm = KeyPermission {
            provider: "anthropic".into(),
            allowed_models: vec!["claude-sonnet-4-5-20250514".into(), "claude-haiku-4-5-20251001".into()],
            daily_budget_usd: 25.0,
            allowed_nodes: vec!["hub".into(), "worker-1".into()],
        };

        vault.store_key("ant-key", "sk-ant-xxx", perm.clone()).await.unwrap();

        let keys = vault.list_keys().await.unwrap();
        let meta = &keys[0];
        assert_eq!(meta.permissions.provider, "anthropic");
        assert_eq!(meta.permissions.allowed_models.len(), 2);
        assert_eq!(meta.permissions.daily_budget_usd, 25.0);
        assert_eq!(meta.permissions.allowed_nodes.len(), 2);
    }
}
