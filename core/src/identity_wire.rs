//! Device identity: keypair generation, persistence, and node-id derivation.
//!
//! This module (`identity_wire`) is the single source of truth for a device's
//! cryptographic identity in phantom-mesh. It owns three concerns:
//!
//! 1. **Generation** — [`build_init_outcome`] creates a fresh ed25519 master
//!    seed via `OsRng` ([`keygen_ed25519`]) on first `phantom keys init`, and
//!    re-derives the verifying (public) key from a pre-existing seed on
//!    subsequent calls. The 32-byte master seed is the root of all identity
//!    material; every subkey is HKDF-derived from it via [`derive_subkey`].
//!
//! 2. **Persistence** — the master seed is stored in the host's native secret
//!    store via the [`KeystoreBackend`] matrix (macOS/iOS Keychain, Android
//!    EncryptedSharedPreferences, Windows Credential Manager + DPAPI, Linux
//!    Secret Service), with a desktop-only `~/.phantom-mesh/<account>.key`
//!    file fallback (mode 0600). See [`write_to_keystore`] /
//!    [`read_from_keystore`] / [`delete_from_keystore`].
//!
//! 3. **Node identity** — the public surface ([`IdentityPublic`]) carries a
//!    12-hex short fingerprint, `SHA-256(verifying_key)[..12]` (see
//!    [`fingerprint_short`]). This fingerprint is the stable, content-derived
//!    handle other devices use to recognise this node across the mesh.
//!
//! ## Analysis observation: hardcoded node-id prefix
//!
//! The runtime-facing *node-id string* is built elsewhere
//! (`core/src/runtime.rs`, `PhantomMeshRuntime::init`) as
//! `format!("mac-{:08x}", ...)`. The `"mac-"` prefix is **hardcoded** there —
//! it does not reflect the actual host platform, so a Linux or Windows node
//! still reports a `mac-`-prefixed id. This is a pre-existing observation only;
//! the value is intentionally left unchanged here. The cryptographic identity
//! in this module (the fingerprint above) is the durable per-device handle and
//! is independent of that display-only prefix.
//!
//! ## 中文
//!
//! 本模組是 phantom-mesh 裝置加密身份（device identity）的唯一真實來源，負責：
//! 身份金鑰產生（generation）、持久化（persistence，存進各 OS 的原生 keystore），
//! 以及 node-id（節點識別碼）衍生。對外可見的識別碼是 12-hex 短指紋
//! `SHA-256(公鑰)[..12]`。
//!
//! **分析觀察**：runtime 端的 node-id 字串在 `core/src/runtime.rs` 以
//! `format!("mac-{:08x}", ...)` 組成，`"mac-"` 前綴是 **hardcoded（寫死）** 的，
//! 不會隨實際平台改變（Linux / Windows 節點仍會回報 `mac-` 前綴）。此處僅記錄
//! 觀察、**不**更動該值。

// SPEC-12 §7 — Identity keypair wire types (single source of truth for the
// public identity surface + per-OS keystore matrix + HKDF subkey purposes that
// every other crypto consumer in phantom-mesh shares).
//
// Stage 3 (partial real impl — core crypto + file fallback + **Linux Secret
// Service** live): the `ed25519-dalek` / `hkdf` / `sha2` / `hex` / `chrono` /
// `dirs` helpers (`keygen_ed25519`, `derive_verifying_from_seed`,
// `hkdf_expand`, `sha256_hex`, `hex_encode_verifying`, `iso8601_now`,
// `default_backend_for_os`) are backed by their real RustCrypto / chrono
// crates (all already in core/Cargo.toml). The desktop `FileChmod0600`
// fallback (`file_chmod0600_*`) is wired to `std::fs` + `PermissionsExt`.
// **New in this commit**: the `libsecret_*` arm is now backed by the
// `secret-service = "5"` crate (`blocking::SecretService`, Linux-only via
// `[target.'cfg(target_os = "linux")'.dependencies]` in core/Cargo.toml) so
// the Linux desktop never needs to fall back to file-on-disk. The remaining
// three native-keystore arms (`keychain_*` / `android_ks_*` / `dpapi_*`)
// stay `unimplemented!("Stage 4: requires <crate>")` until `security-framework`
// (macOS+iOS) / `jni` (Android) / `windows-rs` DPAPI dispatch glue land. The
// matching Stage 4 marker test remains to flag silent slippage on those arms.
//
// 中文: 本檔對應 SPEC-12 §7（資料模型）。master seed（主種子）以 Rust-internal
// `IdentityKey` 持有，**永遠不過 FFI / 不出 core crate**；對 UI / TS 曝光的只有
// `IdentityPublic`（公鑰 + fingerprint + createdAt）。HKDF info-string 公式為
// `phantom-mesh.v1.<purpose>` — purpose（用途）列舉於 `KeyPurpose` enum。
//
// TODO Stage 4:
//   - replace `[u8; 32]` raw seed in `IdentityKey` with `zeroize::Zeroizing<[u8; 32]>`
//     once we wire the existing `zeroize` dep into this module (already in
//     core/Cargo.toml — just hasn't been imported here yet to keep Stage 3
//     dependency-surface minimal & merge-safe).
//   - add `security-framework` (macOS/iOS) / `jni` (Android) / `windows-rs`
//     (Windows DPAPI + CredentialManager) / `libsecret` (Linux) to
//     core/Cargo.toml and wire the four native keystore arms.
//   - migrate existing `core/src/life_node/key_derivation.rs` `derive_event_key`
//     to call `derive_subkey(KeyPurpose::EventEncrypt)` so HKDF info-string
//     becomes canonical across the codebase.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

// ─── §7.1 IdentityPublic — TypeScript-facing public surface ──────────────────

/// Public identity surface exposed to UI / non-Rust frontends.
/// Master seed NEVER crosses this boundary — only the derived public material.
///
/// 中文: UI 端可見的「公開身份」— 公鑰 + 12-hex 短指紋 + 建立時間。
/// **絕對不**包含 master seed。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/identity/")]
#[serde(rename_all = "camelCase")]
pub struct IdentityPublic {
    /// Hex-encoded ed25519 verifying key (32 bytes → 64 hex chars).
    pub public_key: String,
    /// First 12 hex chars of SHA-256(publicKey) — for display & backup confirm.
    pub fingerprint: String,
    /// ISO-8601 of master creation (from `~/.phantom-mesh/identity.key` mtime).
    pub created_at: String,
}

// ─── §6.2 / §9.3 InitOutcome — `phantom keys init` CLI / Tauri result ───────

/// Result of `phantom keys init` (CLI) or `invoke('identity_init')` (Tauri).
/// Idempotent: `created == false` when an existing identity was found and the
/// caller did not pass `--force`.
///
/// 中文: `phantom keys init` 的結果結構。`created=false` 代表已存在身份且
/// 沒有強制覆寫；CLI 會顯示「已存在的身份 abc123 — 略過」。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/identity/")]
#[serde(rename_all = "camelCase")]
pub struct InitOutcome {
    /// `true` if a new master seed was generated; `false` if pre-existing.
    pub created: bool,
    /// 12-hex short fingerprint of the (new or existing) identity.
    pub fingerprint: String,
    /// Hex-encoded ed25519 verifying key (64 hex chars).
    pub public_key_hex: String,
    /// Backend name that was used to persist the seed (mirrors
    /// `KeystoreBackend::name()` — kept as String here so the TS surface stays
    /// stable even if we add more backends later).
    pub keystore_backend: String,
    /// ISO-8601 timestamp the init call completed.
    pub initialized_at: String,
}

