// SPEC-22 §7 / §9 / §11 — Capture-habit wire types (single source of truth
// for habit definitions, check-ins, streak rollups + the wire-facing error
// catalog mirror that Tauri / CLI / iOS-widget all share).
//
// Stage 3 (real impl — rusqlite + chrono + cron live): slug lookup / insert /
// query against the `chip_palette` sqlite table for non-PII palette config,
// while check-ins route through the SPEC-16 encrypted EventStore (DRIFT-GUARD
// #01 — the plaintext `habit_checkins` table that leaked the `note` PII is
// gone). 7/30-day window counts, the lenient-mode streak walker (SPEC-22 §8.3),
// the dashboard rollup, AND the `Custom { cron }` cron-expression validator
// (backed by the `cron = "0.15"` crate added to core/Cargo.toml) are now
// real code. Stage 4 markers have all been promoted.
//
// 中文（zh-TW）: 本檔對應 SPEC-22 §7（資料模型，data model）/ §9（API contracts，
// 介面契約）/ §11（error catalog，錯誤目錄）。本檔是 wire（線路） 邊界 — 即
// Tauri command（命令通道）/ CLI（命令列）/ iOS widget（iOS 桌面小工具）共用
// 的 struct shape（資料形狀）。`HabitDefinition` 是 user-defined chip palette
// 條目（使用者自訂的習慣 chip palette，習慣彩盤），`HabitCheckin` 是一次
// tap-to-log（單次打卡記錄），`HabitStreak` 是 lenient（寬鬆）連續打卡彙總，
// `HabitSummary` 是 dashboard（儀表板）一次回多 chip 的 rollup（聚合）。
//
// > 縮寫對照表（acronym table，集中查詢用；本檔首次出現處仍 inline 註中文）：
// > - TS  = TypeScript（微軟強型別 JavaScript 變體）
// > - ts-rs = Rust crate（套件），把 Rust struct 轉成 TS interface
// > - FFI = Foreign Function Interface（外部函式介面，跨語言邊界）
// > - JSON = JavaScript Object Notation（資料序列化格式）
// > - UTC = Coordinated Universal Time（協調世界時，時區基準）
// > - DDL = Data Definition Language（SQL 建表語言）
// > - UI  = User Interface（使用者介面）
// > - CLI = Command-Line Interface（命令列介面）
// > - MCP = Model Context Protocol（模型上下文協議）
// > - chip = 習慣 chip（彩盤上的單顆「水」「咖啡」「戒菸」等習慣按鈕）
// > - palette = chip palette（彩盤；6–12 個 chip 構成的可自訂排列）
// > - streak = 連續打卡天數
//
// TODO Stage 2:
//   - wire `create_habit` 到 `core/src/life_node/habit.rs::palette_set` 的單 chip
//     新增 path；驗 snake_case slug `[a-z0-9_]{1,32}` per SPEC-22 §8.2。
//   - wire `record_checkin` 到 `EventStore::append` 並走 SPEC-13 metadata
//     encrypt；回傳由 `compute_streak` 算出的最新 streak。
//   - wire `list_habits` 到 §9.2 `HabitModule::streak_all` + 30-day window
//     count（last_7d / last_30d）。
//   - wire `compute_streak` 到 §8.3 lenient-mode 演算法（含 grace-until-EOD
//     寬限期 + user-local timezone day boundary）。
//   - 與 `core/src/life_node/event_storage` 對接 — `HabitCheckin` 寫入時
//     metadata 走 SPEC-22 §7.1.3 `HabitMetadata { chip_id, qty, unit,
//     free_text, correction_of }` shape。
//   - 與 SPEC-04 error catalog 對齊：所有 `HabitCaptureError` variant 對應
//     `HABIT-001..HABIT-005` 區段（SPEC-22 §11）。

use serde::{Deserialize, Serialize};
use ts_rs::TS;

// ─── SPEC-16 EventStore routing (drift-guard #01 fix) ────────────────────────
//
// 中文: 把 check-in（打卡）寫入路徑導向 SPEC-16 加密 EventStore（事件儲存），
// 不再寫明文 `~/.phantom-mesh/habits.sqlite`（會洩漏 note 自由備註 PII，違反
// P4 加密邊界）。所有含 PII 的欄位（`note` / `qty` / `source`）都封進 age v1
// 加密 body；plaintext meta（`tags` / `timestamp`）只放非 PII。
//
// SPEC-13 metadata body shape — this is the SPEC-22 §7.1.3 `HabitMetadata`
// JSON blob that rides INSIDE the encrypted `body.age`. Everything here is
// considered PII / sensitive and MUST NOT appear in plaintext on disk.
use crate::event_storage_wire::{
    self, EventKind, EventMeta, EventStoreError, EventStoreQuery,
};

/// SPEC-22 §7.1.3 `HabitMetadata` — the encrypted-body payload for a single
/// habit check-in event. Distinct from the wire-facing `HabitCheckin`: this is
/// the at-rest shape that gets age-encrypted into `body.age` via the SPEC-16
/// EventStore. The slug is duplicated into the plaintext `EventMeta.tags` for
/// queryability; `qty` / `unit` / `free_text` (PII) live ONLY here.
///
/// 中文: 寫入加密 body 的 metadata。`free_text`（自由備註）= `HabitCheckin.note`，
/// 是潛在 PII，所以只在加密 body 出現、絕不寫明文。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct HabitMetadata {
    chip_id: String,
    timestamp_ms: i64,
    free_text: Option<String>,
    source: String,
}

/// Plaintext tag stamped on every habit-checkin event meta so the read path
/// can `query_events` by `kind=Habit` + `tag=habit:<slug>` without decrypting.
/// SPEC-16 §12.1: tags are plaintext and MUST stay PII-free — a slug
/// (`water`, `stretch`) is a non-PII config identifier, safe to expose.
fn habit_tag(slug: &str) -> String {
    format!("habit:{}", slug)
}

// ─── §7.1 HabitFrequency — target cadence（目標頻率） ─────────────────────────

/// Target frequency（目標頻率） that a habit is meant to be performed at.
/// Mirrors the user's intent — actual checkin density may differ; the streak
/// rollup（連續打卡彙總） in `HabitStreak` is the source of truth for "did
/// the user actually do it".
///
/// 中文: 習慣的目標頻率。`Daily` 每天，`Weekly { times }` 一週幾次，`Weekday`
/// 週一至週五，`Custom { cron }` 自訂 cron expression（時間表達式）— Stage 2
/// 才會真正 parse cron，Stage 1 只是字串透傳。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/capture_habit/")]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HabitFrequency {
    /// Every calendar day in the user's local timezone.
    Daily,
    /// `times` checkins per ISO week（國際標準週，週一起算）.
    Weekly { times: u8 },
    /// Mon–Fri only — Sat / Sun do not count toward streak.
    Weekday,
    /// Arbitrary cron string (e.g. `"0 9 * * 1,3,5"`); Stage 2 parses.
    Custom { cron: String },
}

// ─── §7.1 HabitCheckinSource — where the tap came from ──────────────────────

/// Which surface（介面） originated this check-in. Used for telemetry rollups
/// (e.g. "70% of water-logs come from the iOS widget") and for the dedup
/// (deduplication，去重) heuristic when widget + main-app race within the
/// 5-second tolerance window per SPEC-22 §8.5.
///
/// 中文: 這次 check-in（打卡）從哪個 surface 進來。`Manual` 是主程式 UI 點按、
/// `Watch` 是 watchOS / wearOS（穿戴系統）、`Widget` 是 iOS 桌面小工具、
/// `Shortcut` 是 iOS Shortcuts / Android Tasker（自動化捷徑）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/capture_habit/")]
#[serde(rename_all = "snake_case")]
pub enum HabitCheckinSource {
    /// User tapped a chip inside the main desktop / mobile app.
    Manual,
    /// Apple Watch / Wear OS quick action.
    Watch,
    /// iOS WidgetKit / Android AppWidget home-screen tap.
    Widget,
    /// iOS Shortcuts / Android Tasker automation invocation.
    Shortcut,
}

impl HabitCheckinSource {
    /// Stable lower-kebab slug used in logs and the `events.metadata_json`
    /// telemetry field. `Manual` → `"manual"`, etc.
    ///
    /// 中文: 回傳 lower-kebab slug，寫入 events log（事件紀錄）+ metadata。
    pub const fn slug(self) -> &'static str {
        match self {
            HabitCheckinSource::Manual => "manual",
            HabitCheckinSource::Watch => "watch",
            HabitCheckinSource::Widget => "widget",
            HabitCheckinSource::Shortcut => "shortcut",
        }
    }
}

