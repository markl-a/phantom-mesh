//! Cluster Auth — Ed25519-based RPC request signing and verification.
//!
//! Every inter-node RPC request is signed with the sender's Ed25519 key.
//! The signed message format is:
//!
//! ```text
//! "{method}|{timestamp}|{sha256_hex(body)}|{cluster_secret}"
//! ```
//!
//! Verification checks:
//! 1. `node_id` is in `known_peers`
//! 2. `timestamp` is within ±30 seconds of current time
//! 3. Ed25519 signature is valid for the reconstructed message

use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use sha2::{Sha256, Digest};
use thiserror::Error;
use tracing::{debug, warn};

use super::node_identity::NodeIdentity;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum allowed clock skew between nodes (in seconds).
const MAX_TIMESTAMP_DELTA: i64 = 30;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Auth headers attached to every inter-node RPC request.
pub struct AuthHeaders {
    /// The sender's short hex node ID.
    pub node_id: String,
    /// Unix epoch seconds when the request was signed.
    pub timestamp: u64,
    /// Hex-encoded 64-byte Ed25519 signature.
    pub signature: String,
}

/// Errors returned by signature verification.
#[derive(Debug, Error)]
pub enum AuthError {
    #[error("unknown node: {0}")]
    UnknownNode(String),

    #[error("timestamp expired (delta: {0}s, max: {MAX_TIMESTAMP_DELTA}s)")]
    TimestampExpired(i64),

    #[error("invalid signature")]
    InvalidSignature,

    #[error("invalid public key")]
    InvalidPublicKey,
}

/// Per-node RPC authenticator.
///
/// Holds the local node's identity, the shared cluster secret, and a
/// registry of known peer public keys.
pub struct ClusterAuth {
    identity: NodeIdentity,
    cluster_secret: String,
    known_peers: RwLock<HashMap<String, VerifyingKey>>,
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

impl ClusterAuth {
    /// Create a new `ClusterAuth` instance.
    ///
    /// The local node's own public key is automatically registered in
    /// `known_peers` so that self-signed requests (e.g., loopback) also
    /// verify.
    pub fn new(identity: NodeIdentity, cluster_secret: String) -> Self {
        let mut peers = HashMap::new();
        // Register ourselves so loopback requests also verify.
        peers.insert(
            identity.node_id().to_string(),
            *identity.verifying_key(),
        );

        Self {
            identity,
            cluster_secret,
            known_peers: RwLock::new(peers),
        }
    }

    // -- Signing -----------------------------------------------------------

    /// Create auth headers for an outgoing RPC request.
    ///
    /// Signs: `"{method}|{timestamp}|{sha256_hex(body)}|{cluster_secret}"`
    pub fn sign_request(&self, method: &str, body: &[u8]) -> AuthHeaders {
        let timestamp = now_epoch_secs();
        let message = self.build_signed_message(method, timestamp, body);

        let signature_bytes = self.identity.sign(message.as_bytes());
        let sig_hex = hex::encode(&signature_bytes);

        debug!(
            node_id = %self.identity.node_id(),
            method,
            timestamp,
            "signed RPC request"
        );

        AuthHeaders {
            node_id: self.identity.node_id().to_string(),
            timestamp,
            signature: sig_hex,
        }
    }

    // -- Verification ------------------------------------------------------

    /// Verify an incoming RPC request's auth headers.
    ///
    /// Returns the verified `node_id` on success.
    pub fn verify_request(
        &self,
        headers: &AuthHeaders,
        method: &str,
        body: &[u8],
    ) -> Result<String, AuthError> {
        // 1. Look up peer's public key.
        let verifying_key = {
            let peers = self
                .known_peers
                .read()
                .expect("known_peers RwLock poisoned");
            peers
                .get(&headers.node_id)
                .copied()
                .ok_or_else(|| AuthError::UnknownNode(headers.node_id.clone()))?
        };

        // 2. Check timestamp freshness.
        let now = now_epoch_secs() as i64;
        let ts = headers.timestamp as i64;
        let delta = (now - ts).abs();
        if delta > MAX_TIMESTAMP_DELTA {
            warn!(
                node_id = %headers.node_id,
                delta,
                "RPC timestamp expired"
            );
            return Err(AuthError::TimestampExpired(delta));
        }

        // 3. Reconstruct the signed message.
        let message = self.build_signed_message(method, headers.timestamp, body);

        // 4. Decode signature from hex and verify.
        let sig_bytes = hex::decode(&headers.signature)
            .map_err(|_| AuthError::InvalidSignature)?;
        let sig_array: [u8; 64] = sig_bytes
            .try_into()
            .map_err(|_| AuthError::InvalidSignature)?;
        let signature = Signature::from_bytes(&sig_array);

        verifying_key
            .verify(message.as_bytes(), &signature)
            .map_err(|_| AuthError::InvalidSignature)?;

        debug!(
            node_id = %headers.node_id,
            method,
            "RPC request verified"
        );

        Ok(headers.node_id.clone())
    }

    // -- Peer management ---------------------------------------------------

    /// Register a peer's public key (called during pairing / discovery).
    pub fn register_peer(
        &self,
        node_id: &str,
        public_key_bytes: &[u8],
    ) -> Result<(), AuthError> {
        let key_array: [u8; 32] = public_key_bytes
            .try_into()
            .map_err(|_| AuthError::InvalidPublicKey)?;

        let verifying_key = VerifyingKey::from_bytes(&key_array)
            .map_err(|_| AuthError::InvalidPublicKey)?;

        let mut peers = self
            .known_peers
            .write()
            .expect("known_peers RwLock poisoned");
        peers.insert(node_id.to_string(), verifying_key);

        debug!(node_id, "registered peer public key");
        Ok(())
    }

    /// Check if a peer is known (i.e., has a registered public key).
    pub fn is_known_peer(&self, node_id: &str) -> bool {
        let peers = self
            .known_peers
            .read()
            .expect("known_peers RwLock poisoned");
        peers.contains_key(node_id)
    }

    // -- Internal helpers --------------------------------------------------

    /// Build the canonical message that gets signed / verified.
    ///
    /// Format: `"{method}|{timestamp}|{body_sha256_hex}|{cluster_secret}"`
    fn build_signed_message(&self, method: &str, timestamp: u64, body: &[u8]) -> String {
        let body_hash = hex::encode(Sha256::digest(body));
        format!(
            "{}|{}|{}|{}",
            method, timestamp, body_hash, self.cluster_secret
        )
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn now_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before UNIX epoch")
        .as_secs()
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_auth(dir: &std::path::Path, secret: &str) -> ClusterAuth {
        let identity = NodeIdentity::load_or_generate(dir).unwrap();
        ClusterAuth::new(identity, secret.to_string())
    }

    #[test]
    fn sign_and_verify_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let auth = make_auth(dir.path(), "test-secret");

        let headers = auth.sign_request("POST /rpc/dispatch", b"hello world");
        let result = auth.verify_request(&headers, "POST /rpc/dispatch", b"hello world");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), auth.identity.node_id());
    }

    #[test]
    fn self_is_known_peer() {
        let dir = tempfile::tempdir().unwrap();
        let auth = make_auth(dir.path(), "s");
        assert!(auth.is_known_peer(auth.identity.node_id()));
    }
}
