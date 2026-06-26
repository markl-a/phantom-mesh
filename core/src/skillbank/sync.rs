//! SPEC-25 §8.7 cross-peer skill sync — the security-critical INGEST core.
//!
//! Verifies, decrypts, and last-writer-wins-merges incoming
//! [`EncryptedSkillEnvelope`]s into the local `skills` table. **Fail-CLOSED**
//! throughout: any signature, decrypt, or tamper check that does not pass drops
//! that envelope (counted `rejected`) and NEVER mutates the store. The HTTP
//! transport (`POST /rpc/skill/sync`) lives in `serve.rs` and calls
//! [`ingest_batch`] only after the outer `X-Cluster-Auth` gate.
//!
//! Two keys, two jobs (defence in depth):
//!   - `cluster_secret` — HMAC-signs the envelope (authenticity: only a peer
//!     who shares the cluster secret can produce a verifiable envelope).
//!   - `event_key` (SPEC-13 age x25519, derived from `identity.key`) —
//!     encrypts the `Skill` payload (confidentiality: the skill body never
//!     crosses the wire in clear).
//!
//! DRIFT vs SPEC-25 §9.5 (as-built wins per the source-of-truth rule; spec to be
//! back-filled):
//!   - The envelope is the as-built **4-field** [`EncryptedSkillEnvelope`]
//!     (`skill_id, version, ciphertext_b64, signature_hex`), not the 7-field
//!     spec shape — the locked `encrypted_skill_envelope_has_exactly_four_fields`
//!     test enforces it.
//!   - LWW is **version-only**: the as-built [`Skill`] has no `updated_at_ms`
//!     field, so the spec's "same version → compare `updated_at_ms`" tie-break is
//!     deferred (it would need a new `Skill.updated_at_ms` column). Equal-or-lower
//!     version is a duplicate (never overwrite) — still monotonic + idempotent.
//!   - The spec's per-skill `422 decrypt_failed` is folded into the `rejected`
//!     count so one bad envelope cannot poison an otherwise-good batch (the outer
//!     HMAC already proved the peer is trusted).

use crate::life_node::crypto;
use crate::life_node::key_derivation::EventKey;
use crate::skill_wire::{EncryptedSkillEnvelope, Skill, SkillError};
use base64::Engine as _;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

/// Max envelopes accepted in one batch (SPEC-25 §9.5 — the transport returns
/// `413 batch_too_large` above this).
pub const MAX_BATCH: usize = 100;

/// Request body for `POST /rpc/skill/sync`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSyncBatch {
    pub skills: Vec<EncryptedSkillEnvelope>,
}

/// Outcome counts for an ingested batch — the `200` response body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SyncReport {
    /// Newer-or-new skills stored.
    pub accepted: u32,
    /// Same-or-older `skill_id` seen (no overwrite) — idempotent re-sends.
    pub duplicates: u32,
    /// Envelopes dropped fail-closed (bad signature / undecryptable / tampered).
    pub rejected: u32,
}

/// Canonical signing bytes = the envelope MINUS its `signature_hex`, as
/// deterministic JSON. Both [`seal_skill`] and [`open_envelope`] derive the MAC
/// from this exact function so a signer and a verifier can never drift.
///
/// Uses a `#[derive(Serialize)]` struct rather than a `serde_json::Value`: a
/// struct always serializes its fields in DECLARATION order, immune to whether
/// some other workspace crate turns on serde_json's `preserve_order` feature
/// (cargo features are additive — a `Value` map could otherwise flip from sorted
/// to insertion order and silently change the signed bytes across builds/peers).
fn signing_input(skill_id: &str, version: u16, ciphertext_b64: &str) -> Vec<u8> {
    #[derive(Serialize)]
    struct SigningInput<'a> {
        skill_id: &'a str,
        version: u16,
        ciphertext_b64: &'a str,
    }
    serde_json::to_vec(&SigningInput { skill_id, version, ciphertext_b64 })
        .expect("SigningInput always serializes")
}

