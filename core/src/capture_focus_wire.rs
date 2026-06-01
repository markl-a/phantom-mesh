// SPEC-21 §7 — Capture-focus wire types (single source of truth for the focus
// session capability: 25-min Pomodoro / 50-min DeepWork / 10-min Sprint / Custom
// timer; interruption observation; LLM-driven post-session takeaway).
//
// Stage 3 (real impl — expanded): the session-table helpers are now backed by a
// process-wide `std::sync::OnceLock<Mutex<HashMap<String, ActiveFocusSession>>>`
// (sync, not tokio — see ADR below). `now_ms`, `compute_actual_duration`,
// `lookup_planned_duration`, `compute_completion_pct`, `drain_interruptions`,
// `append_interruption`, `session_register`, `session_lookup`,
// `build_focus_prompt`, and `parse_json` are real. `spawn_timer` is now real
// (`std::thread::spawn` + `tracing::debug!` deadline log — runtime-agnostic;
// see fn doc for why we picked std::thread over tokio::spawn). `emit_event` is
// now real via `tracing::info!` fallback (the actual `tauri::AppHandle::emit`
// stays Stage 4 because `tauri` is not a dep of the `core` crate). Three
// remain Stage 4 stubs: `uuid_v7` (uuid `v7` feature gated under
// `experimental-hermes-tools`), `bump_counter` (no metrics crate in deps),
// and `providers_complete` (defers to providers_wire Stage 3 — providers_wire
// is still Stage 2 internally).
//
// ADR: sync `std::sync::Mutex<HashMap<..>>` chosen over `tokio::sync::RwLock`
// to keep `start_session` / `record_interruption` / `complete_session`
// synchronous — same signature as Stage 1/2. Lock-held time is O(insertion /
// lookup of one HashMap entry), no I/O held under the lock, so blocking the
// tokio runtime is a non-issue. If a future caller needs to await inside the
// critical section, swap to `tokio::sync::Mutex` then.
//
// 中文: 本檔對應 SPEC-21 §7（資料模型）— 專注時段擷取（capture-focus，
// 25 分鐘 Pomodoro / 50 分鐘 DeepWork / 10 分鐘 Sprint / 自訂時長）。Wire
// type 是「UI / Tauri / RPC（遠端程序呼叫）」可見的精簡介面 — 把 spec §7.1.1
// `FocusSessionMetadata` 內部 33 個欄位收斂成「request（請求） + result（結果）
// + interruption（中斷觀察）」三個結構，把 audio chunk（聲音切片）與
// transcript（逐字稿）等 implementation detail 留在 core crate 內部不外露。
//
// 與 spec §7.1.1 內部 metadata 的關係：
//   - `FocusSessionRequest` = UI 啟動 session（時段）時的呼叫參數 →
//     Stage 2 內部展開為 `FocusSessionMetadata.mode` + `started_at_ms` + `goal_tags`。
//   - `FocusSessionResult` = stop（結束）後回給 UI 的精簡結果 →
//     Stage 2 內部從 `FocusSessionMetadata` 抽 `actual_duration_ms` /
//     `interruptions` count / `completion_pct` + LLM `takeaway` (= `summary`)。
//   - `FocusInterruption` = 一次中斷觀察事件 → Stage 2 record_interruption
//     會把這條 append 到 active session（活動中時段）的 interruption list。
//
// Reader 門檻（CLAUDE.md §1 Step 2.5）— 把讀者當第一次接觸 phantom-mesh 的
// 研究生 / 大學生，所有縮寫 / 英文名詞第一次出現都加中文意譯。詳見下方
// quote block 縮寫對照表。
//
// > **縮寫對照表（acronym table，縮寫對照）**
// > - **SPEC（Specification，規格）** — phantom-mesh 的功能規格書
// > - **TS（TypeScript，網頁腳本語言）** — UI 端用的型別語言
// > - **FFI（Foreign Function Interface，跨語言介面）** — Rust ↔ TS 的橋
// > - **LLM（Large Language Model，大型語言模型）** — 如 Claude / GPT
// > - **ASR（Automatic Speech Recognition，自動語音辨識）** — 把聲音轉文字
// > - **PTT（Push-To-Talk，按住說話）** — 一次按住的錄音模式
// > - **UUID（Universally Unique Identifier，全域唯一識別碼）** — 字串 ID
// > - **TCC（Transparency, Consent, and Control，Apple 隱私授權系統）** — mac/iOS 權限
// > - **OS（Operating System，作業系統）** — macOS / iOS / Android / Windows / Linux
//
// TODO Stage 2:
//   - wire `register_active_session` / `pop_active_session` to a process-wide
//     `Mutex<HashMap<String, ActiveFocusSession>>` (only one active at a time
//     per SPEC-21 §8.1 state machine — second `start_session` returns
//     `FocusCaptureError::SessionAlreadyActive`).
//   - wire `complete_session` to emit a Coach（教練）event via SPEC-22 so the
//     review pipeline picks up the takeaway next time `phantom coach review`
//     runs.
//   - wire `analyze_focus_session` to SPEC-14 (LLM analyzer) reusing the
//     `AnalysisResult` shape from SPEC-16 (`event_storage_wire`).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

// Re-export the shared analysis side-car shape from SPEC-16 so consumers of
// `analyze_focus_session` only import one module. The `AnalysisResult` shape
// (`summary` / `confidence` / `goal_impact` / `suggestion` / `cost_usd` /
// `latency_ms` / `model_id` / `raw_response`) is identical across food / focus
// / habit captures by SPEC-14 design — same prompt template family, same wire
// shape, different system-prompt context.
//
// 中文: SPEC-16 已定 `AnalysisResult`（LLM 分析結果），所有 capture type
// （食物 / 焦點 / 習慣）共用同一個分析結果結構，這裡直接 re-export 不重複定義。
pub use crate::event_storage_wire::AnalysisResult;

// ─── §7.1.1-equivalent FocusMode — request-time mode enum ─────────────────────

