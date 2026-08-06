//! Per-user ed25519 identity for the CONTRIBUTOR-FUNNEL
//! (`docs/CONTRIBUTOR-FUNNEL.md` §5).
//!
//! - `spectyn keys init` generates a keypair at
//!   `~/.spectyn-mesh/keys/{ed25519.priv, ed25519.pub}`.
//! - Recipes (Tier 2 / 3 of CO-EVOLUTION) are signed with the
//!   private key; `spectyn evolve adopt` verifies against the
//!   broker-published public key.
//! - The private key NEVER leaves the user's machine. Public key is
//!   broadcast to the broker on first sync (post-v0.2 once broker
//!   ships).
//!
//! This module ships in v0.1.0 as the down-payment on
//! CONTRIBUTOR-FUNNEL §5 (CO-EVO Phase 3 trust chain). Broker
//! integration + `spectyn keys link --github` OAuth land in v0.2.

use anyhow::{anyhow, Context, Result};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey, SECRET_KEY_LENGTH};
use rand::rngs::OsRng;
use rand::RngCore;
use std::fs;
use std::path::{Path, PathBuf};

/// Length of the per-device root identity key (`identity.key`), the IKM that
/// `life_node::key_derivation` HKDF-expands into the EventStore (SPEC-16)
/// encryption key.
const ROOT_IDENTITY_KEY_LEN: usize = 64;

/// Path to `~/.spectyn-mesh/` (the parent of `keys/`).
pub fn spectyn_mesh_dir() -> PathBuf {
    keys_dir()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(".spectyn-mesh"))
}

/// Path to the per-device root identity key, `~/.spectyn-mesh/identity.key`.
pub fn root_identity_key_path() -> PathBuf {
    spectyn_mesh_dir().join("identity.key")
}

/// Ensure `~/.spectyn-mesh/identity.key` exists, generating a fresh 64-byte
/// CSPRNG key (mode 0600) if it is absent. Idempotent: an existing key is
/// never overwritten (overwriting would make every event encrypted under the
/// old key undecryptable). Returns `Ok(true)` if a new key was created.
///
/// This is the device root key whose absence silently forced the "encrypted"
/// EventStore into plaintext fallback — `EventStore::with_key` /
/// `load_event_key` derive the at-rest key from it. Bootstrapping it here
/// (from `init()` and the daemon startup) is what makes SPEC-16 encryption
/// actually happen for real users.
pub fn ensure_root_identity_key() -> Result<bool> {
    ensure_root_identity_key_in(&spectyn_mesh_dir())
}

/// Path-injectable core of [`ensure_root_identity_key`] — `dir` is the
/// `.spectyn-mesh` directory. Kept separate so tests can target a tempdir
/// without mutating the process-global `$HOME`.
pub fn ensure_root_identity_key_in(dir: &Path) -> Result<bool> {
    let path = dir.join("identity.key");
    if path.exists() {
        return Ok(false);
    }
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;

    let mut ikm = [0u8; ROOT_IDENTITY_KEY_LEN];
    OsRng.fill_bytes(&mut ikm);
    // Atomic create (O_EXCL): the exists() check above is not a lock, so two
    // first-boot writers (daemon + `keys init`) could both pass it. create_new
    // makes the winner's key the only one that lands — a racing loser gets
    // AlreadyExists and keeps the winner's key (never clobber, so no event
    // ends up encrypted under a key that gets overwritten).
    let res = write_new_secure(&path, &ikm);
    // Best-effort zeroize of the local buffer regardless of write outcome.
    use zeroize::Zeroize;
    ikm.zeroize();
    match res {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(e) => Err(anyhow!("writing {}: {e}", path.display())),
    }
}