// ─── §7.1.2 HabitDefinition — a single chip palette entry ───────────────────

/// One entry in the user's chip palette（chip 彩盤）— see SPEC-22 §8.1 for
/// the 12 starter chip list (water / coffee / quit_smoke ...). The wire-facing
/// surface intentionally collapses SPEC-22's internal `ChipDef`
/// (`label_zh` / `label_en` / `emoji` / `default_unit` / ...) down to the
/// minimum cross-surface contract: slug + display label + target frequency
/// + tags. Stage 2 will keep the richer `ChipDef` as an internal-only super-
/// set inside `core/src/life_node/habit.rs`.
///
/// 中文: 使用者自訂的 chip palette（彩盤）單一條目。`slug`（短代號）唯一識別，
/// `label`（顯示名）給 UI 用，`target_frequency`（目標頻率）告訴系統「user
/// 希望多久做一次」，`tags`（標籤）給 dashboard 分群用，`created_at`（建立時間）
/// 走 ISO-8601 字串。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/capture_habit/")]
#[serde(rename_all = "camelCase")]
pub struct HabitDefinition {
    /// snake_case slug（蛇形短代號）— must match `[a-z0-9_]{1,32}` per
    /// SPEC-22 §8.2. Unique within a user's palette.
    pub slug: String,
    /// Display label（顯示名） — UTF-8, ≤ 24 chars recommended.
    /// Stage 2 will branch into per-locale `label_zh` / `label_en` storage,
    /// but the wire boundary only carries one already-localized string.
    pub label: String,
    /// How often the user intends to do this habit.
    pub target_frequency: HabitFrequency,
    /// Free-form tag list（標籤清單）for grouping in dashboards (e.g.
    /// `["health", "morning"]`).
    pub tags: Vec<String>,
    /// ISO-8601 UTC（協調世界時）timestamp this chip was added to palette.
    pub created_at: String,
}

// ─── §7.1.3 HabitCheckin — one tap-to-log event ─────────────────────────────

/// A single instance of "I did this habit" — written into `event_storage`
/// (encrypted via SPEC-13) with `kind='habit'`. The wire-facing surface keeps
/// the slug (referring back to a `HabitDefinition`), the millisecond epoch
/// timestamp, an optional free-text note, and which surface the tap came
/// from. Quantitative metadata (qty / unit) lives inside the SPEC-22 §7.1.3
/// `HabitMetadata` JSON blob — Stage 2 will marshal between the two; Stage 1
/// keeps the wire-facing struct deliberately minimal.
///
/// 中文: 單次 check-in（打卡）。`habit_slug` 連回 `HabitDefinition.slug`，
/// `timestamp_ms` 用 UTC 毫秒整數（避過浮點誤差），`note`（備註）可選，
/// `source`（來源介面）給 telemetry 用。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/capture_habit/")]
#[serde(rename_all = "camelCase")]
pub struct HabitCheckin {
    /// References `HabitDefinition.slug`. Stage 2 validates existence.
    pub habit_slug: String,
    /// UTC epoch milliseconds（協調世界時毫秒整數）at which the tap
    /// happened — i64 to match `EventRow.ts_ms` in SPEC-16 §7.1.
    pub timestamp_ms: i64,
    /// Optional free-text note（自由備註）— if present, stored inside the
    /// SPEC-22 `HabitMetadata.free_text` (encrypted).
    pub note: Option<String>,
    /// Which surface originated the tap (telemetry + dedup heuristic).
    pub source: HabitCheckinSource,
}

// ─── §7.1.4 HabitStreak — lenient streak rollup ─────────────────────────────

/// Streak rollup（連續打卡彙總）for a single chip. Computed by Stage 2 via
/// the lenient-mode algorithm in SPEC-22 §8.3: today's checkin extends the
/// streak, a missed day enters a "grace-until-end-of-day" buffer before
/// resetting to 0 at the user-local 23:59:59 boundary.
///
/// 中文: 單一 chip 的連續打卡彙總。`current_streak` 是目前連續天數，
/// `longest_streak` 是歷史最長，`last_checkin_at` 是該 chip 最後一次 log 的
/// ISO-8601 UTC 時間（無 log 過則 `None`）。Stage 1 是 wire shape only — Stage
/// 2 才實作 lenient（寬鬆）演算法。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/capture_habit/")]
#[serde(rename_all = "camelCase")]
pub struct HabitStreak {
    /// References `HabitDefinition.slug`.
    pub habit_slug: String,
    /// Current consecutive-day count (u16: 65535 days ≈ 179 years — ample).
    pub current_streak: u16,
    /// Longest historical consecutive-day count.
    pub longest_streak: u16,
    /// ISO-8601 UTC timestamp of the most recent checkin; `None` if never.
    pub last_checkin_at: Option<String>,
}

// ─── §7.1.5 HabitSummary — dashboard rollup（儀表板聚合） ─────────────────────

/// Multi-window summary for the dashboard's "habit cards" — one row per chip.
/// Returned by `list_habits()`; intended for `phantom habit streak` (CLI) +
/// the macOS menu-bar dropdown + the iOS main app "habits" tab.
///
/// 中文: 儀表板用的多窗格 rollup（聚合）。`last_7d_count` 是過去 7 天內
/// 這個 chip 被 log 過幾次，`last_30d_count` 同理過去 30 天，`last_checkin_at`
/// 是最後一次 check-in 的 ISO-8601 時間（或 `None`），`streak` 內嵌
/// `HabitStreak`（streak 子結構）。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/capture_habit/")]
#[serde(rename_all = "camelCase")]
pub struct HabitSummary {
    /// References `HabitDefinition.slug`.
    pub habit_slug: String,
    /// Count of check-ins in the past 7 days（過去 7 天打卡次數）.
    pub last_7d_count: u32,
    /// Count of check-ins in the past 30 days.
    pub last_30d_count: u32,
    /// ISO-8601 UTC of the most recent checkin; `None` if never.
    pub last_checkin_at: Option<String>,
    /// Embedded streak rollup（連續打卡彙總）for this chip.
    pub streak: HabitStreak,
}

// ─── §11 HabitCaptureError — error catalog wire mirror ──────────────────────

/// Wire-facing error variants for capture-habit. Mirrors SPEC-22 §11 (which
/// references SPEC-04-FOUNDATION-error-catalog `HABIT-001..HABIT-005`).
/// The internal `core::life_node::habit::HabitError` keeps the richer Rust-
/// only shape (e.g. wrapping `event_storage::StoreError`) — Stage 2 will add
/// a `From` mapping between the two.
///
/// 中文: SPEC-22 §11 error catalog（錯誤目錄）的 wire-facing 鏡像。每個 variant
/// 對應 SPEC-04 `HABIT-XXX` 一個 code。serde（序列化框架）用 `{"code": "..."}`
/// tag 形式，UI 端可直接 switch on `code` 字串。
#[derive(Debug, Clone, Serialize, Deserialize, TS, thiserror::Error)]
#[ts(export, export_to = "../../app/src/lib/generated/capture_habit/")]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum HabitCaptureError {
    /// HABIT-001 — chip_id 已存在於 palette（彩盤）.
    #[error("habit.chip_id_conflict: {slug}")]
    ChipIdConflict { slug: String },
    /// HABIT-002 — 找不到指定 chip_id（slug 未註冊）.
    #[error("habit.chip_not_found: {slug}")]
    ChipNotFound { slug: String },
    /// HABIT-003 — palette size 不在 6..=12 範圍內（SPEC-22 §9.2 contract）.
    #[error("habit.palette_size_out_of_range: got {got}, want 6..=12")]
    PaletteSizeOutOfRange { got: u32 },
    /// HABIT-004 — chip_id 非合法 snake_case slug（不符 `[a-z0-9_]{1,32}`）.
    #[error("habit.invalid_slug: {slug}")]
    InvalidSlug { slug: String },
    /// HABIT-005 — underlying event_storage / SPEC-13 encrypt failure.
    #[error("habit.store: {detail}")]
    Store { detail: String },
    /// HABIT-006 — `HabitFrequency::Custom { cron }` 帶了無法解析的 cron
    /// expression（時間表達式）。`expr` 是原 user 輸入字串、`detail` 是
    /// `cron` crate 回的 parse error 解釋。
    #[error("habit.invalid_cron: {expr} ({detail})")]
    InvalidCron { expr: String, detail: String },
}

// ─── §9.2 Stub helpers (Stage 2 implements; Stage 1 leaves `unimplemented!()`) ─