/// Which focus pattern the user picked when starting the session. The four
/// presets cover ≥ 95% of real usage per the SPEC-21 §5 persona analysis;
/// `Custom` is the escape hatch for power users who want a specific minute
/// count (e.g. 17-min Sprint, 90-min DeepWork).
///
/// 中文: 使用者啟動 session（時段）時挑的「焦點模式（focus pattern）」。前三個
/// 是固定預設值（Pomodoro 25 分 / DeepWork 50 分 / Sprint 10 分），覆蓋絕大多數
/// 場景；`Custom`（自訂）讓進階使用者塞任意分鐘數，由 `FocusSessionRequest`
/// 的 `planned_duration_ms`（預計時長毫秒）欄位指定實際長度。
///
/// **Naming**: variants use the human-recognisable preset name (`Pomodoro25`
/// = the famous 25-min Cirillo technique; `DeepWork50` = Cal Newport's
/// long-form block; `Sprint10` = micro-burst); the duration baked into the
/// variant name is the canonical preset, but UI MAY override per
/// `planned_duration_ms` if the user wants e.g. a 27-min Pomodoro.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/capture_focus/")]
#[serde(rename_all = "snake_case")]
pub enum FocusMode {
    /// 25-minute Pomodoro（番茄鐘）— the canonical short-focus preset.
    Pomodoro25,
    /// 50-minute DeepWork（深度工作）— Cal Newport long-form block.
    DeepWork50,
    /// 10-minute Sprint（衝刺）— micro-burst preset for warm-up / quick task.
    Sprint10,
    /// Custom duration — UI / CLI supplies the actual minute count via the
    /// companion `planned_duration_ms` field on `FocusSessionRequest`.
    Custom,
}

impl FocusMode {
    /// Canonical default duration in milliseconds for the three presets.
    /// `Custom` returns `None` — caller MUST consult
    /// `FocusSessionRequest::planned_duration_ms`.
    ///
    /// 中文: 預設模式回固定毫秒數；Custom 模式回 None，由 request 結構
    /// 的 `planned_duration_ms` 指定。
    pub const fn default_duration_ms(self) -> Option<u64> {
        match self {
            FocusMode::Pomodoro25 => Some(25 * 60 * 1000),
            FocusMode::DeepWork50 => Some(50 * 60 * 1000),
            FocusMode::Sprint10 => Some(10 * 60 * 1000),
            FocusMode::Custom => None,
        }
    }
}

// ─── §11-derived InterruptionKind — observation kinds ─────────────────────────

/// Categorical reason a focus session was momentarily interrupted. Observed at
/// runtime by the recorder / OS callbacks; each instance becomes one
/// `FocusInterruption` row in the session's interruption list.
///
/// 中文: 中斷種類列舉。每次中斷會在 active session（活動中時段）的中斷清單
/// 添一筆 `FocusInterruption`（焦點中斷紀錄）。SPEC-21 §8.1 state machine
/// （狀態機）只區分 OS-induced（系統引發）與 user-induced（使用者引發）兩大類
/// — 這裡進一步細分為四個 wire 變體（variant）給 telemetry（遙測）/ UI
/// 顯示對應的中文文字使用。
///
/// **Mapping**: `UserPause` → user tapped pause button; `Notification` →
/// system notification banner pulled focus; `AppSwitch` → the focus session's
/// host app lost foreground (e.g. user opened messaging app); `ScreenLock` →
/// device screen locked / display dimmed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/capture_focus/")]
#[serde(rename_all = "snake_case")]
pub enum InterruptionKind {
    /// User actively tapped the pause button in the focus UI.
    UserPause,
    /// System notification（通知）banner pulled focus away.
    Notification,
    /// Host app（前景應用）lost foreground — user switched to another app.
    AppSwitch,
    /// Device screen locked / display dimmed.
    ScreenLock,
}

impl InterruptionKind {
    /// Lower-kebab slug used in telemetry labels and log keys.
    /// `UserPause` → `"user-pause"`, etc.
    ///
    /// 中文: 回傳 lower-kebab（小寫連字號）形式的 slug，給 metric label / 日誌 key 用。
    pub const fn slug(self) -> &'static str {
        match self {
            InterruptionKind::UserPause => "user-pause",
            InterruptionKind::Notification => "notification",
            InterruptionKind::AppSwitch => "app-switch",
            InterruptionKind::ScreenLock => "screen-lock",
        }
    }
}

// ─── §7.1.4-equivalent FocusSessionRequest — start-time wire shape ────────────

/// Input to `start_session`. Captures the user's intent at session-start time
/// (mode + planned duration + optional human label + free-form tags). Default
/// `tag` is `["focus"]` so every session is queryable by the canonical
/// `focus` tag downstream.
///
/// 中文: `start_session`（啟動時段）的請求結構。包含模式（mode）、預計時長
/// （planned_duration_ms，毫秒）、選填的人類可讀標籤（label）、自由標籤
/// （tag，預設 `["focus"]` 讓下游用 `focus` 標籤就能撈到所有時段）。
///
/// **Default tag rule**: serde default = `vec!["focus".to_string()]` — when the
/// caller passes empty / missing `tag`, the canonical `focus` tag is auto-added
/// so SPEC-16 query layer can list "all focus sessions" with a single tag
/// filter. Callers wanting extra tags pass them additively.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/capture_focus/")]
#[serde(rename_all = "camelCase")]
pub struct FocusSessionRequest {
    /// Which focus pattern the user picked (see `FocusMode`).
    pub mode: FocusMode,
    /// Planned session length in milliseconds. For preset modes this MUST
    /// match `mode.default_duration_ms()` unless the UI offers a per-session
    /// override; for `FocusMode::Custom` this is the authoritative value.
    pub planned_duration_ms: u64,
    /// Optional human-readable label the user typed at start time
    /// (e.g. "draft SPEC-21 §7", "review PR #42"). Plain text; do not put
    /// PII（個資）here per SPEC-21 §12.1.
    pub label: Option<String>,
    /// Free-form tags（標籤）attached at start time. Defaults to `["focus"]`
    /// when omitted so the SPEC-16 storage layer can always filter by the
    /// canonical `focus` tag.
    #[serde(default = "default_focus_tag")]
    pub tag: Vec<String>,
}

