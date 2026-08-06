//! Zero-knowledge cloud relay store (P2-1 minimal v1).
//!
//! A relay/store that accepts and hands back ONLY age-sealed blobs, keyed by
//! `(device_id, blob_id)`. The server NEVER sees plaintext: it refuses any
//! payload that is not an age v1 sealed blob, stores the opaque ciphertext
//! verbatim, and returns it byte-for-byte on retrieval. It holds no key
//! material and performs no decryption — confidentiality rests entirely on the
//! client-side age seal (reusing `life_node::crypto` / `encryption_wire`, NO
//! new key management).
//!
//! Storage mirrors `inbox.rs` / `pending_approvals.rs`: one file per blob under
//! `~/.spectyn-mesh/zk-cloud/blobs/<device_id>/<blob_id>.age`, written
//! atomically (tmp + rename) so a reader never observes a half-written blob.
//! Retrieval FAILS CLOSED: a missing/unknown key returns `Err` — never
//! plaintext, never a different blob.
//!
//! 中文: 零知識雲端中繼 — 只收 age 封裝過的密文 blob，伺服器永不見明文、不持
//! 金鑰、不解密;查無 key 一律 Err(fail closed),絕不回別人的 blob。

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// `~/.spectyn-mesh/zk-cloud` under the given home — the relay's data root.
pub fn store_dir(home: &Path) -> PathBuf {
    crate::cli_config::spectyn_dir_under(home).join("zk-cloud")
}

/// `<store>/blobs` — one subdir per device, one `.age` file per blob.
fn blobs_dir(home: &Path) -> PathBuf {
    store_dir(home).join("blobs")
}

/// Reject ids that are empty or could traverse outside the store dir. Both
/// `device_id` and `blob_id` become path components, so a crafted id with a
/// separator or `..` could otherwise escape `blobs/`.
fn valid_id(id: &str) -> bool {
    !id.is_empty() && !id.contains('/') && !id.contains('\\') && !id.contains("..")
}

/// Strict intake check: the bytes must parse as a structurally valid age v1
/// recipient-mode file — NOT merely start with the age magic line.
///
/// A magic-prefix-only check (`looks_like_age`) is trivially bypassable: prepend
/// `age-encryption.org/v1\n` to plaintext and it passes. `age::Decryptor::new`
/// parses the full header, whose MAC can only be produced by genuine age
/// encryption to one or more recipients, so a forged "magic + plaintext" payload
/// is rejected here. The server still NEVER decrypts — header parsing needs no
/// key. Passphrase-mode files are also refused: this relay is recipient-mode
/// only (matches `life_node::crypto`). (review: codex + opencode)
fn is_sealed_age_blob(bytes: &[u8]) -> bool {
    // Cheap reject for the common non-age case before constructing a Decryptor.
    if !crate::life_node::crypto::looks_like_age(bytes) {
        return false;
    }
    matches!(age::Decryptor::new(bytes), Ok(age::Decryptor::Recipients(_)))
}

/// Store one sealed blob under `(device_id, blob_id)`.
///
/// The server accepts ONLY age-sealed ciphertext: a payload lacking the age v1
/// magic line is REFUSED, so the relay can never hold (or be tricked into
/// holding) plaintext. This is the zero-knowledge intake invariant. Write is
/// atomic (tmp + rename). Overwriting an existing `(device_id, blob_id)` is
/// allowed (last write wins) — the relay is a store, not an append log.
pub fn put_blob(home: &Path, device_id: &str, blob_id: &str, sealed: &[u8]) -> anyhow::Result<()> {
    if !valid_id(device_id) {
        anyhow::bail!("invalid device_id");
    }
    if !valid_id(blob_id) {
        anyhow::bail!("invalid blob_id");
    }
    // Zero-knowledge intake invariant: only structurally valid age v1 sealed
    // blobs are accepted (a forged magic-line prefix on plaintext is rejected).
    // This is the server's sole content check — it proves the relay never
    // stores plaintext, and needs no key (the server never decrypts).
    if !is_sealed_age_blob(sealed) {
        anyhow::bail!("refused: payload is not an age-sealed blob");
    }
    let dir = blobs_dir(home).join(device_id);
    fs::create_dir_all(&dir)?;
    // Atomic write: a lister/reader never observes a half-written blob.
    let tmp = dir.join(format!(".{blob_id}.age.tmp"));
    let dest = dir.join(format!("{blob_id}.age"));
    fs::write(&tmp, sealed)?;
    fs::rename(&tmp, &dest)?;
    // Append-only audit AFTER the blob is durably stored (id + time + size, no
    // payload). A failed/refused put above returns early and audits nothing.
    append_audit(home, "put", device_id, blob_id, sealed.len())?;
    Ok(())
}