/// Create / register a new habit chip in the user's palette.
///
/// Stage 2 must: (a) validate `def.slug` matches `[a-z0-9_]{1,32}`,
/// (b) check uniqueness against existing palette entries, (c) persist to the
/// `chip_palette` sqlite table per SPEC-22 §7.1.1.
///
/// Idempotency: re-calling with the same `slug` returns `ChipIdConflict` —
/// caller should treat that as "already exists" and skip.
///
/// 中文: 新增一個 habit chip（習慣按鈕）到使用者的 palette（彩盤）.
/// Stage 2 要做：(a) slug 格式驗證、(b) 唯一性檢查、(c) 寫入 sqlite
/// `chip_palette` 表。重複 slug 回 ChipIdConflict（呼叫端視為「已存在」略過）.
/// Validate a chip slug against SPEC-22 §8.2: `[a-z0-9_]{1,32}` — 1–32 chars,
/// lowercase ascii alphanumeric + underscore only. Returns `InvalidSlug`
/// (HABIT-004) on any violation. This is the single canonical gate; both the
/// CLI and Tauri create paths route through `create_habit`, so enforcing it
/// there keeps malformed slugs out of sqlite + the plaintext event tags.
fn validate_slug(slug: &str) -> Result<(), HabitCaptureError> {
    let ok = (1..=32).contains(&slug.len())
        && slug
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    if ok {
        Ok(())
    } else {
        Err(HabitCaptureError::InvalidSlug {
            slug: slug.to_string(),
        })
    }
}

pub fn create_habit(def: &HabitDefinition) -> Result<(), HabitCaptureError> {
    // Step 0: validate the slug SHAPE before anything touches sqlite. SPEC-22
    // §8.2 / §11 (HABIT-004): chip_id MUST match `[a-z0-9_]{1,32}` (snake_case,
    // 1–32 chars, only lowercase ascii alnum + underscore). Without this the
    // `InvalidSlug` variant was dead code and malformed slugs (e.g. "BadCase!!",
    // 64-char) silently persisted into chip_palette and flowed into the
    // plaintext `EventMeta.tags` / `events.metadata_json`.
    validate_slug(&def.slug)?;
    // Step 1: enforce slug uniqueness against the `chip_palette` sqlite table —
    // any pre-existing row with the same slug short-circuits to `ChipIdConflict`
    // per SPEC-22 §11 (HABIT-001) so callers can treat the call as idempotent.
    if habit_lookup_pseudo(&def.slug)?.is_some() {
        return Err(HabitCaptureError::ChipIdConflict {
            slug: def.slug.clone(),
        });
    }
    // Step 2: when the target frequency is `Custom { cron }`, validate the cron
    // string up-front via the `cron` crate so a malformed schedule fails fast
    // here instead of inside the timer task SPEC-22 §8.3 spins up later.
    if let HabitFrequency::Custom { cron } = &def.target_frequency {
        parse_cron_schedule(cron)?;
    }
    // Step 3: persist the new chip row into the `chip_palette` sqlite table,
    // mapping any rusqlite failure into `HabitCaptureError::Store` per the
    // SPEC-22 §11 (HABIT-005) error catalog convention.
    habit_insert_pseudo(def)
}

/// Record one check-in（打卡）against an existing habit and return the freshly
/// recomputed streak rollup.
///
/// Stage 2 must: (a) verify `checkin.habit_slug` exists in palette, (b) append
/// an encrypted `kind='habit'` row to `event_storage` with the SPEC-22 §7.1.3
/// `HabitMetadata` JSON, (c) recompute and return `HabitStreak` via the
/// lenient algorithm in SPEC-22 §8.3.
///
/// 中文: 對既有 habit 記錄一次 check-in（打卡），同步回傳重算後的 streak.
/// Stage 2 要做：(a) slug 存在性檢查、(b) 加密寫入 event_storage、(c) 重算
/// streak 並回傳（呼叫端可直接更新 UI 上的「12 天 🔥」角標）.
pub fn record_checkin(checkin: &HabitCheckin) -> Result<HabitStreak, HabitCaptureError> {
    // Step 1: confirm the slug references a real palette entry — an unknown
    // slug short-circuits to `ChipNotFound` (HABIT-002) before we touch the
    // event_storage write path.
    if habit_lookup_pseudo(&checkin.habit_slug)?.is_none() {
        return Err(HabitCaptureError::ChipNotFound {
            slug: checkin.habit_slug.clone(),
        });
    }
    // Step 2: append an encrypted `kind='habit'` row into event_storage with
    // the SPEC-22 §7.1.3 `HabitMetadata` JSON blob attached; rusqlite / SPEC-13
    // encrypt failures map into `HabitCaptureError::Store` (HABIT-005).
    checkin_insert_pseudo(checkin)?;
    // Step 3: recompute and return the freshly-updated streak rollup so the
    // caller can paint the UI badge ("12 天 🔥") without a follow-up query.
    compute_streak(&checkin.habit_slug)
}

/// Return a dashboard summary for every chip in the palette — one
/// `HabitSummary` per chip, including 7-day / 30-day counts + embedded streak.
///
/// Stage 2 must: scan `event_storage` once (filtered by `kind='habit'`),
/// group by chip slug, derive the 7/30-day counts + `last_checkin_at`, and
/// inline-call `compute_streak` for each. Empty-palette returns `Ok(vec![])`.
///
/// 中文: 回傳 dashboard（儀表板）用的多窗格 summary — 每個 chip 一筆。
/// Stage 2 一次 sweep event_storage、group by slug、算 7/30 天 count + streak。
/// Palette 為空時回空 vec（不是 error）。
pub fn list_habits() -> Result<Vec<HabitSummary>, HabitCaptureError> {
    // Step 1: load every row from the `chip_palette` sqlite table — empty
    // palette returns Ok(vec![]) so the dashboard renders an empty grid rather
    // than surfacing an error to the user.
    let defs = habit_query_pseudo()?;
    // Step 2: for each palette entry, count the last-7-day and last-30-day
    // check-ins from event_storage and record the most recent `last_checkin_at`
    // ISO-8601 UTC timestamp; collected into the `HabitSummary` wire shape.
    let mut summaries = Vec::with_capacity(defs.len());
    for def in &defs {
        let (last_7d_count, last_30d_count, last_checkin_at) =
            checkin_window_counts_pseudo(&def.slug)?;
        // Step 3: nest the freshly-computed streak rollup so the caller does
        // not have to issue a second round-trip per chip just to render the
        // "🔥 N 天" badge.
        let streak = compute_streak(&def.slug)?;
        summaries.push(HabitSummary {
            habit_slug: def.slug.clone(),
            last_7d_count,
            last_30d_count,
            last_checkin_at,
            streak,
        });
    }
    Ok(summaries)
}

/// Compute the lenient-mode streak rollup for one chip.
///
/// Stage 2 must implement SPEC-22 §8.3 verbatim:
///   - day boundary = user's local timezone start-of-day,
///   - today with ≥ 1 log → `current_streak = yesterday + 1`,
///   - today with 0 log → keep yesterday's count until local 23:59:59
///     (grace-until-end-of-day),
///   - one full empty day → reset to 0,
///   - `longest_streak` = max consecutive-day run over full history,
///   - `last_checkin_at` = max `ts_ms` formatted as ISO-8601 UTC string.
///
/// 中文: 算單一 chip 的 lenient（寬鬆模式）streak。Stage 2 嚴格照 SPEC-22 §8.3
/// 演算法 — 以 user 本地時區的 start-of-day 為界、含 grace-until-end-of-day
/// 寬限期、空白整天歸 0。slug 不存在回 ChipNotFound.
pub fn compute_streak(habit_slug: &str) -> Result<HabitStreak, HabitCaptureError> {
    // Step 1: pull every check-in row for this slug from event_storage in
    // descending `timestamp_ms` order — newest first lets us walk the local-
    // day buckets straight into the lenient algorithm without a second sort.
    let rows = checkin_query_pseudo(habit_slug)?;
    // Step 2: bucket the rows by user-local calendar day, walk consecutive
    // days starting from "today" honoring the SPEC-22 §8.3 grace-until-EOD
    // window, and track both the live `current_streak` and the historical
    // `longest_streak` over the whole returned set.
    let (current_streak, longest_streak, last_checkin_at) = streak_walk_pseudo(&rows)?;
    // Step 3: emit the rolled-up `HabitStreak` wire row — the UI binds
    // `current_streak` to the badge, `longest_streak` to the achievements
    // chip, and `last_checkin_at` to the "last logged at" subtitle.
    Ok(HabitStreak {
        habit_slug: habit_slug.to_string(),
        current_streak,
        longest_streak,
        last_checkin_at,
    })
}

