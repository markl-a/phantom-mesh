// SPEC-23 §7 + §9 — Coach engine wire types (single source of truth for the
// daily-review request / outcome / Tauri-event payloads + tiered memory
// injection contract shared with SPEC-25).
//
// Stage 3 (real impl — aggregator + shame-free lint + prompt templates +
// response extraction + event-store cross-wire live): the pure helpers
// (`aggregate`, `format_section`, `shame_free_lint`, `load_prompt_template`,
// `extract_response_text`) are now backed by real `std::collections::BTreeMap`
// / `serde_json` / `coach_prompts::{templates,lint}` code, and
// `query_events_for_date_pseudo` is now a real delegate to
// `crate::event_storage_wire::query_events` (Stage 3) with EventRecord →
// (EventMeta, AnalysisResult) projection. Two cross-wire helpers still routed
// through wires that have not finished Stage 3 internally
// (providers_wire::complete inner _pseudo helpers still panic;
// skill_wire::recall_skills calls fts5_search which panics) stay as Stage 4
// panicking markers so the audit grep still finds the boundary:
// `call_providers_complete_pseudo`, `recall_pseudo`, plus
// `run_daily_review` Step 6 (EventStore::append + Tauri emit) and
// `inject_tiered_memory` Step 3 (Vec<String> → tiered MemoryInject).
//
// 中文: 本檔對應 SPEC-23 §7（資料模型）與 §9.7（Tauri events）。教練
// （coach，每日複盤引擎）每日 21:00 從 EventStore（事件儲存）撈昨日 events，
// 經 aggregate → LLM → lint 後 emit `coach.review.ready` 或
// `coach.review.degraded`。Stage 1 只把 wire 型別 + stub 排好；Stage 2 把
// 真實邏輯接進 `life_node::coach_engine` 與 `life_node::daily_review`。
//
// **Cycle-break note (cross-spec)**: 本檔引用 `MemoryInject` 概念，
// 但 **不** import SPEC-25 hermes recall 的 trait — `inject_tiered_memory()`
// 是純 stub，Stage 2 若 SPEC-25 尚未 ready 就 fall back 空 Vec，不阻擋本檔。
//
// TODO Stage 2:
//   - 把 `run_daily_review` 接進 EventStore::query + aggregate + fallback
//     LLM chain + shame/medical lint + EventStore::append + EventBus emit。
//   - 把 `aggregate()` migrate 現有 `core/src/life_node/daily_review.rs` 的
//     markdown formatter 改用本 wire 型別（field 名對齊）。
//   - `propose_tomorrow_action()` 現有實作在 `daily_review.rs`，Stage 2 收
//     斂到此處 wire 統一。
//   - `inject_tiered_memory()` Stage 2 嘗試 `hermes::recall()`；SPEC-25
//     不 ready → 回 `MemoryInject::default()`（三段都空 Vec）。
//   - 把 `CoachReviewReadyPayload` 5 欄位 schema 寫進 SPEC-23 §9.7 / SPEC-24
//     §20.1 round-trip test（已在 §7.2 標明統一 payload）。

use serde::{Deserialize, Serialize};
use ts_rs::TS;

// `EventKey` 是 Rust-private encryption material — 本檔 import 但 **不** 再
// export，呼叫端只能在 core crate 內把它傳給 stub fn。SPEC-13 §6.2 規定它
// 不可跨 FFI；本檔遵守。
#[allow(unused_imports)]
use crate::encryption_wire::EventKey;
// `EventMeta` + `AnalysisResult` 是 SPEC-16 公開 wire 型別，aggregate() 拿
// `&[(EventMeta, AnalysisResult)]` 做純函式 markdown 格式化。
#[allow(unused_imports)]
use crate::event_storage_wire::{AnalysisResult, EventMeta};

// ─── §7.1 / §9.1 DailyReviewRequest — `coach_run_now` / CLI input ────────────

/// Input parameters for `run_daily_review` and the Tauri `coach_run_now`
/// command. `date` is the local-calendar date being reviewed (typically
/// *yesterday* when scheduler-fired at 21:00; user-specified for backfill).
/// `recall_k` controls how many recall-tier memories to inject (SPEC-25 §7
/// audit pinned coach at `recall_k = 5`; raise via Stage 2 config flag).
///
/// 中文: 教練每日複盤的輸入。`date` 是要回顧的「當地日期」（local-tz
/// date，預設昨日，user 可指定 backfill）；`recall_k` 是 SPEC-25 hermes
/// 記憶系統第二層 recall 抓幾筆（教練固定 5 筆，per audit）。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/coach/")]
#[serde(rename_all = "camelCase")]
pub struct DailyReviewRequest {
    /// Local-calendar date being reviewed (`"YYYY-MM-DD"`, e.g.
    /// `"2026-05-24"`). SPEC-16 §7.4 + §9.2 forces local-tz interpretation
    /// to avoid the UTC-vs-local 0-events bug fixed 2026-05-22.
    pub date: String,
    /// Number of recall-tier memories to inject when assembling the LLM
    /// prompt. Defaults to `5` per SPEC-25 audit pinning. `0` disables
    /// recall injection (used by smoke tests + degraded-path fallback).
    #[serde(default = "default_recall_k")]
    pub recall_k: u8,
}

fn default_recall_k() -> u8 {
    5
}

// ─── §7.1 DailyReviewOutcome — `run_daily_review` result ─────────────────────

/// Aggregated outcome of a single daily-review run. Mirrors SPEC-23 §7.1
/// `CoachDailyReview` + `ReviewOutcome` collapsed into the wire-facing shape
/// the UI + CLI actually consume. Returned by both the Tauri `coach_run_now`
/// command and the `phantom coach review` CLI.
///
/// 中文: 一次 daily review 跑完的結果。對應 SPEC-23 §7.1 的
/// `CoachDailyReview` 與 `ReviewOutcome`，但合併成 UI / CLI 真正吃的扁平
/// shape。`event_id` 是 SPEC-16 events 表落地的 row id；`status` 決定 UI
/// 顯示「完整卡片」或「degraded 等等再試」。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/coach/")]
#[serde(rename_all = "camelCase")]
pub struct DailyReviewOutcome {
    /// Local-calendar date that was reviewed (`"YYYY-MM-DD"`).
    pub date: String,
    /// SPEC-16 events row UUIDv7 that holds the encrypted full review.
    /// String form (36 chars with dashes).
    pub event_id: String,
    /// Aggregated markdown summary of the day (≤ 4 KB per §7.1). Pre-LLM —
    /// safe to render even when LLM call failed.
    pub events_summary: String,
    /// 3–5 「昨日觀察」bullets. Always present (aggregator-generated when
    /// LLM failed; LLM-extracted when LLM succeeded).
    pub takeaways: Vec<String>,
    /// 明日 1 建議動作. Empty string when `status == Degraded` (lint reject
    /// or all-providers-failed) — UI shows the「⏳ 建議 still cooking」card.
    pub next_action: String,
    /// Canonical provider:model id of the LLM that produced `next_action`
    /// (`"anthropic:claude-opus-4.7"`, `"groq:llama-3.1-8b-instant"`, …).
    /// Empty string when `status == Degraded`.
    pub model_used: String,
    /// Provider-reported call cost. `None` for local models or when LLM
    /// step was skipped.
    pub cost_usd: Option<f64>,
    /// Wall-clock latency of the full pipeline (aggregate + memory + LLM
    /// + lint + append). Always populated — even degraded runs report it
    /// for the perf-budget audit.
    pub latency_ms: u64,
    /// State-machine endpoint per §8 — `Completed` is the success path,
    /// `Degraded` is lint-reject or all-providers-fail, `Failed` is a
    /// hard error before append (very rare, see `CoachError`).
    pub status: ReviewStatus,
}

