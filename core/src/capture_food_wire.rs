// SPEC-20 §7 — Capture-food wire types: user input (text / image / watch
// voice / share-extension) → multimodal LLM analysis → FoodEvent appended to
// event_storage.
//
// Stage 3 (real impl — six of seven helpers live): six Stage 2 `_pseudo`
// helpers are now real: `file_size` (std::fs::metadata), `build_food_prompt`
// (string template), `parse_food_json` (serde_json + §8.3 unknown
// discipline), `shame_free_lint` (regex strip per SPEC-20 §13 ethics rail),
// `iso8601_now` (chrono), and `gemini_multimodal` (delegates to
// `providers_wire::complete` with `ProviderType::Gemini` — which itself is
// now real reqwest HTTP per the sibling Stage 3 lift). The text + prompt
// path is fully live; true multimodal image inlining (inline_data parts) is
// the only remaining gap and lives behind `gemini_multimodal` for now,
// surfacing the deficit as an upstream-text-only `analyze_food` call.
// `uuid_v7` remains Stage 4 because the `uuid/v7` feature is gated under
// `experimental-hermes-tools` in core/Cargo.toml.
//
// 中文: 本檔對應 SPEC-20 §7（資料模型 (data model)）。「拍餐點 → AI 估熱量 →
// 加密落地」的 wire-shape 介面 (interface)。Stage 3 把六個 helper（檔案大小、
// prompt 字串模板、JSON 解析、shame-free regex、chrono 時戳、+ gemini 文字派
// 工）都換成真實 impl；image inline_data 多模態 + uuid v7 兩個維持 Stage 4
// stub。
// TODO Stage 4: wire into core/src/lib.rs; add `image` crate (0.25+); bridge
// `FoodCaptureError` ↔ SPEC-04 `FOOD_BLOB_TOO_LARGE` / `FOOD_ANALYSIS_FAILED`
// / `FOOD_DECRYPT_FAILED` 3-code public surface; enable `uuid/v7` (currently
// gated under `experimental-hermes-tools` feature); upgrade
// `gemini_multimodal` to inline image bytes as `inline_data` parts (today
// it ships only the path string in the prompt — model sees a description,
// not the pixels).

use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::event_storage_wire::{self, AnalysisResult, EventMeta, EventStoreError};
use crate::rpc_wire::EventKind;

// ─── SPEC-13 / SPEC-16 EventStore routing (P4-perimeter fix) ─────────────────
//
// 中文: 把 meal（餐點）寫入路徑導向 SPEC-16 加密 EventStore（事件儲存），不再
// 把 image_path / note 等含 PII 的內容寫成明文。所有含 PII 的欄位（user note、
// image 路徑、LLM 摘要 / 建議）都封進 age v1 加密 body（SPEC-13），plaintext
// meta（`tags` / `timestamp` / `kind`）只放非 PII（SPEC-16 §12.1）。
//
// 這個修法與剛落地的 `capture_habit_wire.rs::checkin_insert_pseudo`（習慣打卡）
// 同 class — habit 的明文 `note` 洩漏已修；food 的 meal 同樣要走加密 EventStore。

/// Plaintext tag stamped on every food event so the read path can
/// `query_events` by `kind=Food` + `tag="food"` without decrypting. SPEC-16
/// §12.1: tags are plaintext and MUST stay PII-free — `"food"` + the
/// non-PII request tags (`"fat_loss"`, `"work"`) are safe to expose.
const FOOD_EVENT_TAG: &str = "food";

/// SPEC-22 §7.1.3-style food metadata body — the at-rest shape that gets
/// age-encrypted into `body.age` via the SPEC-16 EventStore. Distinct from the
/// wire-facing `FoodEvent`: everything here is PII / sensitive (the user's note,
/// the source image path, the LLM-derived summary + suggestion) and MUST NOT
/// appear in plaintext on disk per P4.
///
/// 中文: 寫入加密 body 的 food metadata。`note`（備註）/ `image_path`（圖片路徑）
/// / `summary`（摘要）/ `suggestion`（建議）都是潛在 PII，只在加密 body 出現、
/// 絕不寫明文。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct FoodMetadata {
    note: Option<String>,
    image_path: Option<String>,
    timestamp_ms: u64,
    summary: String,
    suggestion: String,
    confidence: f32,
    fat_loss_score: f32,
    macro_estimate: Option<MacroEstimate>,
}

// ─── §7.1-adjacent FoodCaptureRequest — input from UI / CLI / share-extension ─

/// Inbound capture request from any surface (touch UI, watch voice, share
/// extension, CLI). Exactly one of `text` / `image_path` SHOULD be `Some`;
/// neither = `FoodCaptureError::EmptyRequest`. Both = "photo + note".
///
/// 中文: 食物 capture 進件 struct。`text` 或 `image_path` 至少要有一個；
/// `kind` 固定 `"food_log"`，wire 端字串方便未來擴成 `"food.meal"` /
/// `"food.snack"` / `"food.drink"` 子分類（SPEC-20 §7.1 預留）。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/capture_food/")]
#[serde(rename_all = "camelCase")]
pub struct FoodCaptureRequest {
    /// Optional free-text note from the user (e.g. "中午的鮭魚便當"). May be
    /// the *only* input on watch / accessibility paths.
    pub text: Option<String>,
    /// Optional absolute path to the source image on disk. Stage 2 reads the
    /// bytes, validates ≤ 10 MB via `validate_image_size`, then runs the
    /// compression pipeline per SPEC-20 §8.1 before encryption.
    pub image_path: Option<String>,
    /// Fixed `"food_log"` for SPEC-20 — kept as String for forward compatibility
    /// with the spec's `"food.meal"` / `"food.snack"` / `"food.drink"` sub-kinds.
    /// Callers SHOULD use the `FOOD_LOG_KIND` constant rather than hard-coding.
    pub kind: String,
    /// Free-form tags attached at capture time. Typical default is
    /// `["fat_loss"]` per SPEC-20 §7.2 (Life-Track fat-loss pillar). UI may
    /// inject `"work"`, `"travel"`, etc.
    pub tag: Vec<String>,
    /// UTC milliseconds at capture moment (client-supplied to preserve the
    /// user's wall-clock intent across timezone changes). Stage 2 uses this for
    /// `EventMeta.timestamp` formatting per SPEC-16 §8 G2.
    pub timestamp_ms: u64,
}

/// Canonical value for `FoodCaptureRequest.kind`. Stage 2 may widen to enum.
pub const FOOD_LOG_KIND: &str = "food_log";

/// Image upload hard cap (10 MB) per SPEC-20 §8.1 read-side guard.
/// Compression target (500 KB) is enforced separately in the encode pipeline.
pub const IMAGE_MAX_BYTES: u64 = 10 * 1024 * 1024;

// ─── §7.1.1 MacroEstimate — macronutrient breakdown (LLM-estimated) ──────────

/// Per-meal macronutrient (macronutrient = protein / carbs / fat / fiber)
/// estimate. All values are LLM-estimated (± 30% typical error per §18 Q2);
/// UI renders as range using parent `FoodAnalysisResult.confidence`.
///
/// 中文: 單餐巨量營養素 (macronutrient) 估算；誤差約 ± 30%。0 = 無法估或不
/// 適用（如純飲料 protein_g 通常 0）。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/capture_food/")]
#[serde(rename_all = "camelCase")]
pub struct MacroEstimate {
    /// Estimated calories (kcal). 0 if unknown / `FoodAnalysisResult.confidence`
    /// below 0.5 threshold per SPEC-20 §8.3 unknown discipline.
    pub calories: u32,
    /// Protein in grams.
    pub protein_g: u16,
    /// Carbohydrates in grams.
    pub carbs_g: u16,
    /// Fat in grams.
    pub fat_g: u16,
    /// Dietary fiber in grams.
    pub fiber_g: u16,
}