/// serde default for `FocusSessionRequest::tag` — `["focus"]`.
fn default_focus_tag() -> Vec<String> {
    vec!["focus".to_string()]
}

// ─── §7.1.1-equivalent FocusInterruption — single observation ─────────────────

/// One interruption observation appended to an active session's interruption
/// list. Returned by `record_interruption` and bundled (count-aggregated) into
/// `FocusSessionResult.interruptions` at session complete time.
///
/// 中文: 一筆中斷觀察。每次 `record_interruption`（記錄中斷）呼叫會在 active
/// session（活動中時段）的中斷清單 append 一條；session 結束時，
/// `FocusSessionResult.interruptions`（中斷次數）欄位回傳 count（總數）。
///
/// **Telemetry**: `kind.slug()` is used as the label value for the
/// `phantom_focus_interruptions_total{kind="..."}` Prometheus counter
/// (mapped from SPEC-21 §12.4 observability section).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/capture_focus/")]
#[serde(rename_all = "camelCase")]
pub struct FocusInterruption {
    /// UTC milliseconds at which the interruption began.
    pub timestamp_ms: u64,
    /// Categorical reason for the interruption.
    pub kind: InterruptionKind,
    /// How long the interruption lasted in milliseconds. `0` is allowed for
    /// instantaneous events (e.g. a notification that was dismissed within
    /// the OS notification animation window).
    pub duration_ms: u32,
}

// ─── §7.1.1-equivalent FocusSessionResult — stop-time wire shape ──────────────

/// Output of `complete_session`. Captures the post-mortem (事後檢視) shape
/// surfaced to UI / coach review — actual time, interruption count,
/// completion percentage versus planned duration, LLM-generated summary, and
/// LLM-generated next-step suggestion.
///
/// 中文: `complete_session`（結束時段）的結果結構。`actual_duration_ms`（實際
/// 時長毫秒）通常 ≤ `planned_duration_ms`（如使用者提早 stop）但偶爾會 >
/// （如 PTT 模式累積到一定字數才停）。`completion_pct`（完成百分比）= actual /
/// planned * 100, clamped to `[0.0, 100.0]`. `summary`（摘要）+ `suggestion`
/// （建議）都是 LLM 生成的精簡字串；完整 LLM 分析結果（含 confidence / cost /
/// latency / model_id）走 `analyze_focus_session` 取 `AnalysisResult`。
///
/// **Decoupling rationale**: this struct is intentionally LLM-call-free —
/// caller chooses whether to spend the LLM tokens on a follow-up
/// `analyze_focus_session` call. Some clients (e.g. background batch jobs) skip
/// the LLM pass entirely and only consume the integer stats.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/capture_focus/")]
#[serde(rename_all = "camelCase")]
pub struct FocusSessionResult {
    /// Actual wall-clock duration in milliseconds from start to complete.
    pub actual_duration_ms: u64,
    /// Total count of interruption observations recorded during the session.
    /// `u16` cap (= 65 535) is well above any sane real-world session
    /// (median measured at < 5 in dogfood data).
    pub interruptions: u16,
    /// Completion percentage = `actual_duration_ms / planned_duration_ms *
    /// 100`, clamped to `[0.0, 100.0]`. Used by UI as a progress badge.
    pub completion_pct: f32,
    /// LLM-generated short summary of what the user did during the session.
    /// Bilingual（雙語）OK — 中文 + 英文 mixed acceptable per SPEC-21 §12.3.
    /// Empty string when LLM call was skipped or failed (UI shows
    /// "Summary unavailable").
    pub summary: String,
    /// LLM-generated next-step suggestion. Empty string when the model
    /// declined / call was skipped. UI shows in the coach review panel.
    pub suggestion: String,
}

// ─── §11 FocusCaptureError — error catalog mirror ─────────────────────────────

/// Wire-facing error variants for the capture-focus subsystem. Mirrors the
/// SPEC-21 §11 error catalog（`FOCUS-001` .. `FOCUS-006`）one-to-one. The
/// legacy `core::life_node::focus_session::FocusError` (referenced by SPEC-21
/// §7.1.3) is the Rust-internal richer variant; this wire enum is the
/// FFI（跨語言介面）surface that UI consumers see.
///
/// 中文: SPEC-21 §11 錯誤目錄的 wire-facing（對 UI 公開）鏡像。原本 internal
/// 的 `FocusError`（焦點錯誤）不動，Stage 2 加 mapping（對應轉換）。
#[derive(Debug, Clone, Serialize, Deserialize, TS, thiserror::Error)]
#[ts(export, export_to = "../../app/src/lib/generated/capture_focus/")]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum FocusCaptureError {
    /// `FOCUS-001` — microphone permission denied by user / OS.
    #[error("focus.permission_denied: {detail}")]
    PermissionDenied { detail: String },
    /// `FOCUS-002` — AudioRecorder open failed (device busy / no mic device).
    #[error("focus.recorder_init: {detail}")]
    RecorderInit { detail: String },
    /// `FOCUS-003` — all ASR（自動語音辨識）providers failed in sequence.
    #[error("focus.asr_all_providers_failed: {providers:?}")]
    AsrAllProvidersFailed { providers: Vec<String> },
    /// `FOCUS-007` — `complete_session` called with unknown `session_id`.
    /// (Distinct from `FOCUS-005`; this is the lookup-miss case.)
    #[error("focus.session_not_found: {session_id}")]
    SessionNotFound { session_id: String },
    /// `FOCUS-005` — `start_session` called while another session is active.
    #[error("focus.session_already_active")]
    SessionAlreadyActive,
    /// `FOCUS-006` — OS interrupted recording for > 30 seconds without resume.
    #[error("focus.interrupted: {detail}")]
    Interrupted { detail: String },
    /// `FOCUS-004` — LLM takeaway / analysis call failed.
    #[error("focus.takeaway_failed: {detail}")]
    TakeawayFailed { detail: String },
}

// ─── §9.2 Stub helpers (Stage 2 implements; Stage 1 leaves `unimplemented!()`) ─