// ─── Stage 3 helpers — real rusqlite + chrono impl ───────────────────────────
//
// Per docs/superpowers/SPEC-TO-CODE-PLAYBOOK.md the seven Stage 2 pseudocode
// stubs below were promoted in this Stage 3 commit:
//   • `rusqlite` (already a non-optional core dep — used by tasks::store +
//     experimental-hermes-memory) backs the chip_palette + habit_checkins
//     tables.
//   • `chrono` (non-optional since E002 Task 8) backs the day-bucket walker
//     in `streak_walk_pseudo`.
// The only helper still on a Stage 4 marker is `cron_parse_pseudo` — the
// `cron` crate is intentionally NOT in core/Cargo.toml (SPEC-22 §8.3 has
// not picked an implementation; pulling one in here would be a Stage 4
// follow-up).
//
// Schema is provisioned lazily inside `open_habits_db` via `CREATE TABLE IF
// NOT EXISTS`, so the first call on a fresh `~/.phantom-mesh/habits.sqlite`
// is the migration. Keeps SPEC-22 §7.1.1 single-file DDL.

/// Resolve `~/.phantom-mesh/habits.sqlite`, open (or create) the sqlite file,
/// and ensure both schema tables exist. Centralised so every helper sees the
/// same connection-handle shape + the same lazy-migration behaviour.
fn open_habits_db() -> Result<rusqlite::Connection, HabitCaptureError> {
    let path = home_dir_join(".phantom-mesh/habits.sqlite")
        .ok_or_else(|| {
            eprintln!("[phantom-habit] home dir unavailable (HOME unset + no home_dir)");
            HabitCaptureError::Store { detail: "home dir unavailable".to_string() }
        })?;
    eprintln!("[phantom-habit] habits.sqlite -> {}", path.display());
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            eprintln!("[phantom-habit] mkdir {} failed: {}", parent.display(), e);
            HabitCaptureError::Store { detail: format!("mkdir {}: {}", parent.display(), e) }
        })?;
    }
    let conn = rusqlite::Connection::open(&path).map_err(|e| {
        eprintln!("[phantom-habit] open {} failed: {}", path.display(), e);
        HabitCaptureError::Store { detail: format!("open habits.sqlite: {}", e) }
    })?;
    // DRIFT-GUARD #01: only the `chip_palette` config table lives in this
    // plaintext sqlite (slug / label / frequency / tags are non-PII palette
    // config per SPEC-16 §12.1). The `habit_checkins` table — which carried the
    // `note` free-text PII in plaintext — has been REMOVED; check-ins now route
    // through the SPEC-16 age-encrypted EventStore (see `checkin_insert_pseudo`).
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS chip_palette (\
            slug TEXT PRIMARY KEY, \
            label TEXT NOT NULL, \
            target_frequency_json TEXT NOT NULL, \
            tags_json TEXT NOT NULL, \
            created_at TEXT NOT NULL\
         );",
    )
    .map_err(|e| HabitCaptureError::Store {
        detail: format!("migrate habits.sqlite: {}", e),
    })?;
    Ok(conn)
}

/// `~/<rel>` resolver. Prefer the `$HOME` env var DIRECTLY (the Tauri app sets
/// it to the Android app-sandbox dir at startup — see app/src-tauri/src/lib.rs;
/// `dirs::home_dir()` does NOT reliably honour a runtime-set `$HOME` on Android,
/// which left `~/.phantom-mesh/habits.sqlite` unresolvable → `habit.store`
/// "寫入失敗"). Fall back to `dirs::home_dir()` on desktop where `$HOME` may be
/// unset. Returns `None` only when neither yields a non-empty path.
fn home_dir_join(rel: &str) -> Option<std::path::PathBuf> {
    let base = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(dirs::home_dir);
    base.map(|h| h.join(rel))
}

/// Look up a chip palette entry by slug from the `chip_palette` sqlite table.
/// Returns `Ok(Some(def))` if found, `Ok(None)` if not, and maps any rusqlite
/// failure into `HabitCaptureError::Store` per SPEC-22 §11 (HABIT-005).
fn habit_lookup_pseudo(slug: &str) -> Result<Option<HabitDefinition>, HabitCaptureError> {
    let conn = open_habits_db()?;
    let mut stmt = conn
        .prepare(
            "SELECT slug, label, target_frequency_json, tags_json, created_at \
             FROM chip_palette WHERE slug = ?1 LIMIT 1",
        )
        .map_err(|e| HabitCaptureError::Store {
            detail: format!("prepare lookup: {}", e),
        })?;
    let mut rows = stmt
        .query(rusqlite::params![slug])
        .map_err(|e| HabitCaptureError::Store {
            detail: format!("execute lookup: {}", e),
        })?;
    let row = rows.next().map_err(|e| HabitCaptureError::Store {
        detail: format!("fetch lookup: {}", e),
    })?;
    let Some(row) = row else {
        return Ok(None);
    };
    Ok(Some(row_to_habit_definition(row)?))
}

/// Parse a cron expression into a validated `cron::Schedule` so a malformed
/// `Custom { cron }` fails fast at `create_habit` time rather than inside the
/// SPEC-22 §8.3 timer task. Backed by `cron = "0.15"` (added to
/// `core/Cargo.toml` for SPEC-22). The schedule object is dropped here — this
/// is a validate-only gate; the SPEC-22 §8.3 timer task will re-parse from
/// the persisted string when it actually schedules ticks.
///
/// Errors map to `HabitCaptureError::InvalidCron` (HABIT-006) carrying both
/// the original expression and the `cron` crate's parse-error detail so the
/// UI can surface a precise message ("expected 6 or 7 fields, got 5") to the
/// user instead of a generic failure.
fn parse_cron_schedule(expr: &str) -> Result<cron::Schedule, HabitCaptureError> {
    use std::str::FromStr;
    cron::Schedule::from_str(expr).map_err(|e| HabitCaptureError::InvalidCron {
        expr: expr.to_string(),
        detail: e.to_string(),
    })
}

/// Insert a freshly-validated `HabitDefinition` row into the `chip_palette`
/// sqlite table. Rusqlite failures map to `HabitCaptureError::Store` per
/// SPEC-22 §11 (HABIT-005). The frequency + tags are serialised to JSON so
/// the wire-shape enum survives unchanged across DB round trips.
fn habit_insert_pseudo(def: &HabitDefinition) -> Result<(), HabitCaptureError> {
    let conn = open_habits_db()?;
    let freq_json = serde_json::to_string(&def.target_frequency).map_err(|e| {
        HabitCaptureError::Store {
            detail: format!("serialize frequency: {}", e),
        }
    })?;
    let tags_json = serde_json::to_string(&def.tags).map_err(|e| HabitCaptureError::Store {
        detail: format!("serialize tags: {}", e),
    })?;
    conn.execute(
        "INSERT INTO chip_palette (slug, label, target_frequency_json, tags_json, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            def.slug,
            def.label,
            freq_json,
            tags_json,
            def.created_at,
        ],
    )
    .map(|_| ())
    .map_err(|e| HabitCaptureError::Store {
        detail: format!("insert chip_palette: {}", e),
    })
}

/// Append a `kind=Habit` check-in to the SPEC-16 encrypted EventStore.
///
/// DRIFT-GUARD #01 FIX: the previous Stage 3 implementation wrote a PLAINTEXT
/// `~/.phantom-mesh/habits.sqlite` row (including the `note` free-text PII),
/// bypassing the SPEC-16 encrypted EventStore — a P4-perimeter leak. This now
/// routes through `event_storage_wire::write_event` so the PII-bearing
/// `HabitMetadata` body is age-encrypted at rest (SPEC-13), while only non-PII
/// meta (`kind` / `timestamp` / `habit:<slug>` tag) stays in plaintext.
///
/// 中文: 把打卡寫進加密 EventStore（事件儲存）— 含 PII 的 `note`（備註）封進
/// age v1 加密 body，不再寫明文 sqlite。SPEC-13 encrypt 失敗 → HABIT-005 Store。
fn checkin_insert_pseudo(checkin: &HabitCheckin) -> Result<(), HabitCaptureError> {
    // Step 1: build the SPEC-22 §7.1.3 `HabitMetadata` body — carries the
    // potentially-PII `free_text` (note) + source; this whole blob is encrypted.
    let body = HabitMetadata {
        chip_id: checkin.habit_slug.clone(),
        timestamp_ms: checkin.timestamp_ms,
        free_text: checkin.note.clone(),
        source: checkin.source.slug().to_string(),
    };
    let plaintext = serde_json::to_vec(&body).map_err(|e| HabitCaptureError::Store {
        detail: format!("serialize habit metadata: {}", e),
    })?;
    // Step 2: age-encrypt the body against the per-process EventKey (SPEC-13)
    // so nothing PII-bearing ever touches the disk in plaintext.
    let encrypted_body = encrypt_to_event_key(&plaintext)?;
    // Step 3: assemble plaintext-safe `EventMeta` — UUIDv7 id (time-ordered),
    // canonical UTC RFC-3339 timestamp, `kind=Habit`, and a `habit:<slug>` tag
    // for queryability. NO PII in meta per SPEC-16 §12.1.
    let meta = EventMeta {
        event_id: uuid::Uuid::now_v7().to_string(),
        timestamp: event_storage_wire::ts_ms_to_rfc3339_utc(checkin.timestamp_ms),
        kind: EventKind::Habit,
        tags: vec!["habit".to_string(), habit_tag(&checkin.habit_slug)],
    };
    // Step 4: append to the encrypted EventStore; map any STORE-* failure to
    // the HABIT-005 catalog entry.
    event_storage_wire::write_event(&meta, &encrypted_body, None)
        .map(|_| ())
        .map_err(store_err_to_habit)
}