/// Retrieve the sealed blob for `(device_id, blob_id)`.
///
/// FAILS CLOSED: a missing or unknown key returns `Err` — never plaintext,
/// never a different blob. The bytes returned are the opaque ciphertext exactly
/// as stored; the server never decrypts. As defense-in-depth against on-disk
/// tampering, a stored file that is no longer a valid age blob is also refused
/// (so a swapped-in plaintext file can never be served).
pub fn get_blob(home: &Path, device_id: &str, blob_id: &str) -> anyhow::Result<Vec<u8>> {
    if !valid_id(device_id) {
        anyhow::bail!("invalid device_id");
    }
    if !valid_id(blob_id) {
        anyhow::bail!("invalid blob_id");
    }
    let path = blobs_dir(home).join(device_id).join(format!("{blob_id}.age"));
    // Fail closed: a missing/unknown key is an Err, never an empty/blank blob
    // and never a fallback to some other entry.
    let bytes = fs::read(&path)
        .map_err(|_| anyhow::anyhow!("no sealed blob for ({device_id}, {blob_id})"))?;
    // Defense-in-depth: a stored file that is no longer a structurally valid age
    // blob (e.g. on-disk tampering swapped in plaintext) is refused rather than
    // served.
    if !is_sealed_age_blob(&bytes) {
        anyhow::bail!("stored blob is not age-sealed — refusing to serve");
    }
    // Append-only audit AFTER a successful resolve (id + time + size, no
    // payload). A fail-closed get above returns early and audits nothing.
    append_audit(home, "get", device_id, blob_id, bytes.len())?;
    Ok(bytes)
}

// ─── Audit trail ──────────────────────────────────────────────────────────────

/// One append-only audit entry: WHAT happened (`op`), to WHICH key
/// (`device_id`, `blob_id`), WHEN (`ts_ms`), and the ciphertext `size`.
///
/// Deliberately carries NO payload and NO key material — only metadata — so the
/// audit log is itself zero-knowledge. The sealed-blob size is bookkeeping that
/// reveals nothing about the plaintext.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditRecord {
    /// `"put"` or `"get"`.
    pub op: String,
    /// Device the blob is keyed under (attribution metadata, not a secret).
    pub device_id: String,
    /// Blob id within the device.
    pub blob_id: String,
    /// Unix milliseconds the operation completed.
    pub ts_ms: u64,
    /// Sealed-blob size in bytes (metadata only).
    pub size: usize,
}

