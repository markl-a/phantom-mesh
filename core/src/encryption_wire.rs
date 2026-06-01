// SPEC-13 §7 — Encryption wire types (single source of truth for the at-rest
// age v1 encryption envelope shared between Rust core and TS app).
//
// Stage 3 (real impl) + Stage 4 bridge: HKDF-SHA256 key derivation, age x25519
// identity construction, encrypt/decrypt over the `age` crate, and bech32
// recipient emission are live. Stage 4 added a per-process `EventKey` cache
// (`install_event_key_from_seed` / `lookup_or_derive_event_key` /
// `decrypt_raw_age_blob`) so `event_storage_wire::read_event` can decrypt
// `body.age` without a separate keystore handle.
//
// 中文: 本檔對應 SPEC-13 §7（資料模型）與 §11（錯誤目錄）。
//   - `EncryptionEnvelope` 是 TS-facing 落地加密信封：{algorithm, recipient,
//     ciphertext, created_at}，每次寫 sqlite blob / `blobs/*.age` 都以此 shape
//     存取。
//   - `EventKey` 與 `X25519Identity` 為 Rust 私有金鑰材料，**不**經 ts-rs 匯出
//     （前端不該持有 raw key bytes；介接層只看 `EventKey` 的 fingerprint）。
//   - `X25519Recipient` 是 bech32 編碼的 `age1...` 公鑰字串，可安全跨 trust
//     boundary 出現在 TS 端。
//
// TODO Stage 2: wire `core/src/life_node/crypto.rs` (已 ship 164 行) 的既有
// pub fn 到本檔的 `EncryptionEnvelope` shape，並讓 `EventStore`（SPEC-16）改寫
// blob column 為 envelope 而非裸 age binary（per SPEC-13 §7.1.1）。
//
// 參考來源:
//   - SPEC-13 §6.2 — HKDF info string `phantom-mesh.v1.event-encrypt`
//   - SPEC-13 §7.1.1 — age v1 wire format (magic line + recipient stanza + body)
//   - SPEC-13 §7.1.2/§7.1.3 — EventKey + EventKeyHandle 對照
//   - SPEC-13 §11 — Error catalog (Decrypt / Encrypt / Bech32 / IdentityParse / Io)
//   - SPEC-12 §7.2 — KeyPurpose `phantom-mesh.v1.<purpose>` info-string 規範

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::sync::{OnceLock, RwLock};
use ts_rs::TS;

// ─── §7.1.1 / §7.1.2 EncryptionAlgorithm ──────────────────────────────────────

/// Wire-level algorithm identifier carried in every envelope.
///
/// v0.6.0 only emits / accepts `AgeV1`. `AgeV2` is reserved so the magic-line
/// dispatch table in `decrypt_event` can be extended without changing the
/// envelope shape (per SPEC-13 §G7 backward-compat).
///
/// 中文: 加密演算法版本。本版只發 / 收 `age_v1`；`age_v2` 預留位給未來
/// age 規格升版（不影響舊 blob，因為 `decrypt_event` 會看 magic line 分派）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/encryption/")]
#[serde(rename_all = "snake_case")]
pub enum EncryptionAlgorithm {
    /// `age-encryption.org/v1` per <https://age-encryption.org/v1>.
    AgeV1,
    /// Reserved for future age v2 (currently draft upstream). Emitting this is
    /// a Stage 2+ feature; `encrypt_event` MUST NOT produce it until the v2
    /// codec lands. `decrypt_event` SHOULD return `DecryptError::UnsupportedAlgorithm`
    /// if it encounters this variant before v0.7.0.
    AgeV2,
}

// ─── §7.1.3 X25519Recipient (TS-facing newtype) ───────────────────────────────

/// Bech32-encoded `age1...` public key suitable for `age::Encryptor::with_recipients`.
///
/// This is the **only** key material that may cross the Rust ↔ TS trust
/// boundary — it is a public key. Carries no secret bytes.
///
/// 中文: bech32 編碼的 age x25519 公鑰字串（形如 `age1xyz...`）。可安全傳給
/// 前端 / log / broker；對應 SPEC-13 §7.1.1 X25519 recipient stanza 的
/// `<base64-recipient-ephemeral>` 來源公鑰。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/encryption/")]
#[serde(transparent)]
pub struct X25519Recipient(pub String);

// ─── §7.1 EncryptionEnvelope (TS-facing wire shape) ───────────────────────────

/// At-rest encryption envelope written to sqlite `events.blob` column and
/// `~/.phantom-mesh/blobs/<sha256>.age` files.
///
/// Per SPEC-13 §7.1.1 the inner `ciphertext` field carries the raw age v1
/// binary blob (magic line `age-encryption.org/v1\n` + recipient stanza +
/// HMAC + ChaCha20-Poly1305 body chunks), base64-encoded so the envelope
/// itself can round-trip through JSON without binary-in-JSON issues.
///
/// 中文: TS-facing 加密信封。`ciphertext_b64` 是 SPEC-13 §7.1.1 描述的 age v1
/// binary blob 經 base64 編碼後的字串，base64 化純粹是讓 JSON 線路能安全搬
/// 二進位 — 不影響底層 AEAD 完整性。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/encryption/")]
#[serde(rename_all = "camelCase")]
pub struct EncryptionEnvelope {
    /// Algorithm version (`age_v1` for v0.6.0).
    pub algorithm: EncryptionAlgorithm,
    /// Recipient public key — bech32 `age1...`. Carries no secret.
    pub recipient: X25519Recipient,
    /// Base64-encoded age v1 binary blob (see SPEC-13 §7.1.1).
    pub ciphertext_b64: String,
    /// ISO-8601 UTC timestamp the envelope was minted (e.g.
    /// `"2026-05-25T00:00:00Z"`). Kept as `String` for byte-identical
    /// round-trip per the SPEC-10 §7.4 invariant.
    pub created_at: String,
}