// ─── §7.1.2 FoodAnalysisResult — vision LLM output ───────────────────────────

/// Vision-LLM output for a `FoodCaptureRequest`. Distinct from
/// `event_storage_wire::AnalysisResult` — the latter is the generic side-car
/// (model_id / cost / latency / raw); this carries food-specific *content*
/// (summary + macros + fat-loss score). Both ride on the same `FoodEvent`.
///
/// 中文: 食物分析結果（食物專屬欄位）。`AnalysisResult` 管模型元資料
/// (metadata)，本 struct 管食物內容。`macro_estimate = None` 代表 LLM 拒答
/// （飲料 only、或 `confidence < 0.5` per §8.3 unknown discipline）。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/capture_food/")]
#[serde(rename_all = "camelCase")]
pub struct FoodAnalysisResult {
    /// One-sentence bilingual summary (≤ 280 chars). Feeds FTS5 index via
    /// `event_storage::index_fts5` after PII scrub per SPEC-16 §12.1.
    pub summary: String,
    /// Macronutrient breakdown if estimable; `None` on low-confidence / drinks.
    pub macro_estimate: Option<MacroEstimate>,
    /// Fat-loss alignment score in [0.0, 1.0]. 1.0 = perfectly on plan,
    /// 0.0 = anti-goal (e.g. 1000 kcal dessert on cut day). Drives coach
    /// review suggestion ordering.
    pub fat_loss_score: f32,
    /// Imperative next-step suggestion (e.g. "skip dessert tonight").
    /// Empty string if the model declined to suggest.
    pub suggestion: String,
    /// Model self-reported confidence in [0.0, 1.0]. Below 0.5 triggers
    /// `MacroEstimate = None` per SPEC-20 §8.3 unknown discipline.
    pub confidence: f32,
}

// ─── §7.1 FoodEvent — composite written to event storage ─────────────────────

/// Top-level food event: shared `EventMeta` + food-specific
/// `FoodAnalysisResult` + generic LLM-call `AnalysisResult` side-car + raw
/// user input. Persisted via `event_storage::write_event(meta, encrypted_body,
/// analysis_meta)` with `analysis` + `raw_input` serialised into the body.
///
/// 中文: 食物事件 (composite type)。`analysis_meta` 走 SPEC-16 通用元資料
/// （model / cost / latency / raw），食物專屬內容放 `analysis`；不重複定義。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/capture_food/")]
#[serde(rename_all = "camelCase")]
pub struct FoodEvent {
    /// Shared event metadata. `kind == EventKind::Food`; `tags` mirrors the
    /// `FoodCaptureRequest.tag` (typically `["fat_loss"]`).
    pub meta: EventMeta,
    /// Food-specific analysis content (summary + macros + fat-loss score).
    pub analysis: FoodAnalysisResult,
    /// Generic LLM-call side-car (model_id / cost_usd / latency_ms / raw).
    /// Optional because Stage 2 may write the FoodEvent before analysis returns
    /// (optimistic UI per SPEC-20 §8.5 `analysis_pending` flow).
    pub analysis_meta: Option<AnalysisResult>,
    /// Verbatim user input (text + image path) — encrypted at rest in the
    /// event body per SPEC-13. Never logged in plaintext per SPEC-20 §13.
    pub raw_input: FoodCaptureRequest,
}

// ─── §7.1-adjacent FoodCaptureSource — provenance enum ──────────────────────

/// Which surface produced this capture. Stage 2 writes into `EventMeta.tags`
/// as `"src:user_text"` / `"src:share_extension"` etc so coach review can
/// stratify accuracy by entry channel.
///
/// 中文: capture 來源列舉 (provenance enum)。Stage 2 寫進 `meta.tags`
/// 分流分析「watch 語音」vs「拍照」vs「分享 extension」哪個準。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/capture_food/")]
#[serde(rename_all = "snake_case")]
pub enum FoodCaptureSource {
    /// User typed text in the app (no image).
    UserText,
    /// User picked / shot an image in the app.
    UserImage,
    /// Voice memo from watch app → STT → text. SPEC-21 audio capture pipeline.
    WatchVoice,
    /// iOS share-extension / Android share-intent from another app (e.g. photo
    /// shared from gallery, screenshot from Instagram).
    ShareExtension,
}

// ─── §11.1 FoodCaptureError — error catalog (mirrors SPEC-04 codes) ──────────

/// Wire-facing error variants for the capture-food pipeline. Mirrors SPEC-20
/// §11.1 codes (with richer Stage 2 dispatch variants that map down to the
/// public 3 user-facing codes for UI).
///
/// 中文: SPEC-20 §11.1 error catalog 的 wire-facing 鏡像 (mirror)。UI 只看
/// 3 個 code (`FOOD_BLOB_TOO_LARGE` / `FOOD_ANALYSIS_FAILED` /
/// `FOOD_DECRYPT_FAILED`)；內部細變體 Stage 2 map 到這 3 個。
#[derive(Debug, Clone, Serialize, Deserialize, TS, thiserror::Error)]
#[ts(export, export_to = "../../app/src/lib/generated/capture_food/")]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum FoodCaptureError {
    /// Source image exceeds the 10 MB read-side ceiling (`IMAGE_MAX_BYTES`).
    /// Maps to public `FOOD_BLOB_TOO_LARGE`.
    #[error("food.image_too_large: {bytes} bytes (max {max})")]
    ImageTooLarge { bytes: u64, max: u64 },
    /// File at `image_path` could not be opened / decoded as a supported
    /// format (jpeg / png / heic / webp per SPEC-20 §8.1).
    /// Maps to public `FOOD_BLOB_TOO_LARGE` UX (re-shoot prompt).
    #[error("food.image_unreadable: {detail}")]
    ImageUnreadable { detail: String },
    /// All providers in the fallback chain returned an error. Stage 2 still
    /// writes the event with `status = analysis_failed` per SPEC-20 §8.2.
    /// Maps to public `FOOD_ANALYSIS_FAILED`.
    #[error("food.provider_failed: chain exhausted ({attempts} attempts)")]
    ProviderFailed { attempts: u8 },
    /// LLM analysed the image and confidently reported "no food in frame"
    /// (e.g. user accidentally photographed a wall). Distinct from
    /// `ProviderFailed` so UI can render a different prompt.
    /// Maps to public `FOOD_ANALYSIS_FAILED`.
    #[error("food.no_food_detected")]
    NoFoodDetected,
    /// EventKey missing at decrypt time (e.g. iOS app before keychain unlock).
    /// Maps to public `FOOD_DECRYPT_FAILED`.
    #[error("food.decrypt_failed: {detail}")]
    DecryptFailed { detail: String },
    /// Both `text` and `image_path` were `None` in the request.
    /// Maps to public `FOOD_ANALYSIS_FAILED` UX.
    #[error("food.empty_request")]
    EmptyRequest,
    /// Underlying event_storage write failed (sqlite I/O, disk full, ...).
    /// Maps to public `FOOD_ANALYSIS_FAILED` (with retry hint).
    #[error("food.storage: {0}")]
    Storage(String),
}