// ─── §7.2 KeyPurpose — HKDF info-string purposes ─────────────────────────────

/// Purpose tags for HKDF subkey derivation. Each variant maps to a fixed
/// info-string `phantom-mesh.v1.<slug>` consumed by exactly one downstream
/// subsystem (see SPEC-12 §7.2 mapping table).
///
/// 中文: HKDF 子金鑰用途列舉。每個 variant 對應一個固定的 info-string，給一個
/// 下游子系統使用 — 嚴禁兩個 consumer 共用同一個 purpose。
///
/// **Reserved prefix**: `phantom-mesh.v1.*` — `v2` is reserved for future
/// master rotation upgrade.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/identity/")]
#[serde(rename_all = "snake_case")]
pub enum KeyPurpose {
    /// SPEC-13 age event encryption (32 B, feeds age `X25519Recipient`).
    EventEncrypt,
    /// SPEC-10 mesh-rpc cluster HMAC (32 B, rotated per membership change).
    ClusterHmac,
    /// SPEC-15 broker-vault JWT HS256 sign secret (32 B, per broker re-pair).
    BrokerJwtSign,
    /// SPEC-29 skill-sync author MAC (32 B, never rotated).
    SkillSyncMac,
    /// `phantom keys backup` wrap key (32 B, per backup).
    BackupWrap,
}

impl KeyPurpose {
    /// Lower-kebab slug used inside the HKDF info-string.
    /// `EventEncrypt` → `"event-encrypt"`, etc.
    ///
    /// 中文: 回傳 lower-kebab purpose slug，組成 `phantom-mesh.v1.<slug>`。
    pub const fn slug(self) -> &'static str {
        match self {
            KeyPurpose::EventEncrypt => "event-encrypt",
            KeyPurpose::ClusterHmac => "cluster-hmac",
            KeyPurpose::BrokerJwtSign => "broker-jwt-sign",
            KeyPurpose::SkillSyncMac => "skill-sync-mac",
            KeyPurpose::BackupWrap => "backup-wrap",
        }
    }

    /// Full HKDF info-string: `"phantom-mesh.v1.<slug>"`.
    /// Stable across versions — bumping the `v1` prefix requires a master
    /// rotation migration (see §7.5).
    pub fn info_string(self) -> String {
        format!("phantom-mesh.v1.{}", self.slug())
    }
}

// ─── §7.3 KeystoreBackend — 5-OS keystore matrix ─────────────────────────────

/// Which OS-native secret-storage backend is being used for the master seed.
/// Reported back to the UI via `identity_keystore_backend` Tauri command so
/// the diagnostic screen can show "macOS Keychain (Touch ID protected)" etc.
///
/// 中文: 平台 keystore 後端列舉，5 OS × 1 fallback。`FileChmod0600` 是 desktop
/// (mac / linux) 在 OS keystore 不可用時的最後手段；**mobile (iOS / Android)
/// 嚴禁降級到 file** — 直接回 `identity.keystore_unavailable` 錯誤。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/identity/")]
#[serde(rename_all = "snake_case")]
pub enum KeystoreBackend {
    /// macOS Keychain (`kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`).
    MacosKeychain,
    /// iOS Keychain (`kSecAttrAccessibleWhenUnlockedThisDeviceOnly`,
    /// access-group=`group.ai.phantommesh.app`).
    IosKeychain,
    /// Android EncryptedSharedPreferences backed by AndroidKeyStore.
    AndroidEncryptedSharedPreferences,
    /// Windows Credential Manager + DPAPI per-user wrap.
    WindowsCredentialManager,
    /// Linux Secret Service (`org.freedesktop.secrets`, default collection).
    LinuxSecretService,
    /// File fallback `~/.phantom-mesh/identity.key` mode 0600 (desktop only).
    FileChmod0600,
}

impl KeystoreBackend {
    /// Stable string name used in logs / `InitOutcome::keystore_backend` and
    /// returned by `identity_keystore_backend` Tauri command.
    pub const fn name(self) -> &'static str {
        match self {
            KeystoreBackend::MacosKeychain => "macos-keychain",
            KeystoreBackend::IosKeychain => "ios-keychain",
            KeystoreBackend::AndroidEncryptedSharedPreferences => "android-encshpref",
            KeystoreBackend::WindowsCredentialManager => "windows-credman",
            KeystoreBackend::LinuxSecretService => "linux-secret-service",
            KeystoreBackend::FileChmod0600 => "file-chmod-0600",
        }
    }

    /// `true` if this backend is permitted on mobile platforms. Mobile
    /// (iOS / Android) MUST NOT silently fall back to `FileChmod0600` — see
    /// §9.2 trait contract.
    pub const fn allowed_on_mobile(self) -> bool {
        matches!(
            self,
            KeystoreBackend::IosKeychain | KeystoreBackend::AndroidEncryptedSharedPreferences
        )
    }
}

// ─── §7.4 BackupArtifact — `phantom keys backup` output ─────────────────────

/// Output of `phantom keys backup --to <path>` — base64 of master seed +
/// SHA-256 footer fingerprint for tamper-evidence and import-time confirm.
///
/// 中文: backup 匯出格式。`master_seed_b64` 是 base64-encoded 32-byte seed；
/// `sha256_hex` 是該 seed 的 SHA-256 hex（用於 restore 時雙重檢查）；
/// `fingerprint_first_12` 是 SHA-256(verifying)[..12]，使用者可肉眼比對。
///
/// **Warning**: this struct contains plaintext master material — callers
/// MUST treat the in-memory value as secret (drop ASAP, never log).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/identity/")]
#[serde(rename_all = "camelCase")]
pub struct BackupArtifact {
    /// Base64-encoded 32-byte ed25519 master seed.
    pub master_seed_b64: String,
    /// Hex-encoded SHA-256 of the raw 32-byte seed (tamper detection footer).
    pub sha256_hex: String,
    /// 12-hex short fingerprint (matches `IdentityPublic.fingerprint`).
    pub fingerprint_first_12: String,
    /// ISO-8601 timestamp the backup was produced.
    pub created_at: String,
    /// Schema version of the backup format — currently `1`.
    pub schema_version: u8,
}

// ─── §11 KeyDerivationErrorWire — error catalog mirror ───────────────────────