// ─── §9.7 CoachReviewReadyPayload — unified 5-field event payload ────────────

/// Payload of the `coach.review.ready` Tauri event. **Unified 5-field
/// schema** per SPEC-23 §9.7 + SPEC-24 §20.1 — this is the cross-spec
/// canonical shape both engine (this file) and delivery (SPEC-24) agree on
/// (cycle-break fix landed in the SPEC-23 / SPEC-24 polish pass).
///
/// 中文: `coach.review.ready` 事件 payload。**5 欄統一 schema**，是
/// SPEC-23 與 SPEC-24 之間 cycle-break 修好後的對齊版本。markdown 內容
/// **不放** payload — delivery 端拿 `markdown_path` 自己走 age v1 decrypt
/// 讀檔，避免明文飄散到 EventBus subscriber log。
///
/// **Field semantics**（per SPEC-23 §9.7）:
/// - `review_id` — `DailyReviewOutcome` UUIDv7（每次 run 一個）
/// - `event_id`  — SPEC-16 events row id（向後相容 alias）
/// - `date`      — local-tz `"YYYY-MM-DD"`
/// - `takeaways_count` — `DailyReviewOutcome.takeaways.len()`（u32 wire-safe）
/// - `markdown_path` — `~/.phantom-mesh/coach/YYYY-MM-DD.md.age` 絕對路徑
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/coach/")]
#[serde(rename_all = "camelCase")]
pub struct CoachReviewReadyPayload {
    /// UUIDv7 string form (36 chars) — same value as
    /// `DailyReviewOutcome.event_id`, kept as separate field for SPEC-24
    /// 對外向後相容。
    pub review_id: String,
    /// SPEC-16 events row id (alias of `review_id` for cross-spec linkage).
    pub event_id: String,
    /// Local-calendar date being reviewed (`"YYYY-MM-DD"`).
    pub date: String,
    /// Number of takeaway bullets (= `takeaways.len()`). u32 to keep the
    /// wire shape stable across 32 / 64-bit targets.
    pub takeaways_count: u32,
    /// Absolute path to the age-encrypted markdown file
    /// (`~/.phantom-mesh/coach/YYYY-MM-DD.md.age`). Delivery (SPEC-24)
    /// reads + decrypts; raw markdown is **never** put on the EventBus.
    pub markdown_path: String,
}

// ─── §9.7 CoachReviewDegradedPayload — degraded variant ──────────────────────

/// Payload of the `coach.review.degraded` Tauri event. Emitted when the LLM
/// fallback chain exhausts all providers OR the shame/medical lint rejects
/// the proposed `next_action`. UI shows the「⏳ 還在想 next step」card +
/// retry button.
///
/// 中文: `coach.review.degraded` 事件 payload。LLM 全失敗或被 shame /
/// medical lint 擋下時送出。`reason` 是機器可讀的降級類別碼（snake_case，
/// 對應 SPEC-23 §7.1 `DegradedReason` enum 序列化形式）。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/coach/")]
#[serde(rename_all = "camelCase")]
pub struct CoachReviewDegradedPayload {
    /// UUIDv7 of the (still-appended) review row — degraded reviews are
    /// **still persisted** so user can retry from history.
    pub review_id: String,
    /// SPEC-16 events row id (alias of `review_id`).
    pub event_id: String,
    /// Local-calendar date being reviewed (`"YYYY-MM-DD"`).
    pub date: String,
    /// Machine-readable degradation cause. Stable strings per SPEC-23
    /// §7.1 `DegradedReason`: `"all_providers_failed"` /
    /// `"shame_leakage"` / `"medical_disclaimer_hit"` /
    /// `"llm_empty_output"`.
    pub reason: String,
}

// ─── §7.1 / SPEC-25 §7 MemoryInject — tiered memory injection ────────────────

/// Three-tier memory bundle injected into the LLM prompt before
/// `propose_tomorrow_action`. Mirrors SPEC-25 hermes recall layering:
///
/// - `core` — always-on identity / goals / non-negotiables
/// - `recall` — top-K FTS5 matches against the day's events (K = 5 per audit)
/// - `archival` — long-tail summaries (off for coach: `archival_k = 0`)
///
/// 中文: SPEC-25 hermes 三層記憶注入 bundle。`core`（核心，永遠注入）/
/// `recall`（即時召回，FTS5 top-K）/ `archival`（長尾摘要，教練固定關閉）。
/// 教練配置：`RecallPolicy { core_all: true, recall_k: 5, archival_k: 0 }`。
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/coach/")]
#[serde(rename_all = "camelCase")]
pub struct MemoryInject {
    /// Always-on core memories (identity, active goals, non-negotiables).
    /// Empty Vec when SPEC-25 not yet built — coach gracefully degrades.
    pub core: Vec<String>,
    /// Recall-tier matches (top-`recall_k` FTS5 against day's events).
    pub recall: Vec<String>,
    /// Archival-tier summaries. Coach pins `archival_k = 0` per audit; this
    /// field is here for forward-compat (other consumers may use it).
    pub archival: Vec<String>,
}

// ─── §7.1 RecallPolicy — caller controls injection depth ─────────────────────

/// Per-call recall policy passed to `inject_tiered_memory`. Coach pins
/// `{ core_all: true, recall_k: 5, archival_k: 0 }` per the SPEC-25 audit;
/// other consumers (skill suggest, evolve planner) may tune differently.
///
/// 中文: 記憶注入策略。`core_all` 是否把 core 層全注入（true = 全注，false
/// = 看 token 預算挑）；`recall_k` / `archival_k` 是 recall / archival 層
/// 各抓幾筆。教練固定 `(true, 5, 0)`。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/coach/")]
#[serde(rename_all = "camelCase")]
pub struct RecallPolicy {
    /// Inject **all** core-tier memories (vs. token-budget-capped subset).
    /// Coach: `true`.
    pub core_all: bool,
    /// Number of recall-tier matches to inject. Coach: `5`.
    pub recall_k: u8,
    /// Number of archival-tier summaries to inject. Coach: `0`.
    pub archival_k: u8,
}