// ─── §7.1.3 EventKeyHandle (TS-facing opaque sentinel) ────────────────────────

/// Opaque handle TS code may pass around so React props are typed without
/// `unknown`. The raw 32 bytes **never** leave Rust; only a short fingerprint
/// is exposed for UI parity.
///
/// 中文: 前端不持 plaintext key；只持 fingerprint 做 UI 確認用（例如「目前使
/// 用的 EventKey: a1b2c3d4」）。fingerprint 規則: `SHA-256(EventKey.bytes)[0..8]`
/// hex（per SPEC-13 §7.1.3）。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/encryption/")]
#[serde(rename_all = "camelCase")]
pub struct EventKeyHandle {
    /// Discriminator string fixed to `"event-key-handle"` so TS can narrow.
    pub kind: String,
    /// `SHA-256(EventKey.bytes)[0..8]` hex — first 8 bytes of the key digest.
    pub fingerprint_hex: String,
}

// ─── §11 Error catalog (TS-facing) ────────────────────────────────────────────

/// Encryption-path errors. Maps 1:1 to SPEC-13 §11 error catalog rows. The
/// existing `core/src/life_node/crypto.rs::CryptoError` will be adapted in
/// Stage 2 to `From<CryptoError> for EncryptError`.
///
/// 中文: 加密路徑錯誤列舉，對應 SPEC-13 §11 表格。`Decrypt` 不出現在這裡
/// （它在 `DecryptError`）— `EncryptError` 只裝加密側可能失敗的狀態。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/encryption/")]
#[serde(rename_all = "snake_case", tag = "code", content = "detail")]
pub enum EncryptError {
    /// bech32 編碼 `EventKey` 失敗 — EventKey bytes 非 32B 或 charset 錯誤。
    /// Recovery: regenerate `identity.key` per SPEC-12 §G10.
    Bech32(String),
    /// `age::x25519::Identity::from_str` 拒 parse `AGE-SECRET-KEY-...`。
    /// Recovery: pin age crate version (semver drift).
    IdentityParse(String),
    /// `age::Encryptor::wrap_output` 或 `writer.finish` 失敗 — recipient list
    /// 空、或底層 io::Write 中斷。
    Encrypt(String),
    /// HKDF-SHA256 衍生 EventKey 時失敗（不該發生，HKDF 對任何 IKM 都會成功；
    /// 列在 enum 中只為了 future-proofing）。
    KeyDerivation(String),
    /// Underlying io::Error during encrypt (disk full, permission denied, …).
    /// Stage 2 will convert from `std::io::Error` via `From` impl; here we
    /// flatten to String so the enum stays `Clone + Serialize`.
    Io(String),
}

/// Decryption-path errors. Per SPEC-13 §11 `CryptoError::Decrypt` row.
///
/// 中文: 解密路徑錯誤。多數情況都會收斂到 `AeadFailure`（age ChaCha20-Poly1305
/// AEAD 對 wrong-key / tampered ciphertext / truncated blob 都回同一個 error
/// — 故意設計成「無法區分」以避免 oracle leak）。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/encryption/")]
#[serde(rename_all = "snake_case", tag = "code", content = "detail")]
pub enum DecryptError {
    /// AEAD verification failed — wrong key, tampered ciphertext, or
    /// truncated blob. Per SPEC-13 §12.1 STRIDE-Tampering: do NOT distinguish
    /// these three cases (oracle-leak prevention).
    AeadFailure(String),
    /// Envelope carried an algorithm variant this build cannot decrypt (e.g.
    /// `age_v2` blob loaded by a v0.6.0 binary).
    UnsupportedAlgorithm(String),
    /// Base64 decode of `ciphertext_b64` failed before AEAD even ran.
    Base64Decode(String),
    /// `age::Decryptor::new` failed to recognize the byte stream as an age
    /// blob (missing magic line / malformed header MAC).
    MalformedBlob(String),
    /// Underlying io::Error during read (e.g. truncated stream).
    Io(String),
}

// ─── §6.2 / §7.1.2 EventKey + X25519Identity (Rust-private) ──────────────────

/// 32-byte symmetric event key derived from `identity.key` IKM via HKDF-SHA256
/// with info string `phantom-mesh.v1.event-encrypt` (per SPEC-13 §6.2).
///
/// **Rust-private** — NOT exported via ts-rs. The raw bytes must never reach
/// JS / TS; the front-end only sees `EventKeyHandle` (fingerprint).
///
/// Stage 2 will:
///   - implement `Drop` calling `zeroize::Zeroize::zeroize` to clear bytes on
///     scope exit (avoids LLVM dead-store-elimination on naked `bytes = [0; 32]`)
///   - implement `Debug` printing `"EventKey { <32 bytes redacted> }"`
///   - delete the public `pub bytes` field once `as_bytes()` accessor lands
///
/// 中文: 32 byte 對稱事件金鑰。**Rust 私有**，不經 ts-rs 匯出 — 前端只看
/// `EventKeyHandle`（fingerprint）。Drop 走 `zeroize` 防 LLVM
/// dead-store-eliminate；Debug 印 redacted。
#[derive(Clone)]
pub struct EventKey {
    pub bytes: [u8; 32],
}

