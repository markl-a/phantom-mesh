// SPEC-29 §7 + §9 + §19 — Release pipeline wire types (single source of truth
// for the 5-platform release pipeline: build → sign → notarize → verify →
// upload → publish to GitHub Releases + TestFlight + Play Internal Track,
// plus rollback contract with manual approval gate).
//
// Stage 1 (spec → interface): types + ts-rs exports + `unimplemented!()` stub
// helpers only. Stage 2 implements the actual signing / notarize wait loop /
// rollback orchestration logic per SPEC-29 §6.3 + §6.4 sequence diagrams.
//
// 中文: 本檔對應 SPEC-29 §7 (ReleaseManifest + Tauri subset) + §9 API contracts
// (rollback workflow_dispatch inputs + Tauri updater endpoint) + §19 rollback
// contract (G6 manual approval gate, 2 reviewer typed confirmation,
// confirm_bad_tag must equal rollback_to or fail-fast)。
//
// Stage-1 scope: 只定義 wire-shape 型別 + stub 函式；業務邏輯 (codesign /
// signtool / xcrun notarytool / apksigner / gpg / gh release / xcrun altool /
// r0adkll-upload-google-play) 全 Stage 2。
//
// Relationship to existing release infra:
//   - `.github/workflows/release-*.yml` 既有 9 個 workflow 是 CI 側落地。
//   - `scripts/build-update-manifest.py` 既有 147 行已會組 latest.json。
//   - 本檔是 Rust 側「型別契約」— phantom CLI / Tauri app / verify-and-publish
//     job 共用同一個 ReleaseManifest serde shape。
//
// TODO Stage 2: wire into core/src/lib.rs + add chrono/url deps if not present
// (chrono already in workspace via E002; url already via reqwest deps).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

// ─── §7.1 ReleaseChannel ─────────────────────────────────────────────────────
//
// SPEC-29 §7.1 lists three channels (stable / beta / nightly). SPEC-62 (待寫)
// will own channel switching + staged rollout; this enum is the wire shape
// shared by both. snake_case serde matches Tauri updater convention
// (`"channel": "stable"` in latest.json).
//
// 中文: 三條 channel 軌道：穩定 / 測試 / 每夜。end user 可在 Settings 切換 (見
// SPEC-29 §10.1 Settings Channel Picker)；nightly 需 operator 手動觸發
// (SPEC-29 NG3，避免燒 Apple notarize quota)。
//
// Why re-defined here instead of re-exported from SPEC-62: SPEC-62 file does
// not yet exist (per SPEC-29 §0 "Blocks" — SPEC-29 ships first); Stage 2 of
// SPEC-62 should re-export *this* enum to avoid double-definition drift.

/// Release channel (stable / beta / nightly).
///
/// snake_case serde: matches Tauri updater `latest.json` `"channel"` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/release_pipeline/")]
#[serde(rename_all = "snake_case")]
pub enum ReleaseChannel {
    /// Default end-user channel. Auto-update on by default. Promoted from beta
    /// after ≥ 7 days no critical issue (SPEC-62 graduation rule, TBD).
    Stable,
    /// Early-tester channel. TestFlight + Play Internal Track audiences map
    /// here. Auto-update on by default.
    Beta,
    /// Cutting-edge channel. Manual trigger only per SPEC-29 NG3 (Apple
    /// notarize quota ~75/day per Apple ID). Auto-update opt-in only.
    Nightly,
}

// ─── §7.1 ArtifactOs + ArtifactArch ─────────────────────────────────────────
//
// SPEC-29 §7.1 `platforms` map keys like `"darwin-aarch64"`, `"windows-x86_64"`
// are flat-string compositions of (os, arch). Wire types model this as two
// separate enums for type-safety; serde uses PascalCase to keep the JSON
// readable and the TS union ergonomic.
//
// 中文: 5 OS × 4 arch 矩陣的 type-safe 拆解。Stage 2 拼 `"darwin-aarch64"`
// 字串時呼叫 `format!("{}-{}", os.slug(), arch.slug())`，但 wire 層保留 enum
// 形式給 type checker 抓錯。

/// Target operating system for a release artifact.
///
/// Five OS per SPEC-29 BIG-GOAL anchor "one Rust codebase → 5 platforms".
/// `Ios` and `Android` ship via store channels (TestFlight + Play Internal
/// Track), not GitHub Releases asset download.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/release_pipeline/")]
pub enum ArtifactOs {
    Macos,
    Windows,
    Linux,
    Ios,
    Android,
}

/// Target CPU architecture for a release artifact.
///
/// `Universal2` is the macOS fat binary (x86_64 + aarch64 in one .dmg).
/// `Arm64v8` and `Armv7` are Android ABI naming; `Aarch64` is the Apple /
/// Linux convention. Stage 2 normalises naming when building the platform key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/release_pipeline/")]
pub enum ArtifactArch {
    X86_64,
    Aarch64,
    /// macOS fat binary (x86_64 + aarch64). Tauri bundler supports via
    /// `--target universal-apple-darwin`.
    Universal2,
    /// Android arm64-v8a ABI string.
    Arm64v8,
    /// Android armeabi-v7a ABI string (32-bit ARM).
    Armv7,
    /// Legacy 32-bit x86 (Windows installer fallback).
    X86,
}

// ─── §7.1 ReleaseArtifact (per-platform asset descriptor) ───────────────────
//
// One row per (os, arch) combination. `signature_url` is optional because not
// every platform ships a detached signature file (macOS notarize ticket is
// stapled into the .dmg itself; Windows signtool embeds; only Linux GPG +
// Tauri updater Ed25519 emit separate .asc / .sig files).
//
// 中文: 每筆 (os, arch) 對應一筆 ReleaseArtifact。簽章檔 URL 可選 — macOS 用
// stapled ticket、Windows 用 embedded signature、Linux 用 .asc 旁註、Tauri
// updater 用 .sig 旁註。

/// Single release artifact (one OS × arch combination).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/release_pipeline/")]
#[serde(rename_all = "camelCase")]
pub struct ReleaseArtifact {
    pub os: ArtifactOs,
    pub arch: ArtifactArch,
    /// Bare file name (no path), e.g. `"phantom-mesh-0.6.0-rc1-darwin-aarch64.dmg"`.
    pub file_name: String,
    /// Lower-case hex `sha256sum` of the artifact bytes. Verified by Tauri
    /// updater before swap-and-restart per SPEC-29 §10.1 verifying state.
    pub sha256_hex: String,
    pub size_bytes: u64,
    /// Detached signature URL (Linux .asc, Tauri updater .sig). `None` for
    /// platforms that embed signature in the artifact (macOS notarize ticket,
    /// Windows Authenticode).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature_url: Option<String>,
    /// Public CDN URL for direct download (GitHub Releases asset URL).
    pub download_url: String,
}