impl Default for RecallPolicy {
    /// Default = coach's pinned policy `(true, 5, 0)` per SPEC-25 audit.
    fn default() -> Self {
        Self {
            core_all: true,
            recall_k: 5,
            archival_k: 0,
        }
    }
}

// ─── §7.1 ReviewStatus — terminal status of a review run ─────────────────────

/// Terminal status of a single daily-review run. Maps onto SPEC-23 §8 state
/// machine `Emitted` exits: `Completed` = lint pass + `coach.review.ready`,
/// `Degraded` = lint reject or all-providers-fail + `coach.review.degraded`.
///
/// 中文: 一次 review 跑完後的終態。`Pending` / `Running` 是 in-flight
/// 中介態（UI polling 看得到）；`Completed` / `Degraded` / `Failed` 是 §8
/// 狀態機的終點。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/coach/")]
#[serde(rename_all = "snake_case")]
pub enum ReviewStatus {
    /// Scheduler fired but engine has not yet started aggregating.
    /// Brief window (< 100 ms typically); UI may flash a spinner.
    Pending,
    /// Engine is mid-pipeline (aggregating / LLM-calling / linting).
    /// Surfaced when UI polls `coach_history` during a run.
    Running,
    /// Lint pass + EventStore append succeeded; `coach.review.ready`
    /// emitted. Full card renders.
    Completed,
    /// LLM all-fail or lint reject; review row still persisted with
    /// `next_action = ""`. UI shows degraded card + retry button.
    Degraded,
    /// Hard error before EventStore append (e.g. db full, decrypt fail).
    /// No row persisted — caller sees `CoachError`. Rare.
    Failed,
}

// ─── §11.1 CoachError — error catalog mirror ─────────────────────────────────

/// Wire-facing error variants for the coach engine subsystem. Mirrors the
/// SPEC-23 §11.1 error catalog one-to-one. Sent back to the UI via Tauri
/// command failure path; CLI maps via `phantom_error::Error::user_message`.
///
/// 中文: SPEC-23 §11.1 error catalog 的 wire-facing 鏡像。每個 variant 對
/// 應一個 user-facing recovery hint（見 §11.1 表）。`#[serde(tag = "code")]`
/// 讓 UI 可以 dispatch on machine-readable code 字串。
#[derive(Debug, Clone, Serialize, Deserialize, TS, thiserror::Error)]
#[ts(export, export_to = "../../app/src/lib/generated/coach/")]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum CoachError {
    /// Same date already has `status=Completed` row and caller did not
    /// pass `force=true`. Recovery: pass `--force` or wait 24h.
    #[error("coach.already_running: date={date}")]
    AlreadyRunning { date: String },
    /// `EventStore::query` raised (sqlite I/O / decrypt fail). Recovery:
    /// check `identity.key` exists + db file readable.
    #[error("coach.events_query_failed: {detail}")]
    EventsQueryFailed { detail: String },
    /// `EventStore::append` raised because data-dir filesystem is full.
    /// Recovery: free space or move data-dir.
    #[error("coach.db_full")]
    DbFull,
    /// Scheduler registration failed because mobile OS denied background
    /// permission (iOS BGTask / Android background restricted).
    /// Recovery: user opens Settings → allow phantom background refresh.
    #[error("coach.scheduler_permission_denied: {os}")]
    SchedulerPermissionDenied { os: String },
    /// Schedule input failed validation (`hour ∉ [0,23]` / `minute ∉
    /// [0,59]` / IANA tz string parse fail). Recovery: fix args + retry.
    #[error("coach.schedule_invalid: {field}")]
    ScheduleInvalid { field: String },
    /// Review row read decrypted to gibberish (identity.key corruption).
    /// Recovery: `phantom keys restore` from backup.
    #[error("coach.decrypt_failed")]
    DecryptFailed,
    /// SPEC-25 hermes recall errored. Stage 2 may downgrade this to a
    /// degraded-but-continue path (inject empty memory + log warning).
    #[error("coach.memory_inject_failed: {detail}")]
    MemoryInjectFailed { detail: String },
    /// LLM fallback chain exhausted (all 4 providers errored). Caller
    /// typically converts to `ReviewStatus::Degraded + reason =
    /// "all_providers_failed"` rather than propagating this error.
    #[error("coach.llm_all_providers_failed")]
    LlmAllProvidersFailed,
}

// ─── §9.6 / §7.1 Stage-1 stub helpers (Stage 2 implements) ───────────────────

/// Run a full daily-review pipeline: query events → aggregate → memory
/// inject → LLM `propose_tomorrow_action` → shame/medical lint → append
/// to EventStore → emit `coach.review.ready` or `coach.review.degraded`.
///
/// `key` is the per-device `EventKey` for SPEC-13 age decryption of the
/// queried event bodies. `None` triggers `CoachError::DecryptFailed` at
/// the read step — caller is expected to load it from SPEC-12 keystore.
///
/// 中文: 一次 daily review 的主入口。Stage 2 串：query → aggregate →
/// memory → LLM → lint → append → emit。`key` 是 SPEC-13 age 解密金鑰，
/// 必須由呼叫端從 SPEC-12 keystore 載入後傳進來。
pub fn run_daily_review(
    req: &DailyReviewRequest,
    key: Option<EventKey>,
) -> Result<DailyReviewOutcome, CoachError> {
    let started = std::time::Instant::now();

    // Step 1 — query SPEC-16 events for `req.date` (local-tz) via decrypted
    // event_storage_wire; this resolves the UTC-vs-local 0-events bug fix.
    let events = query_events_for_date_pseudo(&req.date, key.as_ref())?;

    // Step 2 — aggregate the events into the §7.1 markdown brief (pure fn).
    // This is the stats-only artefact that is ALWAYS available — even when
    // the LLM is unreachable it carries the day's summary for the user.
    let brief = aggregate(&events);

    // Step 3 — inject SPEC-25 tiered memory context. Graceful-empty when the
    // skill index / hermes embedding leg is not yet present, so a missing
    // recall backend never blocks a review (cycle-break per §7.1).
    let policy = RecallPolicy {
        recall_k: req.recall_k,
        ..RecallPolicy::default()
    };
    let _memory = inject_tiered_memory(&brief, &policy).unwrap_or_default();

    // Step 4 — derive aggregator takeaways. Used by BOTH paths: the happy
    // path keeps them as 「昨日觀察」alongside the LLM action; the degraded
    // path surfaces them as the user-facing observations when the LLM is down.
    let takeaways = derive_takeaways(&events);

    // Step 5 — propose tomorrow's 1 動作 via the provider fallback chain.
    // The shame/medical lint gate runs INSIDE `propose_tomorrow_action_rich`.
    // All-providers-failed (FallbackExhausted) and lint-reject BOTH collapse
    // to `CoachError::LlmAllProvidersFailed`, which we DEGRADE rather than
    // propagate (SPEC-23 §11.1 / #142): a dead LLM must never block the daily
    // review. Hard storage errors (Step 6) still propagate normally.
    match propose_tomorrow_action_rich(&brief, DEFAULT_COACH_MODEL) {
        Ok(action) => {
            // Step 6 (happy) — persist the encrypted review + assemble the
            // Completed outcome. Emit is the caller's concern (core has no
            // Tauri handle); the CLI prints, the Tauri command emits
            // `coach.review.ready` from this outcome.
            let event_id = persist_review(&req.date, &brief)?;
            Ok(DailyReviewOutcome {
                date: req.date.clone(),
                event_id,
                events_summary: brief,
                takeaways,
                next_action: action.text,
                model_used: action.model_used,
                cost_usd: action.cost_usd,
                latency_ms: started.elapsed().as_millis() as u64,
                status: ReviewStatus::Completed,
            })
        }
        Err(CoachError::LlmAllProvidersFailed) => {
            // Step 6 (degraded / #142) — stats-only review. Still persisted
            // so the user can retry from history; `next_action` / `model_used`
            // stay empty per the DailyReviewOutcome contract; exit Ok (the
            // failure does NOT propagate). Caller emits `coach.review.degraded`.
            let event_id = persist_review(&req.date, &brief)?;
            Ok(DailyReviewOutcome {
                date: req.date.clone(),
                event_id,
                events_summary: brief,
                takeaways,
                next_action: String::new(),
                model_used: String::new(),
                cost_usd: None,
                latency_ms: started.elapsed().as_millis() as u64,
                status: ReviewStatus::Degraded,
            })
        }
        // Any non-LLM error (currently none surface from the rich path, but
        // kept exhaustive so a future variant fails loud rather than silently
        // degrading) propagates as a hard failure.
        Err(e) => Err(e),
    }
}

