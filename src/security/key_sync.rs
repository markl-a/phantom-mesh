//! Cluster Key Sync — secure distribution of API keys from Hub to Workers.
//!
//! Uses X25519 Diffie-Hellman key exchange to establish a shared secret, then
//! derives an AES-256-GCM session key for envelope encryption of API keys.
//!
//! # Protocol
//!
//! 1. Hub generates ephemeral X25519 keypair
//! 2. Worker sends its public key
//! 3. Both sides derive shared secret → HKDF → session key
//! 4. Hub encrypts API keys with session key, sends to Worker
//! 5. Worker decrypts and stores in its local KeyVault
//!
//! # Key revocation
//!
//! Hub can push a `KeyRevoke` message to invalidate a distributed key.
//! Workers must delete the key and stop using it.

use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Nonce};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use hkdf::Hkdf;
use thiserror::Error;
use tracing::{debug, info, warn};
use x25519_dalek::{EphemeralSecret, PublicKey, SharedSecret};
use zeroize::Zeroize;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const NONCE_SIZE: usize = 12;
const SESSION_KEY_SIZE: usize = 32;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum KeySyncError {
    #[error("key exchange not completed")]
    NoSession,

    #[error("encryption failed: {0}")]
    Encryption(String),

    #[error("decryption failed: {0}")]
    Decryption(String),

    #[error("key revoked: {0}")]
    Revoked(String),

    #[error("invalid message")]
    InvalidMessage,
}

pub type KeySyncResult<T> = Result<T, KeySyncError>;

// ---------------------------------------------------------------------------
// KeySyncMessage
// ---------------------------------------------------------------------------

/// Messages exchanged during key sync protocol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum KeySyncMessage {
    /// Step 1: Exchange public keys
    KeyExchange {
        /// Sender's X25519 public key (32 bytes)
        public_key: Vec<u8>,
        /// Sender identifier
        sender_id: String,
    },

    /// Step 2: Distribute encrypted API keys
    KeyDistribute {
        /// Key name (e.g. "openai-prod-1")
        key_name: String,
        /// Provider name
        provider: String,
        /// AES-256-GCM encrypted key data (nonce || ciphertext)
        encrypted_key: Vec<u8>,
        /// Optional per-key permissions
        permissions: Option<super::key_vault::KeyPermission>,
    },

    /// Step 3: Revoke a previously distributed key
    KeyRevoke {
        /// Key name to revoke
        key_name: String,
        /// Reason for revocation
        reason: String,
        /// Unix timestamp of revocation
        revoked_at: u64,
    },
}

// ---------------------------------------------------------------------------
// SessionKey (derived from X25519 shared secret)
// ---------------------------------------------------------------------------

/// Derives a 32-byte AES-256-GCM session key from an X25519 shared secret
/// using HKDF-SHA256.
fn derive_session_key(shared_secret: &SharedSecret) -> [u8; SESSION_KEY_SIZE] {
    let hk = Hkdf::<sha2::Sha256>::new(None, shared_secret.as_bytes());
    let mut key = [0u8; SESSION_KEY_SIZE];
    hk.expand(b"phantom_mesh-key-sync-v1", &mut key)
        .expect("32 bytes is a valid HKDF-SHA256 output length");
    key
}

/// Encrypt data with a session key using AES-256-GCM.
fn encrypt_with_session(session_key: &[u8; SESSION_KEY_SIZE], plaintext: &[u8]) -> KeySyncResult<Vec<u8>> {
    let key = aes_gcm::Key::<Aes256Gcm>::from_slice(session_key);
    let cipher = Aes256Gcm::new(key);

    let mut nonce_bytes = [0u8; NONCE_SIZE];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| KeySyncError::Encryption(e.to_string()))?;

    let mut combined = Vec::with_capacity(NONCE_SIZE + ciphertext.len());
    combined.extend_from_slice(&nonce_bytes);
    combined.extend_from_slice(&ciphertext);

    Ok(combined)
}

/// Decrypt data with a session key using AES-256-GCM.
fn decrypt_with_session(session_key: &[u8; SESSION_KEY_SIZE], ciphertext: &[u8]) -> KeySyncResult<Vec<u8>> {
    if ciphertext.len() < NONCE_SIZE + 1 {
        return Err(KeySyncError::Decryption("ciphertext too short".into()));
    }

    let key = aes_gcm::Key::<Aes256Gcm>::from_slice(session_key);
    let cipher = Aes256Gcm::new(key);

    let (nonce_bytes, encrypted) = ciphertext.split_at(NONCE_SIZE);
    let nonce = Nonce::from_slice(nonce_bytes);

    let plaintext = cipher
        .decrypt(nonce, encrypted)
        .map_err(|_| KeySyncError::Decryption("decryption failed".into()))?;

    Ok(plaintext)
}