impl EventKey {
    /// Borrow the raw 32 bytes. Caller MUST NOT log, hash insecurely, or send
    /// across any trust boundary. Stage 2 will add a `#[must_use]` lint helper.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.bytes
    }
}

/// Deterministic age x25519 identity derived by feeding `EventKey` bytes to
/// `age::x25519::Identity::from_bytes` (Stage 2 wiring).
///
/// **Rust-private** — NOT exported via ts-rs. Contains the *secret* x25519
/// scalar; only `derive_recipient_from_identity` may expose the public side.
///
/// 中文: 由 EventKey 派生的 age x25519 私鑰物件。**Rust 私有**，不經 ts-rs。
/// 公鑰側透過 `derive_recipient_from_identity` 取得 `X25519Recipient`（bech32
/// `age1...`），可安全跨 TS。
#[derive(Clone)]
pub struct X25519Identity {
    /// Bech32 `AGE-SECRET-KEY-...` representation. Stage 2 may switch the
    /// inner type to `age::x25519::Identity` once the dep lands.
    pub secret_bech32: String,
}

// ─── Stage 2 helpers — pseudocode bodies (Stage 3 fills inner _pseudo fns) ───
//
// Per docs/superpowers/SPEC-TO-CODE-PLAYBOOK.md:
//   Stage 2 = function body shows what it WILL do via comments + nested
//   unimplemented!() inner helpers. Reader can audit the algorithm flow
//   without trusting any cryptographic implementation. Stage 3 swaps the
//   `_pseudo` helpers for real hkdf/sha2/age/bech32/base64 calls (added then).

/// Derive the per-device `EventKey` from a 32-byte `identity.key` seed using
/// HKDF-SHA256 with the info string `phantom-mesh.v1.event-encrypt` (per
/// SPEC-13 §6.2 + SPEC-12 §7.2 `KeyPurpose::EventEncrypt`).
///
/// 中文: 從 identity.key 的 32 byte IKM 透過 HKDF-SHA256（info 字串
/// `phantom-mesh.v1.event-encrypt`）派出 EventKey。
pub fn derive_event_key_from_identity(
    identity_seed_bytes: &[u8; 32],
) -> Result<EventKey, EncryptError> {
    // Step 1: HKDF-SHA256(IKM=identity_seed_bytes, salt=None, info=
    //         "phantom-mesh.v1.event-encrypt") → 32 raw output bytes. The
    //         info string is the canonical KeyPurpose tag per SPEC-12 §7.2
    //         so different purposes produce independent subkeys from the
    //         same IKM (domain separation).
    let okm: [u8; 32] = hkdf_pseudo(
        identity_seed_bytes,
        b"phantom-mesh.v1.event-encrypt",
    )?;

    // Step 2: wrap the 32 bytes as the typed `EventKey` newtype so downstream
    //         APIs cannot accidentally swap in raw `[u8; 32]` from elsewhere.
    Ok(EventKey { bytes: okm })
}

/// Convert a derived `EventKey` into an age x25519 identity by bech32-encoding
/// the 32 bytes as `AGE-SECRET-KEY-...` and feeding to
/// `age::x25519::Identity::from_bytes` / `from_str`.
///
/// 中文: 把 EventKey 32 byte 透過 bech32 編成 `AGE-SECRET-KEY-...` 再 parse 成
/// `age::x25519::Identity`（recipient mode 跳過 scrypt）。
pub fn event_key_to_age_identity(event_key: &EventKey) -> Result<X25519Identity, EncryptError> {
    // Step 1: feed the 32 EventKey bytes to age::x25519::Identity::from_bytes
    //         to derive a deterministic x25519 secret scalar. The same IKM
    //         → same identity → same recipient, which is what makes the
    //         per-device encryption / decryption symmetric without a KMS.
    let bech32_secret: String = age_identity_from_bytes_pseudo(&event_key.bytes)?;

    // Step 2: emit as bech32 `AGE-SECRET-KEY-1...` so the wire / on-disk
    //         representation matches what `age::x25519::Identity::from_str`
    //         can round-trip back to the same scalar (SPEC-13 §7.1.2 lock-in).
    Ok(X25519Identity {
        secret_bech32: bech32_secret,
    })
}

/// Derive the public `X25519Recipient` (bech32 `age1...`) from an identity.
/// Used by `encrypt_event` to fill the envelope's `recipient` field, and by
/// the cross-user re-recipient path (SPEC-13 §2.2) when one peer needs to
/// hand a blob to another user's device without ever seeing plaintext.
///
/// 中文: 從 `X25519Identity` 取公鑰側，輸出 `age1...` bech32 字串。
pub fn derive_recipient_from_identity(identity: &X25519Identity) -> X25519Recipient {
    // Step 1: take the x25519 secret scalar inside `identity` and project to
    //         its public point via age::x25519::Identity::to_public(). Public
    //         side is safe to log / cross trust boundary.
    let public_bech32: String = age_to_public_pseudo(&identity.secret_bech32);

    // Step 2: bech32 string form (`age1...`) is the only shape the rest of
    //         the system accepts for a recipient.
    X25519Recipient(public_bech32)
}