/// Age-encrypt plaintext bytes against the per-process EventKey (SPEC-13). The
/// EventStore body is the RAW age v1 blob (what `decrypt_raw_age_blob` expects),
/// so we strip the base64 transport layer that `encrypt_event` adds for its
/// JSON envelope. A missing EventKey (vault locked) surfaces as
/// `DecryptionUnavailable` → HABIT-005 Store, never a panic.
fn encrypt_to_event_key(plaintext: &[u8]) -> Result<Vec<u8>, HabitCaptureError> {
    use base64::Engine as _;
    let key = crate::encryption_wire::lookup_or_derive_event_key().ok_or_else(|| {
        HabitCaptureError::Store {
            detail: "EventKey not loaded (vault locked)".to_string(),
        }
    })?;
    let identity = crate::encryption_wire::event_key_to_age_identity(&key).map_err(|e| {
        HabitCaptureError::Store {
            detail: format!("derive age identity: {:?}", e),
        }
    })?;
    let recipient = crate::encryption_wire::derive_recipient_from_identity(&identity);
    let envelope =
        crate::encryption_wire::encrypt_event(plaintext, &recipient).map_err(|e| {
            HabitCaptureError::Store {
                detail: format!("age encrypt: {:?}", e),
            }
        })?;
    base64::engine::general_purpose::STANDARD
        .decode(envelope.ciphertext_b64.as_bytes())
        .map_err(|e| HabitCaptureError::Store {
            detail: format!("decode age blob: {}", e),
        })
}

/// Map a SPEC-16 `EventStoreError` into the SPEC-22 §11 HABIT-005 `Store`
/// catalog entry, preserving the underlying STORE-* detail string.
fn store_err_to_habit(e: EventStoreError) -> HabitCaptureError {
    HabitCaptureError::Store {
        detail: format!("event_storage: {}", e),
    }
}

/// Load every row from the `chip_palette` sqlite table (one entry per chip).
/// Empty palette returns `Ok(vec![])` so the dashboard renders an empty grid
/// rather than surfacing an error.
fn habit_query_pseudo() -> Result<Vec<HabitDefinition>, HabitCaptureError> {
    let conn = open_habits_db()?;
    let mut stmt = conn
        .prepare(
            "SELECT slug, label, target_frequency_json, tags_json, created_at \
             FROM chip_palette ORDER BY created_at ASC",
        )
        .map_err(|e| HabitCaptureError::Store {
            detail: format!("prepare query: {}", e),
        })?;
    let mut rows = stmt
        .query([])
        .map_err(|e| HabitCaptureError::Store {
            detail: format!("execute query: {}", e),
        })?;
    let mut out: Vec<HabitDefinition> = Vec::new();
    while let Some(row) = rows.next().map_err(|e| HabitCaptureError::Store {
        detail: format!("fetch query: {}", e),
    })? {
        out.push(row_to_habit_definition(row)?);
    }
    Ok(out)
}

/// Count check-ins for the given slug within the last 7-day and last 30-day
/// windows and return the most-recent `last_checkin_at` ISO-8601 UTC string.
/// Window boundaries use `chrono::Utc::now() - Duration::days(N)` so the
/// math is timezone-stable.
///
/// DRIFT-GUARD #01: reads from the encrypted EventStore (via
/// `checkin_query_pseudo`), not the removed plaintext `habit_checkins` table.
fn checkin_window_counts_pseudo(
    slug: &str,
) -> Result<(u32, u32, Option<String>), HabitCaptureError> {
    let rows = checkin_query_pseudo(slug)?;
    let now = chrono::Utc::now();
    let cutoff_7d = (now - chrono::Duration::days(7)).timestamp_millis();
    let cutoff_30d = (now - chrono::Duration::days(30)).timestamp_millis();

    let last_7d_count = rows.iter().filter(|r| r.timestamp_ms >= cutoff_7d).count() as u32;
    let last_30d_count = rows.iter().filter(|r| r.timestamp_ms >= cutoff_30d).count() as u32;
    let last_checkin_at = rows
        .iter()
        .map(|r| r.timestamp_ms)
        .max()
        .and_then(ts_ms_to_rfc3339);

    Ok((last_7d_count, last_30d_count, last_checkin_at))
}

/// Load every check-in for `slug` from the SPEC-16 encrypted EventStore,
/// ordered `timestamp_ms` DESC so the streak walker can stream newest-first.
///
/// DRIFT-GUARD #01: the plaintext `habit_checkins` table is gone — check-ins
/// now live as `kind=Habit` events with the PII-bearing body age-encrypted.
/// Reading requires the per-process EventKey (vault unlocked); when it is not
/// loaded the EventStore returns `DecryptionUnavailable`, mapped to HABIT-005.
fn checkin_query_pseudo(slug: &str) -> Result<Vec<HabitCheckin>, HabitCaptureError> {
    let query = EventStoreQuery {
        date_iso: None,
        kind: Some(EventKind::Habit),
        tag: Some(habit_tag(slug)),
        // SPEC-16 §7.1.5 hard cap; ample for a single chip's history window.
        limit: Some(1000),
        offset: None,
    };
    let records = event_storage_wire::query_events(&query).map_err(store_err_to_habit)?;
    let mut out: Vec<HabitCheckin> = Vec::with_capacity(records.len());
    for rec in &records {
        // Defensive: the kind+tag filter already narrows to this slug, but
        // re-confirm the tag in case of a future shared-tag collision.
        if !rec.meta.tags.iter().any(|t| t == &habit_tag(slug)) {
            continue;
        }
        let timestamp_ms = event_storage_wire::parse_ts_to_utc_ms(&rec.meta.timestamp)
            .map_err(store_err_to_habit)?;
        // Streak math only needs slug + timestamp (both plaintext-safe). The
        // encrypted body (note / source) is NOT decrypted here — keeping the
        // read path PII-free and decrypt-cost-free for the dashboard.
        out.push(HabitCheckin {
            habit_slug: slug.to_string(),
            timestamp_ms,
            note: None,
            source: HabitCheckinSource::Manual,
        });
    }
    // EventStore sorts ascending; the streak walker wants newest-first.
    out.sort_by(|a, b| b.timestamp_ms.cmp(&a.timestamp_ms));
    Ok(out)
}