// ---------------------------------------------------------------------------
// KeySyncServer (Hub side)
// ---------------------------------------------------------------------------

/// Hub-side key sync: generates ephemeral keypair, processes worker exchanges,
/// encrypts and distributes API keys.
pub struct KeySyncServer {
    /// Node ID for this hub
    node_id: String,
    /// Active sessions: worker_id → session_key
    sessions: std::collections::HashMap<String, [u8; SESSION_KEY_SIZE]>,
    /// Revoked key names
    revoked_keys: std::collections::HashSet<String>,
}

impl KeySyncServer {
    pub fn new(node_id: &str) -> Self {
        Self {
            node_id: node_id.to_string(),
            sessions: std::collections::HashMap::new(),
            revoked_keys: std::collections::HashSet::new(),
        }
    }

    /// Start a key exchange with a worker. Returns our public key bytes.
    /// The caller must send the public key to the worker.
    pub fn initiate_exchange(&mut self, worker_id: &str, worker_public_key: &[u8; 32]) -> KeySyncResult<Vec<u8>> {
        let server_secret = EphemeralSecret::random_from_rng(OsRng);
        let server_public = PublicKey::from(&server_secret);

        let worker_pk = PublicKey::from(*worker_public_key);
        let shared_secret = server_secret.diffie_hellman(&worker_pk);
        let session_key = derive_session_key(&shared_secret);

        self.sessions.insert(worker_id.to_string(), session_key);

        info!("KeySyncServer: established session with '{}'", worker_id);
        Ok(server_public.as_bytes().to_vec())
    }

    /// Encrypt an API key for distribution to a specific worker.
    pub fn encrypt_key_for_worker(
        &self,
        worker_id: &str,
        key_name: &str,
        provider: &str,
        api_key: &str,
        permissions: Option<super::key_vault::KeyPermission>,
    ) -> KeySyncResult<KeySyncMessage> {
        let session_key = self
            .sessions
            .get(worker_id)
            .ok_or(KeySyncError::NoSession)?;

        if self.revoked_keys.contains(key_name) {
            return Err(KeySyncError::Revoked(key_name.to_string()));
        }

        let encrypted = encrypt_with_session(session_key, api_key.as_bytes())?;

        debug!("KeySyncServer: encrypted key '{}' for worker '{}'", key_name, worker_id);

        Ok(KeySyncMessage::KeyDistribute {
            key_name: key_name.to_string(),
            provider: provider.to_string(),
            encrypted_key: encrypted,
            permissions,
        })
    }

    /// Revoke a key — prevents future distribution and creates a revoke message.
    pub fn revoke_key(&mut self, key_name: &str, reason: &str) -> KeySyncMessage {
        self.revoked_keys.insert(key_name.to_string());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        info!("KeySyncServer: revoked key '{}' — {}", key_name, reason);

        KeySyncMessage::KeyRevoke {
            key_name: key_name.to_string(),
            reason: reason.to_string(),
            revoked_at: now,
        }
    }

    /// Check if a session exists for a worker.
    pub fn has_session(&self, worker_id: &str) -> bool {
        self.sessions.contains_key(worker_id)
    }

    /// Remove a worker's session (e.g. on disconnect).
    pub fn remove_session(&mut self, worker_id: &str) -> bool {
        if let Some(mut key) = self.sessions.remove(worker_id) {
            key.zeroize();
            true
        } else {
            false
        }
    }
}

// ---------------------------------------------------------------------------
// KeySyncClient (Worker side)
// ---------------------------------------------------------------------------

/// Worker-side key sync: initiates exchange, decrypts received keys.
pub struct KeySyncClient {
    /// Node ID for this worker
    node_id: String,
    /// Session key (established after exchange)
    session_key: Option<[u8; SESSION_KEY_SIZE]>,
    /// Received keys: key_name → (decrypted API key, optional permissions)
    received_keys: std::collections::HashMap<String, (String, Option<super::key_vault::KeyPermission>)>,
    /// Revoked key names
    revoked_keys: std::collections::HashSet<String>,
}

impl KeySyncClient {
    pub fn new(node_id: &str) -> Self {
        Self {
            node_id: node_id.to_string(),
            session_key: None,
            received_keys: std::collections::HashMap::new(),
            revoked_keys: std::collections::HashSet::new(),
        }
    }

    /// Start key exchange. Returns (our public key bytes, our ephemeral secret).
    /// The secret must be kept until `complete_exchange` is called.
    pub fn begin_exchange() -> (Vec<u8>, EphemeralSecret) {
        let secret = EphemeralSecret::random_from_rng(OsRng);
        let public = PublicKey::from(&secret);
        (public.as_bytes().to_vec(), secret)
    }