/// Canonical default coach model id (`provider:model`). The fallback chain in
/// agents.toml decides the *actual* provider order; this only seeds the
/// request template (and is the model id surfaced when the chain is bypassed).
const DEFAULT_COACH_MODEL: &str = "anthropic:claude-opus-4.7";

/// LLM-proposed next action plus the provenance the outcome needs to report.
struct ProposedAction {
    text: String,
    model_used: String,
    cost_usd: Option<f64>,
}

/// Derive 3–5 「昨日觀察」takeaways straight from the aggregated events. Pure,
/// LLM-free — these are what the degraded (stats-only) path shows when no
/// provider answered, and the baseline observations the happy path keeps. Uses
/// each event's `AnalysisResult.summary`; falls back to a single neutral line
/// when the day has no analysable events.
fn derive_takeaways(events: &[(EventMeta, AnalysisResult)]) -> Vec<String> {
    let mut out: Vec<String> = events
        .iter()
        .filter_map(|(_, a)| {
            let s = a.summary.trim();
            if s.is_empty() {
                None
            } else {
                Some(s.to_string())
            }
        })
        .take(5)
        .collect();
    if out.is_empty() {
        out.push("今日尚無可分析的事件記錄。".to_string());
    }
    out
}

/// Persist a daily-review markdown body to the SPEC-16 encrypted EventStore as
/// a `kind=Text` event tagged `coach_review` + `date:<YYYY-MM-DD>`. The body is
/// age-encrypted at rest (SPEC-13); only the non-PII meta stays plaintext.
/// Returns the assigned event_id (UUIDv7). Both the happy and degraded review
/// paths persist — a degraded review is still a row the user can retry from.
fn persist_review(date: &str, markdown: &str) -> Result<String, CoachError> {
    use crate::event_storage_wire::{ts_ms_to_rfc3339_utc, write_event, EventKind, EventMeta};

    let encrypted_body = encrypt_review_body(markdown.as_bytes())?;
    let meta = EventMeta {
        event_id: uuid::Uuid::now_v7().to_string(),
        timestamp: ts_ms_to_rfc3339_utc(now_unix_ms()),
        kind: EventKind::Text,
        tags: vec!["coach_review".to_string(), format!("date:{}", date)],
    };
    write_event(&meta, &encrypted_body, None).map_err(|e| CoachError::EventsQueryFailed {
        detail: format!("event store write: {}", e),
    })
}

/// Age-encrypt a review body against the per-process EventKey (SPEC-13),
/// returning the RAW age v1 blob the EventStore body expects (what
/// `read_event` → `decrypt_raw_age_blob` decodes). Mirrors the habit/food
/// capture encryption path. A locked keystore → `DecryptFailed` (we never
/// persist coach review bodies in plaintext).
fn encrypt_review_body(plaintext: &[u8]) -> Result<Vec<u8>, CoachError> {
    use base64::Engine as _;

    // Mirror the habit/food capture encryption path: resolve the EventKey →
    // age identity → recipient, age-encrypt, then strip the base64 transport
    // layer `encrypt_event` adds so the on-disk body is the RAW age v1 blob.
    let key =
        crate::encryption_wire::lookup_or_derive_event_key().ok_or(CoachError::DecryptFailed)?;
    let identity = crate::encryption_wire::event_key_to_age_identity(&key)
        .map_err(|_| CoachError::DecryptFailed)?;
    let recipient = crate::encryption_wire::derive_recipient_from_identity(&identity);
    let envelope = crate::encryption_wire::encrypt_event(plaintext, &recipient)
        .map_err(|_| CoachError::DecryptFailed)?;
    base64::engine::general_purpose::STANDARD
        .decode(envelope.ciphertext_b64.as_bytes())
        .map_err(|_| CoachError::DecryptFailed)
}