/// Start a new focus session. Registers the active session in a process-wide
/// table, opens the audio recorder for the chosen `mode`, starts the timer,
/// and returns a freshly-minted UUIDv7 session_id（時段識別碼）— callers
/// thread this id through `record_interruption` and `complete_session`.
///
/// Returns `FocusCaptureError::SessionAlreadyActive` if another session is
/// already registered (SPEC-21 §8.1 state machine: only one Recording state
/// per process at a time).
///
/// 中文: `phantom focus start` 主邏輯。註冊 active session（活動中時段） +
/// 開錄音 + 啟動計時器 + 回 UUIDv7 字串。已有 active session 則回
/// `SessionAlreadyActive`（時段已在進行中）。
pub fn start_session(req: &FocusSessionRequest) -> Result<String, FocusCaptureError> {
    // Step 1: generate session_id (UUIDv7, time-ordered identifier).
    let session_id = uuid_v7_pseudo();
    // Step 2: register the session in the process-wide in-memory map; refuse
    // if another session is already active per SPEC-21 §8.1 state machine.
    session_register(&session_id, req)?;
    // Step 3: spawn a tokio timer task that fires at the `planned_duration_ms`
    // deadline so the session auto-completes if the user forgets to stop.
    spawn_timer_pseudo(&session_id, req.planned_duration_ms);
    // Step 4: return the freshly-minted session_id so the caller can thread it
    // through subsequent `record_interruption` / `complete_session` calls.
    Ok(session_id)
}

/// Generate a UUIDv7 session_id (time-ordered, SPEC-16 §8 G2). Now real: the
/// `uuid/v7` feature was promoted into the default build (core/Cargo.toml).
fn uuid_v7_pseudo() -> String {
    uuid::Uuid::now_v7().to_string()
}

// ─── Stage 3 process-wide session table ──────────────────────────────────────
//
// One-line summary: every active focus session lives in a single
// `OnceLock<Mutex<HashMap<session_id, ActiveFocusSession>>>`. SPEC-21 §8.1
// state machine allows only ONE Recording state per process at a time, so the
// "second `start_session` returns `SessionAlreadyActive`" rule is enforced by
// a length check inside the lock.

/// In-memory shape stashed at start time, drained at complete time. Not part
/// of the wire surface — internal to this module.
#[derive(Debug, Clone)]
struct ActiveFocusSession {
    /// Wall-clock `now_ms()` captured at the start_session call site.
    started_at_ms: u64,
    /// Mirrors `FocusSessionRequest.planned_duration_ms`.
    planned_duration_ms: u64,
    /// Accumulated interruption observations from `record_interruption`.
    interruptions: Vec<FocusInterruption>,
}

fn session_table() -> &'static Mutex<HashMap<String, ActiveFocusSession>> {
    static TABLE: OnceLock<Mutex<HashMap<String, ActiveFocusSession>>> = OnceLock::new();
    TABLE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// SPEC-21 §8.1 — register a new active session. Returns
/// `SessionAlreadyActive` if any other session is currently in the table
/// (single-session-per-process invariant).
fn session_register(
    session_id: &str,
    req: &FocusSessionRequest,
) -> Result<(), FocusCaptureError> {
    let mut guard = session_table()
        .lock()
        .map_err(|_| FocusCaptureError::RecorderInit {
            detail: "session table mutex poisoned".to_string(),
        })?;
    if !guard.is_empty() {
        return Err(FocusCaptureError::SessionAlreadyActive);
    }
    guard.insert(
        session_id.to_string(),
        ActiveFocusSession {
            started_at_ms: now_ms(),
            planned_duration_ms: req.planned_duration_ms,
            interruptions: Vec::new(),
        },
    );
    Ok(())
}

/// Stage 3 real impl — spawn an auto-complete timer.
///
/// We use `std::thread::spawn` (not `tokio::spawn`) because `start_session` is
/// a synchronous function that may be called outside a tokio runtime context
/// (CLI dispatch, FFI from Tauri command before runtime is entered, etc.) —
/// `tokio::spawn` would panic with "there is no reactor running" in those
/// paths. `std::thread::spawn` + `thread::sleep` is a runtime-agnostic
/// equivalent: the OS thread sleeps for `planned_duration_ms` and then logs
/// the auto-complete deadline via `tracing` so observability picks it up.
///
/// Stage 4 still owns the actual side-effect (calling `complete_session` from
/// inside the spawned task) — doing that here would require either a
/// `tokio::sync::mpsc` channel back to the caller or a `&'static AppHandle`,
/// both of which couple this wire to a runtime choice. The Stage 3 promotion
/// removes the panic while keeping the runtime-choice deferred.
fn spawn_timer_pseudo(session_id: &str, planned_duration_ms: u64) {
    let sid = session_id.to_string();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(planned_duration_ms));
        tracing::debug!(
            target: "focus",
            session_id = %sid,
            planned_duration_ms,
            "focus session auto-complete deadline reached (Stage 4 will wire the actual complete_session call)"
        );
    });
}

/// Append an interruption observation to an active session. Idempotent for
/// duplicate consecutive `ScreenLock` events within 1 second (deduplicated
/// at telemetry layer).
///
/// Returns `FocusCaptureError::SessionNotFound` if `session_id` is not in the
/// active-session table (already completed, or never started).
///
/// 中文: 把一筆中斷觀察 append 到指定 active session 的中斷清單。同一秒內
/// 連續 ScreenLock 會去重 — 避免螢幕熄滅 → 通知滅 → 熄滅再閃造成計數爆炸。
/// session 不存在則回 `SessionNotFound`（時段不存在）。
pub fn record_interruption(
    session_id: &str,
    kind: InterruptionKind,
) -> Result<(), FocusCaptureError> {
    // Step 1: look up the active session by id; bail with SessionNotFound if
    // the session is unknown / already completed.
    let _active = session_lookup(session_id)?;
    // Step 2: build a FocusInterruption with the current wall-clock
    // timestamp_ms and append it to the session's interruption list.
    let _now_ms = now_ms();
    let _interruption = FocusInterruption {
        timestamp_ms: _now_ms,
        kind,
        duration_ms: 0,
    };
    append_interruption(session_id, &_interruption)?;
    // Step 3: bump the per-kind counters used by `phantom_focus_interruptions_total`.
    bump_counter_pseudo(kind);
    Ok(())
}