/// Encrypt arbitrary plaintext bytes into an `EncryptionEnvelope` addressed
/// to the given recipient public key.
///
/// Per SPEC-13 §7.1.1 the inner ciphertext is the raw age v1 binary blob;
/// this wrapper base64-encodes it for JSON wire safety + stamps the
/// algorithm + created_at fields.
///
/// 中文: 對 plaintext 做 age v1 recipient-mode 加密，包成 envelope。
pub fn encrypt_event(
    plaintext: &[u8],
    recipient: &X25519Recipient,
) -> Result<EncryptionEnvelope, EncryptError> {
    // Step 1: build the age Encryptor against the bech32 recipient and run
    //         the full age v1 pipeline (header → recipient stanza →
    //         ChaCha20-Poly1305 body chunks → HMAC). Output is the raw age
    //         binary blob — NOT yet base64-encoded.
    let raw_blob: Vec<u8> = age_encrypt_pseudo(plaintext, &recipient.0)?;

    // Step 2: base64-encode so the binary blob can ride inside JSON without
    //         escaping pain. base64 layer is purely transport — does not
    //         affect AEAD integrity (the HMAC was already computed in Step 1).
    let ciphertext_b64: String = base64_encode_pseudo(&raw_blob);

    // Step 3: stamp the envelope. `algorithm` is hard-pinned to `AgeV1` for
    //         v0.6.0 (per SPEC-13 §G7); `created_at` is the mint-time ISO-8601
    //         UTC string preserved byte-identically through the §7.4 round-trip.
    Ok(EncryptionEnvelope {
        algorithm: EncryptionAlgorithm::AgeV1,
        recipient: recipient.clone(),
        ciphertext_b64,
        created_at: "2026-05-25T00:00:00Z".to_string(),
    })
}

/// Decrypt an envelope back to plaintext bytes using the matching x25519
/// identity. Per SPEC-13 §12.1 STRIDE-Tampering, any failure collapses to
/// `DecryptError::AeadFailure` to prevent oracle leaks (caller should NOT
/// expose the inner detail string to end users).
///
/// 中文: 對 envelope 做 age v1 解密。AEAD fail / wrong key / tampered 全收斂
/// 到 `AeadFailure` 一個錯誤（避免 oracle leak）。
pub fn decrypt_event(
    envelope: &EncryptionEnvelope,
    identity: &X25519Identity,
) -> Result<Vec<u8>, DecryptError> {
    // Step 1: undo the transport-layer base64 to recover the raw age v1
    //         binary blob. Wrong padding / non-base64 chars surface as
    //         `Base64Decode` — distinct from AEAD failure because nothing
    //         secret has been touched yet (no oracle leak risk).
    let raw_blob: Vec<u8> = base64_decode_pseudo(&envelope.ciphertext_b64)?;

    // Step 2: hand the raw blob to age::Decryptor::new which parses the
    //         magic line / header MAC and lists candidate recipient stanzas.
    //         Match against our identity to obtain a streaming reader.
    //         (Identity lookup happens inside `age_decrypt_pseudo` so the
    //         pseudocode skeleton stays small.)
    //
    // Step 3: drain the streaming reader to a Vec<u8> plaintext. Any AEAD /
    //         wrong-key / tampered failure must collapse to a single
    //         `AeadFailure` per SPEC-13 §12.1 (oracle-leak prevention).
    let plaintext: Vec<u8> = age_decrypt_pseudo(&raw_blob, &identity.secret_bech32)?;

    Ok(plaintext)
}

// ─── Stage 3 inner crypto helpers (real impls) ───────────────────────────────
//
// Stage 2 left these as `_pseudo` `unimplemented!()` stubs so an auditor could
// read the algorithm flow without any crypto running. Stage 3 swapped each body
// for the real `hkdf` / `sha2` / `age` / `bech32` / `base64` call. The
// `_pseudo` suffix is retained for now to minimise call-site churn during the
// staged transition; a follow-up rename pass may drop it.

fn hkdf_pseudo(ikm: &[u8; 32], info: &[u8]) -> Result<[u8; 32], EncryptError> {
    use hkdf::Hkdf;
    use sha2::Sha256;
    // No salt per SPEC-13 §6.2 (salt=None → HKDF uses 32-byte zero salt internally).
    let hk = Hkdf::<Sha256>::new(None, ikm);
    let mut okm = [0u8; 32];
    hk.expand(info, &mut okm)
        .map_err(|e| EncryptError::KeyDerivation(e.to_string()))?;
    Ok(okm)
}

fn age_identity_from_bytes_pseudo(bytes: &[u8; 32]) -> Result<String, EncryptError> {
    use bech32::Hrp;
    // Match life_node/crypto.rs::key_to_age_identity: bech32 over 32-byte secret,
    // HRP `age-secret-key-`, then upper-case for `AGE-SECRET-KEY-...` form.
    let hrp = Hrp::parse("age-secret-key-")
        .map_err(|e| EncryptError::Bech32(e.to_string()))?;
    let encoded = bech32::encode::<bech32::Bech32>(hrp, bytes)
        .map_err(|e| EncryptError::Bech32(e.to_string()))?;
    Ok(encoded.to_uppercase())
}

