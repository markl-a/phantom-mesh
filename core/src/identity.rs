//! Per-user ed25519 identity for the CONTRIBUTOR-FUNNEL
//! (`docs/CONTRIBUTOR-FUNNEL.md` §5).
//!
//! - `phantom keys init` generates a keypair at
//!   `~/.phantom-mesh/keys/{ed25519.priv, ed25519.pub}`.
//! - Recipes (Tier 2 / 3 of CO-EVOLUTION) are signed with the
//!   private key; `phantom evolve adopt` verifies against the
//!   broker-published public key.
//! - The private key NEVER leaves the user's machine. Public key is
//!   broadcast to the broker on first sync (post-v0.2 once broker
//!   ships).
//!
//! This module ships in v0.1.0 as the down-payment on
//! CONTRIBUTOR-FUNNEL §5 (CO-EVO Phase 3 trust chain). Broker
//! integration + `phantom keys link --github` OAuth land in v0.2.

use anyhow::{anyhow, Context, Result};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey, SECRET_KEY_LENGTH};
use rand::rngs::OsRng;
use std::fs;
use std::path::{Path, PathBuf};

/// Path to `~/.phantom-mesh/keys/`.
pub fn keys_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".phantom-mesh")
        .join("keys")
}

pub fn priv_key_path() -> PathBuf {
    keys_dir().join("ed25519.priv")
}

pub fn pub_key_path() -> PathBuf {
    keys_dir().join("ed25519.pub")
}

/// Result of `phantom keys init`.
#[derive(Debug)]
pub struct InitOutcome {
    pub created: bool,
    pub priv_path: PathBuf,
    pub pub_path: PathBuf,
    /// Hex-encoded public key (display-friendly fingerprint).
    pub pub_hex: String,
}

/// Generate a fresh ed25519 keypair and write it to disk.
///
/// - `~/.phantom-mesh/keys/ed25519.priv` (raw 32-byte seed; mode 0600)
/// - `~/.phantom-mesh/keys/ed25519.pub` (hex-encoded 32-byte verifying key)
///
/// If `force=false` and either file already exists, returns
/// `created=false` so the caller can surface "already initialised"
/// without overwriting. `force=true` overwrites existing keys
/// (destructive — lose all signatures issued by the old key).
pub fn init(force: bool) -> Result<InitOutcome> {
    let dir = keys_dir();
    let priv_path = priv_key_path();
    let pub_path = pub_key_path();

    let already_exists = priv_path.exists() || pub_path.exists();
    if already_exists && !force {
        // Read existing pub key for display.
        let pub_hex = fs::read_to_string(&pub_path)
            .ok()
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "(unreadable)".to_string());
        return Ok(InitOutcome {
            created: false,
            priv_path,
            pub_path,
            pub_hex,
        });
    }

    fs::create_dir_all(&dir)
        .with_context(|| format!("creating {}", dir.display()))?;

    let mut csprng = OsRng;
    let signing = SigningKey::generate(&mut csprng);
    let verifying = signing.verifying_key();

    let priv_bytes = signing.to_bytes();
    let pub_hex = hex::encode(verifying.to_bytes());

    // Write private key as raw 32 bytes; restrict to 0600.
    write_priv_secure(&priv_path, &priv_bytes)
        .with_context(|| format!("writing {}", priv_path.display()))?;

    // Write public key as hex on a single line (human-friendly + safe to grep).
    fs::write(&pub_path, format!("{}\n", pub_hex))
        .with_context(|| format!("writing {}", pub_path.display()))?;

    Ok(InitOutcome {
        created: true,
        priv_path,
        pub_path,
        pub_hex,
    })
}

/// Load this machine's signing key from disk. Errors if the keypair
/// hasn't been initialised yet (`phantom keys init` first).
pub fn load_signing_key() -> Result<SigningKey> {
    let path = priv_key_path();
    let bytes = fs::read(&path).with_context(|| {
        format!(
            "reading {} — run `phantom keys init` first",
            path.display()
        )
    })?;
    if bytes.len() != SECRET_KEY_LENGTH {
        return Err(anyhow!(
            "{} is {} bytes, expected {}",
            path.display(),
            bytes.len(),
            SECRET_KEY_LENGTH
        ));
    }
    let mut buf = [0u8; SECRET_KEY_LENGTH];
    buf.copy_from_slice(&bytes);
    Ok(SigningKey::from_bytes(&buf))
}