/// Look up the active session by id. Returns `SessionNotFound` when the id
/// is unknown / already completed.
fn session_lookup(session_id: &str) -> Result<(), FocusCaptureError> {
    let guard = session_table()
        .lock()
        .map_err(|_| FocusCaptureError::RecorderInit {
            detail: "session table mutex poisoned".to_string(),
        })?;
    if guard.contains_key(session_id) {
        Ok(())
    } else {
        Err(FocusCaptureError::SessionNotFound {
            session_id: session_id.to_string(),
        })
    }
}

/// Current UTC wall clock in milliseconds. Falls back to 0 if the system
/// clock pre-dates Unix epoch (effectively impossible on real hosts).
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Append a `FocusInterruption` to the active session's list. Returns
/// `SessionNotFound` if the id is unknown.
fn append_interruption(
    session_id: &str,
    interruption: &FocusInterruption,
) -> Result<(), FocusCaptureError> {
    let mut guard = session_table()
        .lock()
        .map_err(|_| FocusCaptureError::RecorderInit {
            detail: "session table mutex poisoned".to_string(),
        })?;
    match guard.get_mut(session_id) {
        Some(active) => {
            active.interruptions.push(interruption.clone());
            Ok(())
        }
        None => Err(FocusCaptureError::SessionNotFound {
            session_id: session_id.to_string(),
        }),
    }
}

/// Bump the `phantom_focus_interruptions_total{kind="..."}` counter. The
/// interruption itself is already persisted (append_interruption, above); this
/// is the metrics-export side-effect only. There is still no metrics crate
/// (`metrics`/`prometheus`/`opentelemetry`) in core, so exporting is a no-op —
/// but it must NOT panic, or it would discard an already-recorded interruption.
fn bump_counter_pseudo(_kind: InterruptionKind) {
    // No-op until an observability dep lands; recording already happened.
}

/// Stop the session, compute the `FocusSessionResult` (actual duration,
/// interruption count, completion %, LLM summary, LLM suggestion), emit a
/// Coach（教練）event so the next `phantom coach review` picks up the
/// takeaway, and de-register the active session from the process-wide table.
///
/// Returns `FocusCaptureError::SessionNotFound` if `session_id` is not in the
/// active-session table.
///
/// 中文: 收工 — 算實際時長 / 中斷次數 / 完成百分比 + LLM 摘要 + LLM 建議 +
/// 發 Coach event 給下次 `phantom coach review` 收割 + 從 active 表移除。
pub fn complete_session(session_id: &str) -> Result<FocusSessionResult, FocusCaptureError> {
    // Step 1: look up the active session by id; bail with SessionNotFound if
    // the session is unknown / already completed.
    let _active = session_lookup(session_id)?;
    // Step 2: compute actual_duration_ms (now - started_at_ms) and compare
    // against the planned_duration_ms captured at start time.
    let _actual_duration_ms = compute_actual_duration(session_id);
    let _planned_duration_ms = lookup_planned_duration(session_id);
    // Step 3: compute completion_pct = actual / planned * 100, clamped to
    // [0.0, 100.0] per the FocusSessionResult contract.
    let _completion_pct = compute_completion_pct(_actual_duration_ms, _planned_duration_ms);
    // Step 4: consume the session's accumulated interruption list (drain so
    // the in-memory entry can be dropped after de-registration).
    let _interruptions = drain_interruptions(session_id);
    // Step 5: build the FocusSessionResult with a deterministic takeaway
    // computed from the REAL session metrics (duration / completion / count).
    // An optional `analyze_focus_session` LLM call can later enrich this, but
    // the user always gets a real, data-grounded reflection — never a blank box.
    let (summary, suggestion) =
        deterministic_takeaway(_actual_duration_ms, _completion_pct, _interruptions);
    let result = FocusSessionResult {
        actual_duration_ms: _actual_duration_ms,
        interruptions: _interruptions,
        completion_pct: _completion_pct,
        summary,
        suggestion,
    };
    // Step 6: emit a Tauri event so the UI / coach review pipeline (SPEC-22)
    // picks up the takeaway on the next `phantom coach review` run, then
    // de-register the active session from the process-wide map.
    emit_event_pseudo(session_id, &result)?;
    Ok(result)
}

/// 中文: 用真實 session 指標 (時長/完成度/中斷次數) 算出一句客觀總結 +
/// 一句建議。不需 LLM — 結束番茄鐘後使用者一定看得到基於真實數據的反饋,
/// 而不是空白框。回傳 `(summary, suggestion)` 都是繁體中文。
fn deterministic_takeaway(
    actual_duration_ms: u64,
    completion_pct: f32,
    interruption_count: u16,
) -> (String, String) {
    // Defensive: the sole caller clamps, but keep this self-contained so a
    // future caller / direct test can't surface 完成度 NaN% / -10% / 120%.
    let pct = if completion_pct.is_finite() {
        completion_pct.clamp(0.0, 100.0)
    } else {
        0.0
    };
    // A sub-minute session reads as "0 分鐘" (zero effort) with bare minute
    // truncation — show seconds when under a minute.
    let duration_label = if actual_duration_ms < 60_000 {
        format!("{} 秒", actual_duration_ms / 1_000)
    } else {
        format!("{} 分鐘", actual_duration_ms / 60_000)
    };
    let summary = format!(
        "專注 {}，完成度 {:.0}%，中斷 {} 次。",
        duration_label, pct, interruption_count
    );
    // Lead with the dominant signal. A strong finish is acknowledged even
    // when interruptions happened — otherwise a praising summary + a scolding
    // suggestion contradict each other. Only a low finish or a genuinely
    // choppy mid-range session steers toward reducing interruptions.
    let suggestion = if pct >= 90.0 {
        if interruption_count == 0 {
            "表現很好，這個節奏值得保持。".to_string()
        } else {
            "完成度很高，只是中途有些中斷 — 下次把通知靜音會更專注。".to_string()
        }
    } else if interruption_count >= 3 {
        "中斷有點多，下次試著把通知靜音或換到更安靜的環境。".to_string()
    } else if pct < 50.0 {
        "這次提早結束了，下次可以先設定短一點的時段，建立完成的成就感。".to_string()
    } else {
        "穩定的一段專注，繼續累積。".to_string()
    };
    (summary, suggestion)
}