/// Wire-facing error variants for the identity / HKDF subsystem. Mirrors the
/// SPEC-12 §11 error catalog one-to-one. The legacy
/// `core::life_node::key_derivation::KeyDerivationError` (used by SPEC-13 age)
/// is kept untouched for backward compat — Stage 2 will map between the two.
///
/// 中文: SPEC-12 §11 error catalog 的 wire-facing 鏡像。原本
/// `life_node::key_derivation::KeyDerivationError` 不動，Stage 2 加 mapping。
#[derive(Debug, Clone, Serialize, Deserialize, TS, thiserror::Error)]
#[ts(export, export_to = "../../app/src/lib/generated/identity/")]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum KeyDerivationError {
    /// OS keystore API unavailable (iOS Keychain locked / Android KS uninit).
    #[error("identity.keystore_unavailable: {detail}")]
    KeystoreUnavailable { detail: String },
    /// `derive_subkey` called before `init`.
    #[error("identity.master_not_found")]
    MasterNotFound,
    /// Backup-restore fingerprint confirm failed (input != expected first-12).
    #[error("identity.fingerprint_mismatch: expected={expected} got={got}")]
    FingerprintMismatch { expected: String, got: String },
    /// `purpose` slug not lower-kebab (must match `[a-z0-9-]+`).
    #[error("identity.hkdf_purpose_invalid: {purpose}")]
    HkdfPurposeInvalid { purpose: String },
    /// v0.5.0 legacy key file had wrong byte length.
    #[error("identity.legacy_format_seed: got {got} bytes, expected 32")]
    LegacyFormatSeed { got: usize },
    /// HKDF expand step failed (e.g. zero-length IKM).
    #[error("identity.hkdf_expand_failed: {0}")]
    HkdfExpandFailed(String),
    /// I/O error reading / writing file fallback path.
    #[error("identity.io: {0}")]
    Io(String),
}

// ─── Rust-internal master record (NOT ts-rs exported) ────────────────────────

/// Master identity record — held inside `core` crate only. The `seed` field
/// MUST NOT cross the FFI boundary (no ts-rs derive, no Serialize / Deserialize
/// derive). The companion `IdentityPublic` is the only thing UI ever sees.
///
/// 中文: 主身份結構，僅 `core` crate 內可見。`seed` **不**衍生 Serialize 也
/// **不**過 ts-rs — 嚴禁跨 FFI。對外只能透過 `IdentityPublic`。
///
/// TODO Stage 2: wrap `seed` in `zeroize::Zeroizing<[u8; 32]>` so Drop-time
/// memset is guaranteed even on panic unwind. The `zeroize` crate is already
/// in `core/Cargo.toml` (with `features = ["derive"]`) — kept as raw `[u8; 32]`
/// here to minimise dependency-surface churn during Stage 1 merge.
#[derive(Debug, Clone)]
pub struct IdentityKey {
    /// ed25519 master seed (32 bytes). `pub(crate)` — external callers MUST
    /// go through `derive_subkey()` instead of reading this directly.
    #[allow(dead_code)] // held for completeness; key derivation reads it via the wire path, not direct field access
    pub(crate) seed: [u8; 32],
    /// Hex-encoded ed25519 verifying (public) key — 64 hex chars.
    pub verifying_hex: String,
    /// SHA-256(verifying)[..12] hex — 12 chars for display.
    pub fingerprint: String,
}

// ─── §9.2 Stub helpers (Stage 2 implements; Stage 1 leaves `unimplemented!()`) ─

// ─── Stage 2 helpers — pseudocode bodies (Stage 3 fills inner _pseudo fns) ───
//
// Per docs/superpowers/SPEC-TO-CODE-PLAYBOOK.md:
//   Stage 2 = function body shows what it WILL do via comments + nested
//   unimplemented!() inner helpers. Reader can audit the algorithm flow
//   without trusting any cryptographic implementation. Stage 3 swaps the
//   `_pseudo` helpers for real ed25519-dalek / hkdf / sha2 / per-OS keystore
//   calls (added then).

/// Generate or load the master seed, then build `InitOutcome` per §6.2 flow.
///
/// `force == true` overwrites any pre-existing identity (used by
/// `phantom keys init --force` after explicit user confirmation).
///
/// 中文: `phantom keys init` 主邏輯。`force=true` 會覆寫既有身份 — 配合 CLI
/// 端 `--force` 旗標 + 使用者二次確認。
pub fn build_init_outcome(_force: bool) -> Result<InitOutcome, KeyDerivationError> {
    // Step 1: probe the default backend for an existing `identity-master`
    //         record. When present and `_force == false`, short-circuit with
    //         `created=false` so CLI prints "已存在的身份 <fp> — 略過". Stage
    //         3 picks the backend per OS via the same dispatch table used by
    //         `write_to_keystore`.
    let default_backend = default_backend_for_os();
    let existing: Option<Vec<u8>> =
        match read_from_keystore(default_backend, "identity-master") {
            Ok(seed) => Some(seed),
            Err(KeyDerivationError::MasterNotFound) => None,
            Err(e) => return Err(e),
        };

    // Step 2: if no seed (or force-overwrite), generate a fresh ed25519
    //         master via OsRng. `keygen_ed25519` returns the raw 32-byte
    //         seed + matching verifying-key bytes (deterministic given seed).
    let existing_some_pre = existing.is_some();
    let (seed, verifying_bytes): ([u8; 32], [u8; 32]) = match existing {
        Some(prev) if !_force => {
            // Re-derive the verifying key from the pre-existing seed.
            // Length-check before slicing into the fixed-size buffer to keep
            // a malformed legacy file from triggering a panic.
            if prev.len() != 32 {
                return Err(KeyDerivationError::LegacyFormatSeed { got: prev.len() });
            }
            let mut s = [0u8; 32];
            s.copy_from_slice(&prev);
            let v = derive_verifying_from_seed(&s);
            (s, v)
        }
        _ => keygen_ed25519(),
    };

    // Step 3: compute the canonical 12-hex short fingerprint over the
    //         verifying-key bytes (NOT the seed). `fingerprint_short` is the
    //         single source of truth — both init and delete paths call it so
    //         the value displayed to the user is identical everywhere.
    let fp: String = fingerprint_short(&verifying_bytes);

    // Step 4: persist the new master seed atomically into the OS keystore.
    //         Only required when we actually generated a fresh seed; reusing
    //         an existing one is a no-op write.
    if !existing_some_pre || _force {
        write_to_keystore(default_backend, "identity-master", &seed)?;
    }

    // Step 5: build the wire-facing `InitOutcome`. `created` is `true` iff a
    //         brand-new seed was generated this call; `initialized_at` uses
    //         the wall-clock ISO-8601 timestamp Stage 3 will pull from chrono.
    Ok(InitOutcome {
        created: !existing_some_pre || _force,
        fingerprint: fp,
        public_key_hex: hex_encode_verifying(&verifying_bytes),
        keystore_backend: default_backend.name().to_string(),
        initialized_at: iso8601_now(),
    })
}

/// HKDF-SHA256 subkey derivation. Deterministic for the same master + purpose.
///
/// info-string is `phantom-mesh.v1.<purpose.slug()>` per §7.2.
/// Returns `KeyDerivationError::MasterNotFound` if `init` has not been called.
///
/// 中文: HKDF-SHA256 子金鑰派生器。同一 master + 同一 purpose 永遠回同一結果。
pub fn derive_subkey(_purpose: KeyPurpose) -> Result<[u8; 32], KeyDerivationError> {
    // Step 1: load the master seed from the OS-default keystore. Propagate
    //         `MasterNotFound` straight back so the caller can prompt
    //         `phantom keys init` per the §11 error catalog UX rules.
    let default_backend = default_backend_for_os();
    let master_seed: Vec<u8> =
        read_from_keystore(default_backend, "identity-master")?;

    // Step 2: HKDF-SHA256(extract → expand). `info` is the stable
    //         `phantom-mesh.v1.<slug>` string from `KeyPurpose::info_string()`.
    //         Salt is intentionally empty — the master seed is already 256-bit
    //         uniform from OsRng so HKDF's salt-mixing isn't needed here.
    let info: String = _purpose.info_string();
    let subkey: [u8; 32] = hkdf_expand(&master_seed, info.as_bytes())?;

    // Step 3: return the 32-byte derived subkey. Same (master, purpose) pair
    //         always produces the same bytes — this is the determinism
    //         guarantee SPEC-13 age + SPEC-10 cluster-HMAC rely on.
    Ok(subkey)
}