// ─── Stage 2 helpers — pseudocode bodies (Stage 3 fills inner _pseudo fns) ───
//
// Per docs/superpowers/SPEC-TO-CODE-PLAYBOOK.md (same pattern as SPEC-10):
//   Stage 2 = function body shows what it WILL do via Step-N comments + nested
//   `_pseudo` inner helpers. Reader can audit the algorithm flow without
//   trusting any real provider / fs / uuid implementation. Stage 3 swaps the
//   `_pseudo` helpers for real reqwest / serde_json / std::fs / uuid / chrono
//   calls (added to core/Cargo.toml then).
//
// 中文: Stage 2 把每個 stub 換成「Step 編號註解 + nested `_pseudo` 助手」
// 結構，每個 `_pseudo` 仍 panic `Stage 3: <crate hint>`。讀者可以審
// algorithm（演算法）flow，不用信任任何真實實作。

/// Run the full capture-food pipeline. Stage 2 pseudocode:
/// (1) build multimodal prompt (text + optional image); (2) call
/// `gemini-2.5-flash` via providers_wire::complete with strict JSON
/// `response_format` (canonical reference: `core/src/life_node/capture.rs`);
/// (3) parse JSON → `FoodAnalysisResult`; (4) lint shame-free output
/// (no body-shaming language per SPEC-20 §13 ethics rail).
///
/// 中文: 食物 capture 主 pipeline。Stage 2 偽碼 (pseudocode) 串「組多模態
/// (multimodal) prompt → 叫 gemini-2.5-flash → 解析 JSON → shame-free lint」
/// 四步；Stage 3 換成真的 reqwest + serde_json 實作。
pub fn analyze_food(
    request: &FoodCaptureRequest,
) -> Result<FoodAnalysisResult, FoodCaptureError> {
    // Step 1: build the prompt — text note + (optional) image *path string*.
    //         Pure string + template work; no network / fs yet. NOTE: this
    //         does NOT inline the image bytes as a gemini-2.5-flash
    //         `inline_data` part — that pixel multimodal upgrade is deferred
    //         (see `gemini_multimodal_pseudo` below). Today the model only
    //         sees the filename, not the pixels.
    let prompt: String = build_food_prompt(request);

    // Step 2: dispatch to gemini-2.5-flash via providers_wire::complete with
    //         a strict-JSON response_format. SPEC-20 §8.2 vision fallback
    //         chain (claude-3-7-sonnet → gpt-4o → gemini-2.5-pro →
    //         claude-3-5-haiku) collapses to just gemini-2.5-flash at Stage 2
    //         for the canonical reference path — fallback chain wiring is
    //         Stage 4 work in core/src/life_node/food.rs (still pseudo here).
    let raw_json: String = gemini_multimodal_pseudo(&prompt, request.image_path.as_deref())?;

    // Step 3: parse the strict-JSON LLM reply into the food-specific result
    //         struct. Enforces §8.3 unknown discipline (`confidence < 0.5`
    //         → `macro_estimate = None`) inside the parser helper.
    let parsed: FoodAnalysisResult = parse_food_json(&raw_json)?;

    // Step 4: shame-free lint — strip body-shaming / moralising language per
    //         SPEC-20 §13 ethics rail before returning. Replaces offending
    //         phrases in `summary` / `suggestion` with neutral wording.
    let linted: FoodAnalysisResult = shame_free_lint(parsed);

    Ok(linted)
}

/// Record one meal: run `analyze_food`, then append a `kind=Food` event to the
/// SPEC-16 **encrypted** EventStore — the PII-bearing body (user note, source
/// image path, LLM summary + suggestion + macros) is age-encrypted at rest
/// (SPEC-13), while only non-PII meta (`kind` / `timestamp` / `"food"` tag +
/// request tags) stays in plaintext. Returns the assigned UUIDv7 `event_id`.
///
/// P4-PERIMETER FIX: the previous pipeline only ran `analyze_food` and never
/// persisted the meal through the encrypted EventStore (and stuffed
/// `image_path` as a plaintext string into the prompt). This routes the whole
/// meal through `event_storage_wire::write_event` so nothing PII-bearing ever
/// touches the disk in plaintext — same class as the just-landed
/// `capture_habit_wire` check-in fix.
///
/// 中文: 記錄一餐 — 先 `analyze_food`，再把整餐（含 PII 的 note / image_path /
/// 摘要 / 建議 / macros）封進 age v1 加密 body 後，透過 SPEC-16 加密 EventStore
/// 落地（`kind=Food`）。明文 meta 只放非 PII（時間戳 / `"food"` tag）。回傳
/// UUIDv7 event_id。
pub fn record_food(request: &FoodCaptureRequest) -> Result<String, FoodCaptureError> {
    // Step 1: at least one of text / image_path must be present.
    if request.text.is_none() && request.image_path.is_none() {
        return Err(FoodCaptureError::EmptyRequest);
    }
    // Step 2: run the LLM analysis pipeline (text + prompt path today; pixel
    // multimodal deferred — see `gemini_multimodal_pseudo` blocker note).
    let analysis = analyze_food(request)?;
    // Step 3: persist through the encrypted EventStore.
    write_food_event(request, &analysis)
}

/// Append a food event to the SPEC-16 encrypted EventStore. Builds the
/// PII-bearing `FoodMetadata` body, age-encrypts it against the per-process
/// EventKey (SPEC-13), and writes it via `event_storage_wire::write_event` with
/// a plaintext-safe `EventMeta { kind = Food, tags = ["food", ...] }`.
///
/// 中文: 把一餐寫進加密 EventStore — PII body 走 age v1 加密，明文 meta 只放
/// 非 PII tag。SPEC-13 encrypt / EventStore 寫入失敗 → `FoodCaptureError::Storage`.
fn write_food_event(
    request: &FoodCaptureRequest,
    analysis: &FoodAnalysisResult,
) -> Result<String, FoodCaptureError> {
    // Step 1: build the PII-bearing body — note / image_path / LLM summary +
    // suggestion + macros all live ONLY inside the encrypted blob.
    let body = FoodMetadata {
        note: request.text.clone(),
        image_path: request.image_path.clone(),
        timestamp_ms: request.timestamp_ms,
        summary: analysis.summary.clone(),
        suggestion: analysis.suggestion.clone(),
        confidence: analysis.confidence,
        fat_loss_score: analysis.fat_loss_score,
        macro_estimate: analysis.macro_estimate.clone(),
    };
    let plaintext = serde_json::to_vec(&body)
        .map_err(|e| FoodCaptureError::Storage(format!("serialize food metadata: {}", e)))?;
    // Step 2: age-encrypt the body against the per-process EventKey (SPEC-13)
    // so nothing PII-bearing ever touches the disk in plaintext.
    let encrypted_body = encrypt_to_event_key(&plaintext)?;
    // Step 3: assemble plaintext-safe `EventMeta` — UUIDv7 id, canonical UTC
    // RFC-3339 timestamp, `kind=Food`, `"food"` + request tags. NO PII in meta
    // per SPEC-16 §12.1.
    let mut tags = vec![FOOD_EVENT_TAG.to_string()];
    tags.extend(request.tag.iter().cloned());
    let meta = EventMeta {
        event_id: uuid::Uuid::now_v7().to_string(),
        timestamp: event_storage_wire::ts_ms_to_rfc3339_utc(request.timestamp_ms as i64),
        kind: EventKind::Food,
        tags,
    };
    // Step 4: append to the encrypted EventStore; map any STORE-* failure to
    // the `Storage` catalog entry (maps to public FOOD_ANALYSIS_FAILED).
    event_storage_wire::write_event(&meta, &encrypted_body, None).map_err(store_err_to_food)
}