    /// Complete key exchange with the hub's public key.
    pub fn complete_exchange(&mut self, secret: EphemeralSecret, hub_public_key: &[u8; 32]) {
        let hub_pk = PublicKey::from(*hub_public_key);
        let shared_secret = secret.diffie_hellman(&hub_pk);
        let session_key = derive_session_key(&shared_secret);
        // Zeroize old session key if present
        if let Some(ref mut old_key) = self.session_key {
            old_key.zeroize();
        }
        self.session_key = Some(session_key);
        info!("KeySyncClient [{}]: session established with hub", self.node_id);
    }

    /// Process an incoming KeySyncMessage.
    pub fn process_message(&mut self, msg: KeySyncMessage) -> KeySyncResult<Option<String>> {
        match msg {
            KeySyncMessage::KeyDistribute {
                key_name,
                provider: _,
                encrypted_key,
                permissions,
            } => {
                if self.revoked_keys.contains(&key_name) {
                    warn!("KeySyncClient: ignoring distribute for revoked key '{}'", key_name);
                    return Err(KeySyncError::Revoked(key_name));
                }

                let session_key = self.session_key.ok_or(KeySyncError::NoSession)?;
                let plaintext = decrypt_with_session(&session_key, &encrypted_key)?;
                let api_key = String::from_utf8(plaintext)
                    .map_err(|_| KeySyncError::Decryption("not valid UTF-8".into()))?;

                self.received_keys.insert(key_name.clone(), (api_key.clone(), permissions));
                debug!("KeySyncClient: received key '{}'", key_name);
                Ok(Some(api_key))
            }
            KeySyncMessage::KeyRevoke {
                key_name,
                reason,
                revoked_at: _,
            } => {
                self.received_keys.remove(&key_name);
                self.revoked_keys.insert(key_name.clone());
                info!("KeySyncClient: revoked key '{}' — {}", key_name, reason);
                Ok(None)
            }
            KeySyncMessage::KeyExchange { .. } => {
                // Exchange messages are handled via begin_exchange/complete_exchange
                Err(KeySyncError::InvalidMessage)
            }
        }
    }

    /// Get a previously received key and its optional permissions.
    pub fn get_key(&self, key_name: &str) -> Option<(&str, Option<&super::key_vault::KeyPermission>)> {
        if self.revoked_keys.contains(key_name) {
            return None;
        }
        self.received_keys
            .get(key_name)
            .map(|(key, perms)| (key.as_str(), perms.as_ref()))
    }

    /// Check if a session is established.
    pub fn has_session(&self) -> bool {
        self.session_key.is_some()
    }

    /// List all received key names.
    pub fn received_key_names(&self) -> Vec<String> {
        self.received_keys.keys().cloned().collect()
    }
}