/// Current Unix time in milliseconds. Clamps a pre-epoch clock to 0 so the
/// timestamp formatter never sees a negative instant.
fn now_unix_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Stage 3 real impl — query SPEC-16 events for a local-calendar date and
/// project each `EventRecord` into the `(EventMeta, AnalysisResult)` pair the
/// aggregator consumes. Delegates to `crate::event_storage_wire::query_events`
/// (now Stage 3 — file-per-event read_dir + filter + sort).
///
/// Mapping rule: events whose `analysis` side-car is `None` (capture happened
/// but the LLM pass has not yet run, or was deliberately skipped) are dropped
/// from the aggregator input — coach's `aggregate()` needs both halves to
/// render a meaningful section bullet. A future Stage 4 pass may inject a
/// synthetic empty `AnalysisResult` instead so the bullet still surfaces with
/// `(no summary)`; for now we err on the side of less LLM-token waste.
///
/// `_key` is plumbed through for symmetry with the SPEC-13 keystore contract
/// but is unused at this layer — `event_storage_wire::query_events` calls
/// `read_event` internally which owns the decryption handshake. When the
/// keystore is not yet unlocked the inner read surfaces
/// `EventStoreError::DecryptionUnavailable`, which we map to
/// `CoachError::EventsQueryFailed` for the caller's degraded-path mapping.
fn query_events_for_date_pseudo(
    date: &str,
    _key: Option<&EventKey>,
) -> Result<Vec<(EventMeta, AnalysisResult)>, CoachError> {
    use crate::event_storage_wire::{query_events, EventStoreQuery};

    let q = EventStoreQuery {
        date_iso: Some(date.to_string()),
        kind: None,
        tag: None,
        limit: None,
        offset: None,
    };
    let records = query_events(&q).map_err(|e| CoachError::EventsQueryFailed {
        detail: e.to_string(),
    })?;

    // Project EventRecord → (EventMeta, AnalysisResult), dropping rows whose
    // analysis side-car has not been written yet (None) — see fn doc above.
    let pairs: Vec<(EventMeta, AnalysisResult)> = records
        .into_iter()
        .filter_map(|r| r.analysis.map(|a| (r.meta, a)))
        .collect();
    Ok(pairs)
}

/// Shame-free / medical-disclaimer lint gate over a proposed next-action
/// string. Returns `Ok(())` on pass; on hit returns
/// `CoachError::LlmAllProvidersFailed` so the caller maps to
/// `ReviewStatus::Degraded { reason: "shame_leakage" }` per SPEC-23 §11.1
/// (the §11.1 catalog collapses lint reject + all-providers-fail into the
/// same degraded path).
///
/// Delegates to the existing `coach_prompts::lint::check` shame-pattern
/// catalogue — the single canonical home for the 5 shame patterns + the
/// `all_templates_pass_shame_free_lint` test gate.
fn shame_free_lint(text: &str) -> Result<(), CoachError> {
    match crate::life_node::coach_prompts::lint::check(text) {
        Ok(()) => Ok(()),
        Err(_reason) => Err(CoachError::LlmAllProvidersFailed),
    }
}

/// Pure-function markdown formatter: turn the day's `(EventMeta,
/// AnalysisResult)` pairs into the §7.1 `events_summary` markdown string
/// (`≤ 4 KB`). Stage 2 migrates the existing implementation in
/// `core/src/life_node/daily_review.rs` to call this wire function so the
/// formatter has a single canonical home.
///
/// 中文: 純函式 markdown 格式化器。把當日所有 events 合成 ≤ 4 KB markdown
/// 摘要。Stage 2 把現有 `life_node/daily_review.rs` 的實作搬來、改用本
/// wire 型別（EventMeta / AnalysisResult）。
pub fn aggregate(events: &[(EventMeta, AnalysisResult)]) -> String {
    // Step 1 — derive the date string from the first event's ISO-8601 timestamp
    // (`"2026-05-24T13:45:00Z"` → `"2026-05-24"`); empty input → placeholder.
    let date_str: &str = events
        .first()
        .map(|(m, _)| m.timestamp.get(..10).unwrap_or("<date>"))
        .unwrap_or("<date>");

    // Step 2 — emit the markdown header + total-events count line.
    let mut buf = String::with_capacity(256 + events.len() * 96);
    buf.push_str(&format!("# Daily review — {}\n\n", date_str));
    buf.push_str(&format!("**Events captured:** {}\n\n", events.len()));

    if events.is_empty() {
        buf.push_str("(no events for this date)\n");
        return buf;
    }

    // Step 3 — group events by primary tag into a BTreeMap so section order
    // stays deterministic across runs (alphabetical by tag string).
    let mut grouped: std::collections::BTreeMap<&str, Vec<&(EventMeta, AnalysisResult)>> =
        std::collections::BTreeMap::new();
    for evt in events {
        let (meta, _) = evt;
        if meta.tags.is_empty() {
            grouped.entry("untagged").or_default().push(evt);
        } else {
            for tag in &meta.tags {
                grouped.entry(tag.as_str()).or_default().push(evt);
            }
        }
    }

    // Step 4 — emit one section per bucket via `format_section` (pure fn).
    for (tag, bucket) in grouped.iter() {
        buf.push_str(&format_section(tag, bucket));
    }

    buf
}

/// Render one `## <tag> (count)` section as a markdown string with bulleted
/// event summaries. Pure deterministic string concat, no I/O. Empty `summary`
/// fields render as the literal `(no summary)` to keep the bullet visible.
fn format_section(tag: &str, bucket: &[&(EventMeta, AnalysisResult)]) -> String {
    let mut s = String::with_capacity(64 + bucket.len() * 96);
    s.push_str(&format!("## {} ({})\n", tag, bucket.len()));
    for (meta, analysis) in bucket {
        let summary = analysis.summary.trim();
        let summary = if summary.is_empty() { "(no summary)" } else { summary };
        // EventKind is snake-case serde; render via JSON serialize → strip quotes.
        let kind_str = serde_json::to_string(&meta.kind)
            .unwrap_or_else(|_| "\"unknown\"".to_string());
        let kind_unquoted = kind_str.trim_matches('"');
        s.push_str(&format!(
            "- **{}** ({}): {}\n",
            kind_unquoted, meta.timestamp, summary
        ));
    }
    s.push('\n');
    s
}

/// Call the LLM with the aggregated `brief` markdown to get the明日 1
/// 建議動作 string. Stage 2 wires the shame/medical lint gate **inside**
/// this function — lint reject returns
/// `Err(CoachError::LlmAllProvidersFailed)` so the caller maps to
/// `ReviewStatus::Degraded` with the right `DegradedReason`.
///
/// `model_id` is the canonical `"provider:model"` id (`"anthropic:claude-
/// opus-4.7"`); Stage 2 may switch to a fallback-chain `&[String]` once
/// the SPEC-14 fallback wire lands.
///
/// 中文: 把 aggregated brief 丟給 LLM 取「明日 1 動作」字串。Stage 2 把
/// shame / medical lint gate 包在這函式裡 — lint reject 等同 LLM fail，
/// 呼叫端統一處理。
pub fn propose_tomorrow_action(
    brief: &str,
    model_id: &str,
) -> Result<String, CoachError> {
    propose_tomorrow_action_rich(brief, model_id).map(|a| a.text)
}