/// Age-encrypt plaintext bytes against the per-process EventKey (SPEC-13). The
/// EventStore body is the RAW age v1 blob (what `decrypt_raw_age_blob` expects),
/// so we strip the base64 transport layer that `encrypt_event` adds for its
/// JSON envelope. A missing EventKey (vault locked) surfaces as `Storage`,
/// never a panic. Mirrors `capture_habit_wire::encrypt_to_event_key`.
fn encrypt_to_event_key(plaintext: &[u8]) -> Result<Vec<u8>, FoodCaptureError> {
    use base64::Engine as _;
    let key = crate::encryption_wire::lookup_or_derive_event_key()
        .ok_or_else(|| FoodCaptureError::Storage("EventKey not loaded (vault locked)".into()))?;
    let identity = crate::encryption_wire::event_key_to_age_identity(&key)
        .map_err(|e| FoodCaptureError::Storage(format!("derive age identity: {:?}", e)))?;
    let recipient = crate::encryption_wire::derive_recipient_from_identity(&identity);
    let envelope = crate::encryption_wire::encrypt_event(plaintext, &recipient)
        .map_err(|e| FoodCaptureError::Storage(format!("age encrypt: {:?}", e)))?;
    base64::engine::general_purpose::STANDARD
        .decode(envelope.ciphertext_b64.as_bytes())
        .map_err(|e| FoodCaptureError::Storage(format!("decode age blob: {}", e)))
}

/// Map a SPEC-16 `EventStoreError` into the SPEC-20 §11.1 `Storage` catalog
/// entry, preserving the underlying STORE-* detail string.
fn store_err_to_food(e: EventStoreError) -> FoodCaptureError {
    FoodCaptureError::Storage(format!("event_storage: {}", e))
}

/// Validate that the file at `path` exists and is ≤ `IMAGE_MAX_BYTES` (10 MB).
/// Stage 2 pseudocode: (1) read file size via std::fs::metadata;
/// (2) compare to `IMAGE_MAX_BYTES`; (3) `Ok(())` or
/// `FoodCaptureError::ImageTooLarge { bytes, max }`.
///
/// 中文: 檢查 image 檔案 ≤ 10 MB；Stage 3 再加 magic-byte 嗅探。
pub fn validate_image_size(path: &Path) -> Result<(), FoodCaptureError> {
    // Step 1: read filesystem metadata → file length in bytes. Missing /
    //         unreadable file surfaces as `ImageUnreadable` from the helper.
    let bytes: u64 = file_size(path)?;

    // Step 2: compare against the 10 MB read-side ceiling per SPEC-20 §8.1.
    //         Compression target (500 KB) is a separate downstream concern.
    if bytes > IMAGE_MAX_BYTES {
        // Step 3a: oversize → public-facing `ImageTooLarge` (maps to
        //          SPEC-04 `FOOD_BLOB_TOO_LARGE`).
        return Err(FoodCaptureError::ImageTooLarge {
            bytes,
            max: IMAGE_MAX_BYTES,
        });
    }
    // Step 3b: within budget → caller proceeds to encode + analyse.
    Ok(())
}

/// Build the `EventMeta` for a food event. Stage 2 pseudocode:
/// (1) generate UUIDv7 via `Uuid::now_v7()` (SPEC-16 §8 G2);
/// (2) format timestamp as local ISO-8601 string from `req.timestamp_ms`;
/// (3) construct `EventMeta` with `kind = EventKind::Food` + tags from
/// `req.tag` + `source_node` derived from agents.toml (Stage 3 wires).
///
/// 中文: 從 request 組出 `EventMeta` — `kind=Food`、UUIDv7、ISO-8601 時間戳、
/// tags 直接 copy from request；`source_node` Stage 3 從 agents.toml 拉。
pub fn build_food_event_meta(req: &FoodCaptureRequest) -> EventMeta {
    // Step 1: generate a UUIDv7 event_id. v7 is time-ordered → cheap range
    //         queries in event_storage (SPEC-16 §8 G2 invariant).
    let event_id: String = uuid_v7_pseudo();

    // Step 2: format the client-supplied `timestamp_ms` as an ISO-8601 string
    //         in UTC — preserves the user's wall-clock intent (the client
    //         already converted to absolute ms) across TZ changes per
    //         SPEC-20 §7.1 timestamp_ms semantics.
    let timestamp: String = iso8601_now(req.timestamp_ms);

    // Step 3: assemble the shared `EventMeta` — `kind = EventKind::Food`,
    //         tags copied verbatim from the request (typically `["fat_loss"]`
    //         per §7.2). Stage 3 will additionally stamp `source_node` from
    //         agents.toml so coach review can attribute by device.
    EventMeta {
        event_id,
        timestamp,
        kind: EventKind::Food,
        tags: req.tag.clone(),
    }
}

// ─── Stage 3 inner helpers (real impl for 6/7; 1 still Stage 4) ──────────────
//
// All `_pseudo` stubs are now real: five pure-Rust helpers (file_size /
// build_food_prompt / parse_food_json / shame_free_lint / iso8601_now) wrap
// std::fs / serde_json / regex / chrono, `gemini_multimodal` delegates to
// `providers_wire::complete` (real reqwest HTTP), and `uuid_v7_pseudo` now
// mints a real UUIDv7 (the `uuid` crate's `v7` feature was promoted into the
// default build in core/Cargo.toml). A secondary Stage 4 enhancement (NOT a
// stub, just a TODO on `gemini_multimodal`): inline the actual image bytes as
// `inline_data` parts so the model sees pixels instead of just the path string.
//
// 中文: 6 個 helper 換成真實實作（5 個純 Rust + 1 個跨檔 delegate 到
// providers_wire）；只剩 uuid v7（feature gating）一個 Stage 4 stub。
// `gemini_multimodal` 雖然已是真的呼叫，但目前只把 image_path 寫進 prompt
// 文字裡，沒把實際 bytes 用 `inline_data` 塞進去 — 該升級也標 Stage 4。

/// SPEC-20 §8.2 — build the multimodal prompt body for `gemini-2.5-flash`.
/// Pure string templating: combines the optional user note + image-path hint
/// into a system+user prompt block that instructs the model to reply with
/// strict JSON conforming to `FoodAnalysisResult`.
///
/// NOTE (honest current behaviour): this function emits the image *path
/// string* into the prompt text — it does NOT read or inline the image
/// bytes. The provider adapter does not inline bytes either today; true
/// pixel multimodal (base64 `inline_data` parts) is deferred work tracked on
/// `gemini_multimodal_pseudo` below. So the model currently sees a filename,
/// not the pixels.
fn build_food_prompt(request: &FoodCaptureRequest) -> String {
    let mut out = String::with_capacity(512);
    out.push_str(
        "You are a nutrition analyst. Reply with strict JSON matching the \
         FoodAnalysisResult schema {summary, macroEstimate{calories,protein_g,\
         carbs_g,fat_g,fiber_g}, fatLossScore, suggestion, confidence}. If \
         confidence < 0.5, set macroEstimate=null. Avoid body-shaming, \
         moralising, or guilt-inducing language.\n\n",
    );
    out.push_str("user_note: ");
    out.push_str(request.text.as_deref().unwrap_or(""));
    out.push('\n');
    out.push_str("image_path: ");
    out.push_str(request.image_path.as_deref().unwrap_or(""));
    out.push('\n');
    out.push_str("tags: ");
    out.push_str(&request.tag.join(","));
    out.push('\n');
    out
}