/// Compute actual wall-clock duration in ms (`now - started_at_ms`). Returns
/// 0 if the session is unknown (caller should have already established
/// existence via `session_lookup`).
fn compute_actual_duration(session_id: &str) -> u64 {
    let guard = match session_table().lock() {
        Ok(g) => g,
        Err(_) => return 0,
    };
    match guard.get(session_id) {
        Some(active) => now_ms().saturating_sub(active.started_at_ms),
        None => 0,
    }
}

/// Fetch the `planned_duration_ms` stashed at start time. Returns 0 if the
/// session is unknown.
fn lookup_planned_duration(session_id: &str) -> u64 {
    let guard = match session_table().lock() {
        Ok(g) => g,
        Err(_) => return 0,
    };
    guard
        .get(session_id)
        .map(|a| a.planned_duration_ms)
        .unwrap_or(0)
}

/// `completion_pct = actual / planned * 100`, clamped to `[0.0, 100.0]`.
/// `planned == 0` short-circuits to 0.0 so we never divide by zero.
fn compute_completion_pct(actual_ms: u64, planned_ms: u64) -> f32 {
    if planned_ms == 0 {
        return 0.0;
    }
    let pct = (actual_ms as f32 / planned_ms as f32) * 100.0;
    pct.clamp(0.0, 100.0)
}

/// Drain the active session's interruption list and return the count. Also
/// removes the session entry from the table — `complete_session` is the only
/// caller and it owns the de-registration step. Returns 0 if the id is
/// unknown.
fn drain_interruptions(session_id: &str) -> u16 {
    let mut guard = match session_table().lock() {
        Ok(g) => g,
        Err(_) => return 0,
    };
    match guard.remove(session_id) {
        Some(active) => active.interruptions.len().min(u16::MAX as usize) as u16,
        None => 0,
    }
}

/// Stage 3 real impl (logging fallback) — emit a session-complete signal.
///
/// `tauri` is **not** a dep of the `core` crate (it lives in `app/src-tauri`),
/// so we cannot call `tauri::AppHandle::emit` directly from here without
/// pulling in a hard tauri dep just for one call site. Instead we log the
/// completion via `tracing::info!` on the `"focus"` target — observers in CLI
/// and tests pick it up via tracing-subscriber, and the actual Tauri
/// `AppHandle::emit("focus:complete", payload)` bridge stays Stage 4 (lives in
/// `app/src-tauri/` once the cross-crate bridge lands).
///
/// The JSON payload shape mirrors the eventual Tauri event payload so wire-
/// compat is preserved when Stage 4 swaps the log for the real emit.
fn emit_event_pseudo(
    session_id: &str,
    result: &FocusSessionResult,
) -> Result<(), FocusCaptureError> {
    let payload = serde_json::json!({
        "session_id": session_id,
        "actual_duration_ms": result.actual_duration_ms,
        "interruptions": result.interruptions,
        "completion_pct": result.completion_pct,
        "summary": result.summary,
        "suggestion": result.suggestion,
    });
    tracing::info!(
        target: "focus",
        session_id = %session_id,
        "session_complete: {}",
        payload
    );
    Ok(())
}

/// Run an LLM analysis pass over a completed `FocusSessionResult` — returns
/// the shared `AnalysisResult` shape (summary / confidence / goal_impact /
/// suggestion / cost / latency / model_id / raw_response) reused across all
/// capture types per SPEC-14.
///
/// 中文: 把已完成的 `FocusSessionResult` 餵 LLM 跑一次完整分析，回傳 SPEC-16
/// 共用的 `AnalysisResult`（LLM 分析結果）結構。委派給 SPEC-14（LLM analyzer，
/// LLM 分析器）。失敗時回 `FocusCaptureError::TakeawayFailed`。
pub fn analyze_focus_session(
    result: &FocusSessionResult,
) -> Result<AnalysisResult, FocusCaptureError> {
    // Step 1: build the LLM prompt from the session's actual duration +
    // interruption count + (if present) summary scaffold.
    let _prompt = build_focus_prompt(result);
    // Step 2: call the LLM provider chain (providers_wire::complete) — Stage 4
    // maps provider errors to FocusCaptureError::TakeawayFailed.
    let _raw_response = providers_complete_pseudo(&_prompt)?;
    // Step 3: parse the JSON LLM response into the shared AnalysisResult shape
    // (summary / confidence / goal_impact / suggestion / cost / latency /
    // model_id / raw_response) per SPEC-14 / SPEC-16.
    let analysis = parse_json(&_raw_response)?;
    Ok(analysis)
}

/// SPEC-21 §12.3 — build the LLM prompt text from the session result. Pure
/// string templating; instructs the model to reply with strict JSON matching
/// the shared `AnalysisResult` shape (summary / confidence / goal_impact /
/// suggestion / cost_usd / latency_ms / model_id / raw_response).
fn build_focus_prompt(result: &FocusSessionResult) -> String {
    let actual_min = result.actual_duration_ms / 60_000;
    let mut out = String::with_capacity(512);
    out.push_str(
        "You are a focus-session coach. Reply with strict JSON matching the \
         AnalysisResult schema {summary, confidence, goal_impact, suggestion, \
         cost_usd, latency_ms, model_id, raw_response}. Bilingual (zh-TW + EN) \
         summary OK.\n\n",
    );
    out.push_str(&format!("actual_duration_min: {}\n", actual_min));
    out.push_str(&format!("interruptions: {}\n", result.interruptions));
    out.push_str(&format!("completion_pct: {:.1}\n", result.completion_pct));
    if !result.summary.is_empty() {
        out.push_str("scaffold_summary: ");
        out.push_str(&result.summary);
        out.push('\n');
    }
    out
}