/// CUJ-05 reinstall path: replace `~/.spectyn-mesh/identity.key` with the
/// bytes the user is restoring (e.g. extracted from a prior
/// `spectyn backup` tar.gz, or carried over from another device).
///
/// Validates the input is exactly `ROOT_IDENTITY_KEY_LEN` (64) bytes — the
/// HKDF IKM length — so a corrupt or wrong-length file fails fast instead
/// of producing an unreadable EventStore on first read.
///
/// `force=false` refuses if `~/.spectyn-mesh/identity.key` already exists,
/// because clobbering would orphan every event encrypted under the old
/// key. `force=true` writes a `.bak-<ts>` of the existing file aside first
/// so the operator can recover if they imported the wrong file.
///
/// Returns the fingerprint (lowercase hex of `sha256(bytes)[0..8]`) of the
/// newly-installed key so the caller can show the user a stable handle
/// they can compare against the backup source (e.g. "abc123ef" matches the
/// fingerprint printed at `spectyn backup` time).
pub fn import_root_identity_key(bytes: &[u8], force: bool) -> Result<String> {
    if bytes.len() != ROOT_IDENTITY_KEY_LEN {
        return Err(anyhow!(
            "imported identity.key is {} bytes; expected exactly {} (per-device root IKM length). \
             Source may be corrupt, a different file, or for a different spectyn version.",
            bytes.len(),
            ROOT_IDENTITY_KEY_LEN
        ));
    }
    import_root_identity_key_in(&spectyn_mesh_dir(), bytes, force)
}

/// Path-injectable core of [`import_root_identity_key`]. Tests target a
/// tempdir without mutating process-global `$HOME`.
pub fn import_root_identity_key_in(
    dir: &Path,
    bytes: &[u8],
    force: bool,
) -> Result<String> {
    if bytes.len() != ROOT_IDENTITY_KEY_LEN {
        return Err(anyhow!(
            "imported identity.key is {} bytes; expected exactly {}",
            bytes.len(),
            ROOT_IDENTITY_KEY_LEN
        ));
    }
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let path = dir.join("identity.key");
    if path.exists() {
        if !force {
            return Err(anyhow!(
                "identity.key already exists at {}; pass --force to overwrite \
                 (the existing key will be moved to identity.key.bak-<unix-ts>)",
                path.display()
            ));
        }
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let backup = path.with_file_name(format!("identity.key.bak-{}", ts));
        fs::rename(&path, &backup).with_context(|| {
            format!(
                "backing up existing identity.key to {} before overwrite",
                backup.display()
            )
        })?;
    }
    write_new_secure(&path, bytes).with_context(|| format!("writing {}", path.display()))?;
    Ok(fingerprint_identity(bytes))
}

/// Short stable handle for an identity.key byte stream — lowercase hex of
/// the first 8 bytes of `sha256(bytes)`. Used by `spectyn identity import`
/// + `spectyn backup` so the user can compare "this is the same key" at a
/// glance without exposing the secret.
pub fn fingerprint_identity(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    let out = h.finalize();
    let mut s = String::with_capacity(16);
    for b in out.iter().take(8) {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

/// Path to `~/.spectyn-mesh/keys/`.
///
/// Routed through the canonical `spectyn_data_dir()` (I6/#322) so the identity
/// keys, `identity.key`, and every other spectyn state file share ONE data root
/// — honoring `SPECTYN_HOME` / `$HOME` on Windows. This keeps the W3 DPAPI
/// write/read paths from splitting off onto a different root than the rest of
/// the codebase. Falls back to `./.spectyn-mesh` exactly as before.
pub fn keys_dir() -> PathBuf {
    crate::cli_config::spectyn_data_dir()
        .unwrap_or_else(|_| PathBuf::from(".").join(".spectyn-mesh"))
        .join("keys")
}

pub fn priv_key_path() -> PathBuf {
    keys_dir().join("ed25519.priv")
}

pub fn pub_key_path() -> PathBuf {
    keys_dir().join("ed25519.pub")
}

/// Result of `spectyn keys init` (legacy v0.x CLI-display shape).
///
/// **Status (Phase G, 2026-05-26)**: still the sole shape the CLI consumer
/// in `core/src/bin/spectyn.rs` reads — it prints `priv_path` / `pub_path`
/// / `pub_hex` directly. The SPEC-12 wire shape
/// (`crate::identity_wire::InitOutcome`) deliberately omits filesystem paths
/// because Stage 4 hides on-disk material behind per-OS keystores
/// (`KeystoreBackend` matrix). The two shapes cannot unify until the CLI
/// either (a) stops printing paths or (b) fetches paths via
/// `priv_key_path()` / `pub_key_path()` independently of the init call.
///
/// **Migration path**: see `docs/superpowers/phase-g-init-outcome-notes.md`
/// — once SPEC-12 `identity_wire::build_init_outcome` ships its real keystore
/// plumbing (Stage 4), the CLI flips to wire form + a small `display_paths()`
/// shim, and this struct + `init()` are deleted.
///
/// The previous `From<InitOutcome> for identity_wire::InitOutcome` bridge was
/// removed in Phase G (2026-05-26) — it had zero callers and confused readers
/// about which shape was canonical.
#[deprecated(
    since = "0.6.0",
    note = "v0.x file-based shape; SPEC-12 Stage 4 will replace with identity_wire::InitOutcome. \
            See docs/superpowers/phase-g-init-outcome-notes.md for the migration plan."
)]
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
/// - `~/.spectyn-mesh/keys/ed25519.priv` (raw 32-byte seed; mode 0600)
/// - `~/.spectyn-mesh/keys/ed25519.pub` (hex-encoded 32-byte verifying key)
///
/// If `force=false` and either file already exists, returns
/// `created=false` so the caller can surface "already initialised"
/// without overwriting. `force=true` overwrites existing keys
/// (destructive — lose all signatures issued by the old key).
///
/// `#[allow(deprecated)]`: the legacy `InitOutcome` is intentionally still
/// the return type here — Phase G partial migration. See the struct's
/// deprecation note + `docs/superpowers/phase-g-init-outcome-notes.md` for
/// the Stage 4 cutover plan that drops both `InitOutcome` and `init()`.
#[allow(deprecated)]
pub fn init(force: bool) -> Result<InitOutcome> {
    let dir = keys_dir();
    let priv_path = priv_key_path();
    let pub_path = pub_key_path();

    // Always provision the per-device root identity key (idempotent), even on
    // the "ed25519 already exists" early-return path below — otherwise users
    // who ran `keys init` before this fix would never get encryption.
    ensure_root_identity_key()?;

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

    fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;

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

/// Outcome of `spectyn keys reset` — which of the three identity files were
/// present and removed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResetOutcome {
    /// `keys/ed25519.priv` existed and was removed.
    pub removed_priv: bool,
    /// `keys/ed25519.pub` existed and was removed.
    pub removed_pub: bool,
    /// `identity.key` (per-device root IKM) existed and was removed.
    pub removed_root: bool,
}