/// `HMAC-SHA256(cluster_secret, signing_input)` as lowercase hex (no `0x`).
fn sign_hex(cluster_secret: &[u8], skill_id: &str, version: u16, ciphertext_b64: &str) -> String {
    let mut mac =
        HmacSha256::new_from_slice(cluster_secret).expect("HMAC accepts a key of any length");
    mac.update(&signing_input(skill_id, version, ciphertext_b64));
    hex::encode(mac.finalize().into_bytes())
}

/// Seal a [`Skill`] into a 4-field [`EncryptedSkillEnvelope`]: age-encrypt the
/// canonical-JSON skill under `event_key` (confidentiality), then HMAC-sign the
/// envelope under `cluster_secret` (authenticity). Inverse of [`open_envelope`].
pub fn seal_skill(
    skill: &Skill,
    cluster_secret: &[u8],
    event_key: &EventKey,
) -> Result<EncryptedSkillEnvelope, SkillError> {
    let json = serde_json::to_vec(skill)
        .map_err(|e| SkillError::StoreFailed { detail: format!("seal serialize: {e}") })?;
    let ciphertext = crypto::encrypt(&json, event_key)
        .map_err(|e| SkillError::StoreFailed { detail: format!("seal encrypt: {e}") })?;
    let ciphertext_b64 = base64::engine::general_purpose::STANDARD.encode(&ciphertext);
    let signature_hex = sign_hex(cluster_secret, &skill.id, skill.version, &ciphertext_b64);
    Ok(EncryptedSkillEnvelope {
        skill_id: skill.id.clone(),
        version: skill.version,
        ciphertext_b64,
        signature_hex,
    })
}

/// Verify + decrypt an envelope back into a [`Skill`]. FAIL-CLOSED:
///   - a bad/forged `signature_hex` (constant-time compare) → [`SkillError::SyncSignatureBad`];
///   - undecryptable / non-`Skill` ciphertext → [`SkillError::StoreFailed`] (caller rejects);
///   - plaintext `skill_id`/`version` not matching the authoritative decrypted
///     `Skill` (cleartext-header tampering) → [`SkillError::SyncSignatureBad`].
///
/// The signature is checked BEFORE the ciphertext is ever decrypted, so an
/// unauthenticated payload never reaches the age decryptor.
pub fn open_envelope(
    env: &EncryptedSkillEnvelope,
    cluster_secret: &[u8],
    event_key: &EventKey,
) -> Result<Skill, SkillError> {
    // 1. Authenticate the envelope first — constant-time over the raw MAC bytes.
    let expected = sign_hex(cluster_secret, &env.skill_id, env.version, &env.ciphertext_b64);
    let supplied = hex::decode(&env.signature_hex).map_err(|_| SkillError::SyncSignatureBad)?;
    let expected_bytes = hex::decode(&expected).expect("sign_hex emits valid hex");
    if supplied.len() != expected_bytes.len()
        || supplied.ct_eq(&expected_bytes).unwrap_u8() != 1
    {
        return Err(SkillError::SyncSignatureBad);
    }

    // 2. Decrypt only once authenticity is proven.
    let ciphertext = base64::engine::general_purpose::STANDARD
        .decode(env.ciphertext_b64.as_bytes())
        .map_err(|e| SkillError::StoreFailed { detail: format!("open base64: {e}") })?;
    let plaintext = crypto::decrypt(&ciphertext, event_key)
        .map_err(|e| SkillError::StoreFailed { detail: format!("open decrypt: {e}") })?;
    let skill: Skill = serde_json::from_slice(&plaintext)
        .map_err(|e| SkillError::StoreFailed { detail: format!("open parse: {e}") })?;

    // 3. Anti-tamper: the cleartext envelope header must match the authoritative
    //    (encrypted, HMAC-covered) Skill. A mismatch means the plaintext header
    //    was edited, so reject rather than trust either copy.
    if skill.id != env.skill_id || skill.version != env.version {
        return Err(SkillError::SyncSignatureBad);
    }
    Ok(skill)
}

/// Last-writer-wins outcome for one incoming skill vs the local store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeOutcome {
    /// Newer (or never-seen) — store it.
    Accept,
    /// Same or older version — a duplicate; never overwrite.
    Duplicate,
}