/// Rich variant of [`propose_tomorrow_action`] that also surfaces the
/// `model_used` + `cost_usd` provenance the `DailyReviewOutcome` reports. The
/// String-returning `propose_tomorrow_action` delegates here.
fn propose_tomorrow_action_rich(
    brief: &str,
    model_id: &str,
) -> Result<ProposedAction, CoachError> {
    // Step 1 — load the static COACH_SYSTEM_PROMPT + TOMORROW_ACTION_PROMPT
    // templates (compile-time `pub const` from coach_prompts::templates) and
    // render the user-turn with the aggregated brief substituted into the
    // `{BRIEF}` placeholder.
    let system_prompt: String = load_prompt_template("coach_system");
    let user_prompt: String = load_prompt_template("tomorrow_action").replace("{BRIEF}", brief);

    // Step 2 — run the request down the agents.toml provider fallback chain
    // (`providers_wire::complete_with_fallback`). Any chain exhaustion / auth
    // / network / rate-limit error collapses to `LlmAllProvidersFailed` so the
    // caller takes the SPEC-23 §11.1 degraded path.
    let response = call_providers_complete(model_id, &system_prompt, &user_prompt)?;

    // Step 3 — extract the text content from the provider response envelope
    // (single-shot completion, no streaming aggregation needed here).
    let action: String = extract_response_text(&response.text)?;

    // Step 4 — shame-free / medical lint gate over the LLM output; lint
    // reject is mapped to `CoachError::LlmAllProvidersFailed` (the §11.1
    // catalog collapses lint reject + all-fail into the degraded path).
    shame_free_lint(&action)?;

    // Step 5 — return the lint-clean action + provenance. Empty action is a
    // lint-equivalent fail (empty LLM output → degraded path).
    if action.trim().is_empty() {
        return Err(CoachError::LlmAllProvidersFailed);
    }
    Ok(ProposedAction {
        text: action,
        model_used: response.model_used,
        cost_usd: response.cost_usd,
    })
}

/// Load a named prompt template by short name. Templates live as `pub const`
/// strings in `crate::life_node::coach_prompts::templates` so they are
/// compile-time baked into the binary (version-pinned per build, no I/O,
/// no missing-file risk at run time).
///
/// Returns the empty string for unknown names — callers (`propose_tomorrow_
/// action`) supply only the two known names so this is effectively
/// total in practice; the empty-string fallback keeps the function infallible
/// for the simple two-call site pattern.
fn load_prompt_template(name: &str) -> String {
    use crate::life_node::coach_prompts::templates::{
        COACH_SYSTEM_PROMPT, TOMORROW_ACTION_PROMPT,
    };
    match name {
        "coach_system" => COACH_SYSTEM_PROMPT.to_string(),
        "tomorrow_action" => TOMORROW_ACTION_PROMPT.to_string(),
        _ => String::new(),
    }
}

/// Stage 4 helper — call `providers_wire::complete` with the canonical
/// `provider:model` id + system / user prompts. Surfaces the raw provider
/// response envelope for text extraction. Stays Stage 4 until providers_wire
/// ships its own Stage 3 (currently still Stage 2 pseudocode).
fn call_providers_complete(
    model_id: &str,
    system_prompt: &str,
    user_prompt: &str,
) -> Result<crate::providers_wire::ProviderResponse, CoachError> {
    use crate::providers_wire::{
        complete_with_fallback, Message, MessageRole, ProviderRequest, ResponseFormat,
    };

    // `model_id` is the canonical `provider:model`; `ProviderRequest.model`
    // wants the bare model name. It only seeds the request template — the
    // agents.toml `[routing].fallback_chain` decides the actual provider order.
    let bare_model = model_id
        .split_once(':')
        .map(|(_, m)| m)
        .unwrap_or(model_id);

    let req = ProviderRequest {
        model: bare_model.to_string(),
        system_prompt: Some(system_prompt.to_string()),
        messages: vec![Message::text(MessageRole::User, user_prompt)],
        max_tokens: None,
        temperature: None,
        response_format: ResponseFormat::PlainText,
    };

    // Every provider-side failure (chain exhausted, auth, network, rate
    // limit, model-not-found) collapses to the coach degraded path per
    // SPEC-23 §11.1 — the underlying ProviderError detail is intentionally
    // dropped here; the engine only cares that no provider answered.
    complete_with_fallback(req).map_err(|_| CoachError::LlmAllProvidersFailed)
}

/// Pull the assistant text out of a providers_wire response envelope. The
/// envelope is the JSON body of the provider response; we look for the
/// canonical `{"text": "..."}` field first, then fall back to `{"content":
/// "..."}` (OpenAI-shape) — empty content maps to `LlmAllProvidersFailed`.
///
/// When the response is plain text (non-JSON), we treat the whole thing as
/// the action — providers_wire Stage 3 will normalise envelopes upstream.
fn extract_response_text(response: &str) -> Result<String, CoachError> {
    // Try strict JSON first.
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(response) {
        if let Some(s) = v.get("text").and_then(|t| t.as_str()) {
            return Ok(s.to_string());
        }
        if let Some(s) = v.get("content").and_then(|t| t.as_str()) {
            return Ok(s.to_string());
        }
        // JSON shape with no known text field → degraded.
        return Err(CoachError::LlmAllProvidersFailed);
    }
    // Plain text fallback.
    let trimmed = response.trim();
    if trimmed.is_empty() {
        return Err(CoachError::LlmAllProvidersFailed);
    }
    Ok(trimmed.to_string())
}

/// Inject tiered memories per `policy`. Stage 2 calls SPEC-25
/// `hermes::recall(query, policy)`; if SPEC-25 is **not yet built** (Stage
/// 1 reality — cycle-break note), this returns
/// `Ok(MemoryInject::default())` so coach gracefully degrades to「no extra
/// memory injected」rather than hard-failing.
///
/// 中文: 注入三層記憶。Stage 2 接 SPEC-25 hermes recall；SPEC-25 還沒做
/// 好就回空 `MemoryInject`（cycle-break：不要因為 SPEC-25 還沒 ready 就
/// 卡死 coach engine）。
pub fn inject_tiered_memory(
    query: &str,
    policy: &RecallPolicy,
) -> Result<MemoryInject, CoachError> {
    // Step 1 — call SPEC-25 skill_wire::recall_skills with the day's brief
    // as the query + the caller's `RecallPolicy` (core_all / recall_k /
    // archival_k). Stage 3 wires the actual cross-crate call.
    let recall_result = recall_pseudo(query, policy);

    // Step 2 — graceful degrade per cycle-break note: SPEC-25 missing OR
    // recall errored OR recall returned empty → emit MemoryInject::default()
    // so coach does not hard-fail when hermes is not yet built.
    let _raw = match recall_result {
        Ok(v) if !v.is_empty() => v,
        _ => return Ok(MemoryInject::default()),
    };

    // Step 3 — place the recalled snippets into the `recall` tier. The `core`
    // (always-on identity / goals) and `archival` tiers stay empty until
    // SPEC-25 hermes lands its own tiering; fts5 only populates `recall`.
    Ok(MemoryInject {
        core: Vec::new(),
        recall: _raw,
        archival: Vec::new(),
    })
}