/// Walk the descending-`timestamp_ms` check-in rows, bucket them by user-local
/// calendar day, and apply the SPEC-22 §8.3 lenient streak algorithm.
///
/// Day boundary: this Stage 3 implementation uses **UTC** day boundaries — a
/// follow-up will plug in the user's local timezone via `chrono-tz` once the
/// SPEC-12 user-pref surface settles. The lenient grace-until-EOD window is
/// preserved by treating "no checkin today" as a soft state: the streak only
/// resets when a full empty day has passed (today + yesterday both empty).
fn streak_walk_pseudo(
    rows: &[HabitCheckin],
) -> Result<(u16, u16, Option<String>), HabitCaptureError> {
    use chrono::{DateTime, Datelike, NaiveDate, Utc};

    if rows.is_empty() {
        return Ok((0, 0, None));
    }

    // Step 1: bucket into a set of unique UTC calendar dates.
    let mut day_set: std::collections::BTreeSet<NaiveDate> = std::collections::BTreeSet::new();
    let mut max_ts_ms: i64 = i64::MIN;
    for r in rows {
        max_ts_ms = max_ts_ms.max(r.timestamp_ms);
        let dt =
            DateTime::<Utc>::from_timestamp_millis(r.timestamp_ms).ok_or_else(|| {
                HabitCaptureError::Store {
                    detail: format!("invalid timestamp_ms: {}", r.timestamp_ms),
                }
            })?;
        day_set.insert(NaiveDate::from_ymd_opt(dt.year(), dt.month(), dt.day()).expect(
            "chrono guarantees valid Y/M/D from a valid DateTime — unreachable",
        ));
    }

    // Step 2: current_streak walks back from today (or the most-recent logged
    // day if today is missing — grace-until-EOD). Day-by-day decrement; the
    // moment we hit a gap, stop counting.
    let today = Utc::now().date_naive();
    let mut cursor = today;
    let mut current: u16 = 0;
    // Grace window: if `today` is not in the set but `today - 1` IS, allow
    // the cursor to start from `today - 1` (the spec's "today with 0 log →
    // keep yesterday's count until local 23:59:59" rule, UTC-approximated).
    if !day_set.contains(&today) {
        let yesterday = today.pred_opt().unwrap_or(today);
        if day_set.contains(&yesterday) {
            cursor = yesterday;
        } else {
            // Two empty days in a row → current streak is 0.
            cursor = today; // sentinel; the loop below will exit immediately.
        }
    }
    while day_set.contains(&cursor) && current < u16::MAX {
        current += 1;
        cursor = match cursor.pred_opt() {
            Some(d) => d,
            None => break,
        };
    }

    // Step 3: longest_streak — walk the sorted-ascending day set tracking
    // consecutive-day runs.
    let mut longest: u16 = 0;
    let mut run: u16 = 0;
    let mut prev: Option<NaiveDate> = None;
    for d in &day_set {
        match prev {
            Some(p) if d.signed_duration_since(p).num_days() == 1 => {
                run = run.saturating_add(1);
            }
            _ => {
                run = 1;
            }
        }
        longest = longest.max(run);
        prev = Some(*d);
    }
    // current may exceed the historical-run reading on the same data (it
    // walks through "today's grace") — take the larger of the two.
    longest = longest.max(current);

    let last_checkin_at = ts_ms_to_rfc3339(max_ts_ms);
    Ok((current, longest, last_checkin_at))
}

// ─── Stage 3 inner helpers (row decoders + ts formatting) ────────────────────

/// Materialise a `chip_palette` row into the wire-shape `HabitDefinition`.
fn row_to_habit_definition(row: &rusqlite::Row<'_>) -> Result<HabitDefinition, HabitCaptureError> {
    let slug: String = row.get(0).map_err(|e| HabitCaptureError::Store {
        detail: format!("read slug: {}", e),
    })?;
    let label: String = row.get(1).map_err(|e| HabitCaptureError::Store {
        detail: format!("read label: {}", e),
    })?;
    let freq_json: String = row.get(2).map_err(|e| HabitCaptureError::Store {
        detail: format!("read frequency_json: {}", e),
    })?;
    let tags_json: String = row.get(3).map_err(|e| HabitCaptureError::Store {
        detail: format!("read tags_json: {}", e),
    })?;
    let created_at: String = row.get(4).map_err(|e| HabitCaptureError::Store {
        detail: format!("read created_at: {}", e),
    })?;
    let target_frequency: HabitFrequency =
        serde_json::from_str(&freq_json).map_err(|e| HabitCaptureError::Store {
            detail: format!("parse frequency_json: {}", e),
        })?;
    let tags: Vec<String> =
        serde_json::from_str(&tags_json).map_err(|e| HabitCaptureError::Store {
            detail: format!("parse tags_json: {}", e),
        })?;
    Ok(HabitDefinition {
        slug,
        label,
        target_frequency,
        tags,
        created_at,
    })
}

/// Format an epoch-millisecond timestamp as a UTC RFC 3339 string. `None` on
/// out-of-range input (chrono guards against ~292M years overflow).
fn ts_ms_to_rfc3339(ts_ms: i64) -> Option<String> {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ts_ms).map(|dt| dt.to_rfc3339())
}