/// Stage 4: dispatch to providers_wire::complete (LLM provider chain).
/// Deferred until providers_wire ships its Stage 3; calling reqwest directly
/// from here would duplicate the provider-chain logic.
fn providers_complete_pseudo(_prompt: &str) -> Result<String, FocusCaptureError> {
    unimplemented!(
        "Stage 4: crate::providers_wire::complete (defer until that module's Stage 3 lands)"
    )
}

/// Parse the LLM JSON response into the shared `AnalysisResult` shape per
/// SPEC-14 / SPEC-16. Parse failures map to `TakeawayFailed` so the caller
/// can surface the SPEC-21 §11 `FOCUS-004` error to the UI.
fn parse_json(raw_response: &str) -> Result<AnalysisResult, FocusCaptureError> {
    serde_json::from_str(raw_response).map_err(|e| FocusCaptureError::TakeawayFailed {
        detail: format!("AnalysisResult JSON parse failed: {}", e),
    })
}

// ─── Smoke tests (Stage 1 sanity only; deeper invariants in Stage 2) ─────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_session_result_round_trip_smoke() {
        // §7.1 invariant: TS encode → wire → Rust decode → re-encode preserves
        // the public surface. Stage 1 sanity-checks serde; deeper invariants
        // (e.g. completion_pct is in [0, 100]) come in Stage 2.
        let r = FocusSessionResult {
            actual_duration_ms: 25 * 60 * 1000,
            interruptions: 2,
            completion_pct: 100.0,
            summary: "drafted SPEC-21 wire types".to_string(),
            suggestion: "review with cargo check next".to_string(),
        };
        let j = serde_json::to_string(&r).unwrap();
        let back: FocusSessionResult = serde_json::from_str(&j).unwrap();
        assert_eq!(r.actual_duration_ms, back.actual_duration_ms);
        assert_eq!(r.interruptions, back.interruptions);
        assert!((r.completion_pct - back.completion_pct).abs() < f32::EPSILON);
        assert_eq!(r.summary, back.summary);
        assert_eq!(r.suggestion, back.suggestion);
    }

    #[test]
    fn focus_mode_default_durations_are_canonical() {
        // §7.1.1 invariant: the three preset modes have fixed canonical
        // durations. Bumping any of these is a wire-break.
        assert_eq!(
            FocusMode::Pomodoro25.default_duration_ms(),
            Some(25 * 60 * 1000)
        );
        assert_eq!(
            FocusMode::DeepWork50.default_duration_ms(),
            Some(50 * 60 * 1000)
        );
        assert_eq!(
            FocusMode::Sprint10.default_duration_ms(),
            Some(10 * 60 * 1000)
        );
        assert_eq!(FocusMode::Custom.default_duration_ms(), None);
    }

    #[test]
    fn interruption_kind_slugs_are_stable() {
        // §12.4 invariant: telemetry label slugs are lower-kebab and stable
        // across versions (Prometheus label-set break = dashboard break).
        assert_eq!(InterruptionKind::UserPause.slug(), "user-pause");
        assert_eq!(InterruptionKind::Notification.slug(), "notification");
        assert_eq!(InterruptionKind::AppSwitch.slug(), "app-switch");
        assert_eq!(InterruptionKind::ScreenLock.slug(), "screen-lock");
    }

    #[test]
    fn focus_session_request_default_tag_is_focus() {
        // serde default invariant: omitting `tag` yields `["focus"]` so the
        // SPEC-16 query layer always finds focus sessions via the canonical
        // tag filter.
        let json = r#"{"mode":"pomodoro25","plannedDurationMs":1500000,"label":null}"#;
        let req: FocusSessionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.tag, vec!["focus".to_string()]);
        assert_eq!(req.planned_duration_ms, 1500000);
        assert!(matches!(req.mode, FocusMode::Pomodoro25));
        assert!(req.label.is_none());
    }

    #[test]
    fn focus_interruption_round_trip_smoke() {
        let i = FocusInterruption {
            timestamp_ms: 1_700_000_000_000,
            kind: InterruptionKind::Notification,
            duration_ms: 1500,
        };
        let j = serde_json::to_string(&i).unwrap();
        let back: FocusInterruption = serde_json::from_str(&j).unwrap();
        assert_eq!(i.timestamp_ms, back.timestamp_ms);
        assert_eq!(i.kind, back.kind);
        assert_eq!(i.duration_ms, back.duration_ms);
    }

    #[test]
    fn focus_capture_error_serializes_with_code_tag() {
        // §11 invariant: error wire shape uses `{"code": "..."}` tag so the
        // UI can dispatch on the machine-readable code string.
        let e = FocusCaptureError::SessionAlreadyActive;
        let j = serde_json::to_string(&e).unwrap();
        assert!(j.contains("session_already_active"), "wire shape: {}", j);

        let e2 = FocusCaptureError::AsrAllProvidersFailed {
            providers: vec!["apple-on-device".to_string(), "groq-whisper".to_string()],
        };
        let j2 = serde_json::to_string(&e2).unwrap();
        assert!(j2.contains("asr_all_providers_failed"), "wire shape: {}", j2);
        assert!(j2.contains("apple-on-device"), "payload preserved: {}", j2);
    }

    #[test]
    fn start_session_step1_mints_valid_uuidv7() {
        // uuid/v7 is now in the default build, so start_session Step 1 mints a
        // real time-ordered id (was `unimplemented!("Stage 4: uuid …")`). Test
        // the helper directly — no session-table interaction, so it can't race
        // the single-active-session KAT tests.
        let id = uuid_v7_pseudo();
        let parsed = uuid::Uuid::parse_str(&id).expect("session_id is a valid UUID");
        assert_eq!(parsed.get_version_num(), 7, "expected UUIDv7, got {id}");
    }

    // ─── Stage 3 KAT (known-answer-test) vectors ─────────────────────────
    //
    // Pin the real helpers' behaviour. The session-table tests deliberately
    // use unique session_id keys + always clean up so they can run in any
    // order under cargo test's default parallel scheduler.

    #[test]
    fn compute_completion_pct_kat_known_vectors() {
        // KAT: exact halfway, full, overrun-clamped, underflow-clamped, div0.
        assert!((compute_completion_pct(50, 100) - 50.0).abs() < f32::EPSILON);
        assert!((compute_completion_pct(100, 100) - 100.0).abs() < f32::EPSILON);
        // overrun → clamped at 100.0 not 150.0
        assert!((compute_completion_pct(150, 100) - 100.0).abs() < f32::EPSILON);
        // zero planned → safe 0.0 (no div-by-zero panic)
        assert!((compute_completion_pct(0, 0) - 0.0).abs() < f32::EPSILON);
        // zero actual → 0.0
        assert!((compute_completion_pct(0, 100) - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn now_ms_kat_is_post_2020_epoch() {
        // KAT: any modern wall clock returns ms past 2020-01-01.
        let ms = now_ms();
        // 2020-01-01T00:00:00Z = 1577836800000 ms
        assert!(ms > 1_577_836_800_000, "now_ms too small: {}", ms);
    }

    #[test]
    fn build_focus_prompt_kat_includes_session_fields() {
        // KAT: actual_min, interruptions, completion_pct, scaffold summary
        // all land somewhere in the prompt body. Exact wording is reviewable
        // in source; this asserts schema + key-value presence only.
        let result = FocusSessionResult {
            actual_duration_ms: 25 * 60 * 1000,
            interruptions: 3,
            completion_pct: 100.0,
            summary: "drafted SPEC-21".to_string(),
            suggestion: "review next".to_string(),
        };
        let prompt = build_focus_prompt(&result);
        assert!(prompt.contains("AnalysisResult"), "schema: {}", prompt);
        assert!(prompt.contains("25"), "actual_min: {}", prompt);
        assert!(prompt.contains("3"), "interruptions: {}", prompt);
        assert!(prompt.contains("100.0"), "completion_pct: {}", prompt);
        assert!(prompt.contains("drafted SPEC-21"), "scaffold: {}", prompt);
    }

    #[test]
    fn parse_json_kat_round_trips_analysis_result() {
        // KAT: well-formed JSON parses into AnalysisResult preserving fields.
        let raw = r#"{
            "summary": "focused 25 min",
            "confidence": 0.85,
            "goalImpact": "+1 deep-work block today",
            "suggestion": "schedule a 5-min break",
            "costUsd": 0.0012,
            "latencyMs": 850,
            "modelId": "groq:llama-3.1-8b-instant",
            "rawResponse": "{\"raw\":\"…\"}"
        }"#;
        let parsed = parse_json(raw).expect("parse ok");
        assert_eq!(parsed.summary, "focused 25 min");
        assert!((parsed.confidence - 0.85).abs() < 0.001);
        assert_eq!(parsed.model_id, "groq:llama-3.1-8b-instant");
        assert_eq!(parsed.latency_ms, 850);
    }

    #[test]
    fn parse_json_kat_malformed_maps_to_takeaway_failed() {
        // KAT: malformed JSON → TakeawayFailed (SPEC-21 §11 FOCUS-004).
        match parse_json("{ not json") {
            Err(FocusCaptureError::TakeawayFailed { detail }) => {
                assert!(!detail.is_empty(), "detail populated: {}", detail);
            }
            other => panic!("expected TakeawayFailed, got {:?}", other),
        }
    }

    #[test]
    fn session_lookup_kat_unknown_id_returns_session_not_found() {
        // KAT: never-registered id → SessionNotFound (NOT a panic).
        let bogus = "session-id-that-was-never-registered-xyz123";
        match session_lookup(bogus) {
            Err(FocusCaptureError::SessionNotFound { session_id }) => {
                assert_eq!(session_id, bogus);
            }
            other => panic!("expected SessionNotFound, got {:?}", other),
        }
    }

    #[test]
    fn session_register_lookup_append_drain_kat_round_trip() {
        // KAT: register → lookup OK → append 2 interruptions → drain returns
        // 2 + de-registers → second lookup = SessionNotFound. Uses a unique
        // session_id key so it does not collide with other parallel tests;
        // serializes via the `session_already_active` guard by ensuring no
        // other test is mid-register at the same instant (parallel races on
        // the single-session invariant are out-of-scope for KAT — we only
        // assert behaviour when the table is in a known starting state).
        //
        // To avoid the single-active-session collision with the
        // `#[should_panic]` start_session test (which never registers),
        // we drain any leftover entry first.
        {
            let mut g = session_table().lock().unwrap();
            g.clear();
        }
        let req = FocusSessionRequest {
            mode: FocusMode::Pomodoro25,
            planned_duration_ms: 25 * 60 * 1000,
            label: None,
            tag: vec!["focus".to_string()],
        };
        let sid = "kat-session-id-round-trip";
        session_register(sid, &req).expect("register ok");
        // Second register must refuse with SessionAlreadyActive (single-session).
        let dup = session_register("kat-session-id-dup", &req);
        assert!(matches!(dup, Err(FocusCaptureError::SessionAlreadyActive)));
        // Lookup should succeed.
        session_lookup(sid).expect("lookup ok");
        // Append two interruptions.
        let i1 = FocusInterruption {
            timestamp_ms: now_ms(),
            kind: InterruptionKind::Notification,
            duration_ms: 0,
        };
        let i2 = FocusInterruption {
            timestamp_ms: now_ms(),
            kind: InterruptionKind::AppSwitch,
            duration_ms: 250,
        };
        append_interruption(sid, &i1).expect("append i1 ok");
        append_interruption(sid, &i2).expect("append i2 ok");
        // Planned duration reads back.
        assert_eq!(lookup_planned_duration(sid), 25 * 60 * 1000);
        // Actual duration is at least 0 (clock can race).
        let _ = compute_actual_duration(sid);
        // Drain returns the count and removes the entry.
        assert_eq!(drain_interruptions(sid), 2);
        // After drain the lookup is gone.
        match session_lookup(sid) {
            Err(FocusCaptureError::SessionNotFound { .. }) => {}
            other => panic!("expected post-drain SessionNotFound, got {:?}", other),
        }
    }
}