impl ResetOutcome {
    /// True when nothing was present to remove — reset is idempotent, so a
    /// re-run on an already-clean home is a successful no-op.
    pub fn was_noop(&self) -> bool {
        !self.removed_priv && !self.removed_pub && !self.removed_root
    }
}

/// SYS-D (operator-locked 2026-06-13) symmetric undo of `spectyn keys init`:
/// delete this machine's ed25519 keypair (`keys/ed25519.{priv,pub}`) and the
/// per-device root identity key (`identity.key`), returning `~/.spectyn-mesh`
/// to its pre-`keys init` baseline so a fresh init can re-mint cleanly. Minting
/// an identity must NOT be a one-way street (the SYS-D gap this closes).
///
/// Idempotent — removing an already-absent file is not an error, so a re-run
/// after a partial reset still succeeds.
///
/// DESTRUCTIVE: orphans every signature issued by the old key and makes every
/// event encrypted under the old `identity.key` undecryptable. The CLI gates
/// this behind an explicit `--yes` confirmation; this function assumes the
/// caller already confirmed.
pub fn reset() -> Result<ResetOutcome> {
    reset_in(&spectyn_mesh_dir())
}

/// Path-injectable core of [`reset`] — `dir` is the `.spectyn-mesh` directory.
/// Kept separate so tests target a tempdir without mutating process-global
/// `$HOME` / `SPECTYN_HOME` (mirrors [`ensure_root_identity_key_in`]).
pub fn reset_in(dir: &Path) -> Result<ResetOutcome> {
    let keys_dir = dir.join("keys");
    let removed_priv = remove_if_present(&keys_dir.join("ed25519.priv"))?;
    let removed_pub = remove_if_present(&keys_dir.join("ed25519.pub"))?;
    let removed_root = remove_if_present(&dir.join("identity.key"))?;
    // Fully restore the pre-init baseline: `keys init` created the `keys/`
    // directory, so remove it too once reset emptied it. Best-effort — only an
    // EMPTY dir is removed (`remove_dir` errors on a non-empty dir, e.g. if the
    // user kept their own files under `keys/`, in which case we keep it), and an
    // already-absent dir is a no-op. This is what makes reset symmetric to init.
    if keys_dir.exists() {
        let _ = fs::remove_dir(&keys_dir);
    }
    Ok(ResetOutcome { removed_priv, removed_pub, removed_root })
}