/// Reverse of `HabitCheckinSource::slug` — defensive about unknown values from
/// older rows by defaulting to `Manual` (the safest "I don't know" telemetry
/// bucket; the row still surfaces in the streak so the user's history is
/// honoured even if the source column drifts).
///
/// DRIFT-GUARD #01: currently unused on the read path (streak math reads only
/// plaintext-safe slug+timestamp, never decrypting the body). Retained for the
/// future "decrypt body → reconstruct full `HabitCheckin.source`" path once the
/// dashboard needs the source breakdown; kept so that wiring is a one-liner.
#[allow(dead_code)]
fn parse_source_slug(s: &str) -> HabitCheckinSource {
    match s {
        "watch" => HabitCheckinSource::Watch,
        "widget" => HabitCheckinSource::Widget,
        "shortcut" => HabitCheckinSource::Shortcut,
        _ => HabitCheckinSource::Manual,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn habit_summary_round_trip_smoke() {
        // §7.1.5 invariant: the dashboard wire row survives a JSON round trip
        // without dropping fields — covers the embedded `HabitStreak` nesting
        // (which is the most-likely source of accidental field-name drift
        // between Rust camelCase serde and the TS interface).
        let s = HabitSummary {
            habit_slug: "water".to_string(),
            last_7d_count: 14,
            last_30d_count: 61,
            last_checkin_at: Some("2026-05-25T08:30:00Z".to_string()),
            streak: HabitStreak {
                habit_slug: "water".to_string(),
                current_streak: 12,
                longest_streak: 30,
                last_checkin_at: Some("2026-05-25T08:30:00Z".to_string()),
            },
        };
        let j = serde_json::to_string(&s).unwrap();
        let back: HabitSummary = serde_json::from_str(&j).unwrap();
        assert_eq!(s.habit_slug, back.habit_slug);
        assert_eq!(s.last_7d_count, back.last_7d_count);
        assert_eq!(s.last_30d_count, back.last_30d_count);
        assert_eq!(s.last_checkin_at, back.last_checkin_at);
        assert_eq!(s.streak.current_streak, back.streak.current_streak);
        assert_eq!(s.streak.longest_streak, back.streak.longest_streak);
        assert_eq!(s.streak.habit_slug, back.streak.habit_slug);
        // camelCase invariant — TS side expects `lastCheckinAt`, not the
        // Rust-native `last_checkin_at`. If anyone removes the
        // `rename_all = "camelCase"` attribute, this assertion catches it.
        assert!(j.contains("lastCheckinAt"), "camelCase wire shape: {}", j);
        assert!(j.contains("last7dCount"), "camelCase wire shape: {}", j);
    }

    #[test]
    fn habit_frequency_serializes_with_kind_tag() {
        // §7.1 invariant: HabitFrequency is an internally-tagged enum keyed
        // on `kind` — TS side dispatches on `freq.kind === 'weekly'` etc.
        let f = HabitFrequency::Weekly { times: 3 };
        let j = serde_json::to_string(&f).unwrap();
        assert!(j.contains("\"kind\":\"weekly\""), "wire shape: {}", j);
        assert!(j.contains("\"times\":3"), "payload preserved: {}", j);

        let custom = HabitFrequency::Custom {
            cron: "0 9 * * 1,3,5".to_string(),
        };
        let jc = serde_json::to_string(&custom).unwrap();
        assert!(jc.contains("\"kind\":\"custom\""), "wire shape: {}", jc);
        assert!(jc.contains("0 9 * * 1,3,5"), "cron preserved: {}", jc);
    }

    #[test]
    fn habit_checkin_source_slug_is_stable() {
        // Slug strings appear in events.metadata_json telemetry rows — any
        // change is a wire-break that invalidates historical analytics.
        assert_eq!(HabitCheckinSource::Manual.slug(), "manual");
        assert_eq!(HabitCheckinSource::Watch.slug(), "watch");
        assert_eq!(HabitCheckinSource::Widget.slug(), "widget");
        assert_eq!(HabitCheckinSource::Shortcut.slug(), "shortcut");
    }

    #[test]
    fn habit_capture_error_serializes_with_code_tag() {
        // §11 invariant: error wire shape uses `{"code": "..."}` so the UI
        // can dispatch on the machine-readable code string per SPEC-04
        // error catalog conventions.
        let e = HabitCaptureError::ChipIdConflict {
            slug: "water".to_string(),
        };
        let j = serde_json::to_string(&e).unwrap();
        assert!(j.contains("chip_id_conflict"), "wire shape: {}", j);
        assert!(j.contains("water"), "payload preserved: {}", j);

        let e2 = HabitCaptureError::PaletteSizeOutOfRange { got: 13 };
        let j2 = serde_json::to_string(&e2).unwrap();
        assert!(j2.contains("palette_size_out_of_range"), "wire shape: {}", j2);
        assert!(j2.contains("13"), "payload preserved: {}", j2);

        let e3 = HabitCaptureError::InvalidSlug {
            slug: "BadCase!!".to_string(),
        };
        let j3 = serde_json::to_string(&e3).unwrap();
        assert!(j3.contains("invalid_slug"), "wire shape: {}", j3);
        assert!(j3.contains("BadCase!!"), "payload preserved: {}", j3);
    }

    // ── Stage 3 KAT (known-answer-test) vectors ────────────────────────────
    //
    // The Stage 2 `#[should_panic(expected = "Stage 3")]` marker for the
    // happy path is gone — `create_habit(Daily)` now runs end-to-end against
    // a real `rusqlite` connection backed by a `tempfile::TempDir`. A new
    // Stage 4 marker (`create_habit_custom_cron_stage4_marker`) pins the
    // one helper still unimplemented (`cron_parse_pseudo`).

    /// Process-wide lock that serialises the `$HOME`-mutating tests. `$HOME` is
    /// global process state, so two tests racing `set_var("HOME")` (under the
    /// default parallel `cargo test` / `--include-ignored`) would clobber each
    /// other and make one read the other's tempdir (the "ChipNotFound right
    /// after create" / "readonly database" flakes). Holding this mutex for the
    /// guard's lifetime makes each `$HOME`-isolated test body run exclusively.
    static HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Shared RAII guard so every test isolates `$HOME` to a tempdir and
    /// restores it on drop. `open_habits_db` resolves `~/.phantom-mesh/`
    /// through `dirs::home_dir()` which honours `$HOME` on unix. Carries the
    /// `HOME_LOCK` guard so the exclusive window spans the whole test body.
    struct HomeGuard {
        prev: Option<std::ffi::OsString>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }
    impl Drop for HomeGuard {
        fn drop(&mut self) {
            match &self.prev {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
        }
    }
    fn isolate_home(tmp: &tempfile::TempDir) -> HomeGuard {
        // Recover from a poisoned lock (a panicking test) so unrelated tests
        // still serialise rather than all panicking on `.lock()`.
        let lock = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var_os("HOME");
        std::env::set_var("HOME", tmp.path());
        HomeGuard { prev, _lock: lock }
    }

    /// SPEC-22 §8.2 — create + lookup round trip: a newly-created chip can
    /// be retrieved by slug, frequency JSON survives the round trip, and
    /// duplicate insert returns `ChipIdConflict` (HABIT-001).
    #[ignore = "integration / env-dependent — run via --ignored"]
    #[test]
    fn create_habit_round_trip_against_real_sqlite() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let _guard = isolate_home(&tmp);
        let def = HabitDefinition {
            slug: "water".to_string(),
            label: "喝水".to_string(),
            target_frequency: HabitFrequency::Weekly { times: 5 },
            tags: vec!["health".to_string()],
            created_at: "2026-05-25T00:00:00Z".to_string(),
        };
        create_habit(&def).expect("first insert");
        // Duplicate must short-circuit to ChipIdConflict.
        let dup = create_habit(&def).expect_err("dup must fail");
        assert!(matches!(dup, HabitCaptureError::ChipIdConflict { .. }));
        // list_habits should now return one entry with the right frequency.
        let summaries = list_habits().expect("list_habits");
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].habit_slug, "water");
    }

    /// SPEC-22 §8.3 — record_checkin appends a row + recomputes streak.
    /// One checkin "today" yields current_streak == 1, longest_streak == 1.
    ///
    /// DRIFT-GUARD #01: the check-in now round-trips through the SPEC-16
    /// encrypted EventStore (not the removed plaintext table), so the test
    /// installs a deterministic per-process EventKey first.
    #[ignore = "integration / env-dependent — run via --ignored"]
    #[test]
    fn record_checkin_drives_streak_to_one() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let _guard = isolate_home(&tmp);
        let _kg = install_test_event_key();
        create_habit(&HabitDefinition {
            slug: "stretch".to_string(),
            label: "拉筋".to_string(),
            target_frequency: HabitFrequency::Daily,
            tags: vec![],
            created_at: "2026-05-25T00:00:00Z".to_string(),
        })
        .expect("create");
        let streak = record_checkin(&HabitCheckin {
            habit_slug: "stretch".to_string(),
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
            note: None,
            source: HabitCheckinSource::Manual,
        })
        .expect("record_checkin");
        assert_eq!(streak.habit_slug, "stretch");
        assert_eq!(streak.current_streak, 1, "today's first log → 1");
        assert_eq!(streak.longest_streak, 1);
        assert!(streak.last_checkin_at.is_some());
    }

    /// MAC-CUJ04-LLM-003 (P1 local-first invariant) — capture must NEVER depend
    /// on an LLM provider. With every `*_API_KEY` unset, `record_checkin` must
    /// still return Ok and drive the streak: the habit log is a purely local
    /// EventStore write, no network / LLM round-trip. Regression guard against
    /// accidentally wiring an LLM enrichment call into the capture hot path.
    #[ignore = "integration / env-dependent — run via --ignored"]
    #[test]
    fn record_checkin_is_local_first_with_no_llm_keys() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let _guard = isolate_home(&tmp);
        let _kg = install_test_event_key();

        // Strip every known LLM provider key: a capture that secretly tried to
        // call a model would now fail (no creds), so a green result proves the
        // path is offline/local-first.
        for k in [
            "GROQ_API_KEY",
            "ANTHROPIC_API_KEY",
            "OPENAI_API_KEY",
            "GEMINI_API_KEY",
            "GOOGLE_API_KEY",
            "OPENROUTER_API_KEY",
            "MISTRAL_API_KEY",
            "DEEPSEEK_API_KEY",
        ] {
            std::env::remove_var(k);
        }

        create_habit(&HabitDefinition {
            slug: "water".to_string(),
            label: "喝水".to_string(),
            target_frequency: HabitFrequency::Daily,
            tags: vec![],
            created_at: "2026-05-25T00:00:00Z".to_string(),
        })
        .expect("create");

        let streak = record_checkin(&HabitCheckin {
            habit_slug: "water".to_string(),
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
            note: None,
            source: HabitCheckinSource::Manual,
        })
        .expect("record_checkin must succeed with no LLM keys (local-first)");

        assert_eq!(streak.habit_slug, "water");
        assert!(
            streak.current_streak >= 1,
            "capture without any LLM key must still log + count the streak"
        );
        assert!(streak.last_checkin_at.is_some());
    }

    /// MAC-CUJ02-FH-005 — same-day dedup. Two check-ins on the SAME calendar day
    /// must NOT inflate the streak: the streak counts distinct active days, so
    /// two taps today still yield current_streak == 1. Guards against an
    /// off-by-one where every check-in (not every day) bumped the count.
    #[ignore = "integration / env-dependent — run via --ignored"]
    #[test]
    fn two_checkins_same_day_keep_streak_at_one() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let _guard = isolate_home(&tmp);
        let _kg = install_test_event_key();

        create_habit(&HabitDefinition {
            slug: "water".to_string(),
            label: "喝水".to_string(),
            target_frequency: HabitFrequency::Daily,
            tags: vec![],
            created_at: "2026-05-25T00:00:00Z".to_string(),
        })
        .expect("create");

        // Two taps a few seconds apart, both "today".
        let now = chrono::Utc::now().timestamp_millis();
        let _ = record_checkin(&HabitCheckin {
            habit_slug: "water".to_string(),
            timestamp_ms: now,
            note: None,
            source: HabitCheckinSource::Manual,
        })
        .expect("first checkin");
        let streak = record_checkin(&HabitCheckin {
            habit_slug: "water".to_string(),
            timestamp_ms: now + 3_000,
            note: None,
            source: HabitCheckinSource::Manual,
        })
        .expect("second checkin same day");

        assert_eq!(
            streak.current_streak, 1,
            "two check-ins on the same day must keep the streak at 1 (distinct-day count)"
        );
        assert_eq!(streak.longest_streak, 1);
    }

    /// RAII guard that installs a deterministic per-process EventKey for the
    /// duration of a test and clears it on drop, so the encrypted EventStore
    /// round-trip works in isolation. OSS-safe fixed seed (all-`0x42`).
    struct EventKeyGuard;
    impl Drop for EventKeyGuard {
        fn drop(&mut self) {
            crate::encryption_wire::clear_event_key_cache();
        }
    }
    fn install_test_event_key() -> EventKeyGuard {
        let seed = [0x42u8; 32];
        crate::encryption_wire::install_event_key_from_seed(&seed)
            .expect("install test EventKey");
        EventKeyGuard
    }

    /// DRIFT-GUARD #01 (SPEC-16 / SPEC-13) — a check-in routes through the
    /// encrypted EventStore (kind=Habit) and reappears via the EventStore read
    /// path, AND the sensitive `note` free-text NEVER lands in plaintext on
    /// disk. This is the regression test for the P4-perimeter leak: the old
    /// code wrote `~/.phantom-mesh/habits.sqlite` with the note in plaintext.
    #[ignore = "integration / env-dependent — run via --ignored"]
    #[test]
    fn checkin_routes_through_encrypted_event_store_no_plaintext_pii() {
        use std::io::Read as _;
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let _guard = isolate_home(&tmp);
        let _kg = install_test_event_key();

        create_habit(&HabitDefinition {
            slug: "water".to_string(),
            label: "喝水".to_string(),
            target_frequency: HabitFrequency::Daily,
            tags: vec![],
            created_at: "2026-05-25T00:00:00Z".to_string(),
        })
        .expect("create");

        // The note is the canary PII string — it MUST NOT appear in plaintext
        // anywhere under the data dir.
        let secret_note = "user42-secret-relapse-note";
        let streak = record_checkin(&HabitCheckin {
            habit_slug: "water".to_string(),
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
            note: Some(secret_note.to_string()),
            source: HabitCheckinSource::Manual,
        })
        .expect("record_checkin via EventStore");
        // It came back through the EventStore read path → streak reflects it.
        assert_eq!(streak.current_streak, 1, "EventStore read path found the checkin");
        assert!(streak.last_checkin_at.is_some());

        // Assertion A: the legacy plaintext check-in store must NOT exist (the
        // habits.sqlite, if present, only holds non-PII chip_palette config).
        let events_dir = tmp.path().join(".phantom-mesh/events");
        assert!(events_dir.is_dir(), "EventStore events/ dir was created");

        // Assertion B: grep the entire data dir for the secret note — it must
        // be encrypted (absent in plaintext) in every file on disk.
        let leaked = scan_dir_for_plaintext(tmp.path(), secret_note);
        assert!(
            leaked.is_empty(),
            "PII note leaked in plaintext on disk: {:?}",
            leaked,
        );

        // Helper: recursively scan every file for the needle as raw bytes.
        fn scan_dir_for_plaintext(root: &std::path::Path, needle: &str) -> Vec<String> {
            let mut hits = Vec::new();
            let needle_bytes = needle.as_bytes();
            let mut stack = vec![root.to_path_buf()];
            while let Some(dir) = stack.pop() {
                let Ok(rd) = std::fs::read_dir(&dir) else {
                    continue;
                };
                for entry in rd.flatten() {
                    let p = entry.path();
                    if p.is_dir() {
                        stack.push(p);
                    } else if let Ok(mut f) = std::fs::File::open(&p) {
                        let mut buf = Vec::new();
                        if f.read_to_end(&mut buf).is_ok()
                            && buf
                                .windows(needle_bytes.len())
                                .any(|w| w == needle_bytes)
                        {
                            hits.push(p.display().to_string());
                        }
                    }
                }
            }
            hits
        }
    }

    /// SPEC-22 §11 (HABIT-002) — recording against an unknown slug must
    /// short-circuit before any sqlite write.
    #[test]
    fn record_checkin_unknown_slug_returns_chip_not_found() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let _guard = isolate_home(&tmp);
        let err = record_checkin(&HabitCheckin {
            habit_slug: "ghost".to_string(),
            timestamp_ms: 0,
            note: None,
            source: HabitCheckinSource::Manual,
        })
        .expect_err("must fail");
        assert!(matches!(err, HabitCaptureError::ChipNotFound { .. }));
    }

    /// SPEC-22 §8.3 — streak walker known-answer test on a synthetic 3-day
    /// run ending today: current_streak == 3, longest_streak == 3.
    #[test]
    fn streak_walk_three_consecutive_days_yields_three() {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let day = 24 * 60 * 60 * 1_000_i64;
        let rows = vec![
            HabitCheckin {
                habit_slug: "x".to_string(),
                timestamp_ms: now_ms,
                note: None,
                source: HabitCheckinSource::Manual,
            },
            HabitCheckin {
                habit_slug: "x".to_string(),
                timestamp_ms: now_ms - day,
                note: None,
                source: HabitCheckinSource::Manual,
            },
            HabitCheckin {
                habit_slug: "x".to_string(),
                timestamp_ms: now_ms - 2 * day,
                note: None,
                source: HabitCheckinSource::Manual,
            },
        ];
        let (current, longest, last) = streak_walk_pseudo(&rows).expect("walk");
        assert_eq!(current, 3);
        assert_eq!(longest, 3);
        assert!(last.is_some());
    }

    /// Empty rows → both streaks 0 and `last_checkin_at` None.
    #[test]
    fn streak_walk_empty_returns_zero() {
        let (current, longest, last) = streak_walk_pseudo(&[]).expect("walk");
        assert_eq!(current, 0);
        assert_eq!(longest, 0);
        assert!(last.is_none());
    }

    /// SPEC-22 §8.3 — `parse_cron_schedule` KAT: a well-formed cron string
    /// (7-field form with seconds — what the `cron` crate's `Schedule` parser
    /// accepts) returns Ok, and a clearly malformed string returns
    /// `InvalidCron` (HABIT-006). The 7-field example "every day at 21:00"
    /// is the recommended cron form the SPEC-22 UI surfaces in its picker.
    #[test]
    fn parse_cron_schedule_accepts_valid_and_rejects_invalid() {
        // "sec min hour day-of-month month day-of-week year" → 21:00 daily.
        parse_cron_schedule("0 0 21 * * * *").expect("valid 7-field cron must parse");
        let err = parse_cron_schedule("invalid").expect_err("garbage must fail");
        assert!(
            matches!(err, HabitCaptureError::InvalidCron { ref expr, .. } if expr == "invalid"),
            "expected InvalidCron with echoed expr, got {:?}",
            err,
        );
    }

    /// SPEC-22 §8.3 — `create_habit` with `Custom { cron }` validates the
    /// cron up-front: a malformed expression surfaces `InvalidCron` before
    /// any sqlite write, a valid one completes end-to-end.
    #[ignore = "integration / env-dependent — run via --ignored"]
    #[test]
    fn create_habit_custom_cron_validates_via_real_parser() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let _guard = isolate_home(&tmp);
        // Bad cron → InvalidCron, no row written.
        let bad = HabitDefinition {
            slug: "stand_bad".to_string(),
            label: "起身走動".to_string(),
            target_frequency: HabitFrequency::Custom {
                cron: "this is not a cron".to_string(),
            },
            tags: vec![],
            created_at: "2026-05-25T00:00:00Z".to_string(),
        };
        let err = create_habit(&bad).expect_err("bad cron must fail");
        assert!(matches!(err, HabitCaptureError::InvalidCron { .. }));
        // Good cron (21:00 every day) → inserted cleanly.
        let good = HabitDefinition {
            slug: "stand_ok".to_string(),
            label: "起身走動".to_string(),
            target_frequency: HabitFrequency::Custom {
                cron: "0 0 21 * * * *".to_string(),
            },
            tags: vec![],
            created_at: "2026-05-25T00:00:00Z".to_string(),
        };
        create_habit(&good).expect("good cron must insert");
    }

    // G1 / MAC-CUJ01-FH-008 / Bug#1 — slug SHAPE validation (SPEC-22 §8.2,
    // HABIT-004). Before the fix, `create_habit` never checked slug shape and
    // `InvalidSlug` was dead code, so malformed slugs persisted into sqlite and
    // the plaintext event tags. This pins the validate-before-persist contract.
    #[test]
    fn validate_slug_enforces_spec22_shape() {
        // Pure-function test (no HOME / sqlite): the slug gate `create_habit`
        // Step 0 calls. Each malformed slug violates `[a-z0-9_]{1,32}` and MUST
        // return InvalidSlug; boundary-valid slugs (len 1, len 32, snake_case)
        // MUST pass. This is what makes `InvalidSlug` live code instead of the
        // dead variant the coverage sweep found.
        let bad: &[&str] = &[
            "BadCase!!",      // uppercase + punctuation
            "has space",      // space
            "kebab-case",     // hyphen not allowed
            "água",           // non-ascii
            "",               // empty (len 0)
            &"x".repeat(33),  // 33 chars > 32
        ];
        for s in bad {
            assert!(
                matches!(validate_slug(s), Err(HabitCaptureError::InvalidSlug { .. })),
                "slug {:?} → want InvalidSlug, got {:?}",
                s,
                validate_slug(s)
            );
        }

        for ok in ["a", &"a".repeat(32), "quit_smoke_2", "water", "coffee"] {
            assert!(
                validate_slug(ok).is_ok(),
                "valid slug {:?} must pass, got {:?}",
                ok,
                validate_slug(ok)
            );
        }
    }
}