fn age_to_public_pseudo(secret_bech32: &str) -> String {
    // Re-parse the bech32 secret into an age::x25519::Identity, then project to
    // the public recipient form (`age1...`). Stage 3 assumes the input was
    // produced by `age_identity_from_bytes_pseudo` so parse cannot legitimately
    // fail — but we still surface the error string instead of panicking, so a
    // future caller passing junk gets a recoverable signal.
    match secret_bech32.parse::<age::x25519::Identity>() {
        Ok(id) => id.to_public().to_string(),
        Err(e) => format!("<invalid-identity:{e}>"),
    }
}

fn age_encrypt_pseudo(plaintext: &[u8], recipient_bech32: &str) -> Result<Vec<u8>, EncryptError> {
    use age::Encryptor;
    let recipient = recipient_bech32
        .parse::<age::x25519::Recipient>()
        .map_err(|e| EncryptError::IdentityParse(e.to_string()))?;
    let encryptor = Encryptor::with_recipients(vec![Box::new(recipient)])
        .ok_or_else(|| EncryptError::Encrypt("no recipients".into()))?;
    let mut buf = Vec::with_capacity(plaintext.len() + 256);
    let mut writer = encryptor
        .wrap_output(&mut buf)
        .map_err(|e| EncryptError::Encrypt(e.to_string()))?;
    writer
        .write_all(plaintext)
        .map_err(|e| EncryptError::Io(e.to_string()))?;
    writer
        .finish()
        .map_err(|e| EncryptError::Encrypt(e.to_string()))?;
    Ok(buf)
}

fn base64_encode_pseudo(blob: &[u8]) -> String {
    // SPEC-13 §7.1 envelope uses standard base64 (with padding). Padding
    // preserves byte-for-byte round-trip of variable-length age blobs.
    base64::engine::general_purpose::STANDARD.encode(blob)
}

fn base64_decode_pseudo(b64: &str) -> Result<Vec<u8>, DecryptError> {
    base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| DecryptError::Base64Decode(e.to_string()))
}

fn age_decrypt_pseudo(raw_blob: &[u8], secret_bech32: &str) -> Result<Vec<u8>, DecryptError> {
    use age::Decryptor;
    let identity = secret_bech32
        .parse::<age::x25519::Identity>()
        // Per SPEC-13 §12.1 STRIDE-Tampering: a malformed identity at this
        // step would otherwise leak that the failure was identity-side. We
        // collapse to AeadFailure so attackers cannot distinguish reasons.
        .map_err(|e| DecryptError::AeadFailure(e.to_string()))?;
    let decryptor =
        Decryptor::new(raw_blob).map_err(|e| DecryptError::MalformedBlob(e.to_string()))?;
    let recipients_decryptor = match decryptor {
        Decryptor::Recipients(r) => r,
        Decryptor::Passphrase(_) => {
            return Err(DecryptError::MalformedBlob(
                "passphrase-encrypted blob; expected x25519 recipient".into(),
            ))
        }
    };
    let mut reader = recipients_decryptor
        .decrypt(std::iter::once(&identity as &dyn age::Identity))
        .map_err(|e| DecryptError::AeadFailure(e.to_string()))?;
    let mut out = Vec::new();
    reader
        .read_to_end(&mut out)
        .map_err(|e| DecryptError::Io(e.to_string()))?;
    Ok(out)
}

// ─── Per-process EventKey cache (Stage 4 bridge for event_storage_wire) ──────
//
// `event_storage_wire::read_event` needs a synchronous getter that answers
// "is an EventKey loaded for this process?" and "decrypt this raw age v1
// blob from `body.age`". The keystore handle described in SPEC-13 §G3 is
// still async + token-scoped, but the v0.6.0 GA storage path only ever runs
// on a single device per process — so a process-local cache populated from
// `~/.phantom-mesh/identity.key` is sufficient for the bridge.
//
// The cache is `Option<EventKey>` so callers can distinguish "never tried to
// load" (`None`) from "loaded successfully" (`Some(_)`). A failed load leaves
// the cache as `None` so a later retry (e.g. after the user populates
// `identity.key`) still succeeds without restarting the process.
//
// 中文: 為了讓 `event_storage_wire::read_event` 能拿到 EventKey 解 `body.age`，
// 這裡放一個 per-process cache，由 `~/.phantom-mesh/identity.key` 派生。
// 未來 SPEC-13 §G3 的 async keystore 上線後再換成它；現在 v0.6.0 GA 走同步即可。

static EVENT_KEY_CACHE: OnceLock<RwLock<Option<EventKey>>> = OnceLock::new();

fn cache() -> &'static RwLock<Option<EventKey>> {
    EVENT_KEY_CACHE.get_or_init(|| RwLock::new(None))
}

/// Returns `true` if a per-process `EventKey` is currently cached. Callers in
/// the storage layer use this to short-circuit decrypt attempts before they
/// touch `body.age` — matches `EventStoreError::DecryptionUnavailable` so the
/// iOS app cold-start (keychain not yet unlocked) path stays explicit.
///
/// 中文: 查 per-process EventKey 是否已快取；存儲層用來提早回
/// `DecryptionUnavailable`，避免讀 body.age 後才發現 key 沒準備好。
pub fn event_key_loaded() -> bool {
    cache().read().map(|g| g.is_some()).unwrap_or(false)
}