// ─── §7.1 ReleaseManifest (latest.json superset) ─────────────────────────────

/// Full release manifest (`latest.json` superset per SPEC-29 §7.1).
///
/// Two consumers:
///   1. SPEC-28 30-second hello install.sh — picks `daemon_binary` URL by OS.
///   2. Tauri updater plugin — reads the Tauri-subset projection
///      (`latest-tauri.json`) emitted separately by verify-and-publish job.
///
/// 中文: 完整 release 清單。verify-and-publish job 組成兩個檔輸出：
/// 完整版 (本 struct serialise) 給 install.sh + CLI 升級用；Tauri-subset
/// 給 Tauri updater plugin 用。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/release_pipeline/")]
#[serde(rename_all = "camelCase")]
pub struct ReleaseManifest {
    /// Semantic version string, no `"v"` prefix (e.g. `"0.6.0-rc1"`).
    pub version: String,
    /// 40-char hex git commit SHA the artifacts were built from. Used by
    /// reproducible-build verification (SPEC-29 G4 daemon-only).
    pub git_sha: String,
    pub channel: ReleaseChannel,
    /// Wall-clock publish time as UTC milliseconds since epoch. Mirrors
    /// SPEC-29 §7.1 `pub_date` ISO-8601 string but kept as `u64` here for
    /// byte-cheap diff / sort.
    pub published_at_ms: u64,
    pub artifacts: Vec<ReleaseArtifact>,
    /// `true` if this manifest is the `latest` pointer for its channel; the
    /// `verify-and-publish` job flips this and re-uploads `latest.json`
    /// (SPEC-29 §6.3 step "gh release edit --draft=false").
    pub latest_for_channel: bool,
}

// ─── §19 RollbackRequest / RollbackPlan / RollbackStep / RollbackAction ──────
//
// SPEC-29 §3.1 G6 + §6.4 sequence: rollback is destructive on shared infra
// (GitHub Releases CDN), so the wire layer enforces three guards:
//   (a) `confirm_bad_tag` must equal the current bad tag (typo-guard).
//   (b) `RollbackPlan.required_reviewers` default 2 (matches GitHub Actions
//       `environment: production-rollback` policy).
//   (c) `RollbackStep.idempotent` lets a re-run pick up where it left off
//       (delete-then-retry is safe; upload-then-retry needs `--clobber`).
//
// 中文: rollback 三道閘 — confirm_bad_tag typo 守衛、2 reviewer 強制簽核、
// step idempotent flag。任何一道沒過、execute_rollback 直接 Err 不執行。

/// Operator-issued rollback request (carries the typo-guard input).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/release_pipeline/")]
#[serde(rename_all = "camelCase")]
pub struct RollbackRequest {
    /// Semver of the version to roll back **to** (the known-good previous
    /// release), e.g. `"0.5.9"`. No `"v"` prefix.
    pub rollback_to_version: String,
    /// Operator must re-type the bad tag here (e.g. `"v0.6.0-rc1"`). Must
    /// equal the current bad tag exactly, else `build_rollback_plan` returns
    /// `RollbackConfirmMismatch` per SPEC-29 §9.1 guard step.
    pub confirm_bad_tag: String,
    /// GitHub login of the operator issuing the rollback (audit trail).
    pub requested_by: String,
}

/// Executable rollback plan produced from a `RollbackRequest` after guard
/// checks pass. Walked by `execute_rollback` only after `approvals >= 2`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/release_pipeline/")]
#[serde(rename_all = "camelCase")]
pub struct RollbackPlan {
    /// Tag being rolled back (e.g. `"v0.6.0-rc1"`).
    pub bad_tag: String,
    /// Tag being rolled forward to (e.g. `"v0.5.9"`).
    pub target_tag: String,
    /// Always `"production-rollback"` — matches GitHub Actions environment
    /// name per SPEC-29 §9.1.
    pub environment: String,
    /// Minimum reviewer count (default 2). `execute_rollback` rejects with
    /// `ReviewerApprovalMissing` if actual approvals fall below this.
    pub required_reviewers: u8,
    pub planned_steps: Vec<RollbackStep>,
}

/// One step in a rollback plan. Steps run in `order` ascending.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/release_pipeline/")]
#[serde(rename_all = "camelCase")]
pub struct RollbackStep {
    /// Execution order (1-based). Stage 2 sorts before walking.
    pub order: u8,
    pub action: RollbackAction,
    /// Target identifier the action operates on (tag, URL, build number, ...).
    /// Free-form string; semantics depend on `action` variant.
    pub target: String,
    /// `true` if Stage 2 may re-run this step on failure without observable
    /// side-effect divergence (e.g. delete-then-retry — already deleted is OK).
    pub idempotent: bool,
}

/// Discrete action types a rollback step may take.
///
/// Naming mirrors SPEC-29 §3.1 G6 + §6.4 step labels:
///   - `DeleteGhRelease` → `gh release delete <bad-tag> --cleanup-tag`
///   - `RestoreLatestJson` → re-fetch the previous `latest.json` payload
///   - `UploadLatestJson` → `gh release upload <target> latest.json --clobber`
///   - `TestflightExpire` → App Store Connect API expire-build
///   - `PlayConsoleExpire` → Play Console API halt-rollout
///   - `VerifyEndpoint` → `GET /releases/latest/download/latest.json` smoke
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/release_pipeline/")]
pub enum RollbackAction {
    DeleteGhRelease,
    RestoreLatestJson,
    UploadLatestJson,
    TestflightExpire,
    PlayConsoleExpire,
    VerifyEndpoint,
}

// ─── §7 + §12 ReleaseEvidence (audit bundle attached to each release) ───────
//
// Stage 2 verify-and-publish writes one ReleaseEvidence per published tag and
// attaches to GitHub Release Notes. Auditors / SOC2 reviewers / SBOM consumers
// pull this to confirm "yes this binary was signed and notarized by the real
// operator pipeline, not someone's laptop".
//
// 中文: 每次 publish 都附一份 ReleaseEvidence 在 Release Notes — 證明 binary
// 是真實 pipeline 出貨的，非個人機器手簽。verify-and-publish job 寫入。