impl Drop for KeySyncClient {
    fn drop(&mut self) {
        if let Some(ref mut key) = self.session_key {
            key.zeroize();
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_exchange_and_distribute() {
        let mut server = KeySyncServer::new("hub-1");
        let mut client = KeySyncClient::new("worker-1");

        // 1. Client begins exchange
        let (client_public, client_secret) = KeySyncClient::begin_exchange();
        let client_pk: [u8; 32] = client_public.try_into().unwrap();

        // 2. Server processes exchange, gets session
        let server_public = server.initiate_exchange("worker-1", &client_pk).unwrap();
        let server_pk: [u8; 32] = server_public.try_into().unwrap();

        // 3. Client completes exchange
        client.complete_exchange(client_secret, &server_pk);
        assert!(client.has_session());
        assert!(server.has_session("worker-1"));

        // 4. Server encrypts and distributes a key
        let msg = server
            .encrypt_key_for_worker("worker-1", "openai-prod", "openai", "sk-abc123456", None)
            .unwrap();

        // 5. Client decrypts
        let api_key = client.process_message(msg).unwrap().unwrap();
        assert_eq!(api_key, "sk-abc123456");
        assert_eq!(client.get_key("openai-prod").map(|(k, _)| k), Some("sk-abc123456"));
    }

    #[test]
    fn revocation_flow() {
        let mut server = KeySyncServer::new("hub-1");
        let mut client = KeySyncClient::new("worker-1");

        // Establish session
        let (client_public, client_secret) = KeySyncClient::begin_exchange();
        let client_pk: [u8; 32] = client_public.try_into().unwrap();
        let server_public = server.initiate_exchange("worker-1", &client_pk).unwrap();
        let server_pk: [u8; 32] = server_public.try_into().unwrap();
        client.complete_exchange(client_secret, &server_pk);

        // Distribute a key
        let msg = server
            .encrypt_key_for_worker("worker-1", "key-1", "openai", "sk-123", None)
            .unwrap();
        client.process_message(msg).unwrap();
        assert_eq!(client.get_key("key-1").map(|(k, _)| k), Some("sk-123"));

        // Revoke the key
        let revoke_msg = server.revoke_key("key-1", "compromised");
        client.process_message(revoke_msg).unwrap();

        // Key should be gone
        assert!(client.get_key("key-1").is_none());

        // Server should refuse to re-distribute revoked key
        let err = server.encrypt_key_for_worker("worker-1", "key-1", "openai", "sk-new", None);
        assert!(matches!(err.unwrap_err(), KeySyncError::Revoked(_)));
    }

    #[test]
    fn no_session_cannot_decrypt() {
        let mut client = KeySyncClient::new("worker-1");
        let msg = KeySyncMessage::KeyDistribute {
            key_name: "key-1".into(),
            provider: "openai".into(),
            encrypted_key: vec![0u8; 64],
            permissions: None,
        };
        let err = client.process_message(msg);
        assert!(matches!(err.unwrap_err(), KeySyncError::NoSession));
    }

    #[test]
    fn expired_key_not_accessible() {
        let mut client = KeySyncClient::new("worker-1");
        client.received_keys.insert("old-key".into(), ("sk-old".into(), None));
        assert_eq!(client.get_key("old-key").map(|(k, _)| k), Some("sk-old"));

        // Simulate revocation
        client.revoked_keys.insert("old-key".into());
        assert!(client.get_key("old-key").is_none());
    }

    #[test]
    fn server_remove_session() {
        let mut server = KeySyncServer::new("hub-1");
        let (client_public, _) = KeySyncClient::begin_exchange();
        let client_pk: [u8; 32] = client_public.try_into().unwrap();
        server.initiate_exchange("worker-1", &client_pk).unwrap();

        assert!(server.has_session("worker-1"));
        assert!(server.remove_session("worker-1"));
        assert!(!server.has_session("worker-1"));

        // Cannot distribute without session
        let err = server.encrypt_key_for_worker("worker-1", "k", "p", "v", None);
        assert!(matches!(err.unwrap_err(), KeySyncError::NoSession));
    }

    #[test]
    fn multiple_keys_distributed() {
        let mut server = KeySyncServer::new("hub-1");
        let mut client = KeySyncClient::new("worker-1");

        let (client_public, client_secret) = KeySyncClient::begin_exchange();
        let client_pk: [u8; 32] = client_public.try_into().unwrap();
        let server_public = server.initiate_exchange("worker-1", &client_pk).unwrap();
        let server_pk: [u8; 32] = server_public.try_into().unwrap();
        client.complete_exchange(client_secret, &server_pk);

        // Distribute 3 keys
        for (name, provider, key) in [
            ("openai-1", "openai", "sk-openai-123"),
            ("anthropic-1", "anthropic", "sk-ant-456"),
            ("gemini-1", "gemini", "AI-gemini-789"),
        ] {
            let msg = server.encrypt_key_for_worker("worker-1", name, provider, key, None).unwrap();
            let decrypted = client.process_message(msg).unwrap().unwrap();
            assert_eq!(decrypted, key);
        }

        assert_eq!(client.received_key_names().len(), 3);
        assert_eq!(client.get_key("openai-1").map(|(k, _)| k), Some("sk-openai-123"));
        assert_eq!(client.get_key("anthropic-1").map(|(k, _)| k), Some("sk-ant-456"));
        assert_eq!(client.get_key("gemini-1").map(|(k, _)| k), Some("AI-gemini-789"));
    }

    #[test]
    fn encrypt_decrypt_session_key_roundtrip() {
        // Direct test of session-level encryption
        let mut key = [0u8; SESSION_KEY_SIZE];
        OsRng.fill_bytes(&mut key);

        let plaintext = b"sk-ant-api03-super-secret-key-1234567890";
        let encrypted = encrypt_with_session(&key, plaintext).unwrap();
        let decrypted = decrypt_with_session(&key, &encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn wrong_session_key_fails_decrypt() {
        let mut key1 = [0u8; SESSION_KEY_SIZE];
        let mut key2 = [0u8; SESSION_KEY_SIZE];
        OsRng.fill_bytes(&mut key1);
        OsRng.fill_bytes(&mut key2);

        let encrypted = encrypt_with_session(&key1, b"secret").unwrap();
        let result = decrypt_with_session(&key2, &encrypted);
        assert!(result.is_err());
    }
}