/// `<store>/audit.log` — JSONL, one record per line, append-only.
fn audit_log_path(home: &Path) -> PathBuf {
    store_dir(home).join("audit.log")
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Append one record to the append-only audit log. Each entry is a single JSON
/// line (JSONL), so the log only ever grows and a torn final line is simply
/// skipped on read. Called AFTER a put/get succeeds, so failed (fail-closed)
/// gets and refused puts leave no audit entry.
fn append_audit(
    home: &Path,
    op: &str,
    device_id: &str,
    blob_id: &str,
    size: usize,
) -> anyhow::Result<()> {
    use std::io::Write;
    fs::create_dir_all(store_dir(home))?;
    let rec = AuditRecord {
        op: op.to_string(),
        device_id: device_id.to_string(),
        blob_id: blob_id.to_string(),
        ts_ms: now_ms(),
        size,
    };
    let mut line = serde_json::to_string(&rec)?;
    line.push('\n');
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(audit_log_path(home))?;
    f.write_all(line.as_bytes())?;
    Ok(())
}

/// Read the audit trail in append order. A missing log is an empty trail; an
/// unparseable (e.g. torn) line is skipped rather than failing the whole read.
pub fn read_audit(home: &Path) -> anyhow::Result<Vec<AuditRecord>> {
    let raw = match fs::read_to_string(audit_log_path(home)) {
        Ok(s) => s,
        Err(_) => return Ok(Vec::new()),
    };
    let mut out: Vec<AuditRecord> = Vec::new();
    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(rec) = serde_json::from_str::<AuditRecord>(line) {
            out.push(rec);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::life_node::crypto::{decrypt, encrypt, looks_like_age};
    use crate::life_node::key_derivation::{derive_event_key, EventKey};

    /// Seal `plaintext` with an EventKey derived from `seed`, returning the
    /// opaque age blob a client would upload plus the key (for round-trip
    /// asserts). Mirrors the real client path: client seals, server only ever
    /// sees the ciphertext.
    fn seal(plaintext: &[u8], seed: &[u8]) -> (Vec<u8>, EventKey) {
        let key = derive_event_key(seed).expect("derive key");
        let sealed = encrypt(plaintext, &key).expect("seal");
        assert!(looks_like_age(&sealed), "fixture must be a real age blob");
        (sealed, key)
    }

    /// Test isolation: `store_dir` routes through `spectyn_dir_under`, which
    /// honors a process-global `SPECTYN_HOME` override (correct in production).
    /// Other tests (e.g. the serve-layer roundtrip) set `SPECTYN_HOME`, so
    /// without serialization a parallel run would redirect these tempdir-scoped
    /// tests onto a shared dir. Acquire the crate-wide env lock AND clear
    /// `SPECTYN_HOME` for the test body, restoring the prior value on drop.
    struct HomeIsolation {
        _lock: std::sync::MutexGuard<'static, ()>,
        prev: Option<std::ffi::OsString>,
    }
    impl HomeIsolation {
        fn new() -> Self {
            let lock = crate::env_lock::acquire();
            let prev = std::env::var_os("SPECTYN_HOME");
            std::env::remove_var("SPECTYN_HOME");
            Self { _lock: lock, prev }
        }
    }
    impl Drop for HomeIsolation {
        fn drop(&mut self) {
            match &self.prev {
                Some(v) => std::env::set_var("SPECTYN_HOME", v),
                None => std::env::remove_var("SPECTYN_HOME"),
            }
        }
    }

    #[test]
    fn put_then_get_returns_exact_sealed_bytes() {
        let _iso = HomeIsolation::new();
        let tmp = tempfile::tempdir().unwrap();
        let (sealed, key) = seal("私密午餐 — 雞肉便當".as_bytes(), &[0x11u8; 32]);
        put_blob(tmp.path(), "dev-a", "blob-1", &sealed).unwrap();
        let got = get_blob(tmp.path(), "dev-a", "blob-1").unwrap();
        // Server returns the ciphertext verbatim — byte-for-byte.
        assert_eq!(got, sealed);
        // Only the client (with the key) can recover plaintext from it.
        assert_eq!(decrypt(&got, &key).unwrap(), "私密午餐 — 雞肉便當".as_bytes());
    }

    #[test]
    fn get_missing_key_fails_closed() {
        let _iso = HomeIsolation::new();
        let tmp = tempfile::tempdir().unwrap();
        // Nothing stored: a get must Err, never return an empty/blank blob.
        assert!(get_blob(tmp.path(), "dev-a", "nope").is_err());
        // Unknown device likewise.
        assert!(get_blob(tmp.path(), "ghost", "blob-1").is_err());
    }

    #[test]
    fn get_wrong_id_never_returns_other_blob() {
        let _iso = HomeIsolation::new();
        let tmp = tempfile::tempdir().unwrap();
        let (sealed_a, _ka) = seal(b"secret A", &[0x22u8; 32]);
        put_blob(tmp.path(), "dev-a", "blob-a", &sealed_a).unwrap();
        // A get for a DIFFERENT blob_id under the same device must fail closed —
        // it must NOT fall back to returning the only blob present.
        assert!(get_blob(tmp.path(), "dev-a", "blob-b").is_err());
        // ...and a get for a different device must not see dev-a's blob.
        assert!(get_blob(tmp.path(), "dev-b", "blob-a").is_err());
        // The real key still resolves to exactly its own blob.
        assert_eq!(get_blob(tmp.path(), "dev-a", "blob-a").unwrap(), sealed_a);
    }

    #[test]
    fn put_rejects_non_age_plaintext() {
        let _iso = HomeIsolation::new();
        let tmp = tempfile::tempdir().unwrap();
        // The relay must REFUSE anything that is not an age-sealed blob, so the
        // server can never be coaxed into storing plaintext.
        let plaintext = b"this is plaintext, not sealed";
        assert!(put_blob(tmp.path(), "dev-a", "blob-x", plaintext).is_err());
        // Nothing was persisted — a follow-up get fails closed.
        assert!(get_blob(tmp.path(), "dev-a", "blob-x").is_err());
    }

    #[test]
    fn put_rejects_forged_magic_prefix_plaintext() {
        let _iso = HomeIsolation::new();
        let tmp = tempfile::tempdir().unwrap();
        // Plaintext that merely PREPENDS the age magic line. A magic-prefix-only
        // intake check would accept it (and the relay would store quasi-plaintext);
        // a real age-header parse rejects it (no valid header MAC). The server must
        // never be coaxed into storing this. (review: codex + opencode)
        let forged = b"age-encryption.org/v1\nthis is plaintext wearing the age magic line";
        assert!(
            looks_like_age(forged),
            "fixture must pass the cheap magic pre-check, so only the strict parse can reject it"
        );
        assert!(
            put_blob(tmp.path(), "dev-a", "forged", forged).is_err(),
            "forged magic-prefix plaintext must be REFUSED at intake"
        );
        // Nothing persisted -> a follow-up get fails closed.
        assert!(get_blob(tmp.path(), "dev-a", "forged").is_err());
    }

    #[test]
    fn at_rest_file_is_sealed_not_plaintext() {
        let _iso = HomeIsolation::new();
        let tmp = tempfile::tempdir().unwrap();
        let secret = "TOPSECRET-雞肉-12345".as_bytes();
        let (sealed, key) = seal(secret, &[0x33u8; 32]);
        put_blob(tmp.path(), "dev-a", "blob-1", &sealed).unwrap();

        // Read the on-disk file directly (bypassing get_blob).
        let on_disk =
            fs::read(blobs_dir(tmp.path()).join("dev-a").join("blob-1.age")).expect("file on disk");
        // It is a real age blob...
        assert!(looks_like_age(&on_disk), "at-rest bytes must be age-sealed");
        // ...the plaintext does NOT appear anywhere in it...
        assert!(
            !on_disk.windows(secret.len()).any(|w| w == secret),
            "plaintext leaked into at-rest blob"
        );
        // ...the wrong key cannot read it...
        let (_other, wrong_key) = seal(b"unrelated", &[0x44u8; 32]);
        assert!(decrypt(&on_disk, &wrong_key).is_err(), "wrong key must fail");
        // ...and only the right key recovers the secret.
        assert_eq!(decrypt(&on_disk, &key).unwrap(), secret);
    }

    #[test]
    fn put_and_get_append_audit_records_in_order() {
        let _iso = HomeIsolation::new();
        let tmp = tempfile::tempdir().unwrap();
        let (sealed, _k) = seal(b"audit me", &[0x66u8; 32]);
        put_blob(tmp.path(), "dev-a", "blob-1", &sealed).unwrap();
        get_blob(tmp.path(), "dev-a", "blob-1").unwrap();
        let trail = read_audit(tmp.path()).unwrap();
        assert_eq!(trail.len(), 2, "one put + one get => two audit records");
        assert_eq!(trail[0].op, "put");
        assert_eq!(trail[0].device_id, "dev-a");
        assert_eq!(trail[0].blob_id, "blob-1");
        assert_eq!(trail[0].size, sealed.len());
        assert!(trail[0].ts_ms > 0);
        assert_eq!(trail[1].op, "get");
        assert_eq!(trail[1].blob_id, "blob-1");
        // append-only: order preserved, time non-decreasing.
        assert!(trail[1].ts_ms >= trail[0].ts_ms);
    }

    #[test]
    fn failed_get_and_refused_put_leave_no_audit() {
        let _iso = HomeIsolation::new();
        let tmp = tempfile::tempdir().unwrap();
        // Fail-closed get on a missing key audits nothing.
        assert!(get_blob(tmp.path(), "dev-a", "missing").is_err());
        // Refused (non-age) put audits nothing.
        assert!(put_blob(tmp.path(), "dev-a", "x", b"plaintext").is_err());
        assert!(read_audit(tmp.path()).unwrap().is_empty());
    }

    #[test]
    fn audit_log_carries_no_plaintext_or_payload() {
        let _iso = HomeIsolation::new();
        let tmp = tempfile::tempdir().unwrap();
        let secret = "AUDIT-SECRET-雞肉".as_bytes();
        let (sealed, _k) = seal(secret, &[0x77u8; 32]);
        put_blob(tmp.path(), "dev-a", "blob-1", &sealed).unwrap();
        get_blob(tmp.path(), "dev-a", "blob-1").unwrap();
        let raw = fs::read(audit_log_path(tmp.path())).unwrap();
        assert!(
            !raw.windows(secret.len()).any(|w| w == secret),
            "plaintext leaked into audit log"
        );
        assert!(
            !raw.windows(sealed.len()).any(|w| w == &sealed[..]),
            "sealed payload leaked into audit log"
        );
    }

    #[test]
    fn no_plaintext_or_key_material_leaks_into_store() {
        // Mirrors P0-8's `no_key_material_leaks_into_events_dir`: after real
        // traffic through the relay, sweep EVERY byte of EVERY file under the
        // store dir (sealed blobs + audit log) and prove that neither the
        // plaintext, the identity IKM, nor the derived EventKey ever appears.
        let _iso = HomeIsolation::new();
        let tmp = tempfile::tempdir().unwrap();

        // Distinctive material so a leak can't be mistaken for incidental bytes.
        let ikm = [0xABu8; 64];
        let key = derive_event_key(&ikm).unwrap();
        let event_key_bytes: Vec<u8> = key.as_bytes().to_vec();
        let plaintext = "私密-PLAINTEXT-NEEDLE-雞肉便當-12345".as_bytes();
        let sealed = encrypt(plaintext, &key).unwrap();

        // Real relay traffic: store then retrieve (also exercises the audit log).
        put_blob(tmp.path(), "device-007", "secret-blob", &sealed).unwrap();
        let got = get_blob(tmp.path(), "device-007", "secret-blob").unwrap();
        assert_eq!(got, sealed);

        fn walk(dir: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
            if let Ok(rd) = fs::read_dir(dir) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        walk(&p, out);
                    } else if let Ok(b) = fs::read(&p) {
                        out.push((p, b));
                    }
                }
            }
        }
        fn contains_window(hay: &[u8], needle: &[u8]) -> bool {
            !needle.is_empty() && hay.windows(needle.len()).any(|w| w == needle)
        }

        let mut files: Vec<(PathBuf, Vec<u8>)> = Vec::new();
        walk(&store_dir(tmp.path()), &mut files);
        assert!(!files.is_empty(), "no files written under the store dir");
        // The audit log must be among the swept files (so the sweep is meaningful).
        assert!(
            files.iter().any(|(p, _)| p.ends_with("audit.log")),
            "audit log must be swept too"
        );

        for (path, bytes) in &files {
            assert!(
                !contains_window(bytes, plaintext),
                "plaintext leaked into {}",
                path.display()
            );
            assert!(
                !contains_window(bytes, &ikm),
                "identity IKM (64B) leaked into {}",
                path.display()
            );
            assert!(
                !contains_window(bytes, &event_key_bytes),
                "derived EventKey (32B) leaked into {}",
                path.display()
            );
            // Also catch a partial-prefix leak (P0-8 grepped the first 16 bytes).
            assert!(
                !contains_window(bytes, &ikm[..16]),
                "identity IKM 16-byte prefix leaked into {}",
                path.display()
            );
        }
    }

    #[test]
    fn put_and_get_reject_path_traversal_ids() {
        let _iso = HomeIsolation::new();
        let tmp = tempfile::tempdir().unwrap();
        let (sealed, _k) = seal(b"x", &[0x55u8; 32]);
        for bad in ["../evil", "a/b", "a\\b", ""] {
            assert!(put_blob(tmp.path(), bad, "blob", &sealed).is_err());
            assert!(put_blob(tmp.path(), "dev", bad, &sealed).is_err());
            assert!(get_blob(tmp.path(), bad, "blob").is_err());
            assert!(get_blob(tmp.path(), "dev", bad).is_err());
        }
    }
}