/// Audit evidence bundle attached to each published release.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/release_pipeline/")]
#[serde(rename_all = "camelCase")]
pub struct ReleaseEvidence {
    pub manifest: ReleaseManifest,
    /// Captured stdout lines from `scripts/release/verify-all.sh` (codesign +
    /// signtool + gpg + apksigner). Trimmed to last 200 lines per
    /// platform to keep Release Notes attachment under GitHub's 64 KB limit.
    pub verify_logs: Vec<String>,
    /// Apple notarytool final status string (`"Accepted"` / `"Invalid"`).
    /// `None` if macOS not part of this release matrix (debug build).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notarize_status: Option<String>,
    /// `true` iff all 5 platforms (macOS / Windows / Linux / iOS / Android)
    /// passed `verify-all.sh`. Maps to SPEC-29 G2 acceptance gate.
    pub tested_5os: bool,
}

// ─── §11 ReleaseError ────────────────────────────────────────────────────────
//
// SPEC-29 §0 template deviation row lists 5 `R.release.*` codes + §11 inherits
// SPEC-04 base error set. This enum collapses them into Stage-1 wire shape;
// Stage 2 may grow variants as new failure modes surface (e.g. EV cert HSM
// timeout). Each variant carries a human-readable message; bilingual UI
// strings live in SPEC-05 i18n layer.
//
// 中文: 5 個 release 專屬錯誤碼 + rollback 守衛 + reviewer 不夠 + updater
// endpoint 掛掉。對齊 SPEC-04 R.release.* 命名。

/// Error catalog for the release pipeline layer.
#[derive(Debug, Clone, Serialize, Deserialize, TS, thiserror::Error)]
#[ts(export, export_to = "../../app/src/lib/generated/release_pipeline/")]
#[serde(rename_all = "camelCase", tag = "code", content = "message")]
pub enum ReleaseError {
    /// `R.release.notarize-timeout` — Apple notarytool exceeded 6 × 5 min
    /// retries (SPEC-29 §6.3 macOS notarize wait loop).
    #[error("notarize timeout: {0}")]
    NotarizationTimeout(String),
    /// `R.release.verify-fail` — codesign / signtool / gpg / apksigner
    /// reported a mismatch. Triggers SPEC-29 G2 "整 release fail" gate.
    #[error("verify failed: {0}")]
    VerifyFailed(String),
    /// `R.release.sign-fail` — signing key / cert missing (e.g. Apple ASC
    /// API key secret not loaded, Android keystore absent, EV cert HSM
    /// disconnected).
    #[error("signing key missing: {0}")]
    SignKeyMissing(String),
    /// `R.release.rollback-confirm-mismatch` — `confirm_bad_tag` did not
    /// equal current bad tag. Typo-guard per SPEC-29 §9.1.
    #[error("rollback confirm mismatch: {0}")]
    RollbackConfirmMismatch(String),
    /// `R.release.reviewer-approval-missing` — fewer than
    /// `required_reviewers` approved. Maps to GitHub Actions
    /// `environment: production-rollback` policy enforcement.
    #[error("reviewer approval missing: {0}")]
    ReviewerApprovalMissing(String),
    /// `R.release.updater-endpoint-down` — `GET latest.json` smoke test
    /// returned non-200 or invalid JSON (SPEC-29 G5).
    #[error("updater endpoint down: {0}")]
    UpdaterEndpointDown(String),
}

// ─── Stub functions (Stage 2 implements; Stage 1 leaves `unimplemented!()`) ──

/// Build a fresh `ReleaseManifest` from a version + git sha + channel.
///
/// Stage 2: shell out to `gh release view <tag> --json assets`, fetch each
/// asset's sha256 from the upload metadata, compose `Vec<ReleaseArtifact>`,
/// set `latest_for_channel = true` once verify-and-publish flips the pointer.
///
/// 中文: Stage 2 從 gh release view 拉 assets 元資料、組 ReleaseManifest。
pub fn build_release_manifest(
    version: &str,
    git_sha: &str,
    channel: ReleaseChannel,
) -> Result<ReleaseManifest, ReleaseError> {
    // Step 1: glob 5-OS artifacts from `dist/` directory.
    let artifact_paths = glob_artifacts_pseudo("dist/")?;

    // Step 2: for each path, compute sha256 + size, detect adjacent signature.
    let mut artifacts: Vec<ReleaseArtifact> = Vec::with_capacity(artifact_paths.len());
    for path in &artifact_paths {
        let (sha256_hex, size_bytes) = sha256_file_pseudo(path)?;
        // Step 3: detect signature_url adjacent (e.g. `<file>.asc` / `<file>.sig`).
        let signature_url = detect_adjacent_signature_pseudo(path)?;
        let (os, arch) = parse_os_arch_from_filename_pseudo(path)?;
        artifacts.push(ReleaseArtifact {
            os,
            arch,
            file_name: path.clone(),
            sha256_hex,
            size_bytes,
            signature_url,
            download_url: format!(
                "https://github.com/owner/repo/releases/download/v{}/{}",
                version, path
            ),
        });
    }

    // Step 4: assemble ReleaseManifest with current published_at_ms (unix-ms now).
    let published_at_ms = now_unix_ms_pseudo()?;
    Ok(ReleaseManifest {
        version: version.to_string(),
        git_sha: git_sha.to_string(),
        channel,
        published_at_ms,
        artifacts,
        latest_for_channel: false,
    })
}

/// Stage 3: enumerate release artifacts in `dir` by extension.
///
/// Returns sorted paths of files whose extension is one of dmg / msi /
/// AppImage / apk / ipa. A missing directory is not an error — returns an
/// empty vec so `build_release_manifest` degrades gracefully when `dist/`
/// has not been populated yet.
fn glob_artifacts_pseudo(dir: &str) -> Result<Vec<String>, ReleaseError> {
    const EXTS: [&str; 5] = ["dmg", "msi", "AppImage", "apk", "ipa"];
    let read_dir = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        // Missing dir is benign (Stage 1 contract): empty result, no error.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => {
            return Err(ReleaseError::VerifyFailed(format!(
                "read_dir '{}' failed: {}",
                dir, e
            )))
        }
    };
    let mut out: Vec<String> = Vec::new();
    for entry in read_dir {
        let entry = entry
            .map_err(|e| ReleaseError::VerifyFailed(format!("read_dir entry in '{}': {}", dir, e)))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            // Case-INSENSITIVE extension match: real-world artifacts vary in case
            // (e.g. `.AppImage` / `.APPIMAGE`, `.DMG`). Flagged by the multi-agent
            // review (nvidia) — harden the spec list against case drift.
            if EXTS.iter().any(|x| x.eq_ignore_ascii_case(ext)) {
                if let Some(p) = path.to_str() {
                    out.push(p.to_string());
                }
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Stage 3: compute lowercase hex sha256 + byte size of a file.
///
/// Streams the whole file into a `Sha256` hasher. IO errors map to
/// `VerifyFailed` (the closest general failure variant in this layer).
fn sha256_file_pseudo(path: &str) -> Result<(String, u64), ReleaseError> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path)
        .map_err(|e| ReleaseError::VerifyFailed(format!("read '{}' failed: {}", path, e)))?;
    let size_bytes = bytes.len() as u64;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let digest = hasher.finalize();
    let hex: String = digest.iter().map(|b| format!("{:02x}", b)).collect();
    Ok((hex, size_bytes))
}

