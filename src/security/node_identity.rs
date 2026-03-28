//! Node Identity — Ed25519 keypair management for P2P RPC authentication.
//!
//! Each node in the cluster has a unique identity derived from an Ed25519 keypair.
//! The keypair is persisted to disk as `identity.key` (64 bytes: 32-byte secret + 32-byte public).

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Expected size of the identity key file: 32 (secret) + 32 (public) = 64 bytes.
const IDENTITY_KEY_LEN: usize = 64;

/// Node identity backed by an Ed25519 keypair.
///
/// `node_id` is derived as the first 16 hex characters of the public key,
/// giving 8 bytes of uniqueness — more than enough for small clusters.
pub struct NodeIdentity {
    /// Short hex identifier: `hex(public_key)[..16]`.
    pub node_id: String,
    /// Ed25519 signing (private) key — never exposed directly.
    signing_key: SigningKey,
    /// Ed25519 verifying (public) key.
    pub verifying_key: VerifyingKey,
}

impl NodeIdentity {
    /// Load an existing keypair from `{data_dir}/identity.key`, or generate a
    /// new one if the file does not exist or is malformed.
    ///
    /// The key file is 64 bytes of raw binary: the first 32 bytes are the
    /// secret key, and the remaining 32 bytes are the public key.
    ///
    /// On Unix the file is written with mode `0600`; on Windows no special
    /// permissions are applied.
    pub fn load_or_generate(data_dir: &Path) -> anyhow::Result<Self> {
        let key_path = Self::key_path(data_dir);

        if key_path.exists() {
            match Self::load_from_file(&key_path) {
                Ok(identity) => {
                    info!(node_id = %identity.node_id, "Loaded existing node identity");
                    return Ok(identity);
                }
                Err(e) => {
                    warn!(?e, "Failed to load identity.key — generating new keypair");
                }
            }
        }

        let identity = Self::generate_and_save(data_dir, &key_path)?;
        info!(node_id = %identity.node_id, "Generated new node identity");
        Ok(identity)
    }

    /// Sign a message with this node's private key.
    ///
    /// Returns the 64-byte Ed25519 signature as a `Vec<u8>`.
    pub fn sign(&self, message: &[u8]) -> Vec<u8> {
        let sig: Signature = self.signing_key.sign(message);
        sig.to_bytes().to_vec()
    }

    /// Verify a signature produced by another node.
    ///
    /// * `public_key_bytes` — 32-byte Ed25519 public key of the signer.
    /// * `message` — the original message bytes.
    /// * `signature` — the 64-byte Ed25519 signature.
    ///
    /// Returns `true` if the signature is valid, `false` otherwise.
    pub fn verify(public_key_bytes: &[u8], message: &[u8], signature: &[u8]) -> bool {
        let pk_array: [u8; 32] = match public_key_bytes.try_into() {
            Ok(a) => a,
            Err(_) => {
                debug!("verify: public_key_bytes length != 32");
                return false;
            }
        };

        let verifying_key = match VerifyingKey::from_bytes(&pk_array) {
            Ok(vk) => vk,
            Err(_) => {
                debug!("verify: invalid public key");
                return false;
            }
        };

        let sig = match Signature::from_slice(signature) {
            Ok(s) => s,
            Err(_) => {
                debug!("verify: invalid signature bytes");
                return false;
            }
        };

        verifying_key.verify(message, &sig).is_ok()
    }

    /// Export this node's public key as a 32-byte `Vec<u8>`.
    pub fn public_key_bytes(&self) -> Vec<u8> {
        self.verifying_key.to_bytes().to_vec()
    }

    // -- Backward-compatible accessors (used by cluster_auth.rs) ---------------

    /// Returns the short hex node ID.
    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    /// Returns a reference to the verifying (public) key.
    pub fn verifying_key(&self) -> &VerifyingKey {
        &self.verifying_key
    }

    /// Returns a reference to the signing (private) key.
    pub fn signing_key(&self) -> &SigningKey {
        &self.signing_key
    }

    // ---- internal helpers ----

    /// Canonical path for the identity key file.
    fn key_path(data_dir: &Path) -> PathBuf {
        data_dir.join("identity.key")
    }

    /// Derive the short node id from a verifying key.
    fn derive_node_id(vk: &VerifyingKey) -> String {
        let full_hex = hex::encode(vk.to_bytes());
        full_hex[..16].to_string()
    }

    /// Attempt to load a keypair from an existing key file.
    fn load_from_file(key_path: &Path) -> anyhow::Result<Self> {
        let data = fs::read(key_path)?;
        if data.len() != IDENTITY_KEY_LEN {
            anyhow::bail!(
                "identity.key has wrong size: expected {IDENTITY_KEY_LEN}, got {}",
                data.len()
            );
        }

        let secret_bytes: [u8; 32] = data[..32]
            .try_into()
            .expect("slice length verified above");

        let signing_key = SigningKey::from_bytes(&secret_bytes);
        let verifying_key = signing_key.verifying_key();

        // Sanity-check: the stored public key should match the derived one.
        let stored_public: [u8; 32] = data[32..64]
            .try_into()
            .expect("slice length verified above");

        if stored_public != verifying_key.to_bytes() {
            anyhow::bail!("identity.key public key mismatch — file may be corrupted");
        }

        let node_id = Self::derive_node_id(&verifying_key);
        Ok(Self {
            node_id,
            signing_key,
            verifying_key,
        })
    }

    /// Generate a fresh keypair, persist it, and return the identity.
    fn generate_and_save(data_dir: &Path, key_path: &Path) -> anyhow::Result<Self> {
        fs::create_dir_all(data_dir)?;

        let mut csprng = OsRng;
        let signing_key = SigningKey::generate(&mut csprng);
        let verifying_key = signing_key.verifying_key();

        // Build 64-byte file: secret (32) + public (32)
        let mut key_data = Vec::with_capacity(IDENTITY_KEY_LEN);
        key_data.extend_from_slice(&signing_key.to_bytes());
        key_data.extend_from_slice(&verifying_key.to_bytes());

        fs::write(key_path, &key_data)?;

        // On Unix, restrict permissions to owner-only.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(key_path, fs::Permissions::from_mode(0o600))?;
        }

        let node_id = Self::derive_node_id(&verifying_key);
        Ok(Self {
            node_id,
            signing_key,
            verifying_key,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_node_id_length() {
        let mut csprng = OsRng;
        let sk = SigningKey::generate(&mut csprng);
        let vk = sk.verifying_key();
        let nid = NodeIdentity::derive_node_id(&vk);
        assert_eq!(nid.len(), 16);
        assert!(nid.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