/// Compute the 12-hex short fingerprint: `hex(SHA-256(verifying))[..12]`.
/// Used for display + backup confirm. `verifying_key_bytes` is the 32-byte raw
/// ed25519 public key.
///
/// 中文: 12 字元短指紋 — `SHA-256(verifying)` 取前 6 byte hex。
pub fn fingerprint_short(_verifying_key_bytes: &[u8]) -> String {
    // Step 1: SHA-256 the raw verifying-key bytes → 64-hex digest.
    let full_hex: String = sha256_hex(_verifying_key_bytes);

    // Step 2: truncate to the first 12 hex chars (6 raw bytes) — the §7.1
    //         display fingerprint. 12 hex chars = 48 bits, plenty against
    //         accidental collision for the human-visible identity surface.
    full_hex.chars().take(12).collect()
}

/// Persist a secret blob (the 32-byte master seed) into the OS keystore for
/// the chosen backend. MUST be atomic.
///
/// `account` is the keystore record key (e.g. `"identity-master"`); `secret`
/// is the raw bytes. Mobile backends MUST NOT silently fall back to file
/// storage — caller is responsible for not passing `FileChmod0600` on mobile.
///
/// 中文: 把 master seed 寫進指定的 OS keystore backend。必須 atomic — 寫到一半
/// 失敗不能留半成品。Mobile 嚴禁降級到 file。
pub fn write_to_keystore(
    _backend: KeystoreBackend,
    _account: &str,
    _secret: &[u8],
) -> Result<(), KeyDerivationError> {
    // Step 1: dispatch over the 5-OS keystore matrix from §7.3. Each arm
    //         calls into a per-backend `_pseudo` helper that Stage 3 swaps
    //         for the real native binding (security-framework on macOS/iOS,
    //         jni on Android, windows-rs DPAPI on Windows, libsecret on Linux,
    //         std::fs + chmod 0600 for the desktop fallback).
    match _backend {
        // Step 2: macOS + iOS share the Keychain Services API surface.
        //         Stage 3 will gate the iOS arm behind `cfg(target_os="ios")`
        //         so it picks the access-group `group.ai.phantommesh.app`.
        KeystoreBackend::MacosKeychain | KeystoreBackend::IosKeychain => {
            keychain_write_pseudo(_account, _secret)
        }
        // Step 3: Android EncryptedSharedPreferences backed by AndroidKeyStore.
        //         Crosses the JNI boundary in Stage 3 — pseudo placeholder
        //         here just records the intent.
        KeystoreBackend::AndroidEncryptedSharedPreferences => {
            android_ks_write_pseudo(_account, _secret)
        }
        // Step 4: Windows Credential Manager + DPAPI per-user wrap.
        KeystoreBackend::WindowsCredentialManager => {
            dpapi_write_pseudo(_account, _secret)
        }
        // Step 5: Linux Secret Service via libsecret default collection.
        KeystoreBackend::LinuxSecretService => {
            libsecret_write_pseudo(_account, _secret)
        }
        // Step 6: desktop file fallback — write atomically (tmp + rename),
        //         then chmod 0600 so only the owning user can read.
        KeystoreBackend::FileChmod0600 => {
            file_chmod0600_write_pseudo(_account, _secret)
        }
    }
}

/// Read the master seed back from the OS keystore. Returns the raw bytes.
/// Returns `KeyDerivationError::MasterNotFound` if no record exists.
///
/// 中文: 從指定 OS keystore backend 讀回 master seed。不存在則回 MasterNotFound。
pub fn read_from_keystore(
    _backend: KeystoreBackend,
    _account: &str,
) -> Result<Vec<u8>, KeyDerivationError> {
    // Step 1: same 5-OS dispatch as `write_to_keystore`. Each `_read_pseudo`
    //         returns `Ok(bytes)` on hit or `Err(MasterNotFound)` on miss so
    //         the caller can decide whether to prompt for init.
    match _backend {
        KeystoreBackend::MacosKeychain | KeystoreBackend::IosKeychain => {
            keychain_read_pseudo(_account)
        }
        KeystoreBackend::AndroidEncryptedSharedPreferences => {
            android_ks_read_pseudo(_account)
        }
        KeystoreBackend::WindowsCredentialManager => dpapi_read_pseudo(_account),
        KeystoreBackend::LinuxSecretService => libsecret_read_pseudo(_account),
        KeystoreBackend::FileChmod0600 => file_chmod0600_read_pseudo(_account),
    }
}

/// Permanently delete the master seed from the OS keystore.
///
/// `confirm_fingerprint_first_12` MUST match the first 12 hex chars of the
/// current identity's fingerprint, otherwise this returns
/// `KeyDerivationError::FingerprintMismatch` — see §6.3 / §11. Idempotent
/// when the record is already absent AND the fingerprint check passed.
///
/// 中文: 永久刪除 master seed。呼叫端必須提供前 12 字元 fingerprint 做雙重
/// 確認，避免誤刪別把 key。比對失敗回 FingerprintMismatch。
pub fn delete_from_keystore(
    _backend: KeystoreBackend,
    _account: &str,
    _confirm_fingerprint_first_12: &str,
) -> Result<(), KeyDerivationError> {
    // Step 1: read the current master so we can compute its fingerprint and
    //         compare against the user-supplied confirm. Missing-key path is
    //         a hard error here — you can't confirm a fingerprint for a key
    //         that isn't there.
    let seed = read_from_keystore(_backend, _account)?;
    if seed.len() != 32 {
        return Err(KeyDerivationError::LegacyFormatSeed { got: seed.len() });
    }
    let mut seed_buf = [0u8; 32];
    seed_buf.copy_from_slice(&seed);
    let verifying = derive_verifying_from_seed(&seed_buf);
    let actual_fp = fingerprint_short(&verifying);

    // Step 2: compare the first 12 hex chars. Mismatch returns the canonical
    //         §11 error so the CLI / UI can surface "expected X got Y" to
    //         the user. We deliberately do NOT delete first then check — the
    //         confirm gate is the whole point of this function.
    if actual_fp != _confirm_fingerprint_first_12 {
        return Err(KeyDerivationError::FingerprintMismatch {
            expected: actual_fp,
            got: _confirm_fingerprint_first_12.to_string(),
        });
    }

    // Step 3: confirmed match — dispatch to the per-backend delete. After
    //         this point the keystore record is gone and `derive_subkey`
    //         will return `MasterNotFound` until the next `init`.
    match _backend {
        KeystoreBackend::MacosKeychain | KeystoreBackend::IosKeychain => {
            keychain_delete_pseudo(_account)
        }
        KeystoreBackend::AndroidEncryptedSharedPreferences => {
            android_ks_delete_pseudo(_account)
        }
        KeystoreBackend::WindowsCredentialManager => dpapi_delete_pseudo(_account),
        KeystoreBackend::LinuxSecretService => libsecret_delete_pseudo(_account),
        KeystoreBackend::FileChmod0600 => file_chmod0600_delete_pseudo(_account),
    }
}