/// Stage 3: probe for an adjacent detached signature file.
///
/// Prefers `<path>.asc` (Linux GPG), falls back to `<path>.sig` (Tauri
/// updater Ed25519). Returns `None` when neither exists.
fn detect_adjacent_signature_pseudo(path: &str) -> Result<Option<String>, ReleaseError> {
    let asc = format!("{}.asc", path);
    if std::path::Path::new(&asc).exists() {
        return Ok(Some(asc));
    }
    let sig = format!("{}.sig", path);
    if std::path::Path::new(&sig).exists() {
        return Ok(Some(sig));
    }
    Ok(None)
}

/// Stage 3: parse `(ArtifactOs, ArtifactArch)` from an artifact filename.
///
/// Case-insensitive token scan. OS: macos/darwin, windows/win, linux,
/// android, ios. ARCH: x86_64/amd64, aarch64/arm64, universal. Unrecognised
/// os or arch returns `VerifyFailed`.
fn parse_os_arch_from_filename_pseudo(
    path: &str,
) -> Result<(ArtifactOs, ArtifactArch), ReleaseError> {
    let lower = path.to_ascii_lowercase();

    let os = if lower.contains("macos") || lower.contains("darwin") {
        ArtifactOs::Macos
    } else if lower.contains("windows") || lower.contains("win") {
        ArtifactOs::Windows
    } else if lower.contains("linux") {
        ArtifactOs::Linux
    } else if lower.contains("android") {
        ArtifactOs::Android
    } else if lower.contains("ios") {
        ArtifactOs::Ios
    } else {
        return Err(ReleaseError::VerifyFailed(format!(
            "unrecognized OS token in filename '{}'",
            path
        )));
    };

    // Order matters: check the most specific tokens first.
    let arch = if lower.contains("x86_64") || lower.contains("amd64") {
        ArtifactArch::X86_64
    } else if lower.contains("aarch64") || lower.contains("arm64") {
        ArtifactArch::Aarch64
    } else if lower.contains("universal") {
        ArtifactArch::Universal2
    } else {
        return Err(ReleaseError::VerifyFailed(format!(
            "unrecognized arch token in filename '{}'",
            path
        )));
    };

    Ok((os, arch))
}

/// Stage 3: current UNIX time in milliseconds.
///
/// A clock set before the epoch (`SystemTimeError`) maps to `VerifyFailed`.
fn now_unix_ms_pseudo() -> Result<u64, ReleaseError> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| ReleaseError::VerifyFailed(format!("system clock before epoch: {}", e)))?;
    Ok(dur.as_millis() as u64)
}

/// Verify all 5-platform artifacts are correctly signed.
///
/// Stage 2: spawn per-OS verifier — `codesign --verify --strict` (macOS),
/// `signtool verify /pa` (Windows), `gpg --verify` (Linux .asc), `apksigner
/// verify --verbose` (Android), `xcrun stapler validate` (iOS). Any platform
/// failure triggers `ReleaseError::VerifyFailed` and SPEC-29 G2 release-fail
/// gate. macOS additionally runs `spctl --assess` to confirm notarize ticket.
///
/// 中文: 跨平台 verify 集中器；任一失敗整 release fail，不 publish。
pub fn verify_5os_signed(manifest: &ReleaseManifest) -> Result<(), ReleaseError> {
    // Step 1: iterate artifacts; dispatch by ArtifactOs.
    for artifact in &manifest.artifacts {
        match artifact.os {
            // Step 2: macOS — `codesign --verify --strict <file>`.
            ArtifactOs::Macos => {
                proc_spawn_pseudo("codesign", &["--verify", "--strict", &artifact.file_name])?;
            }
            // Step 3: Windows — `signtool verify /pa <file>`.
            ArtifactOs::Windows => {
                proc_spawn_pseudo("signtool", &["verify", "/pa", &artifact.file_name])?;
            }
            // Step 4: Linux — `gpg --verify <file>.asc <file>`.
            ArtifactOs::Linux => {
                let asc = format!("{}.asc", artifact.file_name);
                proc_spawn_pseudo("gpg", &["--verify", &asc, &artifact.file_name])?;
            }
            // Step 5: Android — `apksigner verify --verbose <file>`.
            ArtifactOs::Android => {
                proc_spawn_pseudo("apksigner", &["verify", "--verbose", &artifact.file_name])?;
            }
            // Step 6: iOS — notarize status check (altool / notarytool).
            ArtifactOs::Ios => {
                notarize_check_pseudo(&artifact.file_name)?;
            }
        }
    }
    // Step 7: any failure above propagates as VerifyFailed via `?`.
    Ok(())
}

fn proc_spawn_pseudo(_cmd: &str, _args: &[&str]) -> Result<(), ReleaseError> {
    unimplemented!(
        "Stage 3: std::process::Command — spawn + wait; non-zero exit → VerifyFailed(stderr)"
    )
}

fn notarize_check_pseudo(_file: &str) -> Result<(), ReleaseError> {
    unimplemented!(
        "Stage 3: altool or notarytool — `xcrun notarytool info <uuid>` parse Accepted/Invalid"
    )
}