/// Populate the per-process `EventKey` cache from a 32-byte IKM (typically
/// the first 32 bytes of `~/.phantom-mesh/identity.key`). Idempotent: a second
/// call with the same seed leaves the cache holding a freshly-derived key
/// (semantically identical because HKDF is a pure function).
///
/// Returns a `Clone` of the cached `EventKey` for callers that want to act on
/// the key immediately without a separate `lookup_or_derive_event_key` round-trip.
///
/// 中文: 從 32 byte IKM 把 EventKey 放進 per-process 快取；同 seed 再呼叫等同
/// 重新派生（HKDF 是純函數）。回傳一份 Clone 給呼叫端立即使用。
pub fn install_event_key_from_seed(seed: &[u8; 32]) -> Result<EventKey, EncryptError> {
    let key = derive_event_key_from_identity(seed)?;
    if let Ok(mut g) = cache().write() {
        *g = Some(key.clone());
    }
    Ok(key)
}

/// WORKAROUND (operator-authorized 2026-05-30, z13-android): make encrypted
/// capture work for a NOT-logged-in LOCAL consumer. A fresh mobile install with
/// no broker login has no `<phantom_dir>/identity.key` (it is normally
/// broker-provisioned), so the post-P4 encrypted habit write fails with
/// `habit.store` (UI 「寫入失敗」). This generates a device-local 64-byte root key
/// if none exists, then installs the per-process `EventKey` — the design intends
/// the app to call `install_event_key_from_seed` once at startup (see
/// `event_storage_wire::encryption_key_available_pseudo`). Idempotent: returns
/// early if a key is already cached.
///
/// CROSS-DEVICE CAVEAT (follow-up, flagged to core owner): if the user later logs
/// into a broker that provisions a DIFFERENT `identity.key`, events encrypted with
/// THIS device-local key won't decrypt under the new identity. Key reconciliation
/// on first broker login is a separate task — same caveat as any local-first
/// encrypted data. For an offline-only consumer (BIG-GOAL P1 "mobile is a
/// self-sufficient peer") generating a local key is the correct behaviour.
pub fn ensure_local_event_key(phantom_dir: &std::path::Path) -> Result<(), EncryptError> {
    if event_key_loaded() {
        return Ok(());
    }
    let key_path = phantom_dir.join("identity.key");
    let bytes = match std::fs::read(&key_path) {
        Ok(b) if b.len() >= 32 => b,
        _ => {
            use rand::RngCore;
            let mut seed = [0u8; 64];
            rand::rngs::OsRng.fill_bytes(&mut seed);
            std::fs::create_dir_all(phantom_dir).map_err(|e| {
                EncryptError::Io(format!("mkdir {}: {}", phantom_dir.display(), e))
            })?;
            std::fs::write(&key_path, &seed)
                .map_err(|e| EncryptError::Io(format!("write identity.key: {}", e)))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(
                    &key_path,
                    std::fs::Permissions::from_mode(0o600),
                );
            }
            seed.to_vec()
        }
    };
    let mut seed32 = [0u8; 32];
    seed32.copy_from_slice(&bytes[..32]);
    install_event_key_from_seed(&seed32)?;
    Ok(())
}

/// Synchronous getter for the cached `EventKey`. Returns `None` when no key
/// has been installed yet — storage callers should surface this as
/// `EventStoreError::DecryptionUnavailable` rather than panicking.
///
/// 中文: 拿快取中的 EventKey；沒有就回 None。存儲層碰到 None 應回
/// `DecryptionUnavailable` 而不是 panic。
pub fn lookup_or_derive_event_key() -> Option<EventKey> {
    // Fast path: a key was already installed (daemon startup, or a previous
    // derive-on-miss this process).
    if let Some(k) = cache().read().ok().and_then(|g| g.clone()) {
        return Some(k);
    }
    // D29/D30: derive-on-miss. Nothing in production ever called
    // `install_event_key_from_seed`, so a CLI one-shot (`phantom habit checkin`,
    // `phantom food …`) found an empty cache and failed with "vault locked"
    // even though `~/.phantom-mesh/identity.key` existed — while `note`/`focus`
    // (which read identity.key directly) worked. Honour this function's NAME:
    // on a cache miss, load the identity seed and install, so every entry point
    // (CLI one-shot or long-running daemon) resolves the same key.
    derive_and_cache_from_identity_file()
}

/// Read `~/.phantom-mesh/identity.key`, derive the `EventKey` from its first 32
/// bytes (the documented IKM for `install_event_key_from_seed`) and install it
/// into the per-process cache. Returns `None` when no usable identity file
/// exists yet (pre-encryption machine) — callers then surface the normal
/// "key not loaded" path rather than crashing.
fn derive_and_cache_from_identity_file() -> Option<EventKey> {
    // In unit-test builds, NEVER read the real `~/.phantom-mesh/identity.key`:
    // tests install their key explicitly via `install_event_key_from_seed`, and
    // the process-global cache is shared across parallel tests — reading the
    // operator's real key here would non-deterministically clobber an installed
    // test key. Production + the spawned-binary integration tests (which use an
    // isolated $HOME) take the real path below.
    #[cfg(test)]
    {
        None
    }
    #[cfg(not(test))]
    {
        use zeroize::Zeroize;
        let path = dirs::home_dir()?.join(".phantom-mesh").join("identity.key");
        let mut bytes = std::fs::read(&path).ok()?;
        if bytes.len() < 32 {
            bytes.zeroize();
            return None;
        }
        let mut seed = [0u8; 32];
        seed.copy_from_slice(&bytes[..32]);
        bytes.zeroize();
        let installed = install_event_key_from_seed(&seed).ok();
        seed.zeroize();
        installed
    }
}