/// Version-only last-writer-wins: a higher incoming version (or no local skill)
/// wins; equal-or-lower is a duplicate. Monotonic + idempotent — re-receiving an
/// already-stored skill is a safe no-op and an older version can never clobber a
/// newer local one.
///
/// Known limitation: `version` is a `u16` and the comparison is strict `>`, so a
/// skill that ever reaches `u16::MAX` (65535) would treat a wrapped-around `0/1`
/// as a duplicate. Realistic user-edit/extract counts never approach this; if it
/// ever matters, widen `Skill.version` (a wire-break + SPEC bump).
pub fn merge_decision(incoming_version: u16, existing_version: Option<u16>) -> MergeOutcome {
    match existing_version {
        None => MergeOutcome::Accept,
        Some(existing) if incoming_version > existing => MergeOutcome::Accept,
        Some(_) => MergeOutcome::Duplicate,
    }
}

/// Ingest a batch: verify + decrypt each envelope and LWW-merge accepted skills
/// into the local store. Envelope-level failures (bad signature / undecryptable /
/// tampered) are counted `rejected` and skipped — they NEVER mutate the store and
/// never abort the batch. Only a genuine store/db failure returns `Err`.
///
/// The caller (the `/rpc/skill/sync` handler) is responsible for the outer
/// `X-Cluster-Auth` gate and the [`MAX_BATCH`] size check before calling this.
pub fn ingest_batch(
    batch: &[EncryptedSkillEnvelope],
    cluster_secret: &[u8],
    event_key: &EventKey,
) -> Result<SyncReport, SkillError> {
    let mut report = SyncReport::default();
    for env in batch {
        let skill = match open_envelope(env, cluster_secret, event_key) {
            Ok(s) => s,
            Err(_) => {
                // Fail-closed: a bad/undecryptable envelope is dropped, never stored.
                report.rejected += 1;
                continue;
            }
        };
        let existing = crate::skill_wire::skill_version(&skill.id)?; // db error → Err
        match merge_decision(skill.version, existing) {
            MergeOutcome::Accept => {
                crate::skill_wire::store_skill(&skill)?; // db error → Err
                report.accepted += 1;
            }
            MergeOutcome::Duplicate => report.duplicates += 1,
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::life_node::key_derivation::derive_event_key;

    fn test_key() -> EventKey {
        // Deterministic 32-byte identity → deterministic EventKey for round-trips.
        derive_event_key(&[42u8; 32]).expect("derive test key")
    }

    fn sample_skill(id: &str, version: u16) -> Skill {
        Skill {
            id: id.into(),
            name: "format rust on save".into(),
            trigger_pattern: "edited a .rs file".into(),
            steps: vec!["run cargo fmt on the file".into()],
            examples: vec![],
            version,
            quality_score: 0.80,
            last_applied_at: 0,
            source_event_count: 5,
        }
    }

    #[test]
    fn seal_then_open_round_trips() {
        let key = test_key();
        let secret = b"cluster-secret-xyz";
        let skill = sample_skill("sk-1", 3);
        let env = seal_skill(&skill, secret, &key).expect("seal");
        // Plaintext header is exposed for dedup; the body is NOT in clear.
        assert_eq!(env.skill_id, "sk-1");
        assert_eq!(env.version, 3);
        assert!(!env.ciphertext_b64.is_empty());
        assert!(
            !env.ciphertext_b64.contains("format rust on save"),
            "skill body must be encrypted, not in clear"
        );
        let back = open_envelope(&env, secret, &key).expect("open");
        assert_eq!(back.id, "sk-1");
        assert_eq!(back.version, 3);
        assert_eq!(back.name, "format rust on save");
        assert_eq!(back.steps, vec!["run cargo fmt on the file".to_string()]);
    }

    #[test]
    fn open_rejects_forged_signature() {
        let key = test_key();
        let secret = b"cluster-secret-xyz";
        let mut env = seal_skill(&sample_skill("sk-2", 1), secret, &key).expect("seal");
        // Flip the last hex nibble of the signature.
        let mut sig = env.signature_hex.clone();
        let last = sig.pop().unwrap();
        sig.push(if last == '0' { '1' } else { '0' });
        env.signature_hex = sig;
        assert!(matches!(
            open_envelope(&env, secret, &key),
            Err(SkillError::SyncSignatureBad)
        ));
    }

    #[test]
    fn open_rejects_wrong_cluster_secret() {
        let key = test_key();
        let env = seal_skill(&sample_skill("sk-3", 1), b"secret-A", &key).expect("seal");
        // A peer that does not share the cluster secret cannot verify the envelope.
        assert!(matches!(
            open_envelope(&env, b"secret-B", &key),
            Err(SkillError::SyncSignatureBad)
        ));
    }

    #[test]
    fn open_rejects_tampered_plaintext_version() {
        let key = test_key();
        let secret = b"cluster-secret-xyz";
        let mut env = seal_skill(&sample_skill("sk-4", 1), secret, &key).expect("seal");
        // Editing the cleartext version changes the signing input → the original
        // signature no longer verifies (caught before decryption).
        env.version = 99;
        assert!(matches!(
            open_envelope(&env, secret, &key),
            Err(SkillError::SyncSignatureBad)
        ));
    }

    #[test]
    fn open_rejects_undecryptable_ciphertext() {
        let secret = b"cluster-secret-xyz";
        // Seal under one key, attempt to open under a DIFFERENT event key. The
        // signature is re-derived from the (matching) secret so auth passes, but
        // the age decrypt fails → StoreFailed (caller will count it rejected).
        let sealing_key = test_key();
        let env = seal_skill(&sample_skill("sk-5", 1), secret, &sealing_key).expect("seal");
        let wrong_key = derive_event_key(&[7u8; 32]).expect("other key");
        assert!(matches!(
            open_envelope(&env, secret, &wrong_key),
            Err(SkillError::StoreFailed { .. })
        ));
    }

    #[test]
    fn merge_decision_is_monotonic_lww() {
        assert_eq!(merge_decision(1, None), MergeOutcome::Accept, "new skill");
        assert_eq!(merge_decision(3, Some(2)), MergeOutcome::Accept, "newer wins");
        assert_eq!(merge_decision(2, Some(2)), MergeOutcome::Duplicate, "equal = dup");
        assert_eq!(merge_decision(1, Some(2)), MergeOutcome::Duplicate, "older never overwrites");
    }

    #[test]
    fn ingest_batch_accepts_dedups_rejects_and_applies_lww() {
        let _g = crate::env_lock::acquire();
        let saved_db = std::env::var_os("PHANTOM_DB_PATH");
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::env::set_var("PHANTOM_DB_PATH", tmp.path());

        let key = test_key();
        let secret = b"cluster-secret-xyz";

        // First sync of two distinct skills → both accepted.
        let batch = vec![
            seal_skill(&sample_skill("a", 1), secret, &key).unwrap(),
            seal_skill(&sample_skill("b", 1), secret, &key).unwrap(),
        ];
        let r1 = ingest_batch(&batch, secret, &key).expect("ingest 1");
        assert_eq!(r1, SyncReport { accepted: 2, duplicates: 0, rejected: 0 });

        // Re-send the same batch → both duplicates (idempotent, no overwrite).
        let r2 = ingest_batch(&batch, secret, &key).expect("ingest 2");
        assert_eq!(r2, SyncReport { accepted: 0, duplicates: 2, rejected: 0 });

        // A newer version of "a" (v2) is accepted; a forged envelope is rejected.
        let mut forged = seal_skill(&sample_skill("c", 1), secret, &key).unwrap();
        forged.signature_hex = "deadbeef".into();
        let batch3 = vec![
            seal_skill(&sample_skill("a", 2), secret, &key).unwrap(),
            forged,
        ];
        let r3 = ingest_batch(&batch3, secret, &key).expect("ingest 3");
        assert_eq!(r3, SyncReport { accepted: 1, duplicates: 0, rejected: 1 });

        // The store now holds a@v2 (the LWW winner).
        assert_eq!(crate::skill_wire::skill_version("a").unwrap(), Some(2));

        // Restore env.
        match saved_db {
            Some(v) => std::env::set_var("PHANTOM_DB_PATH", v),
            None => std::env::remove_var("PHANTOM_DB_PATH"),
        }
    }
}