// ─── Stage 3 inner helpers — core crypto + file fallback + Linux KS live ────
//
// Promoted to real impl (`ed25519-dalek`, `hkdf`, `sha2`, `hex`, `chrono`,
// `dirs`, plus Linux-only `secret-service = "5"` — all already in
// core/Cargo.toml):
//   • `keygen_ed25519` (SigningKey::generate via OsRng)
//   • `derive_verifying_from_seed` (SigningKey::from_bytes → verifying_key)
//   • `hkdf_expand` (Hkdf::<Sha256>::new + expand(info, &mut [u8; 32]))
//   • `sha256_hex` (Sha256::digest + hex::encode)
//   • `hex_encode_verifying` (hex::encode)
//   • `iso8601_now` (chrono::Utc::now → RFC 3339)
//   • `default_backend_for_os` (`cfg!(target_os = …)` 5-way dispatch)
//   • `file_chmod0600_{write,read,delete}` (std::fs + dirs::home_dir +
//     `std::os::unix::fs::PermissionsExt` set 0o600, no-op on Windows)
//   • `libsecret_{write,read,delete}` — **Linux only**, `blocking::SecretService`
//     connect → default collection → create_item / search_items / item.delete.
//     On non-Linux hosts these helpers return `KeystoreUnavailable` (the
//     dispatch path can never legitimately land here off-Linux because
//     `default_backend_for_os` only picks `LinuxSecretService` when
//     `cfg!(target_os = "linux")`).
//
// Still `unimplemented!("Stage 4: requires <crate>")` — these three arms need
// platform crates not yet in Cargo.toml (or wired into the dispatch glue):
//   • `keychain_*`     — `security-framework` (macOS + iOS) — deferred to v0.7.0
//   • `android_ks_*`   — `jni` (+ a Kotlin wrapper for EncryptedSharedPreferences)
//   • `dpapi_*`        — `windows-rs` DPAPI dispatch glue (the `windows`
//     crate IS already a dep but the per-account wrap-and-store wiring isn't)

// --- core crypto primitives (real) ---

/// Generate a fresh ed25519 seed via OsRng and return `(seed, verifying)`.
/// Matches the SPEC-12 §6.2 init flow: caller stores the seed in the
/// keystore and uses the verifying key for fingerprint / public surface.
fn keygen_ed25519() -> ([u8; 32], [u8; 32]) {
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;
    let signing = SigningKey::generate(&mut OsRng);
    let seed = signing.to_bytes();
    let verifying = signing.verifying_key().to_bytes();
    (seed, verifying)
}

/// Re-derive the ed25519 verifying key from a 32-byte seed. Deterministic —
/// same seed always produces the same verifying key.
fn derive_verifying_from_seed(seed: &[u8; 32]) -> [u8; 32] {
    use ed25519_dalek::SigningKey;
    SigningKey::from_bytes(seed).verifying_key().to_bytes()
}

/// HKDF-SHA256(salt=None, ikm, info) → 32 bytes. Salt is `None` because the
/// master seed is already 256-bit uniform from OsRng so HKDF-Extract's
/// salt-mixing isn't needed (RFC 5869 §3.1). An expand failure (only happens
/// for L > 255 * HashLen, unreachable for our fixed 32-byte L) maps to
/// `HkdfExpandFailed`.
fn hkdf_expand(ikm: &[u8], info: &[u8]) -> Result<[u8; 32], KeyDerivationError> {
    use hkdf::Hkdf;
    use sha2::Sha256;
    let h = Hkdf::<Sha256>::new(None, ikm);
    let mut out = [0u8; 32];
    h.expand(info, &mut out)
        .map_err(|e| KeyDerivationError::HkdfExpandFailed(e.to_string()))?;
    Ok(out)
}

/// SHA-256 of `bytes` returned as 64-char lower-case hex. Used by
/// `fingerprint_short` (which truncates to first 12 chars).
fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(bytes))
}

/// Lower-case hex of the 32-byte verifying key (64 chars).
fn hex_encode_verifying(verifying: &[u8; 32]) -> String {
    hex::encode(verifying)
}