/// Load this machine's public key as hex. Errors if not initialised.
pub fn load_pub_hex() -> Result<String> {
    let path = pub_key_path();
    let s = fs::read_to_string(&path).with_context(|| {
        format!(
            "reading {} — run `phantom keys init` first",
            path.display()
        )
    })?;
    Ok(s.trim().to_string())
}

/// Sign arbitrary bytes with this machine's signing key. Returns the
/// signature as a 64-byte hex string. Used by recipe export.
pub fn sign_hex(body: &[u8]) -> Result<String> {
    let key = load_signing_key()?;
    let sig: Signature = key.sign(body);
    Ok(hex::encode(sig.to_bytes()))
}

/// Verify a signature against a body using a hex-encoded public key.
/// Returns `Ok(true)` on valid, `Ok(false)` on invalid (not an error).
/// Errors only when the inputs are malformed (bad hex / wrong length).
///
/// Used by `phantom evolve adopt <recipe>` to verify the recipe's
/// author signature against a known pubkey (from MAINTAINERS.md or
/// a trusted broker response).
pub fn verify(pub_hex: &str, body: &[u8], sig_hex: &str) -> Result<bool> {
    let pub_bytes = hex::decode(pub_hex.trim())
        .map_err(|e| anyhow!("invalid public key hex: {e}"))?;
    if pub_bytes.len() != 32 {
        return Err(anyhow!(
            "public key must be 32 bytes, got {}",
            pub_bytes.len()
        ));
    }
    let mut pub_arr = [0u8; 32];
    pub_arr.copy_from_slice(&pub_bytes);
    let verifying = VerifyingKey::from_bytes(&pub_arr)
        .map_err(|e| anyhow!("invalid public key: {e}"))?;

    let sig_bytes = hex::decode(sig_hex.trim())
        .map_err(|e| anyhow!("invalid signature hex: {e}"))?;
    if sig_bytes.len() != 64 {
        return Err(anyhow!("signature must be 64 bytes, got {}", sig_bytes.len()));
    }
    let mut sig_arr = [0u8; 64];
    sig_arr.copy_from_slice(&sig_bytes);
    let sig = Signature::from_bytes(&sig_arr);

    Ok(verifying.verify(body, &sig).is_ok())
}

#[cfg(unix)]
fn write_priv_secure(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    use std::io::Write;
    f.write_all(bytes)?;
    Ok(())
}

#[cfg(not(unix))]
fn write_priv_secure(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    // Windows: no chmod equivalent in std; rely on filesystem ACL.
    // The file is per-user under %APPDATA% which has user-only ACL by
    // default on standard Windows installs.
    fs::write(path, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn sign_and_verify_round_trip() {
        let tmp = tempdir().unwrap();
        let signing = SigningKey::generate(&mut OsRng);
        let verifying = signing.verifying_key();
        let body = b"hello world recipe";
        let sig: Signature = signing.sign(body);
        let pub_hex = hex::encode(verifying.to_bytes());
        let sig_hex = hex::encode(sig.to_bytes());
        assert!(verify(&pub_hex, body, &sig_hex).unwrap(), "valid sig must verify");

        // Tampered body must fail.
        let tampered = b"hello world recipe!"; // extra char
        assert!(!verify(&pub_hex, tampered, &sig_hex).unwrap(),
            "tampered body must NOT verify");

        // Tampered sig must fail (flip last byte).
        let mut bad_sig_bytes = sig.to_bytes();
        bad_sig_bytes[63] ^= 0x01;
        let bad_sig_hex = hex::encode(bad_sig_bytes);
        assert!(!verify(&pub_hex, body, &bad_sig_hex).unwrap(),
            "tampered sig must NOT verify");

        // unused tmp suppresses warning
        drop(tmp);
    }

    #[test]
    fn verify_rejects_malformed_inputs() {
        assert!(verify("not-hex", b"x", &"00".repeat(64)).is_err());
        assert!(verify(&"00".repeat(31), b"x", &"00".repeat(64)).is_err()); // wrong pub len
        assert!(verify(&"00".repeat(32), b"x", &"00".repeat(63)).is_err()); // wrong sig len
    }
}