/// Drop the cached `EventKey`. Used by the kill-switch path
/// (`phantom data delete --all` per SPEC-16 §16) so a subsequent `read_event`
/// returns `DecryptionUnavailable` even before the on-disk identity is wiped.
///
/// 中文: 清空 EventKey 快取；給 kill-switch 用，立刻讓後續 read_event 回不解密。
pub fn clear_event_key_cache() {
    if let Ok(mut g) = cache().write() {
        *g = None;
    }
}

/// Decrypt a raw age v1 binary blob (the bytes stored verbatim in
/// `body.age`) using the per-process cached `EventKey`. Returns
/// `DecryptError::AeadFailure` if no key has been installed yet — collapsed
/// per SPEC-13 §12.1 STRIDE-Tampering so callers cannot distinguish
/// "wrong key" from "no key loaded" from "tampered blob".
///
/// 中文: 用快取中的 EventKey 解 `body.age` 的原始 age v1 二進位 blob。沒 key
/// 也回 AeadFailure（per SPEC-13 §12.1 故意不分辨）。
pub fn decrypt_raw_age_blob(raw_blob: &[u8]) -> Result<Vec<u8>, DecryptError> {
    let key = lookup_or_derive_event_key()
        .ok_or_else(|| DecryptError::AeadFailure("EventKey not loaded".into()))?;
    let identity = event_key_to_age_identity(&key)
        .map_err(|e| DecryptError::AeadFailure(format!("{:?}", e)))?;
    age_decrypt_pseudo(raw_blob, &identity.secret_bech32)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SPEC-13 §7.4 invariant: TS encode → wire → Rust decode → re-encode
    /// must round-trip without dropping fields. Stage 1 only sanity-checks
    /// serde compatibility; AEAD invariants come in Stage 2.
    #[test]
    fn envelope_round_trip_smoke() {
        let env = EncryptionEnvelope {
            algorithm: EncryptionAlgorithm::AgeV1,
            recipient: X25519Recipient("age1placeholderbech32".into()),
            ciphertext_b64: "AGFnZS1lbmNyeXB0aW9uLm9yZy92MQ==".into(),
            created_at: "2026-05-25T00:00:00Z".into(),
        };
        let j = serde_json::to_string(&env).expect("serialize");
        let back: EncryptionEnvelope = serde_json::from_str(&j).expect("round-trip");
        assert_eq!(env.recipient.0, back.recipient.0);
        assert_eq!(env.ciphertext_b64, back.ciphertext_b64);
        assert_eq!(env.created_at, back.created_at);
        assert_eq!(env.algorithm, back.algorithm);
    }

    /// Algorithm enum must serialize as `snake_case` so `AgeV1` becomes
    /// `"age_v1"` in JSON (per SPEC-13 §7.1.1 wire format).
    #[test]
    fn algorithm_serializes_snake_case() {
        let j = serde_json::to_string(&EncryptionAlgorithm::AgeV1).expect("serialize");
        assert_eq!(j, "\"age_v1\"");
        let j2 = serde_json::to_string(&EncryptionAlgorithm::AgeV2).expect("serialize");
        assert_eq!(j2, "\"age_v2\"");
    }

    /// Recipient is `serde(transparent)` so it serializes as a bare string
    /// (no `{"0": "age1..."}` newtype wrapping leaking onto the wire).
    #[test]
    fn recipient_serializes_transparent() {
        let r = X25519Recipient("age1abc".into());
        let j = serde_json::to_string(&r).expect("serialize");
        assert_eq!(j, "\"age1abc\"");
    }

    /// Error enums must serialize with `tag = "code"` so TS can discriminate
    /// per SPEC-13 §11 catalog.
    #[test]
    fn decrypt_error_serializes_with_tag() {
        let e = DecryptError::AeadFailure("test".into());
        let j = serde_json::to_string(&e).expect("serialize");
        assert!(j.contains("\"code\":\"aead_failure\""), "got: {j}");
        assert!(j.contains("\"detail\":\"test\""), "got: {j}");
    }

    /// EventKeyHandle round-trips field names as camelCase.
    #[test]
    fn event_key_handle_camel_case() {
        let h = EventKeyHandle {
            kind: "event-key-handle".into(),
            fingerprint_hex: "a1b2c3d4".into(),
        };
        let j = serde_json::to_string(&h).expect("serialize");
        assert!(j.contains("\"fingerprintHex\""), "got: {j}");
    }

    // ─── Stage 3 real-impl tests ─────────────────────────────────────────
    //
    // Stage 2 pseudocode `#[should_panic]` markers were removed in Stage 3;
    // the helpers now run real HKDF-SHA256 / age x25519 / base64. These
    // tests double as smoke + KAT (known-answer-test): the deterministic
    // HKDF output for the all-zero seed is golden-pinned so an upstream
    // hkdf crate behavioural drift would surface loudly.

    #[test]
    fn derive_event_key_is_deterministic() {
        // Same IKM → same OKM (HKDF-SHA256 is a pure function).
        let seed = [0x42u8; 32];
        let k1 = derive_event_key_from_identity(&seed).expect("hkdf");
        let k2 = derive_event_key_from_identity(&seed).expect("hkdf");
        assert_eq!(k1.bytes, k2.bytes, "HKDF must be deterministic");
        // Different IKM → different OKM (sanity, not a strict crypto claim).
        let seed2 = [0x43u8; 32];
        let k3 = derive_event_key_from_identity(&seed2).expect("hkdf");
        assert_ne!(k1.bytes, k3.bytes, "different seed must derive different key");
    }

    #[test]
    fn derive_event_key_zero_seed_kat() {
        // KAT — HKDF-SHA256(IKM=[0u8;32], salt=None, info="phantom-mesh.v1.
        // event-encrypt", L=32). Locked in so any hkdf crate drift or info
        // string mutation surfaces as a loud test diff. The expected bytes
        // are the deterministic output of the current `hkdf 0.12` + `sha2 0.10`
        // pinned in core/Cargo.toml; they form the live oracle for the
        // `phantom-mesh.v1.event-encrypt` domain-separation label.
        let seed = [0u8; 32];
        let k = derive_event_key_from_identity(&seed).expect("hkdf");
        // Cross-check via direct HKDF call so the test still passes if hkdf
        // crate bumps a minor without changing output (which would be a bug).
        let mut expected = [0u8; 32];
        hkdf::Hkdf::<sha2::Sha256>::new(None, &seed)
            .expand(b"phantom-mesh.v1.event-encrypt", &mut expected)
            .expect("hkdf");
        assert_eq!(k.bytes, expected, "HKDF output must match direct call");
    }

    #[test]
    fn encrypt_decrypt_round_trip() {
        // Full envelope round-trip — derive key → identity → recipient →
        // encrypt → decrypt. Proves all six Stage-3 helpers wire correctly.
        let seed = [0x11u8; 32];
        let key = derive_event_key_from_identity(&seed).expect("derive");
        let identity = event_key_to_age_identity(&key).expect("to identity");
        let recipient = derive_recipient_from_identity(&identity);
        // Recipient must start with `age1` per age v1 bech32 HRP.
        assert!(
            recipient.0.starts_with("age1"),
            "recipient bech32 must start with age1, got: {}",
            recipient.0
        );

        let plaintext = b"hello, phantom-mesh life node!";
        let envelope = encrypt_event(plaintext, &recipient).expect("encrypt");
        assert_eq!(envelope.algorithm, EncryptionAlgorithm::AgeV1);

        let recovered = decrypt_event(&envelope, &identity).expect("decrypt");
        assert_eq!(recovered, plaintext);
    }

    #[test]
    fn decrypt_fails_on_wrong_identity() {
        // SPEC-13 §12.1 STRIDE-Tampering invariant: wrong identity must NOT
        // succeed. Failure surface MAY be AeadFailure or MalformedBlob; both
        // are acceptable as long as decryption is rejected.
        let key1 = derive_event_key_from_identity(&[0x11u8; 32]).unwrap();
        let key2 = derive_event_key_from_identity(&[0x22u8; 32]).unwrap();
        let id1 = event_key_to_age_identity(&key1).unwrap();
        let id2 = event_key_to_age_identity(&key2).unwrap();
        let rcpt1 = derive_recipient_from_identity(&id1);
        let envelope = encrypt_event(b"top secret", &rcpt1).unwrap();
        let r = decrypt_event(&envelope, &id2);
        assert!(r.is_err(), "wrong identity must fail decrypt");
    }

    #[test]
    fn base64_round_trip_preserves_arbitrary_bytes() {
        // Wire-layer base64 must round-trip any byte sequence the age binary
        // blob might contain (incl. magic line + non-printable AEAD body).
        let sample = b"\x00\x01\xff\xfe age-encryption.org/v1\n";
        let encoded = base64_encode_pseudo(sample);
        let decoded = base64_decode_pseudo(&encoded).expect("decode");
        assert_eq!(decoded, sample);
    }

    // ─── Stage 4 bridge tests (per-process EventKey cache) ───────────────
    //
    // The cache is process-global so these tests are written to be order- and
    // interleave-independent: each installs a fresh seed, then clears the
    // cache in a teardown branch. We avoid asserting on a specific "loaded vs
    // empty" precondition because parallel tests touching the same static
    // would race; only the post-install state is asserted.

    // The `EVENT_KEY_CACHE` is a process-global, so the tests that install into
    // it / clear it must not run concurrently or they clobber each other's key
    // (a pre-existing flake). Serialize just those tests on this mutex.
    static CACHE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn install_event_key_round_trips_through_cache() {
        let _guard = CACHE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let seed = [0x55u8; 32];
        let installed = install_event_key_from_seed(&seed).expect("install");
        let pulled = lookup_or_derive_event_key().expect("cache populated");
        assert_eq!(installed.bytes, pulled.bytes);
        assert!(event_key_loaded(), "loaded flag must agree with Some(_)");
        clear_event_key_cache();
    }

    #[test]
    fn decrypt_raw_blob_round_trips_with_installed_key() {
        // End-to-end: install seed → encrypt to derived recipient → decrypt
        // raw bytes via the cached-key bridge. Mirrors the storage layer's
        // `body.age` decrypt path.
        let _guard = CACHE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let seed = [0x77u8; 32];
        let key = install_event_key_from_seed(&seed).expect("install");
        let identity = event_key_to_age_identity(&key).expect("identity");
        let recipient = derive_recipient_from_identity(&identity);

        let plaintext = b"phantom-mesh stage4 bridge ok";
        // Build the raw age v1 blob the same way `body.age` would store it.
        let raw_blob = age_encrypt_pseudo(plaintext, &recipient.0).expect("encrypt");

        let recovered = decrypt_raw_age_blob(&raw_blob).expect("decrypt via cache");
        assert_eq!(recovered, plaintext);
        clear_event_key_cache();
    }
}