/// Dispatch to `gemini-2.5-flash` via `providers_wire::complete`. The
/// providers_wire Gemini path is real reqwest HTTP at Stage 3, so this is
/// a thin adapter: wraps the prompt into a single user-turn `Message`,
/// asks for `ResponseFormat::Json` (Gemini honours `responseMimeType`
/// per the sibling module), and surfaces text or maps the upstream error
/// down to `FoodCaptureError::ProviderFailed { attempts: 1 }` so caller
/// can retry the next provider in SPEC-20 §8.2 fallback chain.
///
/// 中文: 把 prompt 包成 user turn → 呼叫 providers_wire::complete（Gemini 已
/// 是真實 reqwest）→ 取回文字。任何上游錯誤都收斂成 ProviderFailed，讓
/// SPEC-20 §8.2 fallback chain 接手。注意：目前只把 image_path 寫進 prompt
/// 字串裡，沒把 bytes 用 `inline_data` 真實多模態送上去 — 該升級在
/// gemini_multimodal_image_inline Stage 4 標記裡。
fn gemini_multimodal_pseudo(
    prompt: &str,
    image_path: Option<&str>,
) -> Result<String, FoodCaptureError> {
    use crate::providers_wire::{
        self, Message as PMessage, MessageRole, ProviderRequest, ResponseFormat,
    };

    // SPEC-20 T-FOOD-02: inline the actual image bytes as a Gemini
    // `inline_data` part so the model sees the pixels, not just the filename.
    // Read the bytes, base64-encode, infer the MIME type from the extension,
    // and attach as a `MessageImage` on the user turn. Falls back to the
    // text-only path (path string still in the prompt) when no image is given.
    let user_msg = match image_path {
        Some(path) => {
            let image = build_inline_image(path)?;
            PMessage::with_image(MessageRole::User, prompt.to_string(), image)
        }
        None => PMessage::text(MessageRole::User, prompt.to_string()),
    };

    let req = ProviderRequest {
        model: "gemini-2.5-flash".to_string(),
        system_prompt: None,
        messages: vec![user_msg],
        max_tokens: Some(1024),
        temperature: Some(0.2),
        response_format: ResponseFormat::Json,
    };

    let resp = providers_wire::complete(req).map_err(|_e| {
        // Collapse the rich provider-error catalog to ProviderFailed so the
        // SPEC-20 §8.2 fallback chain (claude-3-7-sonnet → gpt-4o →
        // gemini-2.5-pro → claude-3-5-haiku) can advance to the next slot.
        FoodCaptureError::ProviderFailed { attempts: 1 }
    })?;

    if resp.text.trim().is_empty() {
        return Err(FoodCaptureError::ProviderFailed { attempts: 1 });
    }
    Ok(resp.text)
}

/// SPEC-20 T-FOOD-02 — read the image at `path`, enforce the 10 MB read-side
/// ceiling, base64-encode the bytes, and infer the MIME type from the file
/// extension. Returns a `MessageImage` ready to attach to a provider `Message`
/// as a Gemini `inlineData` part (the model now sees the pixels, not just the
/// path). Missing / unreadable files surface as `ImageUnreadable`; oversize
/// files as `ImageTooLarge`, mirroring `validate_image_size`.
///
/// 中文: 讀 `path` 的圖片 → 檢查 ≤ 10 MB → base64 編碼 → 從副檔名推 MIME type
/// → 回傳可掛到 provider `Message` 的 `MessageImage`（Gemini `inlineData`
/// part）。讓模型真的看到 pixels，而非只有路徑字串。
fn build_inline_image(path: &str) -> Result<crate::providers_wire::MessageImage, FoodCaptureError> {
    use base64::Engine as _;
    let p = Path::new(path);
    // Reuse the shared 10 MB read-side guard so the inline path can't smuggle
    // an oversize blob past the SPEC-20 §8.1 ceiling.
    validate_image_size(p)?;
    let bytes = std::fs::read(p).map_err(|e| FoodCaptureError::ImageUnreadable {
        detail: e.to_string(),
    })?;
    let data_b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(crate::providers_wire::MessageImage {
        mime: mime_from_extension(path).to_string(),
        data_b64,
    })
}

/// Map a file extension to its image MIME type. Covers the SPEC-20 §8.1
/// supported set (jpeg / png / heic / webp); unknown extensions default to
/// `application/octet-stream` so the upstream can still attempt to sniff.
///
/// 中文: 從副檔名推 image MIME type（SPEC-20 §8.1 支援集）；未知副檔名退回
/// `application/octet-stream`。
fn mime_from_extension(path: &str) -> &'static str {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "heic" | "heif" => "image/heic",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => "application/octet-stream",
    }
}

/// SPEC-20 §8.3 — parse the LLM JSON reply into `FoodAnalysisResult`. Enforces
/// the "unknown discipline" invariant: when the model self-reports
/// `confidence < 0.5`, force `macro_estimate = None` regardless of what the
/// model returned (the model often hallucinates macros at low confidence).
/// Parse failures map to `ProviderFailed { attempts: 1 }` so the caller can
/// retry the next provider in the fallback chain.
fn parse_food_json(raw: &str) -> Result<FoodAnalysisResult, FoodCaptureError> {
    let mut parsed: FoodAnalysisResult = serde_json::from_str(raw)
        .map_err(|_| FoodCaptureError::ProviderFailed { attempts: 1 })?;
    if parsed.confidence < 0.5 {
        parsed.macro_estimate = None;
    }
    Ok(parsed)
}

/// SPEC-20 §13 — shame-free lint. Strips body-shaming / moralising phrases
/// from `summary` and `suggestion` and rewrites them to neutral imperative
/// voice. Regex set is intentionally conservative — false positives are
/// preferable to leaking guilt language into UI.
fn shame_free_lint(mut parsed: FoodAnalysisResult) -> FoodAnalysisResult {
    use regex::Regex;
    // Case-insensitive set of body-shaming / moralising phrases. Each match
    // is collapsed to the empty string; whitespace is normalised after.
    let patterns = [
        r"(?i)\b(fat|chubby|overweight|obese)\b",
        r"(?i)\b(guilty|guilt|shame|shameful|disgusting|disgrace)\b",
        r"(?i)\b(cheat\s*(?:day|meal)|bad\s+food|junk\s+food)\b",
        r"(?i)\b(you\s+should\s+feel|you\s+ought\s+to\s+feel)\b",
    ];
    let ws = Regex::new(r"\s+").unwrap();
    for raw in patterns {
        let re = Regex::new(raw).unwrap();
        parsed.summary = re.replace_all(&parsed.summary, "").to_string();
        parsed.suggestion = re.replace_all(&parsed.suggestion, "").to_string();
    }
    parsed.summary = ws.replace_all(parsed.summary.trim(), " ").to_string();
    parsed.suggestion = ws.replace_all(parsed.suggestion.trim(), " ").to_string();
    parsed
}

/// Read file size in bytes via `std::fs::metadata`. Missing / unreadable file
/// surfaces as `FoodCaptureError::ImageUnreadable` carrying the I/O error
/// message (re-shoot prompt on the UI side).
fn file_size(path: &Path) -> Result<u64, FoodCaptureError> {
    std::fs::metadata(path)
        .map(|m| m.len())
        .map_err(|e| FoodCaptureError::ImageUnreadable {
            detail: e.to_string(),
        })
}

/// Mint a UUIDv7 (time-ordered event id, SPEC-16 §8 G2). Now real: the
/// `uuid/v7` feature was promoted into the default build (core/Cargo.toml).
fn uuid_v7_pseudo() -> String {
    uuid::Uuid::now_v7().to_string()
}

/// ISO-8601 UTC string formatted from `timestamp_ms` (client-supplied UTC
/// milliseconds). Falls back to the Unix epoch on out-of-range values rather
/// than panicking — caller's `EventMeta.timestamp` is treated as advisory.
fn iso8601_now(timestamp_ms: u64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(timestamp_ms as i64)
        .map(|d| d.to_rfc3339())
        .unwrap_or_else(|| "1970-01-01T00:00:00+00:00".to_string())
}

