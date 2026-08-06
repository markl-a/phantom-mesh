//! Device identity: keypair generation, persistence, and node-id derivation.
//!
//! This module (`identity_wire`) is the single source of truth for a device's
//! cryptographic identity in spectyn-mesh. It owns three concerns:
//!
//! 1. **Generation** — [`build_init_outcome`] creates a fresh ed25519 master
//!    seed via `OsRng` ([`keygen_ed25519`]) on first `spectyn keys init`, and
//!    re-derives the verifying (public) key from a pre-existing seed on
//!    subsequent calls. The 32-byte master seed is the root of all identity
//!    material; every subkey is HKDF-derived from it via [`derive_subkey`].
//!
//! 2. **Persistence** — the master seed is stored in the host's native secret
//!    store via the [`KeystoreBackend`] matrix (macOS/iOS Keychain, Android
//!    EncryptedSharedPreferences, Windows Credential Manager + DPAPI, Linux
//!    Secret Service), with a desktop-only `~/.spectyn-mesh/<account>.key`
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
//! (`core/src/runtime.rs`, `SpectynMeshRuntime::init`) as
//! `format!("mac-{:08x}", ...)`. The `"mac-"` prefix is **hardcoded** there —
//! it does not reflect the actual host platform, so a Linux or Windows node
//! still reports a `mac-`-prefixed id. This is a pre-existing observation only;
//! the value is intentionally left unchanged here. The cryptographic identity
//! in this module (the fingerprint above) is the durable per-device handle and
//! is independent of that display-only prefix.
//!
//! ## 中文
//!
//! 本模組是 spectyn-mesh 裝置加密身份（device identity）的唯一真實來源，負責：
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
// every other crypto consumer in spectyn-mesh shares).
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
// `spectyn-mesh.v1.<purpose>` — purpose（用途）列舉於 `KeyPurpose` enum。
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
    /// ISO-8601 of master creation (from `~/.spectyn-mesh/identity.key` mtime).
    pub created_at: String,
}

// ─── §6.2 / §9.3 InitOutcome — `spectyn keys init` CLI / Tauri result ───────

/// Result of `spectyn keys init` (CLI) or `invoke('identity_init')` (Tauri).
/// Idempotent: `created == false` when an existing identity was found and the
/// caller did not pass `--force`.
///
/// 中文: `spectyn keys init` 的結果結構。`created=false` 代表已存在身份且
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
/// info-string `spectyn-mesh.v1.<slug>` consumed by exactly one downstream
/// subsystem (see SPEC-12 §7.2 mapping table).
///
/// 中文: HKDF 子金鑰用途列舉。每個 variant 對應一個固定的 info-string，給一個
/// 下游子系統使用 — 嚴禁兩個 consumer 共用同一個 purpose。
///
/// **Reserved prefix**: `spectyn-mesh.v1.*` — `v2` is reserved for future
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
    /// `spectyn keys backup` wrap key (32 B, per backup).
    BackupWrap,
}