/// Stage 4 helper — invoke SPEC-25 `skill_wire::recall_skills(query, policy)`.
/// Returns the flat `Vec<String>` of recalled snippets the wire converts into
/// the tiered `MemoryInject`. Cycle-break: error path is swallowed upstream
/// into `MemoryInject::default()` rather than propagating. Stays Stage 4 until
/// skill_wire ships its own Stage 3 (currently still Stage 2 pseudocode).
fn recall_pseudo(query: &str, _policy: &RecallPolicy) -> Result<Vec<String>, CoachError> {
    // Force fts5-only recall (`recall_k = 0`): the embedding leg
    // (`skill_wire::embedding_search`) is `unimplemented!` until the `ort` +
    // all-MiniLM crate lands, and `recall_skills` only invokes it when
    // `recall_k > 0`. `fts5_search` degrades to an empty result when the
    // `skills_fts` index is absent, so this returns `Ok(vec![])` today and a
    // real keyword-recalled set once the skill index ships — never panicking.
    let policy = RecallPolicy {
        core_all: true,
        recall_k: 0,
        archival_k: 0,
    };
    match crate::skill_wire::recall_skills(query, policy) {
        Ok(r) => Ok(r.skills.into_iter().map(|s| s.name).collect()),
        // Recall failures are non-fatal — the caller swallows an empty/err
        // result into `MemoryInject::default()` (cycle-break note).
        Err(_) => Ok(Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coach_review_ready_payload_round_trip_smoke() {
        // §9.7 invariant: the unified 5-field payload schema (review_id,
        // event_id, date, takeaways_count, markdown_path) survives a
        // Rust → JSON → Rust round-trip byte-identical. SPEC-24 delivery
        // consumes the same shape; any field rename here is a wire-break.
        let p = CoachReviewReadyPayload {
            review_id: "01923f8e-7a4c-7000-8c2d-2b9f0e1d4a55".to_string(),
            event_id: "01923f8e-7a4c-7000-8c2d-2b9f0e1d4a55".to_string(),
            date: "2026-05-24".to_string(),
            takeaways_count: 3,
            markdown_path: "/home/u/.phantom-mesh/coach/2026-05-24.md.age".to_string(),
        };
        let j = serde_json::to_string(&p).unwrap();
        let back: CoachReviewReadyPayload = serde_json::from_str(&j).unwrap();
        assert_eq!(p.review_id, back.review_id);
        assert_eq!(p.event_id, back.event_id);
        assert_eq!(p.date, back.date);
        assert_eq!(p.takeaways_count, back.takeaways_count);
        assert_eq!(p.markdown_path, back.markdown_path);

        // Verify the 5-field schema is exactly 5 keys — guards against
        // accidental field additions sneaking in without a SPEC bump.
        let v: serde_json::Value = serde_json::from_str(&j).unwrap();
        let obj = v.as_object().expect("payload is JSON object");
        assert_eq!(
            obj.len(),
            5,
            "CoachReviewReadyPayload must stay 5 fields (SPEC-23 §9.7 + SPEC-24 §20.1); got keys: {:?}",
            obj.keys().collect::<Vec<_>>()
        );
        assert!(obj.contains_key("reviewId"));
        assert!(obj.contains_key("eventId"));
        assert!(obj.contains_key("date"));
        assert!(obj.contains_key("takeawaysCount"));
        assert!(obj.contains_key("markdownPath"));
    }

    #[test]
    fn daily_review_request_default_recall_k_is_five() {
        // §7.1 / SPEC-25 audit: coach pins recall_k = 5. Verify serde
        // `default` attribute fills it in when JSON omits the field.
        let j = r#"{"date":"2026-05-24"}"#;
        let req: DailyReviewRequest = serde_json::from_str(j).unwrap();
        assert_eq!(req.date, "2026-05-24");
        assert_eq!(req.recall_k, 5, "default recall_k must be 5 per SPEC-25 audit pin");
    }

    #[test]
    fn recall_policy_default_is_coach_pinned() {
        // SPEC-25 §7 audit: coach pinned at (core_all=true, recall_k=5,
        // archival_k=0). RecallPolicy::default() must match.
        let p = RecallPolicy::default();
        assert!(p.core_all);
        assert_eq!(p.recall_k, 5);
        assert_eq!(p.archival_k, 0);
    }

    #[test]
    fn review_status_serializes_snake_case() {
        // §8 state machine surface: status strings on the wire must stay
        // snake_case (`"completed"` / `"degraded"`), not PascalCase.
        let j = serde_json::to_string(&ReviewStatus::Completed).unwrap();
        assert_eq!(j, "\"completed\"");
        let j = serde_json::to_string(&ReviewStatus::Degraded).unwrap();
        assert_eq!(j, "\"degraded\"");
        let j = serde_json::to_string(&ReviewStatus::Failed).unwrap();
        assert_eq!(j, "\"failed\"");
    }

    #[test]
    fn coach_error_serializes_with_code_tag() {
        // §11.1 invariant: error wire shape uses `{"code": "..."}` tag so
        // the UI can dispatch on the machine-readable code string. Verify
        // a couple of variants survive round-trip.
        let e = CoachError::DbFull;
        let j = serde_json::to_string(&e).unwrap();
        assert!(j.contains("db_full"), "wire shape: {}", j);

        let e2 = CoachError::AlreadyRunning {
            date: "2026-05-24".to_string(),
        };
        let j2 = serde_json::to_string(&e2).unwrap();
        assert!(j2.contains("already_running"), "wire shape: {}", j2);
        assert!(j2.contains("2026-05-24"), "payload preserved: {}", j2);
    }

    #[test]
    fn memory_inject_default_is_all_empty() {
        // Cycle-break invariant: when SPEC-25 is not yet built,
        // `inject_tiered_memory` Stage 2 falls back to
        // `MemoryInject::default()` — verify that fallback shape is
        // safe (all three Vecs empty, no panics on `.iter()`).
        let m = MemoryInject::default();
        assert!(m.core.is_empty());
        assert!(m.recall.is_empty());
        assert!(m.archival.is_empty());
    }

    // ─── Stage 3 KAT (known-answer-test) vectors ─────────────────────────
    //
    // The Stage 2 `#[should_panic(expected = "Stage 3")]` markers for
    // `aggregate` and `inject_tiered_memory` were deleted in the Stage 3
    // commit. `aggregate` is now real (pure string concat); the KAT below
    // pins the exact markdown shape consumers (LLM brief + UI render) read.
    // `inject_tiered_memory` still routes through Stage 4 `recall_pseudo` so
    // its panic marker is upgraded to `"Stage 4"` rather than removed.

    #[test]
    fn aggregate_empty_returns_placeholder_markdown() {
        // Empty input → header + zero-count + the canonical "no events" line.
        // This shape is what coach UI renders when query_events finds nothing
        // (e.g. brand-new install or backfill date with no captures).
        let md = aggregate(&[]);
        assert!(md.contains("# Daily review — <date>"), "header: {}", md);
        assert!(md.contains("**Events captured:** 0"), "count: {}", md);
        assert!(md.contains("(no events for this date)"), "stub: {}", md);
    }

    #[test]
    fn aggregate_one_event_pins_section_shape() {
        // Single event with one tag → exactly one `## <tag> (1)` section
        // with a `- **<kind>** (<timestamp>): <summary>` bullet. This shape
        // is the contract LLM prompt + ts-rs UI rendering both depend on.
        let evt = (
            EventMeta {
                event_id: "evt-1".to_string(),
                timestamp: "2026-05-24T08:00:00Z".to_string(),
                kind: crate::rpc_wire::EventKind::Food,
                tags: vec!["fat_loss".to_string()],
            },
            AnalysisResult {
                summary: "Caesar salad in target range".to_string(),
                confidence: 0.9,
                goal_impact: "-150 kcal vs target".to_string(),
                suggestion: "keep it up".to_string(),
                cost_usd: 0.0,
                latency_ms: 50,
                model_id: "test:m".to_string(),
                raw_response: "{}".to_string(),
            },
        );
        let md = aggregate(std::slice::from_ref(&evt));
        assert!(md.contains("# Daily review — 2026-05-24"), "date: {}", md);
        assert!(md.contains("**Events captured:** 1"), "count: {}", md);
        assert!(md.contains("## fat_loss (1)"), "section: {}", md);
        assert!(
            md.contains("- **food** (2026-05-24T08:00:00Z): Caesar salad in target range"),
            "bullet: {}",
            md
        );
    }

    #[test]
    fn aggregate_untagged_event_lands_in_untagged_section() {
        // Events with empty `tags` Vec fall under the `untagged` bucket so
        // they remain visible in the review (rather than getting silently
        // dropped). This is the SPEC-23 §7.1 capture-completeness rule.
        let evt = (
            EventMeta {
                event_id: "evt-2".to_string(),
                timestamp: "2026-05-24T09:00:00Z".to_string(),
                kind: crate::rpc_wire::EventKind::Text,
                tags: vec![],
            },
            AnalysisResult {
                summary: "".to_string(),
                confidence: 0.0,
                goal_impact: "".to_string(),
                suggestion: "".to_string(),
                cost_usd: 0.0,
                latency_ms: 0,
                model_id: "".to_string(),
                raw_response: "".to_string(),
            },
        );
        let md = aggregate(std::slice::from_ref(&evt));
        assert!(md.contains("## untagged (1)"), "section: {}", md);
        assert!(md.contains("(no summary)"), "empty-summary placeholder: {}", md);
    }

    #[test]
    fn shame_free_lint_rejects_known_pattern() {
        // SPEC-23 G3 invariant: shame patterns map to `LlmAllProvidersFailed`
        // so the engine collapses to the same `degraded` path as the
        // all-providers-fail case (per §11.1 catalog).
        let r = shame_free_lint("你又熬夜了");
        assert!(matches!(r, Err(CoachError::LlmAllProvidersFailed)));
    }

    #[test]
    fn shame_free_lint_accepts_clean_text() {
        // Positive control — neutral imperative copy must pass so legitimate
        // LLM output does not get false-positive rejected into degraded.
        let r = shame_free_lint("明天試試早上 10 分鐘散步");
        assert!(r.is_ok(), "expected Ok, got {:?}", r);
    }

    #[test]
    fn load_prompt_template_returns_known_templates() {
        // Pins the wire ↔ templates module integration: both known names
        // must resolve to non-empty `pub const` strings. Unknown names
        // return empty so caller renders a defensive empty prompt.
        let sys = load_prompt_template("coach_system");
        let usr = load_prompt_template("tomorrow_action");
        assert!(!sys.is_empty(), "coach_system must be non-empty");
        assert!(usr.contains("{BRIEF}"), "tomorrow_action must keep {{BRIEF}}");
        assert_eq!(load_prompt_template("does_not_exist"), "");
    }

    #[test]
    fn extract_response_text_json_text_field() {
        // Canonical providers_wire envelope shape: `{"text": "..."}`.
        let r = extract_response_text(r#"{"text":"go for a walk"}"#).unwrap();
        assert_eq!(r, "go for a walk");
    }

    #[test]
    fn extract_response_text_json_content_fallback() {
        // OpenAI-shape fallback: `{"content": "..."}`.
        let r = extract_response_text(r#"{"content":"do 10 pushups"}"#).unwrap();
        assert_eq!(r, "do 10 pushups");
    }

    #[test]
    fn extract_response_text_plain_text_passthrough() {
        // Non-JSON response → treated as plain text body (whitespace trimmed).
        let r = extract_response_text("  walk 5 min  \n").unwrap();
        assert_eq!(r, "walk 5 min");
    }

    #[test]
    fn extract_response_text_empty_maps_to_degraded() {
        // Empty content (JSON or plain) must surface as
        // `LlmAllProvidersFailed` so the caller's degraded-path
        // mapping triggers (UI shows the "建議 still cooking" card).
        let r = extract_response_text("   ");
        assert!(matches!(r, Err(CoachError::LlmAllProvidersFailed)));
    }

    #[test]
    fn inject_tiered_memory_returns_ok_via_real_recall() {
        // Stage 4: `inject_tiered_memory` now routes through the real
        // `skill_wire::recall_skills` (fts5-only — recall_k forced to 0 to skip
        // the still-`unimplemented!` embedding leg) instead of panicking with a
        // Stage-4 marker. It must return Ok and never panic. We do NOT assert
        // emptiness here: this lib test runs against the real $HOME, which may
        // legitimately carry a populated skill index — the degraded-to-empty
        // path is covered hermetically in tests/cuj04_stats_only_fallback.rs.
        let mem = inject_tiered_memory("brief", &RecallPolicy::default())
            .expect("inject_tiered_memory must not error");
        // core + archival tiers are always empty until hermes lands its tiering.
        assert!(mem.core.is_empty() && mem.archival.is_empty());
    }

    #[test]
    fn coach_review_degraded_payload_round_trip_smoke() {
        let p = CoachReviewDegradedPayload {
            review_id: "01923f8e-7a4c-7000-8c2d-2b9f0e1d4a55".to_string(),
            event_id: "01923f8e-7a4c-7000-8c2d-2b9f0e1d4a55".to_string(),
            date: "2026-05-24".to_string(),
            reason: "all_providers_failed".to_string(),
        };
        let j = serde_json::to_string(&p).unwrap();
        let back: CoachReviewDegradedPayload = serde_json::from_str(&j).unwrap();
        assert_eq!(p.review_id, back.review_id);
        assert_eq!(p.date, back.date);
        assert_eq!(p.reason, back.reason);
    }
}