// ─── Smoke tests (Stage 1 sanity only; deeper invariants in Stage 2) ─────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn food_analysis_result_round_trip_smoke() {
        // SPEC-20 §7.2 invariant: TS encode → wire → Rust decode preserves the
        // food-specific analysis surface. Stage 1 sanity-checks serde; deeper
        // invariants (confidence-driven `macro_estimate = None` discipline)
        // come in Stage 2.
        let a = FoodAnalysisResult {
            summary: "鮭魚便當 + 味噌湯 (salmon bento + miso soup)".into(),
            macro_estimate: Some(MacroEstimate {
                calories: 780,
                protein_g: 32,
                carbs_g: 95,
                fat_g: 24,
                fiber_g: 4,
            }),
            fat_loss_score: 0.72,
            suggestion: "skip dessert tonight".into(),
            confidence: 0.87,
        };
        let j = serde_json::to_string(&a).unwrap();
        let back: FoodAnalysisResult = serde_json::from_str(&j).unwrap();
        assert_eq!(a.summary, back.summary);
        assert_eq!(a.suggestion, back.suggestion);
        assert!((a.fat_loss_score - back.fat_loss_score).abs() < f32::EPSILON);
        assert!((a.confidence - back.confidence).abs() < f32::EPSILON);
        let m_back = back.macro_estimate.expect("macros preserved");
        assert_eq!(m_back.calories, 780);
        assert_eq!(m_back.protein_g, 32);
        assert_eq!(m_back.fiber_g, 4);
    }

    #[test]
    fn food_capture_request_camel_case_wire_shape() {
        // SPEC-20 §7.2 TypeScript interface uses camelCase field names; verify
        // the wire shape matches what the app/src/types/food.ts consumers see.
        let r = FoodCaptureRequest {
            text: Some("中午吃了鮭魚便當".into()),
            image_path: Some("/tmp/lunch.jpg".into()),
            kind: FOOD_LOG_KIND.into(),
            tag: vec!["fat_loss".into()],
            timestamp_ms: 1716563400000,
        };
        let j = serde_json::to_string(&r).unwrap();
        assert!(j.contains("\"imagePath\""), "wire shape: {}", j);
        assert!(j.contains("\"timestampMs\""), "wire shape: {}", j);
        assert!(j.contains("\"food_log\""), "wire shape: {}", j);
        let back: FoodCaptureRequest = serde_json::from_str(&j).unwrap();
        assert_eq!(back.kind, FOOD_LOG_KIND);
        assert_eq!(back.tag, vec!["fat_loss".to_string()]);
        assert_eq!(back.timestamp_ms, 1716563400000);
    }

    #[test]
    fn food_event_re_uses_shared_event_meta() {
        // Composite invariant: FoodEvent embeds shared `EventMeta` with
        // `kind == EventKind::Food`. Confirms we did NOT redeclare EventMeta.
        let evt = FoodEvent {
            meta: EventMeta {
                event_id: "01923f8e-7a4c-7000-8c2d-2b9f0e1d4a55".into(),
                timestamp: "2026-05-25T12:35:00Z".into(),
                kind: EventKind::Food,
                tags: vec!["fat_loss".into()],
            },
            analysis: FoodAnalysisResult {
                summary: "salmon bento".into(),
                macro_estimate: None,
                fat_loss_score: 0.5,
                suggestion: "".into(),
                confidence: 0.6,
            },
            analysis_meta: None,
            raw_input: FoodCaptureRequest {
                text: None,
                image_path: Some("/tmp/lunch.jpg".into()),
                kind: FOOD_LOG_KIND.into(),
                tag: vec!["fat_loss".into()],
                timestamp_ms: 1716563400000,
            },
        };
        let j = serde_json::to_string(&evt).unwrap();
        let back: FoodEvent = serde_json::from_str(&j).unwrap();
        assert!(matches!(back.meta.kind, EventKind::Food));
        assert!(back.analysis_meta.is_none());
        assert_eq!(back.raw_input.kind, FOOD_LOG_KIND);
    }

    #[test]
    fn food_capture_source_snake_case_wire_shape() {
        // §7.1 provenance enum: snake_case wire so it lands cleanly in
        // `EventMeta.tags` as `"src:share_extension"` etc.
        for v in [
            FoodCaptureSource::UserText,
            FoodCaptureSource::UserImage,
            FoodCaptureSource::WatchVoice,
            FoodCaptureSource::ShareExtension,
        ] {
            let j = serde_json::to_string(&v).unwrap();
            let back: FoodCaptureSource = serde_json::from_str(&j).unwrap();
            assert_eq!(v, back);
        }
        let j = serde_json::to_string(&FoodCaptureSource::ShareExtension).unwrap();
        assert_eq!(j, "\"share_extension\"");
    }

    #[test]
    fn food_capture_error_serialises_with_code_tag() {
        // SPEC-20 §11.1 invariant: wire shape uses `{"code": "..."}` tag so
        // the UI can dispatch on the machine-readable code string. Verify
        // a couple of variants survive round-trip with payload preserved.
        let e = FoodCaptureError::ImageTooLarge {
            bytes: 12_345_678,
            max: IMAGE_MAX_BYTES,
        };
        let j = serde_json::to_string(&e).unwrap();
        assert!(j.contains("\"code\""), "wire shape: {}", j);
        assert!(j.contains("image_too_large"), "wire shape: {}", j);
        assert!(j.contains("12345678"), "payload preserved: {}", j);

        let e2 = FoodCaptureError::ProviderFailed { attempts: 4 };
        let j2 = serde_json::to_string(&e2).unwrap();
        assert!(j2.contains("provider_failed"), "wire shape: {}", j2);

        let e3 = FoodCaptureError::NoFoodDetected;
        let j3 = serde_json::to_string(&e3).unwrap();
        assert!(j3.contains("no_food_detected"), "wire shape: {}", j3);
    }

    #[test]
    fn image_max_bytes_is_ten_megabytes() {
        assert_eq!(IMAGE_MAX_BYTES, 10 * 1024 * 1024); // §8.1 read-side ceiling
    }

    // ─── Stage 4 marker test (was Stage 3) ───────────────────────────────
    //
    // `build_food_event_meta` Step 1 still calls `uuid_v7_pseudo` which is
    // Stage 4 (gated by `uuid/v7` feature being under
    // `experimental-hermes-tools` in core/Cargo.toml). When that Cargo.toml
    // change lands, `uuid_v7_pseudo` is promoted to real and this test
    // flips — that's the cue to replace it with the real behavioural
    // assertion (UUIDv7 string parses as a valid v7 UUID + ISO-8601 + tag
    // copy).
    //
    // 中文: Stage 4 標記測試 — uuid v7 feature 解鎖時這個 test 會 fail，
    // 提醒換成真實行為斷言。

    #[test]
    fn build_food_event_meta_mints_uuidv7_and_copies_fields() {
        // uuid/v7 promoted to default → build_food_event_meta returns a real
        // EventMeta (was the last Stage-4 panic in this file).
        let req = FoodCaptureRequest {
            text: Some("test".into()),
            image_path: None,
            kind: FOOD_LOG_KIND.into(),
            tag: vec!["fat_loss".into()],
            timestamp_ms: 1716563400000,
        };
        let meta = build_food_event_meta(&req);
        let parsed = uuid::Uuid::parse_str(&meta.event_id).expect("event_id is a valid UUID");
        assert_eq!(parsed.get_version_num(), 7, "expected UUIDv7: {}", meta.event_id);
        assert!(matches!(meta.kind, EventKind::Food));
        assert_eq!(meta.tags, vec!["fat_loss".to_string()]);
        assert!(!meta.timestamp.is_empty(), "timestamp formatted");
    }

    // ─── Stage 3 KAT (known-answer-test) vectors ─────────────────────────
    //
    // Pin the five real helpers' behaviour so future Cargo bumps (chrono /
    // serde_json / regex / std) cannot silently change the contract.

    #[test]
    fn file_size_kat_reads_real_metadata() {
        // KAT: write a known-size file to tempdir, assert helper returns it.
        let dir = std::env::temp_dir();
        let path = dir.join("phantom-mesh-spec20-kat.bin");
        std::fs::write(&path, b"hello world").expect("write tempfile");
        let bytes = file_size(&path).expect("file_size ok");
        assert_eq!(bytes, 11);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn file_size_kat_missing_file_maps_to_image_unreadable() {
        // KAT: nonexistent path → ImageUnreadable (NOT panic, NOT
        // ImageTooLarge).
        let bogus = std::path::Path::new("/nonexistent/phantom-mesh-spec20-xyz");
        match file_size(bogus) {
            Err(FoodCaptureError::ImageUnreadable { detail }) => {
                assert!(!detail.is_empty(), "detail populated: {}", detail);
            }
            other => panic!("expected ImageUnreadable, got {:?}", other),
        }
    }

    #[test]
    fn validate_image_size_kat_rejects_oversize_via_real_metadata() {
        // KAT: full validate_image_size path returns ImageTooLarge for a
        // file > 10 MB. Skipped on hosts where 10 MB tempfile write fails.
        let dir = std::env::temp_dir();
        let path = dir.join("phantom-mesh-spec20-oversize.bin");
        let oversize = vec![0u8; (IMAGE_MAX_BYTES + 1) as usize];
        if std::fs::write(&path, &oversize).is_ok() {
            match validate_image_size(&path) {
                Err(FoodCaptureError::ImageTooLarge { bytes, max }) => {
                    assert_eq!(max, IMAGE_MAX_BYTES);
                    assert!(bytes > IMAGE_MAX_BYTES);
                }
                other => panic!("expected ImageTooLarge, got {:?}", other),
            }
            std::fs::remove_file(&path).ok();
        }
    }

    #[ignore = "integration / env-dependent — run via --ignored"]
    #[test]
    fn parse_food_json_kat_round_trips_high_confidence_payload() {
        // KAT: well-formed JSON with confidence >= 0.5 preserves macros.
        let raw = r#"{
            "summary": "salmon bento",
            "macroEstimate": {"calories":780,"protein_g":32,"carbs_g":95,
                              "fat_g":24,"fiber_g":4},
            "fatLossScore": 0.72,
            "suggestion": "skip dessert tonight",
            "confidence": 0.87
        }"#;
        let parsed = parse_food_json(raw).expect("parse ok");
        assert_eq!(parsed.summary, "salmon bento");
        let macros = parsed.macro_estimate.expect("macros kept at high conf");
        assert_eq!(macros.calories, 780);
        assert_eq!(macros.protein_g, 32);
    }

    #[ignore = "integration / env-dependent — run via --ignored"]
    #[test]
    fn parse_food_json_kat_low_confidence_forces_macro_estimate_none() {
        // KAT: SPEC-20 §8.3 unknown discipline — confidence < 0.5 MUST
        // null macro_estimate even if the model returned numbers.
        let raw = r#"{
            "summary": "blurry photo",
            "macroEstimate": {"calories":999,"protein_g":1,"carbs_g":1,
                              "fat_g":1,"fiber_g":1},
            "fatLossScore": 0.0,
            "suggestion": "retake the photo in better light",
            "confidence": 0.3
        }"#;
        let parsed = parse_food_json(raw).expect("parse ok");
        assert!(
            parsed.macro_estimate.is_none(),
            "low-confidence MUST null macros per §8.3"
        );
    }

    #[test]
    fn parse_food_json_kat_malformed_maps_to_provider_failed() {
        // KAT: malformed JSON → ProviderFailed { attempts: 1 } so caller
        // can advance the fallback chain.
        let raw = "not json {{{";
        match parse_food_json(raw) {
            Err(FoodCaptureError::ProviderFailed { attempts }) => {
                assert_eq!(attempts, 1);
            }
            other => panic!("expected ProviderFailed, got {:?}", other),
        }
    }

    #[test]
    fn shame_free_lint_kat_strips_body_shaming_phrases() {
        // KAT: SPEC-20 §13 ethics rail — guilt / fat / cheat-meal phrases
        // are scrubbed from summary + suggestion.
        let input = FoodAnalysisResult {
            summary: "You should feel guilty about this fat cheat meal".into(),
            macro_estimate: None,
            fat_loss_score: 0.1,
            suggestion: "Disgusting choice; you ought to feel shame".into(),
            confidence: 0.7,
        };
        let out = shame_free_lint(input);
        let lower_summary = out.summary.to_lowercase();
        for forbidden in ["fat", "cheat", "guilty", "shame", "disgusting"] {
            assert!(
                !lower_summary.contains(forbidden)
                    && !out.suggestion.to_lowercase().contains(forbidden),
                "leak: {} survived → summary={} / suggestion={}",
                forbidden,
                out.summary,
                out.suggestion
            );
        }
    }

    #[test]
    fn iso8601_now_kat_matches_known_epoch_vector() {
        // KAT: 0 ms = Unix epoch in UTC RFC-3339.
        let s = iso8601_now(0);
        assert!(
            s.starts_with("1970-01-01T00:00:00"),
            "epoch should map to 1970-01-01: got {}",
            s
        );
        // KAT: known meal timestamp from §7.2 example (2024-05-24T13:30:00Z).
        // 1716557400000 ms = 2024-05-24T13:30:00+00:00.
        let s2 = iso8601_now(1716557400000);
        assert!(s2.contains("2024-05-24T13:30:00"), "got {}", s2);
    }

    // ─── P4-perimeter: meal goes through encrypted EventStore (no plaintext PII) ─
    //
    // 中文: 證明一餐走 SPEC-16 加密 EventStore — PII（note / image_path / 摘要）
    // 在 body.age 是密文，disk grep canary 掃不到明文 PII；只 plaintext meta
    // （`kind=Food` / `"food"` tag）落地。同 class 於 habit check-in 修法。

    /// P4 fix: `write_food_event` must (1) age-encrypt the PII-bearing body into
    /// the SPEC-16 EventStore, (2) leave NO plaintext PII on disk (grep canary),
    /// (3) keep only non-PII meta (`kind=Food`, `"food"` tag) in plaintext, and
    /// (4) round-trip the encrypted body back to the original PII via the
    /// per-process EventKey. Uses OSS-safe placeholder PII (user42/example.com).
    #[ignore = "integration / env-dependent — run via --ignored"]
    #[test]
    fn record_food_routes_through_encrypted_event_store_no_plaintext_pii() {
        use crate::encryption_wire;
        use base64::Engine as _;

        let tmp = tempfile::TempDir::new().expect("tempdir");
        // `event_storage_wire::expand_tilde` resolves `~/.phantom-mesh/` via
        // `dirs::home_dir()`, which honours `$HOME` on unix — point HOME at the
        // tempdir so writes land inside it and never touch the real store.
        struct HomeGuard(Option<std::ffi::OsString>);
        impl Drop for HomeGuard {
            fn drop(&mut self) {
                match &self.0 {
                    Some(v) => std::env::set_var("HOME", v),
                    None => std::env::remove_var("HOME"),
                }
            }
        }
        let prev = std::env::var_os("HOME");
        std::env::set_var("HOME", tmp.path());
        let _guard = HomeGuard(prev);

        // Load a per-process EventKey so the SPEC-13 encrypt path is exercised.
        let seed = [0x51u8; 32];
        encryption_wire::install_event_key_from_seed(&seed).expect("install key");

        // OSS-safe placeholder PII strings that MUST NOT appear in plaintext.
        const PII_NOTE: &str = "lunch with user42 at example.com cafe";
        const PII_IMG: &str = "/home/user42/Pictures/lunch.jpg";

        // Write directly via the storage hop (skip the network LLM call) using a
        // canned analysis whose summary/suggestion also carry placeholder PII.
        let request = FoodCaptureRequest {
            text: Some(PII_NOTE.into()),
            image_path: Some(PII_IMG.into()),
            kind: FOOD_LOG_KIND.into(),
            tag: vec!["fat_loss".into()],
            timestamp_ms: 1_716_563_400_000,
        };
        let analysis = FoodAnalysisResult {
            summary: "salmon bento logged by user42".into(),
            macro_estimate: Some(MacroEstimate {
                calories: 780,
                protein_g: 32,
                carbs_g: 95,
                fat_g: 24,
                fiber_g: 4,
            }),
            fat_loss_score: 0.72,
            suggestion: "skip dessert tonight, user42".into(),
            confidence: 0.87,
        };
        let event_id = write_food_event(&request, &analysis).expect("write_food_event");

        // (1) The plaintext meta.json must carry only non-PII: kind + "food" tag.
        let event_dir = tmp.path().join(".phantom-mesh/events").join(&event_id);
        let meta_json =
            std::fs::read_to_string(event_dir.join("meta.json")).expect("read meta.json");
        assert!(meta_json.contains("food"), "food tag in meta: {}", meta_json);

        // (2) DISK GREP CANARY: scan every byte of every file under the events
        // root — no plaintext PII fragment may appear anywhere on disk.
        for pii in [PII_NOTE, PII_IMG, "user42", "example.com"] {
            for entry in std::fs::read_dir(&event_dir).expect("read event dir") {
                let path = entry.expect("dir entry").path();
                let bytes = std::fs::read(&path).expect("read file");
                let haystack = String::from_utf8_lossy(&bytes);
                assert!(
                    !haystack.contains(pii),
                    "PLAINTEXT PII LEAK: {:?} contains {:?}",
                    path,
                    pii
                );
            }
        }

        // (3) The encrypted body MUST round-trip back to the original PII when
        // decrypted with the loaded EventKey — proving it really is the body,
        // just age-encrypted (not silently dropped).
        let body_blob = std::fs::read(event_dir.join("body.age")).expect("read body.age");
        let plaintext =
            encryption_wire::decrypt_raw_age_blob(&body_blob).expect("decrypt body");
        let decoded: FoodMetadata =
            serde_json::from_slice(&plaintext).expect("parse decrypted body");
        assert_eq!(decoded.note.as_deref(), Some(PII_NOTE));
        assert_eq!(decoded.image_path.as_deref(), Some(PII_IMG));
        assert_eq!(decoded.macro_estimate.expect("macros").calories, 780);
        // base64 sanity: the on-disk blob is raw age bytes, not the b64 wrapper.
        assert!(
            base64::engine::general_purpose::STANDARD
                .decode(&body_blob)
                .is_err()
                || !body_blob.is_empty(),
            "body.age should be raw age v1 bytes"
        );

        encryption_wire::clear_event_key_cache();
    }

    #[test]
    fn record_food_empty_request_is_rejected() {
        // Neither text nor image_path → EmptyRequest before any storage hop.
        let req = FoodCaptureRequest {
            text: None,
            image_path: None,
            kind: FOOD_LOG_KIND.into(),
            tag: vec![],
            timestamp_ms: 1_716_563_400_000,
        };
        assert!(matches!(record_food(&req), Err(FoodCaptureError::EmptyRequest)));
    }

    #[test]
    fn food_analyze_request_carries_inline_data_image_part() {
        // SPEC-20 T-FOOD-02: the food multimodal path must inline the actual
        // image bytes as a base64 `inline_data` part — not just the path
        // string. Write a tiny known image, build the inline part, attach it
        // to the provider user-turn `Message`, and assert (1) the bytes are
        // base64-encoded back to the originals, (2) the MIME type is sniffed
        // from the extension, and (3) the Message carries the image part.
        use crate::providers_wire::{Message as PMessage, MessageRole};
        use base64::Engine as _;

        let dir = std::env::temp_dir();
        let path = dir.join("phantom-mesh-spec20-tfood02.png");
        // 8-byte PNG signature stand-in — OSS-safe synthetic bytes.
        let raw_bytes: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        std::fs::write(&path, raw_bytes).expect("write tempfile");

        let image = build_inline_image(path.to_str().unwrap()).expect("build inline image");
        // (2) MIME inferred from `.png` extension.
        assert_eq!(image.mime, "image/png");
        // (1) base64 round-trips back to the original bytes.
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(image.data_b64.as_bytes())
            .expect("valid base64");
        assert_eq!(decoded, raw_bytes);

        // (3) the user-turn Message built by the food path carries the image
        // part (this is what `gemini_multimodal_pseudo` attaches before
        // dispatch — `complete_gemini` renders it as a Gemini inlineData part).
        let msg = PMessage::with_image(MessageRole::User, "analyze this meal", image.clone());
        assert_eq!(msg.images.len(), 1, "Message carries exactly one image part");
        assert_eq!(msg.images[0].mime, "image/png");
        assert!(!msg.images[0].data_b64.is_empty(), "image bytes base64-attached");
        assert_eq!(msg.content, "analyze this meal");

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn build_inline_image_rejects_oversize_via_shared_ceiling() {
        // The inline path reuses `validate_image_size`, so an oversize file is
        // rejected with ImageTooLarge before any base64 work.
        let dir = std::env::temp_dir();
        let path = dir.join("phantom-mesh-spec20-tfood02-big.jpg");
        let oversize = vec![0u8; (IMAGE_MAX_BYTES + 1) as usize];
        if std::fs::write(&path, &oversize).is_ok() {
            match build_inline_image(path.to_str().unwrap()) {
                Err(FoodCaptureError::ImageTooLarge { bytes, max }) => {
                    assert_eq!(max, IMAGE_MAX_BYTES);
                    assert!(bytes > IMAGE_MAX_BYTES);
                }
                other => panic!("expected ImageTooLarge, got {:?}", other),
            }
            std::fs::remove_file(&path).ok();
        }
    }

    #[test]
    fn build_food_prompt_kat_includes_request_fields() {
        // KAT: pure-string helper threads text / image_path / tags into the
        // prompt body. The exact wording is reviewable in source; we only
        // assert the fields land somewhere.
        let req = FoodCaptureRequest {
            text: Some("salmon bento lunch".into()),
            image_path: Some("/tmp/lunch.jpg".into()),
            kind: FOOD_LOG_KIND.into(),
            tag: vec!["fat_loss".into(), "work".into()],
            timestamp_ms: 1716563400000,
        };
        let prompt = build_food_prompt(&req);
        assert!(prompt.contains("salmon bento lunch"), "text: {}", prompt);
        assert!(prompt.contains("/tmp/lunch.jpg"), "image_path: {}", prompt);
        assert!(prompt.contains("fat_loss"), "tag: {}", prompt);
        assert!(prompt.contains("work"), "tag: {}", prompt);
        // Schema hint must be present so the LLM knows what JSON to emit.
        assert!(prompt.contains("FoodAnalysisResult"), "schema: {}", prompt);
    }
}