/// Plan a rollback after verifying the typo-guard.
///
/// Enforces SPEC-29 §9.1 guard: `req.confirm_bad_tag` MUST equal `bad_tag`,
/// else returns `RollbackConfirmMismatch` (no plan generated; nothing
/// destructive happens). On success, emits a 6-step plan walking through
/// `DeleteGhRelease → RestoreLatestJson → UploadLatestJson →
/// TestflightExpire → PlayConsoleExpire → VerifyEndpoint`.
///
/// 中文: typo-guard 守衛；輸入錯就 Err，連 plan 都不生成；過了才回 6-step
/// rollback plan、等 2 reviewer 批准後 execute_rollback 才會走。
pub fn build_rollback_plan(
    req: &RollbackRequest,
    bad_tag: &str,
) -> Result<RollbackPlan, ReleaseError> {
    // Step 1: CRITICAL guard per SPEC-29 §6.4 audit fix — typo-guard must pass
    // BEFORE any Stage 3 helper or destructive call. Pure logic, no panic.
    if req.confirm_bad_tag != bad_tag {
        return Err(ReleaseError::RollbackConfirmMismatch(format!(
            "expected '{}', got '{}'",
            bad_tag, req.confirm_bad_tag
        )));
    }

    // Step 2: build the 6-step plan in order. `target_tag` derived from
    // req.rollback_to_version (prepend "v").
    let target_tag = format!("v{}", req.rollback_to_version);
    let planned_steps = vec![
        RollbackStep {
            order: 1,
            action: RollbackAction::DeleteGhRelease,
            target: bad_tag.to_string(),
            idempotent: true,
        },
        RollbackStep {
            order: 2,
            action: RollbackAction::RestoreLatestJson,
            target: target_tag.clone(),
            idempotent: true,
        },
        RollbackStep {
            order: 3,
            action: RollbackAction::UploadLatestJson,
            target: target_tag.clone(),
            idempotent: false,
        },
        RollbackStep {
            order: 4,
            action: RollbackAction::TestflightExpire,
            target: bad_tag.to_string(),
            idempotent: true,
        },
        RollbackStep {
            order: 5,
            action: RollbackAction::PlayConsoleExpire,
            target: bad_tag.to_string(),
            idempotent: true,
        },
        RollbackStep {
            order: 6,
            action: RollbackAction::VerifyEndpoint,
            target:
                "https://github.com/owner/repo/releases/latest/download/latest-stable.json"
                    .to_string(),
            idempotent: true,
        },
    ];

    // Step 3: set environment + required_reviewers per SPEC-29 §9.1.
    Ok(RollbackPlan {
        bad_tag: bad_tag.to_string(),
        target_tag,
        environment: "production-rollback".to_string(),
        required_reviewers: 2,
        planned_steps,
    })
}

/// Execute a rollback plan after reviewer approval gate.
///
/// Requires `approvals >= plan.required_reviewers` (default 2). Fewer than
/// required → `ReviewerApprovalMissing`. After approval, walks
/// `plan.planned_steps` in `order` ascending, surfacing per-step errors via
/// the appropriate `ReleaseError` variant. Steps with `idempotent = true`
/// auto-retry once on transient failure (Stage 2 implements the retry policy).
///
/// 中文: 2 reviewer 簽核強制檢查；過了才走 step；idempotent step transient
/// fail 自動 retry 1 次；非 idempotent step fail 整 plan 中止、不繼續往下。
pub fn execute_rollback(plan: &RollbackPlan, approvals: u8) -> Result<(), ReleaseError> {
    // Step 1: CRITICAL guard — reviewer approval gate per SPEC-29 §9.1.
    // Pure logic; no Stage 3 helper called yet if this fails.
    if approvals < plan.required_reviewers {
        return Err(ReleaseError::ReviewerApprovalMissing(format!(
            "need {} approvals, got {}",
            plan.required_reviewers, approvals
        )));
    }

    // Step 2: walk planned_steps in `order` ascending; dispatch by action.
    let mut steps = plan.planned_steps.clone();
    steps.sort_by_key(|s| s.order);
    for step in &steps {
        match step.action {
            // Step 3: DeleteGhRelease → gh CLI release delete.
            RollbackAction::DeleteGhRelease => {
                gh_release_delete_pseudo(&step.target)?;
            }
            // Step 4: RestoreLatestJson → fetch prev manifest + rewrite local.
            RollbackAction::RestoreLatestJson => {
                restore_manifest_pseudo(&step.target)?;
            }
            // Step 5: UploadLatestJson → `gh release upload --clobber`.
            RollbackAction::UploadLatestJson => {
                gh_release_upload_pseudo(&step.target, "latest.json")?;
            }
            // Step 6: TestflightExpire → App Store Connect API expire-build.
            RollbackAction::TestflightExpire => {
                tf_expire_pseudo(&step.target)?;
            }
            // Step 7: PlayConsoleExpire → Play Developer API halt-rollout.
            RollbackAction::PlayConsoleExpire => {
                play_expire_pseudo(&step.target)?;
            }
            // Step 8: VerifyEndpoint → real check_updater_endpoint call
            // (already a Stage 2 function; defaults to Stable channel for
            // the smoke probe).
            RollbackAction::VerifyEndpoint => {
                let _latency_ms = check_updater_endpoint(ReleaseChannel::Stable)?;
            }
        }
    }
    Ok(())
}

fn gh_release_delete_pseudo(_tag: &str) -> Result<(), ReleaseError> {
    unimplemented!("Stage 3: octocrab or shell — `gh release delete <tag> --cleanup-tag --yes`")
}

fn restore_manifest_pseudo(_target_tag: &str) -> Result<(), ReleaseError> {
    unimplemented!(
        "Stage 3: reqwest + serde_json — fetch <target_tag>/latest.json, rewrite local pointer"
    )
}

fn gh_release_upload_pseudo(_target_tag: &str, _file: &str) -> Result<(), ReleaseError> {
    unimplemented!("Stage 3: octocrab or shell — `gh release upload <target> <file> --clobber`")
}

fn tf_expire_pseudo(_bad_tag: &str) -> Result<(), ReleaseError> {
    unimplemented!(
        "Stage 3: App Store Connect API — POST /v1/builds/[id]/relationships/betaAppReviewSubmission expire"
    )
}

fn play_expire_pseudo(_bad_tag: &str) -> Result<(), ReleaseError> {
    unimplemented!(
        "Stage 3: Play Developer API — edits.tracks.update with userFraction=0.0 halt-rollout"
    )
}