impl KeyPurpose {
    /// Lower-kebab slug used inside the HKDF info-string.
    /// `EventEncrypt` → `"event-encrypt"`, etc.
    ///
    /// 中文: 回傳 lower-kebab purpose slug，組成 `spectyn-mesh.v1.<slug>`。
    pub const fn slug(self) -> &'static str {
        match self {
            KeyPurpose::EventEncrypt => "event-encrypt",
            KeyPurpose::ClusterHmac => "cluster-hmac",
            KeyPurpose::BrokerJwtSign => "broker-jwt-sign",
            KeyPurpose::SkillSyncMac => "skill-sync-mac",
            KeyPurpose::BackupWrap => "backup-wrap",
        }
    }

    /// Full HKDF info-string: `"spectyn-mesh.v1.<slug>"`.
    /// Stable across versions — bumping the `v1` prefix requires a master
    /// rotation migration (see §7.5).
    pub fn info_string(self) -> String {
        format!("spectyn-mesh.v1.{}", self.slug())
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
    /// access-group=`group.ai.spectynmesh.app`).
    IosKeychain,
    /// Android EncryptedSharedPreferences backed by AndroidKeyStore.
    AndroidEncryptedSharedPreferences,
    /// Windows Credential Manager + DPAPI per-user wrap.
    WindowsCredentialManager,
    /// Linux Secret Service (`org.freedesktop.secrets`, default collection).
    LinuxSecretService,
    /// File fallback `~/.spectyn-mesh/identity.key` mode 0600 (desktop only).
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

// ─── §7.4 BackupArtifact — `spectyn keys backup` output ─────────────────────

/// Output of `spectyn keys backup --to <path>` — base64 of master seed +
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
/// `spectyn keys init --force` after explicit user confirmation).
///
/// 中文: `spectyn keys init` 主邏輯。`force=true` 會覆寫既有身份 — 配合 CLI
/// 端 `--force` 旗標 + 使用者二次確認。
pub fn build_init_outcome(_force: bool) -> Result<InitOutcome, KeyDerivationError> {
    // Step 1: probe the default backend for an existing `identity-master`
    //         record. When present and `_force == false`, short-circuit with
    //         `created=false` so CLI prints "已存在的身份 <fp> — 略過". Stage
    //         3 picks the backend per OS via the same dispatch table used by
    //         `write_to_keystore`.
    let default_backend = default_backend_for_os();
    let existing: Option<Vec<u8>> =
        match read_from_default_backend_with_migration(default_backend, "identity-master") {
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
/// info-string is `spectyn-mesh.v1.<purpose.slug()>` per §7.2.
/// Returns `KeyDerivationError::MasterNotFound` if `init` has not been called.
///
/// 中文: HKDF-SHA256 子金鑰派生器。同一 master + 同一 purpose 永遠回同一結果。
pub fn derive_subkey(_purpose: KeyPurpose) -> Result<[u8; 32], KeyDerivationError> {
    // Step 1: load the master seed from the OS-default keystore. Propagate
    //         `MasterNotFound` straight back so the caller can prompt
    //         `spectyn keys init` per the §11 error catalog UX rules.
    let default_backend = default_backend_for_os();
    let master_seed: Vec<u8> =
        read_from_default_backend_with_migration(default_backend, "identity-master")?;

    // Step 2: HKDF-SHA256(extract → expand). `info` is the stable
    //         `spectyn-mesh.v1.<slug>` string from `KeyPurpose::info_string()`.
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
        //         so it picks the access-group `group.ai.spectynmesh.app`.
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

/// Clear the local identity master seed for `spectyn logout`.
///
/// Unlike [`delete_from_keystore`] (the §6.3 destroy-my-key flow that requires
/// a fingerprint confirmation so a user can't nuke their key by accident), this
/// is the session-clearing path: logout drops the stored identity unconditionally
/// and idempotently. It deletes the `identity-master` record from the OS-default
/// keystore and returns `Ok(())` when no record exists — there is no
/// confirmation prompt because logout is an expected, reversible action
/// (`spectyn login` / `spectyn keys init` re-establish identity).
///
/// 中文: `spectyn logout` 用的清除路徑。和 [`delete_from_keystore`]（需指紋二次
/// 確認的「永久砍 key」流程）不同，這裡是登出時無條件、幂等地刪掉 keystore 裡的
/// `identity-master` 記錄；不存在時回 `Ok(())`，因為登出本來就可逆。
pub fn logout_clear_keystore() -> Result<(), KeyDerivationError> {
    let backend = default_backend_for_os();
    // Dispatch straight to the per-backend delete, which is idempotent on a
    // missing record (absent key = already gone = success). We deliberately
    // skip the fingerprint gate used by `delete_from_keystore`.
    match backend {
        KeystoreBackend::MacosKeychain | KeystoreBackend::IosKeychain => {
            keychain_delete_pseudo("identity-master")
        }
        KeystoreBackend::AndroidEncryptedSharedPreferences => {
            android_ks_delete_pseudo("identity-master")
        }
        KeystoreBackend::WindowsCredentialManager => dpapi_delete_pseudo("identity-master"),
        KeystoreBackend::LinuxSecretService => libsecret_delete_pseudo("identity-master"),
        KeystoreBackend::FileChmod0600 => file_chmod0600_delete_pseudo("identity-master"),
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

/// Environment override (LIN-KS-1): when set to `file` (case-insensitive),
/// [`default_backend_for_os`] returns [`KeystoreBackend::FileChmod0600`] on
/// every platform, forcing the headless-friendly plaintext-file backend
/// instead of the OS-native secret store. `auto` (or unset) keeps the normal
/// per-OS dispatch. Any other value is ignored (warn-once) and falls through
/// to the OS default.
///
/// 中文: keystore 後端的環境變數覆寫。`SPECTYN_KEYSTORE=file` 會在所有平台強制
/// 使用 `~/.spectyn-mesh/<account>.key` 純檔案後端（無需 D-Bus / OS keystore，
/// 適合 CI / headless）；`auto` 或未設定則走正常的各 OS 預設。其他值會被忽略
/// （只警告一次）並退回 OS 預設。
const SPECTYN_KEYSTORE_ENV: &str = "SPECTYN_KEYSTORE";

/// Emit a one-shot `warn!` for an unrecognised `SPECTYN_KEYSTORE` value so a
/// typo (e.g. `SPECTYN_KEYSTORE=fil`) is visible without spamming the log on
/// every backend lookup. Backed by a `std::sync::Once` (the crate-wide
/// warn-once idiom) keyed to this single message.
fn warn_unrecognized_keystore_override(raw: &str) {
    use std::sync::Once;
    static WARN_ONCE: Once = Once::new();
    WARN_ONCE.call_once(|| {
        tracing::warn!(
            value = %raw,
            "ignoring unrecognized {SPECTYN_KEYSTORE_ENV}={raw:?}; \
             expected `file` or `auto` (unset) — falling back to the OS default keystore"
        );
    });
}

/// Resolve the explicit [`SPECTYN_KEYSTORE_ENV`] override, if any.
///
/// Returns `Some(FileChmod0600)` for `file` (case-insensitive, trimmed) so a
/// user / CI can force the plaintext-file backend regardless of OS. `auto` (or
/// unset / blank) returns `None` so the caller uses the per-OS default. Any
/// other value warns once and returns `None` (OS default). Kept separate from
/// [`default_backend_for_os`] so backend selection stays unit-testable without
/// mutating process-global env around the cfg!-driven dispatch.
fn keystore_override_from_env() -> Option<KeystoreBackend> {
    match std::env::var(SPECTYN_KEYSTORE_ENV) {
        Ok(raw) => {
            let trimmed = raw.trim();
            if trimmed.eq_ignore_ascii_case("file") {
                Some(KeystoreBackend::FileChmod0600)
            } else if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("auto") {
                None
            } else {
                warn_unrecognized_keystore_override(trimmed);
                None
            }
        }
        // Unset (or non-UTF-8) → no override.
        Err(_) => None,
    }
}

/// 5-OS dispatch — pick the canonical native keystore backend for the host.
/// `FileChmod0600` is only chosen as fallback when no OS arm matches (no
/// other target_os we ship to currently lands here, but the fall-through
/// keeps the function total).
///
/// **Override (LIN-KS-1)**: the [`SPECTYN_KEYSTORE_ENV`] env var takes
/// precedence over the per-OS dispatch — `SPECTYN_KEYSTORE=file` forces
/// [`KeystoreBackend::FileChmod0600`] on every platform (headless / CI escape
/// hatch). See [`keystore_override_from_env`]. This is purely additive: it
/// does NOT change the missing-D-Bus → `KeystoreUnavailable` behaviour of the
/// `LinuxSecretService` arm, which callers reach via `write_to_keystore`
/// directly, not through this function.
fn default_backend_for_os() -> KeystoreBackend {
    if let Some(backend) = keystore_override_from_env() {
        return backend;
    }
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

fn read_from_default_backend_with_migration(
    backend: KeystoreBackend,
    account: &str,
) -> Result<Vec<u8>, KeyDerivationError> {
    match read_from_keystore(backend, account) {
        Ok(seed) => Ok(seed),
        Err(KeyDerivationError::MasterNotFound)
            if backend == KeystoreBackend::AndroidEncryptedSharedPreferences =>
        {
            migrate_plaintext_master_seed_to_android(account)
        }
        Err(e) => Err(e),
    }
}

#[cfg(target_os = "android")]
fn migrate_plaintext_master_seed_to_android(account: &str) -> Result<Vec<u8>, KeyDerivationError> {
    let seed = file_chmod0600_read_pseudo(account)?;
    if seed.len() != 32 {
        return Err(KeyDerivationError::LegacyFormatSeed { got: seed.len() });
    }
    android_ks_write_pseudo(account, &seed)?;
    file_chmod0600_delete_pseudo(account)?;
    Ok(seed)
}

#[cfg(not(target_os = "android"))]
fn migrate_plaintext_master_seed_to_android(_account: &str) -> Result<Vec<u8>, KeyDerivationError> {
    Err(KeyDerivationError::MasterNotFound)
}

// --- macOS / iOS Keychain (`security-framework = "3"`, Apple-only dep) ---
//
// All three helpers are `#[cfg(any(target_os = "macos", target_os = "ios"))]`-
// gated for the real Keychain Services path; non-Apple builds get a small shim
// that returns `KeystoreUnavailable` so the dispatch `match` in
// `write_to_keystore` / `read_from_keystore` / `delete_from_keystore` still
// type-checks on Linux / Windows. The non-Apple branch is unreachable in
// practice because `default_backend_for_os` only returns
// `MacosKeychain` / `IosKeychain` on the matching `cfg!(target_os = …)`.
//
// Schema: every record is a generic-password Keychain item keyed by
// `(service, account)`. The service is the constant `KEYCHAIN_SERVICE`
// (`"spectyn-mesh"`); the account is the `account` string the caller passes
// (e.g. `"identity-master"`). The secret payload is the raw 32-byte master
// seed. `set_generic_password` upserts (creates or replaces in place) so a
// re-`init` overwrites cleanly instead of erroring on a duplicate item — this
// mirrors the Linux `replace_if_exists = true` behaviour above. The macOS
// login Keychain is the default item store, so the seed lands in the user's
// login keychain instead of a plaintext file on disk.

/// Generic-password service name shared by every spectyn-mesh Keychain item.
/// The `(service, account)` pair is the item's primary key, so this constant
/// plus the per-record `account` uniquely identify the master seed.
#[cfg(any(target_os = "macos", target_os = "ios"))]
const KEYCHAIN_SERVICE: &str = "spectyn-mesh";

/// `errSecItemNotFound` (`-25300`) — the `OSStatus` the Keychain returns when a
/// `(service, account)` item does not exist. Inlined as a named constant so we
/// don't have to take a direct dependency on the `security-framework-sys` crate
/// just for one status code; `security_framework::base::Error::code()` returns
/// the raw `OSStatus` we compare against here.
#[cfg(any(target_os = "macos", target_os = "ios"))]
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn keychain_write_pseudo(
    account: &str,
    secret: &[u8],
) -> Result<(), KeyDerivationError> {
    // `set_generic_password` upserts: it adds the item, or updates the secret
    // in place if a `(service, account)` item already exists. That makes a
    // re-`init` idempotent without a separate "delete then add" dance.
    security_framework::passwords::set_generic_password(KEYCHAIN_SERVICE, account, secret)
        .map_err(|e| KeyDerivationError::KeystoreUnavailable { detail: e.to_string() })
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn keychain_read_pseudo(account: &str) -> Result<Vec<u8>, KeyDerivationError> {
    match security_framework::passwords::get_generic_password(KEYCHAIN_SERVICE, account) {
        Ok(secret) => Ok(secret),
        // Absent item is not a hard error — map it to `MasterNotFound` so the
        // caller can branch into the init flow, exactly like the file / Linux
        // arms surface a missing record.
        Err(e) if e.code() == ERR_SEC_ITEM_NOT_FOUND => Err(KeyDerivationError::MasterNotFound),
        Err(e) => Err(KeyDerivationError::KeystoreUnavailable { detail: e.to_string() }),
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn keychain_delete_pseudo(account: &str) -> Result<(), KeyDerivationError> {
    match security_framework::passwords::delete_generic_password(KEYCHAIN_SERVICE, account) {
        Ok(()) => Ok(()),
        // Idempotent: no item = already gone = success (matches file / Linux arms).
        Err(e) if e.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(()),
        Err(e) => Err(KeyDerivationError::KeystoreUnavailable { detail: e.to_string() }),
    }
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
fn keychain_write_pseudo(
    _account: &str,
    _secret: &[u8],
) -> Result<(), KeyDerivationError> {
    // Unreachable in practice: `default_backend_for_os` only picks
    // `MacosKeychain` / `IosKeychain` on Apple targets. Direct callers (tests,
    // manual dispatch) get a typed error instead of `unimplemented!()` so the
    // binary still links on Linux / Windows hosts.
    Err(KeyDerivationError::KeystoreUnavailable {
        detail: "macOS/iOS Keychain backend is Apple-only (security-framework crate)".to_string(),
    })
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
fn keychain_read_pseudo(_account: &str) -> Result<Vec<u8>, KeyDerivationError> {
    Err(KeyDerivationError::KeystoreUnavailable {
        detail: "macOS/iOS Keychain backend is Apple-only (security-framework crate)".to_string(),
    })
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
fn keychain_delete_pseudo(_account: &str) -> Result<(), KeyDerivationError> {
    Err(KeyDerivationError::KeystoreUnavailable {
        detail: "macOS/iOS Keychain backend is Apple-only (security-framework crate)".to_string(),
    })
}

// --- Android EncryptedSharedPreferences (jni) ---

#[cfg(target_os = "android")]
const ANDROID_IDENTITY_BRIDGE_CLASS: &str = "ai/spectynmesh/app/IdentityKeystore";
#[cfg(target_os = "android")]
const ANDROID_IDENTITY_WRITE_METHOD: &str = "write";
#[cfg(target_os = "android")]
const ANDROID_IDENTITY_READ_METHOD: &str = "read";
#[cfg(target_os = "android")]
const ANDROID_IDENTITY_DELETE_METHOD: &str = "delete";
#[cfg(target_os = "android")]
const ANDROID_SIG_WRITE: &str = "(Ljava/lang/String;Ljava/lang/String;)V";
#[cfg(target_os = "android")]
const ANDROID_SIG_READ: &str = "(Ljava/lang/String;)Ljava/lang/String;";
#[cfg(target_os = "android")]
const ANDROID_SIG_DELETE: &str = "(Ljava/lang/String;)V";

#[cfg(target_os = "android")]
static ANDROID_JVM: std::sync::OnceLock<usize> = std::sync::OnceLock::new();

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "system" fn JNI_OnLoad(
    vm: *mut jni::sys::JavaVM,
    _reserved: *mut std::ffi::c_void,
) -> jni::sys::jint {
    let _ = ANDROID_JVM.set(vm as usize);
    jni::sys::JNI_VERSION_1_6
}

#[cfg(target_os = "android")]
fn android_keystore_unavailable(detail: impl Into<String>) -> KeyDerivationError {
    KeyDerivationError::KeystoreUnavailable {
        detail: format!("Android identity keystore JNI bridge unavailable: {}", detail.into()),
    }
}

#[cfg(target_os = "android")]
fn android_with_env<T>(
    op: &str,
    f: impl FnOnce(&mut jni::JNIEnv<'_>) -> jni::errors::Result<T>,
) -> Result<T, KeyDerivationError> {
    use jni::JavaVM;

    let vm_ptr = ANDROID_JVM
        .get()
        .copied()
        .ok_or_else(|| android_keystore_unavailable("JNI_OnLoad has not registered JavaVM"))?;
    let vm = unsafe { JavaVM::from_raw(vm_ptr as *mut jni::sys::JavaVM) }
        .map_err(|e| android_keystore_unavailable(e.to_string()))?;
    let mut env = vm
        .attach_current_thread()
        .map_err(|e| android_keystore_unavailable(e.to_string()))?;
    let result = env.with_local_frame(16, |env| f(env));
    let result = match result {
        Ok(result) => result,
        Err(e) => {
            if env.exception_check().unwrap_or(false) {
                let _ = env.exception_describe();
                let _ = env.exception_clear();
            }
            return Err(android_keystore_unavailable(format!("{op}: {e}")));
        }
    };
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_describe();
        let _ = env.exception_clear();
        return Err(android_keystore_unavailable(format!("{op}: Java exception")));
    }
    Ok(result)
}

#[cfg(target_os = "android")]
fn android_ks_write_pseudo(
    account: &str,
    secret: &[u8],
) -> Result<(), KeyDerivationError> {
    use base64::Engine;

    let encoded = base64::engine::general_purpose::STANDARD.encode(secret);
    android_with_env("write Android encrypted identity seed", |env| {
        let account = env.new_string(account)?;
        let encoded = env.new_string(&encoded)?;
        let class = env.find_class(ANDROID_IDENTITY_BRIDGE_CLASS)?;
        env.call_static_method(
            class,
            ANDROID_IDENTITY_WRITE_METHOD,
            ANDROID_SIG_WRITE,
            &[(&account).into(), (&encoded).into()],
        )?;
        Ok(())
    })
}

#[cfg(target_os = "android")]
fn android_ks_read_pseudo(account: &str) -> Result<Vec<u8>, KeyDerivationError> {
    use base64::Engine;
    use jni::objects::JString;

    let encoded = android_with_env("read Android encrypted identity seed", |env| {
        let account = env.new_string(account)?;
        let class = env.find_class(ANDROID_IDENTITY_BRIDGE_CLASS)?;
        let value = env
            .call_static_method(
                class,
                ANDROID_IDENTITY_READ_METHOD,
                ANDROID_SIG_READ,
                &[(&account).into()],
            )?
            .l()?;
        if value.is_null() {
            return Ok(None);
        }
        let value = JString::from(value);
        let value: String = env.get_string(&value)?.into();
        Ok(Some(value))
    })?;
    let encoded = encoded.ok_or(KeyDerivationError::MasterNotFound)?;
    base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|e| KeyDerivationError::KeystoreUnavailable {
            detail: format!("Android identity keystore stored invalid base64: {e}"),
        })
}

#[cfg(target_os = "android")]
fn android_ks_delete_pseudo(account: &str) -> Result<(), KeyDerivationError> {
    android_with_env("delete Android encrypted identity seed", |env| {
        let account = env.new_string(account)?;
        let class = env.find_class(ANDROID_IDENTITY_BRIDGE_CLASS)?;
        env.call_static_method(
            class,
            ANDROID_IDENTITY_DELETE_METHOD,
            ANDROID_SIG_DELETE,
            &[(&account).into()],
        )?;
        Ok(())
    })
}

#[cfg(not(target_os = "android"))]
fn android_ks_write_pseudo(
    _account: &str,
    _secret: &[u8],
) -> Result<(), KeyDerivationError> {
    Err(KeyDerivationError::KeystoreUnavailable {
        detail: "Android EncryptedSharedPreferences backend is Android-only (jni bridge)"
            .to_string(),
    })
}

#[cfg(not(target_os = "android"))]
fn android_ks_read_pseudo(_account: &str) -> Result<Vec<u8>, KeyDerivationError> {
    Err(KeyDerivationError::KeystoreUnavailable {
        detail: "Android EncryptedSharedPreferences backend is Android-only (jni bridge)"
            .to_string(),
    })
}

#[cfg(not(target_os = "android"))]
fn android_ks_delete_pseudo(_account: &str) -> Result<(), KeyDerivationError> {
    Err(KeyDerivationError::KeystoreUnavailable {
        detail: "Android EncryptedSharedPreferences backend is Android-only (jni bridge)"
            .to_string(),
    })
}

// --- Windows Credential Manager + DPAPI (windows-rs) ---
//
// All three `dpapi_*_pseudo` helpers are `#[cfg(target_os = "windows")]`-gated
// for the real DPAPI + Credential Manager path; non-Windows builds get a small
// shim that returns `KeystoreUnavailable` so the dispatch `match` in
// `write_to_keystore` / `read_from_keystore` / `delete_from_keystore` still
// type-checks on macOS / Linux (mirrors the Apple-only Keychain arm above). The
// non-Windows branch is unreachable in practice because `default_backend_for_os`
// only returns `WindowsCredentialManager` when `cfg!(target_os = "windows")`.
//
// Schema: every record is a `CRED_TYPE_GENERIC` Credential Manager entry keyed
// by the caller's `account` string (the credential `TargetName`). The secret
// payload is the raw seed bytes wrapped per-user with DPAPI
// (`CryptProtectData` / `CryptUnprotectData`, `CRYPTPROTECT_UI_FORBIDDEN` so it
// never prompts), so the seed is never written to the credential store in the
// clear. `CredWriteW` upserts (creates or replaces in place) so a re-`init`
// overwrites cleanly, matching the macOS `set_generic_password` and Linux
// `replace_if_exists = true` behaviour.

/// UTF-16, NUL-terminated wide string for a credential `TargetName` /
/// `UserName`. `CredWriteW` / `CredReadW` / `CredDeleteW` all take wide strings.
#[cfg(target_os = "windows")]
fn dpapi_target_name(account: &str) -> Vec<u16> {
    account.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Map a `windows::core::Error` into a `KeystoreUnavailable` with context, so a
/// DPAPI / CredMan failure surfaces as the same typed keystore error every other
/// backend uses (instead of panicking).
#[cfg(target_os = "windows")]
fn dpapi_error_detail(context: &str, err: windows::core::Error) -> KeyDerivationError {
    KeyDerivationError::KeystoreUnavailable {
        detail: format!("{context}: {err}"),
    }
}

/// `true` when the `windows::core::Error` is `ERROR_NOT_FOUND` (the credential
/// does not exist), so the read / delete arms can map it to `MasterNotFound` /
/// idempotent-success rather than a hard failure.
#[cfg(target_os = "windows")]
fn dpapi_is_not_found(err: &windows::core::Error) -> bool {
    const HRESULT_FROM_WIN32_ERROR_NOT_FOUND: i32 = 0x80070490u32 as i32;
    err.code().0 == HRESULT_FROM_WIN32_ERROR_NOT_FOUND
}

/// DPAPI per-user wrap. `CRYPTPROTECT_UI_FORBIDDEN` keeps it headless (never
/// prompts). Frees the kernel-allocated output blob with `LocalFree` after
/// copying it into an owned `Vec`.
#[cfg(target_os = "windows")]
fn dpapi_protect(secret: &[u8]) -> Result<Vec<u8>, KeyDerivationError> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let input = CRYPT_INTEGER_BLOB {
        cbData: secret.len() as u32,
        pbData: secret.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();

    unsafe {
        CryptProtectData(
            &input,
            PCWSTR::null(),
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
        .map_err(|e| dpapi_error_detail("CryptProtectData failed", e))?;

        let protected = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        let _ = LocalFree(HLOCAL(output.pbData.cast()));
        Ok(protected)
    }
}

/// DPAPI per-user unwrap (inverse of [`dpapi_protect`]). Frees the
/// kernel-allocated output blob with `LocalFree` after copying it out.
#[cfg(target_os = "windows")]
fn dpapi_unprotect(protected: &[u8]) -> Result<Vec<u8>, KeyDerivationError> {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Cryptography::{
        CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let input = CRYPT_INTEGER_BLOB {
        cbData: protected.len() as u32,
        pbData: protected.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();

    unsafe {
        CryptUnprotectData(
            &input,
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
        .map_err(|e| dpapi_error_detail("CryptUnprotectData failed", e))?;

        let secret = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        let _ = LocalFree(HLOCAL(output.pbData.cast()));
        Ok(secret)
    }
}

// --- At-rest wrap for the CLI's on-disk identity files (W3) ---
//
// The CLI persists two raw secrets as files: the 64-byte root IKM
// (`identity.key`) and the 32-byte ed25519 signing seed (`keys/ed25519.priv`).
// On Unix those rely on `mode 0600`; on Windows there is no chmod, so pre-W3
// they sat in `%APPDATA%` protected only by NTFS ACL — recoverable by a raw
// file copy. These helpers bring the Windows CLI in line with the app's
// keystore by wrapping the bytes per-user with DPAPI before they touch disk.
//
// Format: `DPAPI_AT_REST_MAGIC (8 bytes) || CryptProtectData(secret)`. The
// magic lets the readers distinguish a wrapped file from a legacy plaintext
// seed, so the upgrade is backward-compatible (old files still load) and there
// is no forced migration that could brick an existing identity. The helpers are
// defined on ALL targets (the DPAPI work is gated INTERNALLY by
// `#[cfg(target_os = "windows")]` blocks): on Windows they DPAPI-wrap; on unix /
// other targets `unprotect_at_rest` is a no-op `Ok(None)` and `protect_at_rest`
// passes the bytes through unchanged. Defining them unconditionally lets the two
// readers call them with NO statement-level `#[cfg]` — which a save-time
// formatter strips, which would otherwise break the unix build.

/// Prefix marking a DPAPI-wrapped at-rest secret. 8 bytes so a legacy 64-byte
/// random IKM colliding with it is ~2^-64 (and no legacy file carries it).
pub(crate) const DPAPI_AT_REST_MAGIC: &[u8] = b"PMDPAPI1";

/// Wrap `secret` for at-rest storage in a CLI identity file. On Windows this is
/// `MAGIC || DPAPI(secret)`; if DPAPI is unavailable it logs and falls back to
/// the plaintext bytes so the user is never blocked from having an identity. On
/// unix / other targets it is an identity passthrough. `allow(dead_code)`: only
/// the non-unix `write_*_secure` paths call it, so it is unused on unix.
#[allow(dead_code)]
pub(crate) fn protect_at_rest(secret: &[u8]) -> Vec<u8> {
    #[cfg(target_os = "windows")]
    {
        match dpapi_protect(secret) {
            Ok(blob) => {
                let mut out = Vec::with_capacity(DPAPI_AT_REST_MAGIC.len() + blob.len());
                out.extend_from_slice(DPAPI_AT_REST_MAGIC);
                out.extend_from_slice(&blob);
                return out;
            }
            Err(e) => {
                tracing::warn!(
                    "DPAPI at-rest wrap unavailable ({e}); writing identity seed as plaintext"
                );
            }
        }
    }
    secret.to_vec()
}

/// Inverse of [`protect_at_rest`]. Returns a module-neutral `std::io::Error` so
/// both callers (which have different `KeyDerivationError` enums) can convert it:
///   * `Ok(None)`       — no magic → legacy plaintext; caller uses the raw bytes.
///   * `Ok(Some(seed))` — magic present and unwrapped cleanly.
///   * `Err(..)`        — magic present but DPAPI unwrap failed (corrupt / wrong
///                        user). Callers MUST surface this, never fall back to
///                        the wrapped bytes, or they would derive a wrong key.
pub(crate) fn unprotect_at_rest(blob: &[u8]) -> std::io::Result<Option<Vec<u8>>> {
    if blob.len() < DPAPI_AT_REST_MAGIC.len() || &blob[..DPAPI_AT_REST_MAGIC.len()] != DPAPI_AT_REST_MAGIC {
        return Ok(None);
    }
    #[cfg(target_os = "windows")]
    {
        dpapi_unprotect(&blob[DPAPI_AT_REST_MAGIC.len()..])
            .map(Some)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
    }
    #[cfg(not(target_os = "windows"))]
    {
        // Non-Windows non-Unix never writes the magic, so seeing it here means a
        // file moved off Windows — there is no DPAPI to unwrap it.
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "DPAPI-wrapped identity seed cannot be unwrapped off Windows",
        ))
    }
}

#[cfg(target_os = "windows")]
fn dpapi_write_pseudo(account: &str, secret: &[u8]) -> Result<(), KeyDerivationError> {
    use windows::core::PWSTR;
    use windows::Win32::Security::Credentials::{
        CredWriteW, CREDENTIALW, CRED_FLAGS, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
    };

    let protected = dpapi_protect(secret)?;
    let mut target_name = dpapi_target_name(account);
    let mut user_name = dpapi_target_name("spectyn-mesh");

    let credential = CREDENTIALW {
        Flags: CRED_FLAGS(0),
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(target_name.as_mut_ptr()),
        CredentialBlobSize: protected.len() as u32,
        CredentialBlob: protected.as_ptr() as *mut u8,
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        UserName: PWSTR(user_name.as_mut_ptr()),
        ..Default::default()
    };

    unsafe { CredWriteW(&credential, 0).map_err(|e| dpapi_error_detail("CredWriteW failed", e)) }
}

#[cfg(target_os = "windows")]
fn dpapi_read_pseudo(account: &str) -> Result<Vec<u8>, KeyDerivationError> {
    use windows::core::PCWSTR;
    use windows::Win32::Security::Credentials::{
        CredFree, CredReadW, CREDENTIALW, CRED_TYPE_GENERIC,
    };

    let target_name = dpapi_target_name(account);
    let mut credential: *mut CREDENTIALW = std::ptr::null_mut();

    unsafe {
        match CredReadW(PCWSTR(target_name.as_ptr()), CRED_TYPE_GENERIC, 0, &mut credential) {
            Ok(()) => {
                let cred = &*credential;
                let protected =
                    std::slice::from_raw_parts(cred.CredentialBlob, cred.CredentialBlobSize as usize)
                        .to_vec();
                CredFree(credential.cast());
                dpapi_unprotect(&protected)
            }
            Err(e) if dpapi_is_not_found(&e) => Err(KeyDerivationError::MasterNotFound),
            Err(e) => Err(dpapi_error_detail("CredReadW failed", e)),
        }
    }
}

#[cfg(target_os = "windows")]
fn dpapi_delete_pseudo(account: &str) -> Result<(), KeyDerivationError> {
    use windows::core::PCWSTR;
    use windows::Win32::Security::Credentials::{CredDeleteW, CRED_TYPE_GENERIC};

    let target_name = dpapi_target_name(account);
    unsafe {
        match CredDeleteW(PCWSTR(target_name.as_ptr()), CRED_TYPE_GENERIC, 0) {
            Ok(()) => Ok(()),
            Err(e) if dpapi_is_not_found(&e) => Ok(()),
            Err(e) => Err(dpapi_error_detail("CredDeleteW failed", e)),
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn dpapi_write_pseudo(
    _account: &str,
    _secret: &[u8],
) -> Result<(), KeyDerivationError> {
    // Unreachable in practice: `default_backend_for_os` only picks
    // `WindowsCredentialManager` on Windows. Direct callers (tests, manual
    // dispatch) get a typed error instead of `unimplemented!()` so the binary
    // still links on macOS / Linux hosts (mirrors the Apple-only Keychain arm).
    Err(KeyDerivationError::KeystoreUnavailable {
        detail: "Windows Credential Manager / DPAPI backend is Windows-only (windows crate)"
            .to_string(),
    })
}

#[cfg(not(target_os = "windows"))]
fn dpapi_read_pseudo(_account: &str) -> Result<Vec<u8>, KeyDerivationError> {
    Err(KeyDerivationError::KeystoreUnavailable {
        detail: "Windows Credential Manager / DPAPI backend is Windows-only (windows crate)"
            .to_string(),
    })
}

#[cfg(not(target_os = "windows"))]
fn dpapi_delete_pseudo(_account: &str) -> Result<(), KeyDerivationError> {
    Err(KeyDerivationError::KeystoreUnavailable {
        detail: "Windows Credential Manager / DPAPI backend is Windows-only (windows crate)"
            .to_string(),
    })
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
// `{ "application": "spectyn-mesh", "account": <account> }`. The label is the
// account string itself (shown verbatim in GNOME Seahorse / KDE Wallet UIs).
// The secret payload is the raw 32-byte master seed; we pin
// `content_type = "application/octet-stream"` so the keystore doesn't try to
// interpret it as text. `replace_if_exists = true` so re-init overwrites
// cleanly instead of stacking duplicate items.

#[cfg(target_os = "linux")]
fn libsecret_attributes(account: &str) -> std::collections::HashMap<&str, &str> {
    let mut attrs = std::collections::HashMap::new();
    attrs.insert("application", "spectyn-mesh");
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
/// `<home>/.spectyn-mesh/<account>.key`. The parent directory is created
/// (mode 0o700 on unix) if it does not exist so callers don't have to
/// pre-create `~/.spectyn-mesh/` themselves.
fn file_chmod0600_path(account: &str) -> Result<std::path::PathBuf, KeyDerivationError> {
    let dir = crate::cli_config::spectyn_data_dir().map_err(|_| {
        KeyDerivationError::Io("home_dir unavailable for FileChmod0600 backend".to_string())
    })?;
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

/// Write `secret` to `~/.spectyn-mesh/<account>.key` atomically: write to a
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

/// Read `~/.spectyn-mesh/<account>.key` back as raw bytes. NotFound maps to
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

/// Delete `~/.spectyn-mesh/<account>.key`. Idempotent — NotFound is silently
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

    // W3: at-rest wrap for the CLI identity files.
    #[test]
    fn at_rest_legacy_plaintext_is_passed_through() {
        // A pre-W3 file is raw seed bytes with no magic prefix. unprotect must
        // report `None` so the reader uses the bytes verbatim — i.e. existing
        // identities never brick on upgrade.
        let legacy = [0x42u8; 64];
        assert!(
            !legacy.starts_with(DPAPI_AT_REST_MAGIC),
            "test fixture must not collide with the magic"
        );
        assert_eq!(
            unprotect_at_rest(&legacy).unwrap(),
            None,
            "no magic => treat as legacy plaintext"
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn at_rest_dpapi_round_trips() {
        // protect → file bytes carry the magic and are NOT the plaintext;
        // unprotect recovers the exact seed.
        let seed = [0x37u8; 64];
        let wrapped = protect_at_rest(&seed);
        assert!(wrapped.starts_with(DPAPI_AT_REST_MAGIC), "wrapped blob is tagged");
        assert_ne!(&wrapped[..], &seed[..], "seed must not appear in the clear");
        let back = unprotect_at_rest(&wrapped).unwrap();
        assert_eq!(back.as_deref(), Some(&seed[..]), "round-trip restores the seed");
    }

    #[test]
    fn key_purpose_info_string_is_stable() {
        // §7.2 reserved prefix invariant: all v1 purposes use
        // `spectyn-mesh.v1.<slug>`. Any change to this string is a wire-break.
        assert_eq!(
            KeyPurpose::EventEncrypt.info_string(),
            "spectyn-mesh.v1.event-encrypt"
        );
        assert_eq!(
            KeyPurpose::ClusterHmac.info_string(),
            "spectyn-mesh.v1.cluster-hmac"
        );
        assert_eq!(
            KeyPurpose::BrokerJwtSign.info_string(),
            "spectyn-mesh.v1.broker-jwt-sign"
        );
        assert_eq!(
            KeyPurpose::SkillSyncMac.info_string(),
            "spectyn-mesh.v1.skill-sync-mac"
        );
        assert_eq!(
            KeyPurpose::BackupWrap.info_string(),
            "spectyn-mesh.v1.backup-wrap"
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
    /// identity material in `~/.spectyn-mesh/`.
    #[test]
    fn file_chmod0600_round_trip() {
        let account =
            format!("spectyn-test-{}-{}", std::process::id(), uuid::Uuid::new_v4());
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

    // ─── SPECTYN_KEYSTORE env override (LIN-KS-1) ────────────────────────────
    //
    // `SPECTYN_KEYSTORE` is process-global, so these tests serialise their env
    // mutation through a shared mutex and ALWAYS restore the prior value before
    // returning (even on the assertion path) so they never race or leak the
    // override into sibling tests in the same binary. We test the pure
    // selection seam `keystore_override_from_env()` (no cfg! OS dispatch) plus
    // one end-to-end check that `default_backend_for_os()` honours `file`, then
    // a headless write→read round-trip through the public keystore API.

    use std::sync::Mutex;
    /// Serialises every test that mutates the process-global `SPECTYN_KEYSTORE`.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// RAII guard: snapshots `SPECTYN_KEYSTORE` on construction and restores it
    /// on drop, so an override never leaks past the test that set it — even if
    /// an assertion panics mid-test.
    struct KeystoreEnvGuard {
        prev: Option<String>,
    }
    impl KeystoreEnvGuard {
        fn set(value: Option<&str>) -> Self {
            let prev = std::env::var(SPECTYN_KEYSTORE_ENV).ok();
            match value {
                Some(v) => std::env::set_var(SPECTYN_KEYSTORE_ENV, v),
                None => std::env::remove_var(SPECTYN_KEYSTORE_ENV),
            }
            KeystoreEnvGuard { prev }
        }
    }
    impl Drop for KeystoreEnvGuard {
        fn drop(&mut self) {
            match &self.prev {
                Some(v) => std::env::set_var(SPECTYN_KEYSTORE_ENV, v),
                None => std::env::remove_var(SPECTYN_KEYSTORE_ENV),
            }
        }
    }

    #[test]
    fn keystore_override_file_forces_file_backend() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        // `file` → FileChmod0600 regardless of host OS.
        let _g = KeystoreEnvGuard::set(Some("file"));
        assert_eq!(
            keystore_override_from_env(),
            Some(KeystoreBackend::FileChmod0600),
            "SPECTYN_KEYSTORE=file must select FileChmod0600"
        );
        // And the public selection function must honour it on every platform.
        assert_eq!(
            default_backend_for_os(),
            KeystoreBackend::FileChmod0600,
            "default_backend_for_os must honour SPECTYN_KEYSTORE=file on this OS"
        );
    }

    #[test]
    fn keystore_override_is_case_insensitive_and_trims() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        for v in ["FILE", "File", "  file  ", "fIlE"] {
            let _g = KeystoreEnvGuard::set(Some(v));
            assert_eq!(
                keystore_override_from_env(),
                Some(KeystoreBackend::FileChmod0600),
                "SPECTYN_KEYSTORE={v:?} must select FileChmod0600 (case/space-insensitive)"
            );
        }
    }

    #[test]
    fn keystore_override_auto_and_unset_use_os_default() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        // Explicit `auto` → no override (fall through to OS default).
        {
            let _g = KeystoreEnvGuard::set(Some("auto"));
            assert_eq!(keystore_override_from_env(), None, "`auto` must not override");
        }
        // Blank value → no override.
        {
            let _g = KeystoreEnvGuard::set(Some("   "));
            assert_eq!(keystore_override_from_env(), None, "blank must not override");
        }
        // Unset → no override.
        {
            let _g = KeystoreEnvGuard::set(None);
            assert_eq!(keystore_override_from_env(), None, "unset must not override");
        }
    }

    #[test]
    fn keystore_override_unrecognized_falls_through_to_os_default() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // A typo must NOT silently force a backend — it falls through to the OS
        // default (and warns once, which we don't assert on here to avoid
        // coupling to the logging subscriber).
        let _g = KeystoreEnvGuard::set(Some("fil"));
        assert_eq!(
            keystore_override_from_env(),
            None,
            "unrecognized SPECTYN_KEYSTORE value must fall through to OS default"
        );
    }

    /// End-to-end headless proof: with `SPECTYN_KEYSTORE=file`, the selected
    /// backend is `FileChmod0600` AND a write→read round-trip through the
    /// public `write_to_keystore` / `read_from_keystore(FileChmod0600, ...)`
    /// API works with no D-Bus / OS keystore. Uses a throwaway account name
    /// (pid + uuid) so it never touches the real `identity-master` record in
    /// `~/.spectyn-mesh/`, and cleans up after itself.
    #[test]
    fn keystore_override_file_round_trip_headless() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = KeystoreEnvGuard::set(Some("file"));

        // Selection honours the override.
        let backend = default_backend_for_os();
        assert_eq!(backend, KeystoreBackend::FileChmod0600);

        // Round-trip through the PUBLIC keystore API on the selected backend —
        // no D-Bus needed, so this passes on a headless CI box.
        let account =
            format!("spectyn-test-ksenv-{}-{}", std::process::id(), uuid::Uuid::new_v4());
        let secret = b"lin-ks-1-env-override-roundtrip-secret";

        write_to_keystore(backend, &account, secret)
            .expect("file backend write must succeed headless");
        let got = read_from_keystore(backend, &account)
            .expect("file backend read must succeed headless");
        assert_eq!(got, secret, "round-tripped secret must match");

        // Clean up the throwaway record. `delete_from_keystore` requires a
        // fingerprint confirm over a valid 32-byte seed, which our test secret
        // is not, so we use the per-backend file delete helper directly.
        file_chmod0600_delete_pseudo(&account)
            .expect("cleanup delete must succeed");
        match read_from_keystore(backend, &account) {
            Err(KeyDerivationError::MasterNotFound) => {} // cleaned up
            other => panic!("record should be gone after cleanup, got {other:?}"),
        }
    }

    // ─── macOS / iOS Keychain round-trip KAT (Stage 4 — security-framework) ──
    //
    // The Stage 4 marker test that asserted `keychain_write_pseudo` still
    // panics with `"Stage 4"` was removed when the `security-framework = "3"`
    // dependency landed and the three `keychain_*` arms were wired to the real
    // Keychain Services API. It is replaced by the live round-trip below.
    //
    // This test exercises the real macOS login Keychain end-to-end:
    // store → load → assert equal → delete → load-returns-`MasterNotFound`.
    // Marked Apple-only via `cfg`; on a headless / no-keychain environment the
    // Keychain API can surface an auth / entitlement error (mapped to
    // `KeystoreUnavailable`) instead of succeeding — in that case the test is
    // skipped with a logged note rather than failing, so it stays honest about
    // sandboxed CI while still proving the round trip on a real login session.
    //
    // A unique throwaway `account` (suffixed with the process id + a uuid)
    // keeps the test from ever touching the real `"identity-master"` record,
    // and the final delete cleans up regardless of outcome.

    #[cfg(any(target_os = "macos", target_os = "ios"))]
    #[test]
    fn keychain_round_trip_apple_only() {
        let account = format!(
            "spectyn-test-keychain-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        );
        // A known 32-byte key (the master-seed length), distinct from all-zero.
        let secret: [u8; 32] = {
            let mut s = [0u8; 32];
            for (i, b) in s.iter_mut().enumerate() {
                *b = (i as u8).wrapping_mul(7).wrapping_add(3);
            }
            s
        };

        // Write. If the Keychain is unavailable in this environment (headless
        // CI / no login session / entitlement denied), skip honestly rather
        // than fake a pass.
        match keychain_write_pseudo(&account, &secret) {
            Ok(()) => {}
            Err(KeyDerivationError::KeystoreUnavailable { detail }) => {
                eprintln!(
                    "skipping keychain_round_trip_apple_only — Keychain unavailable \
                     in this environment: {detail}"
                );
                return;
            }
            Err(other) => panic!("unexpected keychain write error: {other:?}"),
        }

        // Read back — must equal what we stored.
        let got = keychain_read_pseudo(&account)
            .expect("keychain read must return the bytes just written");
        assert_eq!(got, secret, "round-tripped secret must match");

        // Delete.
        keychain_delete_pseudo(&account).expect("keychain delete must succeed");

        // Read after delete must be MasterNotFound (absent, not a hard error).
        match keychain_read_pseudo(&account) {
            Err(KeyDerivationError::MasterNotFound) => {} // expected
            other => panic!(
                "keychain read after delete must be MasterNotFound, got {:?}",
                other
            ),
        }

        // Delete again must be idempotent (no-op when no item matches).
        keychain_delete_pseudo(&account)
            .expect("keychain delete must be idempotent on missing");
    }

    // ─── `spectyn logout` keystore clear (A3) ────────────────────────────────
    //
    // Proves the exact mechanism `logout_clear_keystore` uses on macOS/iOS:
    // a stored identity record is removed and subsequent reads return
    // `MasterNotFound`. We write/delete a THROWAWAY account (suffixed with pid
    // + uuid) so the test never touches the real `"identity-master"` record,
    // mirroring `keychain_round_trip_apple_only`. On a headless / sandboxed
    // environment where the Keychain is unavailable, we skip honestly instead
    // of faking a pass.
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    #[test]
    fn logout_clears_keystore_identity_apple_only() {
        let account = format!(
            "spectyn-test-logout-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        );
        let secret = [0x5au8; 32];

        // Seed a throwaway "identity" into the real login Keychain.
        match keychain_write_pseudo(&account, &secret) {
            Ok(()) => {}
            Err(KeyDerivationError::KeystoreUnavailable { detail }) => {
                eprintln!(
                    "skipping logout_clears_keystore_identity_apple_only — \
                     Keychain unavailable in this environment: {detail}"
                );
                return;
            }
            Err(other) => panic!("unexpected keychain write error: {other:?}"),
        }
        // Sanity: it's really there before we clear.
        assert_eq!(
            keychain_read_pseudo(&account).expect("seeded identity must be readable"),
            secret
        );

        // The clearing mechanism logout uses (idempotent per-backend delete).
        keychain_delete_pseudo(&account)
            .expect("logout keystore clear must succeed");

        // After clearing, the identity must be gone (load → not-found).
        match keychain_read_pseudo(&account) {
            Err(KeyDerivationError::MasterNotFound) => {} // expected: cleared
            other => panic!(
                "identity must be gone after logout clear, got {:?}",
                other
            ),
        }

        // Clearing again is a no-op (absent record = already logged out).
        keychain_delete_pseudo(&account)
            .expect("logout keystore clear must be idempotent on an already-empty record");
    }

    // `logout_clear_keystore` itself targets the real `"identity-master"`
    // record on the OS-default backend, so we deliberately do NOT invoke it in
    // a unit test — on a dev macOS box that would delete the user's actual
    // identity. Instead we prove the same clear-then-not-found contract on the
    // SAFE file-fallback backend via a THROWAWAY account, exercising the exact
    // per-backend delete `logout_clear_keystore` dispatches to. This never
    // touches the real keystore record.
    #[test]
    fn logout_clear_mechanism_file_fallback() {
        let account = format!(
            "spectyn-test-logout-file-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        );
        let secret = [0xa5u8; 32];

        // Seed a throwaway identity into the file fallback store.
        file_chmod0600_write_pseudo(&account, &secret)
            .expect("file fallback write must succeed");
        assert_eq!(
            file_chmod0600_read_pseudo(&account).expect("seeded identity must be readable"),
            secret
        );

        // Clear it the way logout does (per-backend delete, idempotent).
        file_chmod0600_delete_pseudo(&account)
            .expect("logout clear must succeed");

        // Identity must be gone after clearing (load → not-found).
        match file_chmod0600_read_pseudo(&account) {
            Err(KeyDerivationError::MasterNotFound) => {} // expected: cleared
            other => panic!("identity must be gone after logout clear, got {:?}", other),
        }

        // Idempotent: clearing an already-empty record is a no-op success.
        file_chmod0600_delete_pseudo(&account)
            .expect("logout clear must be idempotent on an already-empty record");

        // Smoke: the public logout helper exists, links, and has the expected
        // signature (a function pointer reference forces the compiler to check
        // it without invoking it against the real `identity-master` record).
        let _f: fn() -> Result<(), KeyDerivationError> = logout_clear_keystore;
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
    //       -p spectyn_core identity_wire::tests::libsecret_round_trip \
    //       --release -- --ignored --nocapture
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore]
    fn libsecret_round_trip_linux_only() {
        let account =
            format!("spectyn-test-libsecret-{}-{}", std::process::id(), uuid::Uuid::new_v4());
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