/// Remove `path`. `Ok(true)` when a file was removed, `Ok(false)` when it was
/// already absent. Idempotent helper for [`reset_in`]. Removal is attempted
/// directly (no `exists()` pre-check) so there is no TOCTOU window — a
/// concurrently-vanished file maps to `Ok(false)`, not an error.
fn remove_if_present(path: &Path) -> Result<bool> {
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(anyhow!("removing {}: {e}", path.display())),
    }
}

/// Load this machine's signing key from disk. Errors if the keypair
/// hasn't been initialised yet (`spectyn keys init` first).
pub fn load_signing_key() -> Result<SigningKey> {
    let path = priv_key_path();
    let bytes = fs::read(&path)
        .with_context(|| format!("reading {} — run `spectyn keys init` first", path.display()))?;
    // W3: on Windows the seed is DPAPI-wrapped at rest; unwrap it back to the
    // raw 32-byte seed. `Ok(None)` = legacy plaintext (use bytes as-is).
    let bytes = match crate::identity_wire::unprotect_at_rest(&bytes)
        .map_err(|e| anyhow!("unwrapping DPAPI-protected {}: {e}", path.display()))?
    {
        Some(seed) => seed,
        None => bytes,
    };
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
    let s = fs::read_to_string(&path)
        .with_context(|| format!("reading {} — run `spectyn keys init` first", path.display()))?;
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
/// Used by `spectyn evolve adopt <recipe>` to verify the recipe's
/// author signature against a known pubkey (from MAINTAINERS.md or
/// a trusted broker response).
pub fn verify(pub_hex: &str, body: &[u8], sig_hex: &str) -> Result<bool> {
    let pub_bytes =
        hex::decode(pub_hex.trim()).map_err(|e| anyhow!("invalid public key hex: {e}"))?;
    if pub_bytes.len() != 32 {
        return Err(anyhow!(
            "public key must be 32 bytes, got {}",
            pub_bytes.len()
        ));
    }
    let mut pub_arr = [0u8; 32];
    pub_arr.copy_from_slice(&pub_bytes);
    let verifying =
        VerifyingKey::from_bytes(&pub_arr).map_err(|e| anyhow!("invalid public key: {e}"))?;

    let sig_bytes =
        hex::decode(sig_hex.trim()).map_err(|e| anyhow!("invalid signature hex: {e}"))?;
    if sig_bytes.len() != 64 {
        return Err(anyhow!(
            "signature must be 64 bytes, got {}",
            sig_bytes.len()
        ));
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
    // Windows (W3): no chmod equivalent in std. NTFS ACL alone leaves the seed
    // recoverable by a raw file copy, so wrap it per-user with DPAPI at rest
    // (MAGIC || CryptProtectData) to match the app's keystore. `load_signing_key`
    // detects the magic and unwraps; legacy plaintext files still load.
    fs::write(path, crate::identity_wire::protect_at_rest(bytes))
}

/// Like `write_priv_secure` but fails with `AlreadyExists` instead of
/// truncating when the file is already present (O_EXCL). Used to provision
/// `identity.key` atomically so a first-boot race can never clobber an
/// already-written root key. Not shared with the ed25519 path, which
/// legitimately truncates on `--force`.
#[cfg(unix)]
fn write_new_secure(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(bytes)
}

#[cfg(not(unix))]
fn write_new_secure(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    // Windows (W3): DPAPI-wrap the root IKM at rest (see write_priv_secure).
    // `create_new` still gives the atomic no-clobber first-boot guarantee; only
    // the on-disk bytes change. `load_event_key` detects the magic and unwraps.
    let payload = crate::identity_wire::protect_at_rest(bytes);
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    f.write_all(&payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn ensure_root_identity_key_creates_64_byte_key_when_absent() {
        let tmp = tempdir().unwrap();
        let dir = tmp.path().join(".spectyn-mesh");
        let created = ensure_root_identity_key_in(&dir).unwrap();
        assert!(created, "should report a freshly-created key");
        let key_path = dir.join("identity.key");
        assert!(key_path.exists(), "identity.key must exist after ensure");
        let bytes = fs::read(&key_path).unwrap();
        assert_eq!(bytes.len(), ROOT_IDENTITY_KEY_LEN, "must be 64-byte IKM");
        assert_ne!(bytes, vec![0u8; ROOT_IDENTITY_KEY_LEN], "must not be all-zero");
    }

    #[test]
    fn ensure_root_identity_key_is_idempotent_and_never_overwrites() {
        let tmp = tempdir().unwrap();
        let dir = tmp.path().join(".spectyn-mesh");
        assert!(ensure_root_identity_key_in(&dir).unwrap());
        let first = fs::read(dir.join("identity.key")).unwrap();
        // Second call must be a no-op (returns false) and leave bytes untouched —
        // overwriting would make all prior encrypted events undecryptable.
        let created_again = ensure_root_identity_key_in(&dir).unwrap();
        assert!(!created_again, "second ensure must not create/overwrite");
        let second = fs::read(dir.join("identity.key")).unwrap();
        assert_eq!(first, second, "existing key bytes must be preserved");
    }

    #[cfg(unix)]
    #[test]
    fn ensure_root_identity_key_is_mode_0600() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempdir().unwrap();
        let dir = tmp.path().join(".spectyn-mesh");
        ensure_root_identity_key_in(&dir).unwrap();
        let mode = fs::metadata(dir.join("identity.key"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "identity.key must be owner-only (0600)");
    }

    #[test]
    fn provisioned_key_derives_an_event_key() {
        // End-to-end: the key this fn writes must be valid IKM for the
        // EventStore encryption-key derivation (the whole point of the fix).
        use crate::life_node::key_derivation::load_event_key;
        let tmp = tempdir().unwrap();
        let dir = tmp.path().join(".spectyn-mesh");
        ensure_root_identity_key_in(&dir).unwrap();
        let key = load_event_key(&dir.join("identity.key"));
        assert!(key.is_ok(), "load_event_key must succeed on the provisioned key");
    }

    /// P0-7 S1 — identity creation is OFFLINE by construction. The keypair +
    /// root IKM are minted from the OS CSPRNG and the filesystem only; this
    /// module must never reach for an HTTP client. A self-scan of the source
    /// documents that invariant in-tree (the real enforcement is the
    /// `p0_7_identity_offline_hermetic` integration test + the
    /// `p0_7_no_boot_network_static` gate). Kept tiny on purpose.
    #[test]
    fn identity_creation_has_no_network_symbol() {
        let src = include_str!("identity.rs");
        // Build the needles from fragments so the literals do NOT appear
        // verbatim in this source — otherwise the self-scan would match its
        // own assertion lines.
        let http_client = concat!("req", "west");
        let http_url = concat!("ht", "tp://");
        let https_url = concat!("ht", "tps://");
        assert!(
            !src.contains(http_client),
            "identity.rs must not pull in an HTTP client — keys are made offline"
        );
        assert!(
            !src.contains(http_url) && !src.contains(https_url),
            "identity.rs must not reference any remote URL — keys never leave the device"
        );
    }

    #[test]
    fn sign_and_verify_round_trip() {
        let tmp = tempdir().unwrap();
        let signing = SigningKey::generate(&mut OsRng);
        let verifying = signing.verifying_key();
        let body = b"hello world recipe";
        let sig: Signature = signing.sign(body);
        let pub_hex = hex::encode(verifying.to_bytes());
        let sig_hex = hex::encode(sig.to_bytes());
        assert!(
            verify(&pub_hex, body, &sig_hex).unwrap(),
            "valid sig must verify"
        );

        // Tampered body must fail.
        let tampered = b"hello world recipe!"; // extra char
        assert!(
            !verify(&pub_hex, tampered, &sig_hex).unwrap(),
            "tampered body must NOT verify"
        );

        // Tampered sig must fail (flip last byte).
        let mut bad_sig_bytes = sig.to_bytes();
        bad_sig_bytes[63] ^= 0x01;
        let bad_sig_hex = hex::encode(bad_sig_bytes);
        assert!(
            !verify(&pub_hex, body, &bad_sig_hex).unwrap(),
            "tampered sig must NOT verify"
        );

        // unused tmp suppresses warning
        drop(tmp);
    }

    #[test]
    fn verify_rejects_malformed_inputs() {
        assert!(verify("not-hex", b"x", &"00".repeat(64)).is_err());
        assert!(verify(&"00".repeat(31), b"x", &"00".repeat(64)).is_err()); // wrong pub len
        assert!(verify(&"00".repeat(32), b"x", &"00".repeat(63)).is_err()); // wrong sig len
    }

    #[test]
    #[allow(deprecated)] // exercises the real (legacy) init() public path
    fn keys_init_then_reset_returns_to_baseline() {
        // SYS-D round-trip: `spectyn keys init` mints the keypair + root IKM;
        // `spectyn keys reset` is the symmetric undo that returns the home to
        // its pre-init baseline so a fresh init can re-mint cleanly. Hermetic
        // via SPECTYN_HOME (the verbatim data-root) under env_lock.
        let _env = crate::env_lock::acquire();
        let tmp = tempdir().unwrap();
        struct HomeGuard(Option<std::ffi::OsString>);
        impl Drop for HomeGuard {
            fn drop(&mut self) {
                match &self.0 {
                    Some(v) => std::env::set_var("SPECTYN_HOME", v),
                    None => std::env::remove_var("SPECTYN_HOME"),
                }
            }
        }
        let prev = std::env::var_os("SPECTYN_HOME");
        std::env::set_var("SPECTYN_HOME", tmp.path());
        let _guard = HomeGuard(prev);

        // Baseline: nothing minted yet.
        assert!(
            !priv_key_path().exists() && !pub_key_path().exists() && !root_identity_key_path().exists(),
            "a fresh home must have no identity files"
        );

        // DO: init mints both the ed25519 keypair and the per-device root IKM.
        let out = init(false).expect("keys init");
        assert!(out.created, "first init must create the keypair");
        assert!(priv_key_path().exists(), "ed25519.priv minted");
        assert!(pub_key_path().exists(), "ed25519.pub minted");
        assert!(root_identity_key_path().exists(), "identity.key minted");

        // UNDO: reset removes all three → baseline restored.
        let r = reset().expect("keys reset");
        assert_eq!(
            r,
            ResetOutcome { removed_priv: true, removed_pub: true, removed_root: true },
            "reset removes the keypair + root IKM"
        );
        assert!(
            !priv_key_path().exists() && !pub_key_path().exists() && !root_identity_key_path().exists(),
            "after reset the home is back to baseline"
        );
        assert!(
            !keys_dir().exists(),
            "reset removes the now-empty keys/ dir too — full pre-init baseline"
        );

        // SYMMETRY: a fresh init after reset re-mints cleanly (not a one-way street).
        let out2 = init(false).expect("re-init after reset");
        assert!(out2.created, "init after reset must re-mint, not report 'already exists'");

        // Idempotent: reset on an already-clean home is a successful no-op.
        let _ = reset().expect("reset the re-minted keys");
        let noop = reset().expect("reset on clean home");
        assert!(noop.was_noop(), "reset on an already-clean home is a no-op");
    }

    #[test]
    fn reset_in_is_idempotent_and_path_injected() {
        // Path-injected core: hermetic, no env mutation (mirrors the
        // ensure_root_identity_key_in tests).
        let tmp = tempdir().unwrap();
        let dir = tmp.path().join(".spectyn-mesh");
        fs::create_dir_all(dir.join("keys")).unwrap();
        fs::write(dir.join("keys").join("ed25519.priv"), b"seed").unwrap();
        fs::write(dir.join("keys").join("ed25519.pub"), b"pub\n").unwrap();
        ensure_root_identity_key_in(&dir).unwrap(); // lays down identity.key

        let r = reset_in(&dir).unwrap();
        assert_eq!(
            r,
            ResetOutcome { removed_priv: true, removed_pub: true, removed_root: true }
        );
        assert!(!dir.join("keys").join("ed25519.priv").exists());
        assert!(!dir.join("keys").join("ed25519.pub").exists());
        assert!(!dir.join("identity.key").exists());
        assert!(!dir.join("keys").exists(), "the emptied keys/ dir is removed too");

        // Idempotent: a second reset on the now-clean dir is a no-op, not an error.
        let again = reset_in(&dir).unwrap();
        assert!(again.was_noop(), "second reset_in must be a successful no-op");
    }
}