/// Smoke-test the updater endpoint for the given channel.
///
/// Stage 2: `GET https://github.com/<owner>/<repo>/releases/latest/download/
/// latest.json` (or channel-specific URL for beta / nightly), assert HTTP 200,
/// parse as `ReleaseManifest` (schema validation), return ms latency.
/// Non-200 / parse-fail → `UpdaterEndpointDown`. SPEC-29 G5 target: p50 < 100 ms.
///
/// 中文: GET updater endpoint smoke test；回 ms 延遲；非 200 / 非合法 JSON
/// 即 UpdaterEndpointDown；對齊 G5 p50 < 100ms 目標。
pub fn check_updater_endpoint(channel: ReleaseChannel) -> Result<u32, ReleaseError> {
    // Step 1: GET latest-<channel>.json with wall-clock timing.
    let channel_slug = match channel {
        ReleaseChannel::Stable => "stable",
        ReleaseChannel::Beta => "beta",
        ReleaseChannel::Nightly => "nightly",
    };
    // Base is the GitHub releases-latest download dir by default; tests (and a
    // self-hosted updater) override it via PHANTOM_UPDATER_BASE_URL so the GET is
    // redirectable to a wiremock server instead of hitting github.com.
    let base = std::env::var("PHANTOM_UPDATER_BASE_URL")
        .unwrap_or_else(|_| "https://github.com/owner/repo/releases/latest/download".to_string());
    let url = format!("{}/latest-{}.json", base.trim_end_matches('/'), channel_slug);
    let (body, latency_ms) = https_get_timing_pseudo(&url)?;

    // Step 2: on 200, parse body as ReleaseManifest (schema validation).
    let _manifest = manifest_parse_pseudo(&body)?;

    // Step 3: return latency_ms (G5 p50 < 100 ms target).
    Ok(latency_ms)
}

/// GET `url`, timing the round-trip. Non-200 or any transport error →
/// `UpdaterEndpointDown`. Bridges the sync wire to the async reqwest client via
/// the crate-wide `block_on_async` helper (same pattern as the other
/// `https_*_pseudo` wire helpers), so it works from a sync caller.
fn https_get_timing_pseudo(url: &str) -> Result<(String, u32), ReleaseError> {
    let url = url.to_string();
    crate::providers_wire::block_on_async(async move {
        let start = std::time::Instant::now();
        let resp = reqwest::Client::new()
            .get(&url)
            .send()
            .await
            .map_err(|e| ReleaseError::UpdaterEndpointDown(e.to_string()))?;
        let status = resp.status();
        // Read the body before the latency stop so a slow transfer counts too.
        let body = resp
            .text()
            .await
            .map_err(|e| ReleaseError::UpdaterEndpointDown(e.to_string()))?;
        let latency_ms = start.elapsed().as_millis() as u32;
        if !status.is_success() {
            return Err(ReleaseError::UpdaterEndpointDown(format!(
                "HTTP {}",
                status.as_u16()
            )));
        }
        Ok((body, latency_ms))
    })
}

fn manifest_parse_pseudo(body: &str) -> Result<ReleaseManifest, ReleaseError> {
    // Stage 3: parse `latest.json` body into a typed manifest. A malformed
    // payload (non-JSON / schema mismatch) is treated as an updater endpoint
    // failure per SPEC-29 G5 ("non-200 or invalid JSON").
    serde_json::from_str::<ReleaseManifest>(body)
        .map_err(|e| ReleaseError::UpdaterEndpointDown(e.to_string()))
}