/// Current UTC time as RFC 3339 (`2026-05-25T12:34:56Z`). Seconds precision +
/// `use_z=true` so the wire string never carries timezone-offset noise.
fn iso8601_now() -> String {
    use chrono::SecondsFormat;
    chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// 5-OS dispatch — pick the canonical native keystore backend for the host.
/// `FileChmod0600` is only chosen as fallback when no OS arm matches (no
/// other target_os we ship to currently lands here, but the fall-through
/// keeps the function total).
fn default_backend_for_os() -> KeystoreBackend {
    if cfg!(target_os = "macos") {
        KeystoreBackend::MacosKeychain
    } else if cfg!(target_os = "ios") {
        KeystoreBackend::IosKeychain
    } else if cfg!(target_os = "android") {
        KeystoreBackend::AndroidEncryptedSharedPreferences
    } else if cfg!(target_os = "windows") {
        KeystoreBackend::WindowsCredentialManager
    } else if cfg!(target_os = "linux") {
        KeystoreBackend::LinuxSecretService
    } else {
        KeystoreBackend::FileChmod0600
    }
}

// --- macOS / iOS Keychain (security-framework crate) ---

fn keychain_write_pseudo(
    _account: &str,
    _secret: &[u8],
) -> Result<(), KeyDerivationError> {
    unimplemented!(
        "Stage 4: requires security-framework — SecKeychain::default().set_generic_password(service=\"ai.phantommesh\", _account, _secret); accessibility=AfterFirstUnlockThisDeviceOnly (macOS) / WhenUnlockedThisDeviceOnly + access-group on iOS"
    )
}

fn keychain_read_pseudo(_account: &str) -> Result<Vec<u8>, KeyDerivationError> {
    unimplemented!(
        "Stage 4: requires security-framework — SecKeychain::default().find_generic_password(\"ai.phantommesh\", _account); map errSecItemNotFound → MasterNotFound"
    )
}

fn keychain_delete_pseudo(_account: &str) -> Result<(), KeyDerivationError> {
    unimplemented!(
        "Stage 4: requires security-framework — locate then SecKeychainItem::delete; idempotent on missing"
    )
}

// --- Android EncryptedSharedPreferences (jni) ---

fn android_ks_write_pseudo(
    _account: &str,
    _secret: &[u8],
) -> Result<(), KeyDerivationError> {
    unimplemented!(
        "Stage 4: requires jni — call into Kotlin wrapper EncryptedSharedPreferences.create(...).edit().putString(_account, base64(_secret)).apply()"
    )
}

fn android_ks_read_pseudo(_account: &str) -> Result<Vec<u8>, KeyDerivationError> {
    unimplemented!(
        "Stage 4: requires jni — getString(_account, null); base64 decode; map null → MasterNotFound"
    )
}

fn android_ks_delete_pseudo(_account: &str) -> Result<(), KeyDerivationError> {
    unimplemented!("Stage 4: requires jni — edit().remove(_account).apply()")
}

// --- Windows Credential Manager + DPAPI (windows-rs) ---

fn dpapi_write_pseudo(
    _account: &str,
    _secret: &[u8],
) -> Result<(), KeyDerivationError> {
    unimplemented!(
        "Stage 4: requires windows-rs — CryptProtectData per-user wrap, then CredWrite (CRED_TYPE_GENERIC, target=_account)"
    )
}

fn dpapi_read_pseudo(_account: &str) -> Result<Vec<u8>, KeyDerivationError> {
    unimplemented!(
        "Stage 4: requires windows-rs — CredRead then CryptUnprotectData; map ERROR_NOT_FOUND → MasterNotFound"
    )
}

fn dpapi_delete_pseudo(_account: &str) -> Result<(), KeyDerivationError> {
    unimplemented!("Stage 4: requires windows-rs — CredDelete(target=_account, CRED_TYPE_GENERIC)")
}

// --- Linux Secret Service (`secret-service = "5"`, Linux-only dep) ---
//
// All three helpers are `#[cfg(target_os = "linux")]`-gated for the real D-Bus
// path; non-Linux builds get a small shim that returns `KeystoreUnavailable`
// so the dispatch `match` in `write_to_keystore` / `read_from_keystore` /
// `delete_from_keystore` still type-checks on macOS / Windows. The non-Linux
// branch is unreachable in practice because `default_backend_for_os` only
// returns `LinuxSecretService` when `cfg!(target_os = "linux")`.
//
// Schema: every item is stored in the default collection under attributes
// `{ "application": "phantom-mesh", "account": <account> }`. The label is the
// account string itself (shown verbatim in GNOME Seahorse / KDE Wallet UIs).
// The secret payload is the raw 32-byte master seed; we pin
// `content_type = "application/octet-stream"` so the keystore doesn't try to
// interpret it as text. `replace_if_exists = true` so re-init overwrites
// cleanly instead of stacking duplicate items.

#[cfg(target_os = "linux")]
fn libsecret_attributes(account: &str) -> std::collections::HashMap<&str, &str> {
    let mut attrs = std::collections::HashMap::new();
    attrs.insert("application", "phantom-mesh");
    attrs.insert("account", account);
    attrs
}

#[cfg(target_os = "linux")]
fn libsecret_write_pseudo(
    account: &str,
    secret: &[u8],
) -> Result<(), KeyDerivationError> {
    use secret_service::{blocking::SecretService, EncryptionType};
    let ss = SecretService::connect(EncryptionType::Dh)
        .map_err(|e| KeyDerivationError::KeystoreUnavailable { detail: e.to_string() })?;
    let collection = ss
        .get_default_collection()
        .map_err(|e| KeyDerivationError::KeystoreUnavailable { detail: e.to_string() })?;
    // Make sure the default collection is unlocked before we write — locked
    // collections will prompt the user via the desktop secret-agent and
    // surface as a `KeystoreUnavailable` if the prompt is dismissed.
    collection
        .ensure_unlocked()
        .map_err(|e| KeyDerivationError::KeystoreUnavailable { detail: e.to_string() })?;
    let attrs = libsecret_attributes(account);
    collection
        .create_item(
            account,
            attrs,
            secret,
            true, // replace_if_exists — idempotent re-init
            "application/octet-stream",
        )
        .map_err(|e| KeyDerivationError::KeystoreUnavailable { detail: e.to_string() })?;
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn libsecret_write_pseudo(
    _account: &str,
    _secret: &[u8],
) -> Result<(), KeyDerivationError> {
    // Unreachable in practice: `default_backend_for_os` only picks
    // `LinuxSecretService` on Linux. Direct callers (tests, manual dispatch)
    // get a typed error instead of `unimplemented!()` so the binary still
    // links on macOS / Windows hosts.
    Err(KeyDerivationError::KeystoreUnavailable {
        detail: "LinuxSecretService backend is Linux-only (secret-service crate)".to_string(),
    })
}

#[cfg(target_os = "linux")]
fn libsecret_read_pseudo(account: &str) -> Result<Vec<u8>, KeyDerivationError> {
    use secret_service::{blocking::SecretService, EncryptionType};
    let ss = SecretService::connect(EncryptionType::Dh)
        .map_err(|e| KeyDerivationError::KeystoreUnavailable { detail: e.to_string() })?;
    let collection = ss
        .get_default_collection()
        .map_err(|e| KeyDerivationError::KeystoreUnavailable { detail: e.to_string() })?;
    collection
        .ensure_unlocked()
        .map_err(|e| KeyDerivationError::KeystoreUnavailable { detail: e.to_string() })?;
    let attrs = libsecret_attributes(account);
    let items = collection
        .search_items(attrs)
        .map_err(|e| KeyDerivationError::KeystoreUnavailable { detail: e.to_string() })?;
    let item = items
        .into_iter()
        .next()
        .ok_or(KeyDerivationError::MasterNotFound)?;
    item.ensure_unlocked()
        .map_err(|e| KeyDerivationError::KeystoreUnavailable { detail: e.to_string() })?;
    let secret = item
        .get_secret()
        .map_err(|e| KeyDerivationError::KeystoreUnavailable { detail: e.to_string() })?;
    Ok(secret)
}

#[cfg(not(target_os = "linux"))]
fn libsecret_read_pseudo(_account: &str) -> Result<Vec<u8>, KeyDerivationError> {
    Err(KeyDerivationError::KeystoreUnavailable {
        detail: "LinuxSecretService backend is Linux-only (secret-service crate)".to_string(),
    })
}

#[cfg(target_os = "linux")]
fn libsecret_delete_pseudo(account: &str) -> Result<(), KeyDerivationError> {
    use secret_service::{blocking::SecretService, EncryptionType};
    let ss = SecretService::connect(EncryptionType::Dh)
        .map_err(|e| KeyDerivationError::KeystoreUnavailable { detail: e.to_string() })?;
    let collection = ss
        .get_default_collection()
        .map_err(|e| KeyDerivationError::KeystoreUnavailable { detail: e.to_string() })?;
    collection
        .ensure_unlocked()
        .map_err(|e| KeyDerivationError::KeystoreUnavailable { detail: e.to_string() })?;
    let attrs = libsecret_attributes(account);
    let items = collection
        .search_items(attrs)
        .map_err(|e| KeyDerivationError::KeystoreUnavailable { detail: e.to_string() })?;
    // Idempotent: no hit = already gone = success (matches file fallback).
    for item in items {
        item.delete()
            .map_err(|e| KeyDerivationError::KeystoreUnavailable { detail: e.to_string() })?;
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn libsecret_delete_pseudo(_account: &str) -> Result<(), KeyDerivationError> {
    Err(KeyDerivationError::KeystoreUnavailable {
        detail: "LinuxSecretService backend is Linux-only (secret-service crate)".to_string(),
    })
}

// --- Desktop file fallback (std::fs + chmod 0600) — real ---

/// Compute the on-disk path for a per-account key file:
/// `<home>/.phantom-mesh/<account>.key`. The parent directory is created
/// (mode 0o700 on unix) if it does not exist so callers don't have to
/// pre-create `~/.phantom-mesh/` themselves.
fn file_chmod0600_path(account: &str) -> Result<std::path::PathBuf, KeyDerivationError> {
    let home = dirs::home_dir().ok_or_else(|| {
        KeyDerivationError::Io("home_dir unavailable for FileChmod0600 backend".to_string())
    })?;
    let dir = home.join(".phantom-mesh");
    if !dir.exists() {
        std::fs::create_dir_all(&dir).map_err(|e| KeyDerivationError::Io(e.to_string()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        }
    }
    Ok(dir.join(format!("{}.key", account)))
}

/// Write `secret` to `~/.phantom-mesh/<account>.key` atomically: write to a
/// `.tmp` sibling, fsync, rename, then chmod 0600 (no-op on Windows). The
/// tmp-then-rename pattern is the standard atomic file write trick — a crash
/// between steps leaves either the old file intact or the new file intact,
/// never a half-written blob.
fn file_chmod0600_write_pseudo(
    account: &str,
    secret: &[u8],
) -> Result<(), KeyDerivationError> {
    use std::io::Write;
    let path = file_chmod0600_path(account)?;
    let tmp = path.with_extension("key.tmp");
    {
        let mut f = std::fs::File::create(&tmp)
            .map_err(|e| KeyDerivationError::Io(e.to_string()))?;
        f.write_all(secret).map_err(|e| KeyDerivationError::Io(e.to_string()))?;
        f.sync_all().map_err(|e| KeyDerivationError::Io(e.to_string()))?;
    }
    std::fs::rename(&tmp, &path).map_err(|e| KeyDerivationError::Io(e.to_string()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| KeyDerivationError::Io(e.to_string()))?;
    }
    Ok(())
}

/// Read `~/.phantom-mesh/<account>.key` back as raw bytes. NotFound maps to
/// `MasterNotFound` so callers can branch into the init flow; any other I/O
/// error surfaces as `Io`.
fn file_chmod0600_read_pseudo(account: &str) -> Result<Vec<u8>, KeyDerivationError> {
    let path = file_chmod0600_path(account)?;
    match std::fs::read(&path) {
        Ok(bytes) => Ok(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(KeyDerivationError::MasterNotFound)
        }
        Err(e) => Err(KeyDerivationError::Io(e.to_string())),
    }
}

/// Delete `~/.phantom-mesh/<account>.key`. Idempotent — NotFound is silently
/// treated as success so a double-delete doesn't error.
fn file_chmod0600_delete_pseudo(account: &str) -> Result<(), KeyDerivationError> {
    let path = file_chmod0600_path(account)?;
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(KeyDerivationError::Io(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_public_round_trip_smoke() {
        // §7.1 invariant: TS encode → wire → Rust decode → re-encode preserves
        // the public surface. Stage 1 sanity-checks serde; deeper invariants
        // (e.g. fingerprint matches SHA-256 of public key) come in Stage 2.
        let p = IdentityPublic {
            public_key: "00".repeat(32),
            fingerprint: "0".repeat(12),
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let j = serde_json::to_string(&p).unwrap();
        let back: IdentityPublic = serde_json::from_str(&j).unwrap();
        assert_eq!(p.fingerprint, back.fingerprint);
        assert_eq!(p.public_key, back.public_key);
        assert_eq!(p.created_at, back.created_at);
    }

    #[test]
    fn key_purpose_info_string_is_stable() {
        // §7.2 reserved prefix invariant: all v1 purposes use
        // `phantom-mesh.v1.<slug>`. Any change to this string is a wire-break.
        assert_eq!(
            KeyPurpose::EventEncrypt.info_string(),
            "phantom-mesh.v1.event-encrypt"
        );
        assert_eq!(
            KeyPurpose::ClusterHmac.info_string(),
            "phantom-mesh.v1.cluster-hmac"
        );
        assert_eq!(
            KeyPurpose::BrokerJwtSign.info_string(),
            "phantom-mesh.v1.broker-jwt-sign"
        );
        assert_eq!(
            KeyPurpose::SkillSyncMac.info_string(),
            "phantom-mesh.v1.skill-sync-mac"
        );
        assert_eq!(
            KeyPurpose::BackupWrap.info_string(),
            "phantom-mesh.v1.backup-wrap"
        );
    }

    #[test]
    fn keystore_backend_mobile_gate() {
        // §7.3 invariant: mobile backends are exactly {IosKeychain,
        // AndroidEncryptedSharedPreferences}. Desktop file fallback must
        // never be considered mobile-allowed (§9.2 trait contract).
        assert!(KeystoreBackend::IosKeychain.allowed_on_mobile());
        assert!(KeystoreBackend::AndroidEncryptedSharedPreferences.allowed_on_mobile());
        assert!(!KeystoreBackend::FileChmod0600.allowed_on_mobile());
        assert!(!KeystoreBackend::MacosKeychain.allowed_on_mobile());
        assert!(!KeystoreBackend::WindowsCredentialManager.allowed_on_mobile());
        assert!(!KeystoreBackend::LinuxSecretService.allowed_on_mobile());
    }

    #[test]
    fn backup_artifact_round_trip_smoke() {
        let b = BackupArtifact {
            master_seed_b64: "AAAA".to_string(),
            sha256_hex: "0".repeat(64),
            fingerprint_first_12: "abcdef012345".to_string(),
            created_at: "2026-05-25T00:00:00Z".to_string(),
            schema_version: 1,
        };
        let j = serde_json::to_string(&b).unwrap();
        let back: BackupArtifact = serde_json::from_str(&j).unwrap();
        assert_eq!(b.fingerprint_first_12, back.fingerprint_first_12);
        assert_eq!(b.schema_version, back.schema_version);
    }

    // ─── Stage 3 KAT (known-answer-test) vectors ─────────────────────────
    //
    // These replace the Stage 2 `#[should_panic(expected = "Stage 3")]`
    // markers now that the core crypto + file-fallback paths are real. The
    // four native-keystore arms (`keychain_*` / `android_ks_*` / `dpapi_*` /
    // `libsecret_*`) still panic with `Stage 4` until their platform crates
    // land; that path is exercised by the marker test at the bottom.

    /// `fingerprint_short` over the all-zero 32-byte verifying key — pinned
    /// against the well-known SHA-256 of 32 zero bytes
    /// (`66687aadf862bd776c8fc18b8e9f8e20089714856ee233b3902a591d0d5f2925`),
    /// truncated to the first 12 hex chars per §7.1.
    #[test]
    fn fingerprint_short_matches_zero_key_kat() {
        let fp = fingerprint_short(&[0u8; 32]);
        assert_eq!(fp.len(), 12, "fingerprint MUST be 12 hex chars per §7.1");
        assert_eq!(fp, "66687aadf862", "SHA-256(00…)[..12] hex pin");
    }

    /// RFC 5869 Test Case 1 (truncated to 32 bytes): salt=empty (we use None),
    /// IKM=22 zero bytes, info=empty → first 32 bytes of OKM. Our
    /// `hkdf_expand` uses `None` salt + `info = purpose.info_string()`, so we
    /// verify the stable info-string mapping rather than RFC bytes directly:
    /// same `(ikm, purpose)` MUST always yield the same 32-byte subkey.
    #[test]
    fn hkdf_expand_is_deterministic_per_purpose() {
        let ikm = b"deterministic-master-seed-32-byt";
        let info = KeyPurpose::EventEncrypt.info_string();
        let a = hkdf_expand(ikm, info.as_bytes()).unwrap();
        let b = hkdf_expand(ikm, info.as_bytes()).unwrap();
        assert_eq!(a, b, "same (ikm, info) MUST yield same subkey");
        // Different purpose → different subkey.
        let c = hkdf_expand(
            ikm,
            KeyPurpose::ClusterHmac.info_string().as_bytes(),
        )
        .unwrap();
        assert_ne!(
            a, c,
            "different purpose info MUST yield different subkey"
        );
    }

    /// `keygen_ed25519` MUST produce a verifying key that re-derives
    /// deterministically from its seed. Catches accidental seed/verifying
    /// pair swaps in the returned tuple.
    #[test]
    fn keygen_ed25519_seed_re_derives_verifying() {
        let (seed, verifying) = keygen_ed25519();
        let re = derive_verifying_from_seed(&seed);
        assert_eq!(verifying, re, "verifying key must round-trip from seed");
    }

    /// `iso8601_now` returns an RFC 3339 string ending in `Z` (UTC) with
    /// seconds precision. Verifies the chrono call doesn't accidentally emit
    /// a timezone offset like `+00:00`.
    #[test]
    fn iso8601_now_ends_with_z_utc() {
        let s = iso8601_now();
        assert!(s.ends_with('Z'), "RFC 3339 UTC timestamp must end in Z: {}", s);
        // Format example: 2026-05-25T12:34:56Z → 20 chars
        assert_eq!(s.len(), 20, "expected 20-char RFC 3339 UTC: {}", s);
    }

    /// `default_backend_for_os` MUST return a backend that is `allowed_on_mobile()`
    /// when targeting iOS or Android, and MUST NOT return `FileChmod0600` on
    /// any of the 5 first-class OS targets. Pins the §7.3 trait contract.
    #[test]
    fn default_backend_for_os_matches_target() {
        let backend = default_backend_for_os();
        if cfg!(target_os = "macos") {
            assert_eq!(backend, KeystoreBackend::MacosKeychain);
        } else if cfg!(target_os = "ios") {
            assert_eq!(backend, KeystoreBackend::IosKeychain);
            assert!(backend.allowed_on_mobile());
        } else if cfg!(target_os = "android") {
            assert_eq!(backend, KeystoreBackend::AndroidEncryptedSharedPreferences);
            assert!(backend.allowed_on_mobile());
        } else if cfg!(target_os = "windows") {
            assert_eq!(backend, KeystoreBackend::WindowsCredentialManager);
        } else if cfg!(target_os = "linux") {
            assert_eq!(backend, KeystoreBackend::LinuxSecretService);
        }
    }

    /// File fallback round trip: write → read → delete → read-returns-NotFound.
    /// Uses a one-off account name so the test doesn't collide with any real
    /// identity material in `~/.phantom-mesh/`.
    #[test]
    fn file_chmod0600_round_trip() {
        let account =
            format!("phantom-test-{}-{}", std::process::id(), uuid::Uuid::new_v4());
        let secret = b"stage3-test-secret-bytes";

        // Write
        file_chmod0600_write_pseudo(&account, secret)
            .expect("file fallback write must succeed");
        // Read back
        let got = file_chmod0600_read_pseudo(&account)
            .expect("file fallback read must succeed");
        assert_eq!(got, secret);
        // Delete
        file_chmod0600_delete_pseudo(&account)
            .expect("file fallback delete must succeed");
        // Read after delete must be MasterNotFound
        match file_chmod0600_read_pseudo(&account) {
            Err(KeyDerivationError::MasterNotFound) => {} // expected
            other => panic!(
                "read after delete must be MasterNotFound, got {:?}",
                other
            ),
        }
        // Delete again must be idempotent (no error).
        file_chmod0600_delete_pseudo(&account)
            .expect("delete must be idempotent on missing");
    }

    // ─── Stage 4 marker test ─────────────────────────────────────────────
    //
    // Tracks the four still-pseudocode native keystore arms (`keychain_*` /
    // `android_ks_*` / `dpapi_*` / `libsecret_*`). When Stage 4 lands (adds
    // `security-framework` / `jni` / `windows-rs` / `libsecret` to Cargo.toml
    // and wires the native arms), this marker will start failing on at least
    // one host — the cue to delete it and replace with per-OS integration
    // tests.

    #[test]
    #[should_panic(expected = "Stage 4")]
    fn keychain_write_panics_pending_security_framework() {
        // Stage 3 → Stage 4 marker — proves the macOS/iOS Keychain arm is
        // still unimplemented because the `security-framework` crate is not
        // yet a dependency. Calling the helper directly (not via
        // `write_to_keystore`) keeps the marker independent of the OS the
        // test runs on.
        let _ = keychain_write_pseudo("dummy", b"dummy");
    }

    // ─── Linux Secret Service round-trip KAT (ignored by default) ───────
    //
    // Exercises the live `secret-service = "5"` `blocking::SecretService` path
    // end-to-end on a Linux box that has a running D-Bus session bus + an
    // unlocked default collection (gnome-keyring / kwallet / KeePassXC secret
    // service / etc). Marked `#[ignore]` because:
    //   • CI / macOS / Windows have no D-Bus session bus → connect() fails
    //   • Even on Linux headless CI the default collection is usually locked
    //     and would block waiting for a desktop prompt
    // Run manually on a Linux desktop with:
    //     CARGO_TARGET_DIR=target cargo test \
    //       -p phantom_core identity_wire::tests::libsecret_round_trip \
    //       --release -- --ignored --nocapture
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore]
    fn libsecret_round_trip_linux_only() {
        let account =
            format!("phantom-test-libsecret-{}-{}", std::process::id(), uuid::Uuid::new_v4());
        let secret = b"stage3-libsecret-test-secret-bytes-32b!";

        // Write
        libsecret_write_pseudo(&account, secret)
            .expect("libsecret write must succeed on unlocked default collection");
        // Read back
        let got = libsecret_read_pseudo(&account)
            .expect("libsecret read must return the bytes just written");
        assert_eq!(got, secret);
        // Delete
        libsecret_delete_pseudo(&account)
            .expect("libsecret delete must succeed");
        // Read after delete must be MasterNotFound
        match libsecret_read_pseudo(&account) {
            Err(KeyDerivationError::MasterNotFound) => {} // expected
            other => panic!(
                "libsecret read after delete must be MasterNotFound, got {:?}",
                other
            ),
        }
        // Delete again must be idempotent (no-op when no item matches).
        libsecret_delete_pseudo(&account)
            .expect("libsecret delete must be idempotent on missing");
    }

    #[test]
    fn key_derivation_error_serializes_with_code_tag() {
        // §11 invariant: error wire shape uses `{"code": "..."}` tag so the
        // UI can dispatch on the machine-readable code string. Verify a
        // couple of variants survive round-trip.
        let e = KeyDerivationError::MasterNotFound;
        let j = serde_json::to_string(&e).unwrap();
        assert!(j.contains("master_not_found"), "wire shape: {}", j);

        let e2 = KeyDerivationError::HkdfPurposeInvalid {
            purpose: "BadCase".to_string(),
        };
        let j2 = serde_json::to_string(&e2).unwrap();
        assert!(j2.contains("hkdf_purpose_invalid"), "wire shape: {}", j2);
        assert!(j2.contains("BadCase"), "payload preserved: {}", j2);
    }
}