// ─── Smoke tests (Stage 1 sanity only; deeper invariants in Stage 2) ─────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_manifest_round_trip_smoke() {
        let m = ReleaseManifest {
            version: "0.6.0-rc1".into(),
            git_sha: "e7cdb70deadbeefcafe1234567890abcdef01234".into(),
            channel: ReleaseChannel::Beta,
            published_at_ms: 1_780_000_000_000,
            artifacts: vec![ReleaseArtifact {
                os: ArtifactOs::Macos,
                arch: ArtifactArch::Aarch64,
                file_name: "phantom-mesh-0.6.0-rc1-darwin-aarch64.dmg".into(),
                sha256_hex: "a".repeat(64),
                size_bytes: 29_360_128,
                signature_url: None,
                download_url:
                    "https://github.com/owner/repo/releases/download/v0.6.0-rc1/phantom-mesh.dmg"
                        .into(),
            }],
            latest_for_channel: true,
        };
        let j = serde_json::to_string(&m).unwrap();
        let back: ReleaseManifest = serde_json::from_str(&j).unwrap();
        assert_eq!(m.version, back.version);
        assert_eq!(m.git_sha, back.git_sha);
        assert_eq!(m.channel, back.channel);
        assert_eq!(m.artifacts.len(), back.artifacts.len());
        assert_eq!(m.artifacts[0].file_name, back.artifacts[0].file_name);
        assert_eq!(m.artifacts[0].os, back.artifacts[0].os);
        assert_eq!(m.artifacts[0].arch, back.artifacts[0].arch);
        assert_eq!(m.latest_for_channel, back.latest_for_channel);
    }

    #[test]
    fn manifest_parse_pseudo_round_trips_valid_json() {
        // camelCase keys per `#[serde(rename_all = "camelCase")]` on the struct.
        let body = r#"{
            "version": "0.6.0-rc1",
            "gitSha": "e7cdb70deadbeefcafe1234567890abcdef01234",
            "channel": "beta",
            "publishedAtMs": 1780000000000,
            "artifacts": [],
            "latestForChannel": true
        }"#;
        let m = manifest_parse_pseudo(body).expect("valid manifest JSON should parse");
        assert_eq!(m.version, "0.6.0-rc1");
        assert_eq!(m.channel, ReleaseChannel::Beta);
        assert!(m.latest_for_channel);
        assert!(m.artifacts.is_empty());

        // Malformed JSON maps to the updater-endpoint-down variant (SPEC-29 G5).
        let err = manifest_parse_pseudo("{ not json").unwrap_err();
        assert!(matches!(err, ReleaseError::UpdaterEndpointDown(_)));
    }

    #[test]
    fn check_updater_endpoint_via_mock_handles_200_non200_and_bad_json() {
        // SPEC-29 G5: the live updater-endpoint smoke is now wired through reqwest
        // (https_get_timing_pseudo). Drive it against a wiremock server via the
        // PHANTOM_UPDATER_BASE_URL redirect. All 3 scenarios run sequentially in
        // ONE test fn so the process-global env var never races a sibling test.
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let rt = tokio::runtime::Runtime::new().unwrap();
        let manifest_json = serde_json::to_string(&ReleaseManifest {
            version: "0.6.0-rc1".into(),
            git_sha: "e7cdb70deadbeefcafe1234567890abcdef01234".into(),
            channel: ReleaseChannel::Stable,
            published_at_ms: 1_780_000_000_000,
            artifacts: vec![],
            latest_for_channel: true,
        })
        .unwrap();

        // (1) 200 + valid manifest → Ok(latency_ms).
        let ok_mock = rt.block_on(MockServer::start());
        rt.block_on(
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(200).set_body_string(manifest_json))
                .mount(&ok_mock),
        );
        std::env::set_var("PHANTOM_UPDATER_BASE_URL", ok_mock.uri());
        let ok = check_updater_endpoint(ReleaseChannel::Stable);
        assert!(matches!(ok, Ok(_)), "200 + valid manifest must be Ok: {ok:?}");

        // (2) non-200 → UpdaterEndpointDown (NOT a panic, NOT Ok).
        let down_mock = rt.block_on(MockServer::start());
        rt.block_on(
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(503))
                .mount(&down_mock),
        );
        std::env::set_var("PHANTOM_UPDATER_BASE_URL", down_mock.uri());
        let down = check_updater_endpoint(ReleaseChannel::Stable);
        assert!(
            matches!(down, Err(ReleaseError::UpdaterEndpointDown(_))),
            "503 must be UpdaterEndpointDown: {down:?}"
        );

        // (3) 200 but malformed JSON → UpdaterEndpointDown (schema-validation gate).
        let bad_mock = rt.block_on(MockServer::start());
        rt.block_on(
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(200).set_body_string("{ not json"))
                .mount(&bad_mock),
        );
        std::env::set_var("PHANTOM_UPDATER_BASE_URL", bad_mock.uri());
        let bad = check_updater_endpoint(ReleaseChannel::Stable);
        assert!(
            matches!(bad, Err(ReleaseError::UpdaterEndpointDown(_))),
            "malformed JSON must be UpdaterEndpointDown: {bad:?}"
        );

        std::env::remove_var("PHANTOM_UPDATER_BASE_URL");
    }

    #[test]
    fn release_channel_serialises_snake_case() {
        let c = ReleaseChannel::Nightly;
        let j = serde_json::to_string(&c).unwrap();
        assert_eq!(j, "\"nightly\"");
        let back: ReleaseChannel = serde_json::from_str("\"stable\"").unwrap();
        assert_eq!(back, ReleaseChannel::Stable);
    }

    #[test]
    fn release_error_serialises_with_code_tag() {
        let e = ReleaseError::NotarizationTimeout("apple notary 30min cap exceeded".into());
        let j = serde_json::to_string(&e).unwrap();
        // tag=code, content=message → `{"code":"notarizationTimeout","message":"..."}`
        assert!(j.contains("\"code\""));
        assert!(j.contains("notarizationTimeout"));
    }

    /// SPEC-29 §9.1 typo-guard, **pure logic at Stage 2**: `confirm_bad_tag`
    /// ≠ current bad tag must return `RollbackConfirmMismatch` **before** any
    /// Stage 3 helper is reached. This test no longer panics — Stage 2 now
    /// owns the guard branch; Stage 3 helpers only fire on the success path.
    #[test]
    fn build_rollback_plan_rejects_typo_confirm_pure_logic() {
        let req = RollbackRequest {
            rollback_to_version: "0.5.9".into(),
            // Operator typed v0.5.10-baf (typo of v0.5.10-bad). Stage 2 guard
            // rejects this before any `gh release delete` runs.
            confirm_bad_tag: "v0.5.10-baf".into(),
            requested_by: "operator".into(),
        };
        let result = build_rollback_plan(&req, "v0.5.10-bad");
        assert!(
            matches!(result, Err(ReleaseError::RollbackConfirmMismatch(_))),
            "expected RollbackConfirmMismatch, got {:?}",
            result
        );
    }

    /// Stage 3: the success path through `build_release_manifest` now runs the
    /// real helpers. Against a non-existent `dist/` dir, `glob_artifacts_pseudo`
    /// returns an empty vec (missing-dir is benign), so the manifest builds
    /// cleanly with zero artifacts and a real `published_at_ms` timestamp.
    #[test]
    fn build_release_manifest_empty_dist_ok() {
        let m = build_release_manifest("0.6.0-rc1", "e7cdb70", ReleaseChannel::Beta)
            .expect("should build manifest with empty dist/");
        assert_eq!(m.version, "0.6.0-rc1");
        assert!(m.artifacts.is_empty());
        assert!(m.published_at_ms > 0);
    }

    #[test]
    fn rollback_plan_round_trip_smoke() {
        let plan = RollbackPlan {
            bad_tag: "v0.6.0-rc1".into(),
            target_tag: "v0.5.9".into(),
            environment: "production-rollback".into(),
            required_reviewers: 2,
            planned_steps: vec![
                RollbackStep {
                    order: 1,
                    action: RollbackAction::DeleteGhRelease,
                    target: "v0.6.0-rc1".into(),
                    idempotent: true,
                },
                RollbackStep {
                    order: 2,
                    action: RollbackAction::RestoreLatestJson,
                    target: "v0.5.9".into(),
                    idempotent: true,
                },
                RollbackStep {
                    order: 3,
                    action: RollbackAction::UploadLatestJson,
                    target: "v0.5.9".into(),
                    idempotent: false,
                },
                RollbackStep {
                    order: 4,
                    action: RollbackAction::VerifyEndpoint,
                    target: "https://github.com/owner/repo/releases/latest/download/latest.json"
                        .into(),
                    idempotent: true,
                },
            ],
        };
        let j = serde_json::to_string(&plan).unwrap();
        let back: RollbackPlan = serde_json::from_str(&j).unwrap();
        assert_eq!(plan.bad_tag, back.bad_tag);
        assert_eq!(plan.required_reviewers, back.required_reviewers);
        assert_eq!(plan.planned_steps.len(), back.planned_steps.len());
        assert_eq!(plan.planned_steps[0].action, back.planned_steps[0].action);
    }

    #[test]
    fn release_evidence_round_trip_smoke() {
        let m = ReleaseManifest {
            version: "0.6.0-rc1".into(),
            git_sha: "e7cdb70".into(),
            channel: ReleaseChannel::Beta,
            published_at_ms: 1_780_000_000_000,
            artifacts: vec![],
            latest_for_channel: true,
        };
        let ev = ReleaseEvidence {
            manifest: m,
            verify_logs: vec!["codesign --verify OK".into(), "signtool verify OK".into()],
            notarize_status: Some("Accepted".into()),
            tested_5os: true,
        };
        let j = serde_json::to_string(&ev).unwrap();
        let back: ReleaseEvidence = serde_json::from_str(&j).unwrap();
        assert_eq!(ev.manifest.version, back.manifest.version);
        assert_eq!(ev.verify_logs.len(), back.verify_logs.len());
        assert_eq!(ev.notarize_status, back.notarize_status);
        assert_eq!(ev.tested_5os, back.tested_5os);
    }

    // ─── Stage 3 helper unit tests ──────────────────────────────────────────

    /// Build a unique scratch path under the OS temp dir for a test.
    fn tmp_path(suffix: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = now_unix_ms_pseudo().unwrap();
        std::env::temp_dir().join(format!("phantom_rpw_{}_{}_{}", nanos, n, suffix))
    }

    #[test]
    fn now_unix_ms_pseudo_is_recent() {
        let ms = now_unix_ms_pseudo().expect("clock should be after epoch");
        // Sanity: after 2020-01-01 (1_577_836_800_000) and before year 2100.
        assert!(ms > 1_577_836_800_000, "ms={} too small", ms);
        assert!(ms < 4_102_444_800_000, "ms={} too large", ms);
    }

    #[test]
    fn sha256_file_pseudo_known_value() {
        let p = tmp_path("abc.bin");
        std::fs::write(&p, b"abc").unwrap();
        let (hex, size) = sha256_file_pseudo(p.to_str().unwrap()).unwrap();
        std::fs::remove_file(&p).ok();
        assert_eq!(
            hex,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(size, 3);
    }

    #[test]
    fn sha256_file_pseudo_missing_is_error() {
        let p = tmp_path("does_not_exist.bin");
        let r = sha256_file_pseudo(p.to_str().unwrap());
        assert!(matches!(r, Err(ReleaseError::VerifyFailed(_))));
    }

    #[test]
    fn glob_artifacts_pseudo_filters_and_sorts() {
        let dir = tmp_path("globdir");
        std::fs::create_dir_all(&dir).unwrap();
        // Matching extensions.
        std::fs::write(dir.join("z-app.dmg"), b"x").unwrap();
        std::fs::write(dir.join("a-app.msi"), b"x").unwrap();
        std::fs::write(dir.join("b-app.AppImage"), b"x").unwrap();
        std::fs::write(dir.join("c-app.apk"), b"x").unwrap();
        std::fs::write(dir.join("d-app.ipa"), b"x").unwrap();
        // Mixed/upper-case extension must also match (case-insensitive).
        std::fs::write(dir.join("e-app.DMG"), b"x").unwrap();
        // Non-matching extensions, should be excluded.
        std::fs::write(dir.join("notes.txt"), b"x").unwrap();
        std::fs::write(dir.join("checksum.sha256"), b"x").unwrap();

        let mut found = glob_artifacts_pseudo(dir.to_str().unwrap()).unwrap();
        // Compare on file names only (dir prefix is the temp path).
        let names: Vec<String> = found
            .drain(..)
            .map(|p| {
                std::path::Path::new(&p)
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(
            names,
            vec![
                "a-app.msi".to_string(),
                "b-app.AppImage".to_string(),
                "c-app.apk".to_string(),
                "d-app.ipa".to_string(),
                "e-app.DMG".to_string(),
                "z-app.dmg".to_string(),
            ],
            "must be filtered to release exts (case-insensitive) and sorted ascending"
        );
    }

    #[test]
    fn glob_artifacts_pseudo_missing_dir_ok_empty() {
        let dir = tmp_path("no_such_dir");
        let r = glob_artifacts_pseudo(dir.to_str().unwrap()).unwrap();
        assert!(r.is_empty(), "missing dir must yield empty vec, not error");
    }

    #[test]
    fn detect_adjacent_signature_pseudo_prefers_asc_then_sig_then_none() {
        // None case.
        let base = tmp_path("artifact.dmg");
        std::fs::write(&base, b"x").unwrap();
        let base_s = base.to_str().unwrap();
        assert_eq!(detect_adjacent_signature_pseudo(base_s).unwrap(), None);

        // .sig only.
        let sig = format!("{}.sig", base_s);
        std::fs::write(&sig, b"sig").unwrap();
        assert_eq!(
            detect_adjacent_signature_pseudo(base_s).unwrap(),
            Some(sig.clone())
        );

        // .asc present → preferred over .sig.
        let asc = format!("{}.asc", base_s);
        std::fs::write(&asc, b"asc").unwrap();
        assert_eq!(
            detect_adjacent_signature_pseudo(base_s).unwrap(),
            Some(asc.clone())
        );

        std::fs::remove_file(&base).ok();
        std::fs::remove_file(&sig).ok();
        std::fs::remove_file(&asc).ok();
    }

    #[test]
    fn parse_os_arch_from_filename_pseudo_matrix() {
        let cases = [
            (
                "phantom-mesh-0.6.0-darwin-aarch64.dmg",
                ArtifactOs::Macos,
                ArtifactArch::Aarch64,
            ),
            (
                "phantom-mesh-0.6.0-macos-x86_64.dmg",
                ArtifactOs::Macos,
                ArtifactArch::X86_64,
            ),
            (
                "phantom-mesh-0.6.0-windows-amd64.msi",
                ArtifactOs::Windows,
                ArtifactArch::X86_64,
            ),
            (
                "phantom-mesh-0.6.0-linux-arm64.AppImage",
                ArtifactOs::Linux,
                ArtifactArch::Aarch64,
            ),
            (
                "phantom-mesh-0.6.0-android-arm64.apk",
                ArtifactOs::Android,
                ArtifactArch::Aarch64,
            ),
            (
                "phantom-mesh-0.6.0-macos-universal.dmg",
                ArtifactOs::Macos,
                ArtifactArch::Universal2,
            ),
            // Case-insensitivity.
            (
                "Phantom-Mesh-0.6.0-DARWIN-X86_64.dmg",
                ArtifactOs::Macos,
                ArtifactArch::X86_64,
            ),
        ];
        for (name, want_os, want_arch) in cases {
            let (os, arch) = parse_os_arch_from_filename_pseudo(name)
                .unwrap_or_else(|e| panic!("parse '{}' failed: {:?}", name, e));
            assert_eq!(os, want_os, "os mismatch for '{}'", name);
            assert_eq!(arch, want_arch, "arch mismatch for '{}'", name);
        }
    }

    #[test]
    fn parse_os_arch_from_filename_pseudo_rejects_unknown() {
        // Unknown OS.
        assert!(matches!(
            parse_os_arch_from_filename_pseudo("phantom-plan9-x86_64.bin"),
            Err(ReleaseError::VerifyFailed(_))
        ));
        // Known OS, unknown arch.
        assert!(matches!(
            parse_os_arch_from_filename_pseudo("phantom-linux-riscv128.AppImage"),
            Err(ReleaseError::VerifyFailed(_))
        ));
    }
}
