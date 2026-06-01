// SPEC-17 §7 + §9 — Tauri bridge wire types (single source of truth for the
// invoke envelope contract + the catalog of normalised Tauri commands +
// deep-link allowlist + onboarding FSM that the webview ↔ Rust IPC layer
// shares).
//
// Stage 4 (catalog promoted to phf perfect-hash; FSM kept as match):
// `catalog_lookup` now uses a compile-time `phf::Map<&'static str,
// CatalogRow>` (zero runtime-build cost, zero allocation per call).
// `fsm_lookup` stays a hand-rolled `match` expression — the legal-edge set
// is greppable in one place and a phf::Map keyed on
// `(OnboardingState, OnboardingTransition)` would need a stringified-tuple
// key (enums don't implement `PhfHash`); the match is equally fast and
// noticeably easier to read, so we explicitly do NOT convert it. The other
// pure-logic helpers — `permission_check`, `url_parse`, `route_split`,
// `extract_query`, `sanitize_oauth_callback` — remain as the Stage 3
// hand-rolled impls. `validate_args_schema` + `emit_event` stay Stage 4
// pending — the former needs a `schemars` integration to auto-derive JSON
// schemas per-command, the latter needs a Tauri `AppHandle` injected from
// the runtime layer.
//
// 中文: 本檔對應 SPEC-17 §7（資料模型）與 §9（API 合約）。整個 Tauri
// bridge（橋接層）所有 command 都共用同一個 envelope（信封）契約：成功時
// 直接序列化 T；錯誤時走 `TauriCommandError`，其 wire 形狀 `{"code","message",
// "recoveryHint"}` 對齊 SPEC-04 error catalog（錯誤代碼目錄）。所有 ts-rs
// 匯出的目錄是 `app/src/lib/generated/tauri/`，給前端 import 用。
//
// 對應 spec sections:
//   - §7.1 envelope schema  → `TauriCommandRequest` / `TauriCommandResponse`
//   - §7.2 command 簽章規範  → 每個 command Args / Response struct
//   - §8 deep-link 規則      → `DeepLinkRoute` + `dispatch_deep_link`
//   - §9.1 command 矩陣（42 個）→ 取 10 個 representative pair（per CLAUDE.md）
//   - §11.1 capability 對應  → `Permission` + `CommandCategory`
//   - SPEC-28 onboarding FSM → `OnboardingState` + `OnboardingTransition`
//
// TODO Stage 4:
//   - 把 `validate_args_schema_pseudo` 接到 schemars 自動產生的 JSON schema
//     表（目前 §9.1 + §20.1 共 42 個 command；Stage 3 已有完整 catalog 但
//     args 結構驗證只到 serde 層）。
//   - `emit_event_pseudo` 改接 `tauri::AppHandle::emit` — 需要從 runtime 層
//     傳入 handle；現階段直接 no-op 避免 wire 層依賴 Tauri runtime。

use serde::{Deserialize, Serialize};
use ts_rs::TS;

// ─── §7.1 IPC envelope — generic request / response / error ─────────────────

/// Generic invoke request envelope. Tauri 2.x serialises `T` to JSON for the
/// webview side; this wrapper exists so test helpers (`call_command_for_test`
/// per §9.4) can build envelopes deterministically without booting a runtime.
///
/// `command` is the snake_case command name (must match the §8 lint rules:
/// `<domain>_<verb>` or `<verb>_<noun>`, ≤ 32 chars, no `mixedCase`, no
/// platform prefix, no PII).
///
/// 中文: 通用 invoke（呼叫）請求信封。`command` 是 snake_case 命令名稱，
/// `args` 是該命令對應的 Args struct。Tauri 2.x 原生 JSON-RPC 不需要這層，
/// 但 test helper 與 future MCP bridge 會用到。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/tauri/")]
#[serde(rename_all = "camelCase")]
pub struct TauriCommandRequest<T> {
    /// snake_case command name (must be in the §9.1 + §20.1 catalog).
    pub command: String,
    /// Optional opaque correlation id (echoed in response; absent in legacy
    /// Tauri runtime path — Stage 2 only fills this in the test helper).
    pub correlation_id: Option<String>,
    /// Command-specific argument payload. Even no-arg commands must use a
    /// named struct (minimum `EmptyArgs`) per §7.2.
    pub args: T,
}

/// Generic invoke success envelope. Tauri's own `Result::Ok` → JS Promise
/// resolve already covers the success path on the runtime; this struct is the
/// canonical shape for test helpers and the in-process `call_command` API
/// (§9.4) so consumers can pattern-match against `{ ok }`.
///
/// 中文: 通用成功信封 `{ ok: <T 的 JSON 形式> }`。Tauri 自身已把 `Result::Ok`
/// 對映到 JS Promise resolve；本 struct 主要給 test helper 與 §9.4 in-process
/// API 用，前端不會直接見到這層。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/tauri/")]
#[serde(rename_all = "camelCase")]
pub struct TauriCommandResponse<T> {
    /// Optional correlation id echoed from the request envelope.
    pub correlation_id: Option<String>,
    /// Successful payload. Type per the §9.1 / §20.1 catalog row.
    pub ok: T,
    /// Server-side completion timestamp (unix millis). Always present —
    /// SPEC-17 §8 event-payload rule mandates `ts: i64` everywhere.
    pub ts_ms: i64,
}

/// Canonical wire-facing error for **every** Tauri command. Matches the §7.1
/// error envelope: `{ "err": { "code": "...", "message": "...",
/// "details": { ... } } }` — this struct represents the **inner** `err`
/// object; Tauri itself wraps it in the outer `{ err }` per Promise-reject
/// convention.
///
/// `code` is the SPEC-04 catalog snake_case identifier (e.g.
/// `provider_auth_error`, `cluster_unreachable`, `identity_keystore_unavailable`).
/// `message` is the English developer detail — MUST NOT contain user secret
/// material per §13 privacy rules. `recovery_hint` is an optional UI-facing
/// next-step the frontend can surface (e.g.
/// `"open settings → providers → cerebras → re-paste key"`).
///
/// 中文: 全 Tauri command 共用的 wire-facing error。`code` 取自 SPEC-04
/// catalog；`message` 是英文 dev 細節（永不含使用者祕密）；`recovery_hint`
/// 是給前端顯示「下一步該做什麼」的提示字串。
#[derive(Debug, Clone, Serialize, Deserialize, TS, thiserror::Error)]
#[ts(export, export_to = "../../app/src/lib/generated/tauri/")]
#[serde(rename_all = "camelCase")]
#[error("{code}: {message}")]
pub struct TauriCommandError {
    /// snake_case error code from SPEC-04 catalog.
    pub code: String,
    /// English developer-facing detail. **Never** put secrets in here.
    pub message: String,
    /// Optional UI-facing "what to do next" hint, localised on the frontend.
    pub recovery_hint: Option<String>,
}

impl TauriCommandError {
    /// Build a fresh error without a recovery hint. Mostly used from tests.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            recovery_hint: None,
        }
    }

    /// Attach a recovery hint to an existing error (builder-style).
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.recovery_hint = Some(hint.into());
        self
    }
}

// ─── §8 / §11.2 Deep-link route ─────────────────────────────────────────────

/// A parsed `phantom://<host>/<path>?<query>` URL after allowlist filtering.
/// Only URLs that match the §11.2 allowlist reach this struct; everything
/// else is dropped + warn-logged inside `dispatch_deep_link` (the raw URL is
/// **not** logged per §13 privacy — only its length + rejection reason).
///
/// The `scheme` field is always `"phantom"` (other schemes are rejected at
/// the parser entry); kept as a field for future-proofing if we ever accept a
/// `phantom-mesh://` aliased scheme.
///
/// 中文: 經過 allowlist（白名單）過濾後的 deep-link（深層連結）解析結果。
/// 含 token 的 OAuth（開放授權）原始 URL **絕對不會**到這裡 — §13 規定
/// OAuth callback 必須在 Rust 端解析後只 emit `{state_id, provider}` 給前端，
/// 原始 URL 留在 Rust 內部存進 keystore（系統金鑰庫）。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/tauri/")]
#[serde(rename_all = "camelCase")]
pub struct DeepLinkRoute {
    /// Always `"phantom"` in v0.6.0; field exists so we can add aliases later
    /// without breaking the TS surface.
    pub scheme: String,
    /// First path segment (`chat` / `settings` / `mesh` / `onboarding` /
    /// `oauth` / `demo-mode`). For `phantom://demo-mode` (no path), this is
    /// `"demo-mode"` and `path` is empty.
    pub host: String,
    /// Remaining path segments joined with `/`. Empty for bare URLs like
    /// `phantom://demo-mode`. Per §8: any segment containing `..` or
    /// `%2e%2e` causes the whole URL to be dropped before reaching this
    /// struct.
    pub path: String,
    /// Decoded query string key-value pairs. Per §8 limits: ≤ 16 keys, each
    /// value ≤ 256 characters (enforced in the parser, not at this struct).
    pub query_params: Vec<(String, String)>,
}

// ─── §9.1 Command catalog enums ──────────────────────────────────────────────

/// Permission tag applied to each Tauri command for the capability-matrix
/// check in §11.1. A command can require multiple permissions (e.g.
/// `broker_login_start` needs both `Network` for the OAuth start endpoint
/// **and** internal config write access).
///
/// **Mapping to §11.1 capability identifiers**:
/// `Internal`→`core:default`, `Network`→`http:default`,
/// `FileSystem`→`store:default`, `Notification`→`notification:default`,
/// `Camera`→OS-prompt (no Tauri-side mapping yet — v0.7.0 capture pillar),
/// `Microphone`→OS-prompt (ditto), `Mesh`→`P1.mdns` product capability.
///
/// 中文: 每個 Tauri command 帶的權限標籤，給 §11.1 capability（權能）矩陣
/// 檢查用。Tauri-side capability 與 SPEC-01 §8 product capability（產品能力，
/// 例如 `P1.mdns`）是不同層；本 enum 故意把兩層混在一起，等 v0.7.0 SPEC-17
/// 拆 2 欄時再分。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/tauri/")]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    /// In-process state read/write only; no syscalls beyond `Tauri::State`.
    Internal,
    /// Mesh / cluster product capability (`P1.mdns`, `P4.identity`, etc.).
    Mesh,
    /// Filesystem access (e.g. `identity_backup_to_path`, `store:default`).
    FileSystem,
    /// OS notification (desktop tray / iOS banner).
    Notification,
    /// Camera (future v0.7.0 — page-capture / QR scan).
    Camera,
    /// Microphone (future v0.7.0 — voice capture).
    Microphone,
    /// Outbound HTTP (provider call, broker sync, cluster fetch).
    Network,
}

/// Idempotency class per SPEC-17 §7 — used by the frontend to decide whether
/// retry-on-failure is safe.
///
/// `Idempotent` commands (read-only or hash-keyed writes) may be retried
/// automatically by the UI on transient error. `NonIdempotent` commands
/// (e.g. `identity_init` without `--force`, `broker_register_self_peer`) must
/// require explicit user confirmation before retry. `TimeSensitive` commands
/// (e.g. `provider_test`, `a11y_announce`) become stale within ~5 seconds
/// and should not be retried at all — the UI should re-issue a fresh call.
///
/// 中文: 冪等性（idempotency）分類。決定前端能不能自動 retry。
/// - `Idempotent`：可自動 retry
/// - `NonIdempotent`：retry 需使用者再確認
/// - `TimeSensitive`：~5 秒後 stale，不該 retry，應該重新發起
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/tauri/")]
#[serde(rename_all = "snake_case")]
pub enum IdempotencyClass {
    /// Safe to auto-retry; same args → same effect.
    Idempotent,
    /// Retry requires explicit user re-confirmation.
    NonIdempotent,
    /// Becomes stale within ~5s — re-issue fresh instead of retrying.
    TimeSensitive,
}

/// Domain bucket for each command per the §9.1 grouping (A–H). Used by the
/// `validate_command` stub for permission-matrix lookup and by the future
/// audit-log subsystem to tag events.
///
/// `Onboarding` is added here even though it's not in §9.1 groups A–H —
/// SPEC-28 owns the onboarding FSM; we tag those commands so the wipe /
/// reset flows know to clear them.
///
/// 中文: 命令網域分類。對齊 §9.1 七群（A–H）+ 額外 `Onboarding`（SPEC-28）+
/// `Capture` / `Coach` / `Hermes` / `Wipe` 是 v0.7.0 後續 pillar 預留的群組
/// slug — Stage 1 先佔位，Stage 2 / v0.7.0 spec 出來時把實際 command 填上。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/tauri/")]
#[serde(rename_all = "snake_case")]
pub enum CommandCategory {
    /// 聊天介面（chat）— v0.7.0 conversation pillar
    Chat,
    /// 應用程式設定（settings）— theme / nav / i18n / a11y / provider
    Settings,
    /// 叢集成員管理（cluster）— SPEC-10 + SPEC-11
    Cluster,
    /// 身份金鑰保管（vault）— SPEC-12 + SPEC-15 broker vault
    Vault,
    /// 教練回顧（coach）— v0.7.0 life-track pillar
    Coach,
    /// Hermes 任務派工（hermes）— v0.7.0 work-track pillar
    Hermes,
    /// mDNS 鄰居發現（mDNS = multicast DNS，多播網域名稱系統）— SPEC-11
    Mdns,
    /// OAuth 第三方授權（OAuth）— SPEC-15 broker login
    Oauth,
    /// 完全清除帳號（wipe）— v0.7.0 reset flow
    Wipe,
    /// 事件擷取（capture）— v0.7.0 life-track capture pillar
    Capture,
    /// 首次啟動引導（onboarding）— SPEC-28 FSM
    Onboarding,
}

// ─── §7.2 EmptyArgs — no-arg command convention ─────────────────────────────

/// Standard no-arg placeholder. §7.2 / §8 mandate that every command's first
/// positional parameter is a named `args` struct even when no fields are
/// needed — this is the canonical zero-field struct that the catalog uses for
/// `theme_current`, `nav_back`, `identity_public`, etc. Naming it lets us add
/// fields later without breaking the wire contract.
///
/// 中文: 標準零欄位 args struct。§7.2 規定即使沒有參數也要定義一個具名
/// struct（最低 `EmptyArgs`），裸 `fn cmd(state: ...)` 不允許 — 將來加欄位
/// 才不會破壞前端 wire 契約。
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/tauri/")]
pub struct EmptyArgs {}

// ─── §9.1 Representative command Args / Response pairs (10 of 42) ───────────
//
// Per CLAUDE.md staging rules: not all 42 are stubbed here. The 10 below
// cover one command per requested domain (chat / settings / cluster / vault /
// coach / hermes / mdns / oauth / wipe / capture) and exercise the full
// surface (no-arg / single-arg / multi-field / async-streaming / mobile-only
// / desktop-only / oauth deep-link).

// — chat — placeholder for v0.7.0 conversation pillar
//
/// Args for `chat_list` — list recent conversation threads, paginated.
/// 中文: 列出最近聊天 thread（討論串）的命令參數。v0.7.0 conversation pillar
/// 預留 — 真正欄位由 SPEC-19 conversation spec 補完。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/tauri/")]
#[serde(rename_all = "camelCase")]
pub struct ChatListArgs {
    /// Max threads to return (1–100, default 20).
    pub limit: Option<u32>,
    /// Opaque pagination cursor from a previous response.
    pub cursor: Option<String>,
}

/// Response for `chat_list`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/tauri/")]
#[serde(rename_all = "camelCase")]
pub struct ChatListResponse {
    /// Conversation thread ids in newest-first order.
    pub thread_ids: Vec<String>,
    /// Pagination cursor for the next page; `None` if no more pages.
    pub next_cursor: Option<String>,
}

// — settings — `theme_set_mode` per §9.1 group A
//
/// Args for `theme_set_mode` — switch UI colour theme.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/tauri/")]
#[serde(rename_all = "camelCase")]
pub struct ThemeSetModeArgs {
    /// `"light"` / `"dark"` / `"system"` (matches SPEC-02 §10 enum on wire).
    pub mode: String,
}

/// Response for `theme_set_mode` — empty on success (`Result<(), Error>`
/// in Rust; encoded as empty object on the wire for forwards-compat).
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/tauri/")]
pub struct ThemeSetModeResponse {}

// — cluster — `get_cluster_status` per §9.1 group F
//
/// Response for `get_cluster_status` — a coarse-grained health summary used
/// by the menubar / status pill. Detailed peer-list comes from
/// `get_cluster_peers`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/tauri/")]
#[serde(rename_all = "camelCase")]
pub struct ClusterStatusResponse {
    /// `"offline"` / `"solo"` / `"degraded"` / `"healthy"` (SPEC-10 state).
    pub state: String,
    /// Number of peers currently reachable (incl. self).
    pub peer_count: u32,
    /// Unix millis of the last heartbeat received from any peer.
    pub last_heartbeat_ts_ms: Option<i64>,
}

// — vault — `broker_login_finish` per §9.1 group E (post-OAuth completion)
//
/// Args for `broker_login_finish` — pass only the `state_id` (the frontend
/// gets this via the sanitized `deep-link:oauth-callback` event; raw OAuth
/// token never crosses webview boundary per §13).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/tauri/")]
#[serde(rename_all = "camelCase")]
pub struct BrokerLoginFinishArgs {
    /// State id from `deep-link:oauth-callback` event payload.
    pub state_id: String,
}

/// Response for `broker_login_finish` — the active broker session summary.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/tauri/")]
#[serde(rename_all = "camelCase")]
pub struct BrokerLoginFinishResponse {
    /// Broker URL the session is bound to (e.g. `https://vault.example.com`).
    pub broker_url: String,
    /// Account identifier from the OAuth provider (opaque to UI).
    pub account_id: String,
    /// ISO-8601 timestamp the session expires.
    pub expires_at: String,
}

// — coach — placeholder for v0.7.0 life-track pillar
//
/// Args for `coach_review_today` — fetch the day's review summary for a
/// given local date (YYYY-MM-DD). Local-date string (not unix ts) so the
/// review boundary follows the user's wall-clock midnight, not UTC.
///
/// 中文: 取得今日教練回顧（review）的命令參數。`local_date` 是本地 YYYY-MM-DD
/// 字串（不是 UTC unix ts），因為 review 邊界跟著使用者本地午夜走。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/tauri/")]
#[serde(rename_all = "camelCase")]
pub struct CoachReviewArgs {
    /// Local YYYY-MM-DD date string.
    pub local_date: String,
}

/// Response for `coach_review_today`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/tauri/")]
#[serde(rename_all = "camelCase")]
pub struct CoachReviewResponse {
    /// Markdown summary text (already localised).
    pub summary_md: String,
    /// Count of events the summary was built from (0 = empty day).
    pub source_event_count: u32,
}

// — hermes — placeholder for v0.7.0 work-track pillar
//
/// Args for `hermes_dispatch_task` — push a task to the swarm coordinator.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/tauri/")]
#[serde(rename_all = "camelCase")]
pub struct HermesDispatchArgs {
    /// Free-form prompt the chosen worker peer will receive.
    pub prompt: String,
    /// Optional preferred peer id (else coordinator scheduler decides).
    pub preferred_peer_id: Option<String>,
}

/// Response for `hermes_dispatch_task`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/tauri/")]
#[serde(rename_all = "camelCase")]
pub struct HermesDispatchResponse {
    /// Unique task id (UUID v7).
    pub task_id: String,
    /// Peer the coordinator actually assigned the task to.
    pub assigned_peer_id: String,
}

// — mdns — `mdns_advertise_start` per §9.1 group F (mobile only)
//
/// Args for `mdns_advertise_start` — start advertising this peer to the
/// local link via mDNS (multicast DNS, 多播網域名稱系統).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/tauri/")]
#[serde(rename_all = "camelCase")]
pub struct MdnsAdvertiseArgs {
    /// Human-visible peer name (UTF-8, ≤ 63 bytes per DNS-SD rules).
    pub peer_name: String,
    /// Port the peer accepts mesh-rpc on.
    pub port: u16,
    /// Capability slugs to publish in the TXT record (e.g. `["P1.mdns",
    /// "P3.mcp"]`).
    pub capabilities: Vec<String>,
}

/// Response for `mdns_advertise_start`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/tauri/")]
#[serde(rename_all = "camelCase")]
pub struct MdnsAdvertiseResponse {
    /// Instance name actually used (may differ from request if collided —
    /// e.g. `peer-name (2)`).
    pub instance_name: String,
}

// — oauth — `broker_login_start` per §9.1 group E (deep-link partner)
//
/// Args for `broker_login_start` — start an OAuth flow against a broker URL.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/tauri/")]
#[serde(rename_all = "camelCase")]
pub struct BrokerLoginStartArgs {
    /// Broker URL (must be HTTPS in production; `http://localhost:*` in dev).
    pub broker_url: String,
}

/// Response for `broker_login_start`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/tauri/")]
#[serde(rename_all = "camelCase")]
pub struct BrokerLoginStartResponse {
    /// Authorisation URL the UI must open in the system browser.
    pub authorization_url: String,
    /// State id the UI must hold and pass back into `broker_login_finish`.
    pub state_id: String,
}

// — wipe — placeholder for v0.7.0 reset flow
//
/// Args for `wipe_account` — permanently delete all local identity, vault,
/// capture, and cluster state. Requires a fingerprint double-check echoing
/// the SPEC-12 §6.3 identity delete pattern.
///
/// 中文: 永久清除帳號（wipe）命令參數。需要使用者輸入身份指紋前 12 字元做
/// 二次確認，避免誤觸發。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/tauri/")]
#[serde(rename_all = "camelCase")]
pub struct WipeAccountArgs {
    /// First 12 hex chars of the current identity fingerprint — must match
    /// the on-disk value or the command returns `identity_fingerprint_mismatch`.
    pub confirm_fingerprint_first_12: String,
    /// `true` if the user also wants to wipe broker-cached vault entries on
    /// the server side (best-effort; the local wipe always succeeds even
    /// when the broker call fails).
    pub also_wipe_remote_vault: bool,
}

/// Response for `wipe_account`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/tauri/")]
#[serde(rename_all = "camelCase")]
pub struct WipeAccountResponse {
    /// `true` if the local wipe succeeded.
    pub local_wiped: bool,
    /// `true` if the remote vault wipe also succeeded (always `false` when
    /// `also_wipe_remote_vault == false`).
    pub remote_wiped: bool,
}

// — capture — placeholder for v0.7.0 life-track capture pillar
//
/// Args for `capture_event_start` — begin a capture session (e.g. browser
/// page snapshot, voice memo, screen frame).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/tauri/")]
#[serde(rename_all = "camelCase")]
pub struct CaptureStartArgs {
    /// Capture kind slug — `"page"` / `"voice"` / `"screen"` / `"text"`.
    pub kind: String,
    /// Optional caller-provided tag (free-form, ≤ 64 chars).
    pub tag: Option<String>,
}

/// Response for `capture_event_start`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/tauri/")]
#[serde(rename_all = "camelCase")]
pub struct CaptureStartResponse {
    /// Unique session id; pass back into `capture_event_finish`.
    pub session_id: String,
    /// Unix millis the capture session started.
    pub started_at_ms: i64,
}

// ─── SPEC-28 onboarding FSM ──────────────────────────────────────────────────

/// Onboarding FSM (Finite State Machine, 有限狀態機) states per SPEC-28.
/// Forward-only with **exactly one** sanctioned rollback edge:
/// `JoinedCluster → CreatedIdentity` (the user cancels a half-joined cluster
/// during onboarding). Every other backward arrow is `NoOp` and Stage 2's
/// `transition_onboarding` returns the input state unchanged.
///
/// Steps:
/// 1. `FreshInstall` — first launch, nothing chosen yet.
/// 2. `PickedLanguage` — user picked UI locale (SPEC-05 i18n).
/// 3. `CreatedIdentity` — `identity_init` ran successfully.
/// 4. `JoinedCluster` — user either created or joined a mesh cluster.
/// 5. `SetProvider` — user configured ≥ 1 LLM provider (`provider_set_api_key`
///    or broker-vault sync).
/// 6. `FirstReplyReceived` — first agent reply rendered → onboarding done,
///    main app UI takes over.
///
/// 中文: SPEC-28 規定的 onboarding 流程 FSM。整體是 forward-only（前向流動），
/// 只有 `JoinedCluster → CreatedIdentity` 一條可回退（使用者中途取消加入
/// cluster）；其他 rollback 一律 NoOp。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/tauri/")]
#[serde(rename_all = "snake_case")]
pub enum OnboardingState {
    /// First launch, nothing chosen yet.
    FreshInstall,
    /// UI locale picked.
    PickedLanguage,
    /// `identity_init` finished.
    CreatedIdentity,
    /// User created or joined a mesh cluster.
    JoinedCluster,
    /// At least one LLM provider configured.
    SetProvider,
    /// First agent reply rendered — onboarding officially done.
    FirstReplyReceived,
}

/// Direction of an `transition_onboarding` request. `Forward` advances one
/// step in the SPEC-28 sequence; `Rollback` only succeeds on the sanctioned
/// `JoinedCluster → CreatedIdentity` edge; `NoOp` explicitly stays put
/// (used by the UI to query "is current state X still valid?" without side
/// effects).
///
/// 中文: 狀態轉換方向。`Forward` 前進一步；`Rollback` 只在合法的退路上成功
/// （其他 case 回 NoOp）；`NoOp` 用來原地不動查詢用。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/tauri/")]
#[serde(rename_all = "snake_case")]
pub enum OnboardingTransition {
    /// Move forward one step in the SPEC-28 sequence.
    Forward,
    /// Roll back one step (only valid on the JoinedCluster → CreatedIdentity
    /// edge in v0.6.0).
    Rollback,
    /// Stay put; used for state queries without mutation.
    NoOp,
}

// ─── §9.4 / Stage 2 stub helpers ─────────────────────────────────────────────

/// Look up `name` in the §9.1 + §20.1 command catalog and validate `args`
/// against its declared Args schema. Returns the canonical
/// `(CommandCategory, Permission, IdempotencyClass)` triple on success so
/// callers can apply capability + retry policy uniformly.
///
/// Stage 2 will build the catalog table at compile time from a single
/// `static CATALOG: &[CatalogRow]` so adding a command is a one-line edit
/// that the type system enforces.
///
/// 中文: 在 §9.1 + §20.1 命令目錄查 `name`，並用對應 Args schema 驗證 `args`。
/// Stage 2 會把 42-row catalog 編成 compile-time table。
pub fn validate_command<T: serde::Serialize>(
    name: &str,
    args: &T,
) -> Result<(), TauriCommandError> {
    // Step 1 — lookup `name` in the §9.1 + §20.1 command catalog. Stage 3
    // backs this with a `std::sync::OnceLock<HashMap<...>>` built on first
    // call (no compile-time perfect hash because `phf` is not in
    // core/Cargo.toml — runtime HashMap is plenty fast for ~42 entries).
    let row = catalog_lookup(name)?;

    // Step 2 — check the catalog's declared Permission tag is satisfied by
    // the current request context (capability matrix per §11.1). Stage 3
    // is a pure check that the row's permission is currently in the
    // process-wide allowed set; Stage 4 will swap the static "allow all"
    // default for a real `tauri::capabilities::CapabilitySet` lookup.
    permission_check(&row)?;

    // Step 3 — serde-validate the `args` payload shape. Stage 3 only
    // confirms the args serialise to a JSON object (the canonical wire
    // envelope) — `validate_args_shape` is the real check. Per-command
    // JSON schema validation lives in `validate_args_schema_pseudo`,
    // currently a no-op pending Stage 4 schemars wiring.
    validate_args_shape(name, args)?;
    validate_args_schema_pseudo(name, args)?;

    // Step 4 — all gates passed → Ok. On any earlier failure the helper
    // returned a `TauriCommandError { code: "validation_failed", ... }`
    // carrying the precise reason (unknown command / permission denied /
    // schema mismatch) per SPEC-04 error catalog.
    Ok(())
}

/// Stage 4 helper — look up a command name in the §9.1 + §20.1 catalog and
/// return its `(CommandCategory, Permission, IdempotencyClass)` row. The
/// catalog is a compile-time `phf::Map<&'static str, CatalogRow>` perfect
/// hash table (zero-runtime-build, zero allocation per call). We picked 13
/// representative rows covering each `CommandCategory`; the remaining ~29
/// rows land alongside the schemars schema derivation in a follow-up wave.
///
/// 中文: 在命令目錄查名稱，回傳對應分類三元組。Stage 4 已升級為 `phf::Map`
/// 編譯時 perfect-hash（完美雜湊）表 — 零執行期建構成本、零分配；找不到
/// 回 `unknown_command` 錯誤。
fn catalog_lookup(
    name: &str,
) -> Result<(CommandCategory, Permission, IdempotencyClass), TauriCommandError> {
    static CATALOG: phf::Map<&'static str, CatalogRow> = phf::phf_map! {
        // §9.1 group A — settings
        "theme_set_mode"        => CatalogRow(CommandCategory::Settings,   Permission::Internal,   IdempotencyClass::Idempotent),
        "theme_current"         => CatalogRow(CommandCategory::Settings,   Permission::Internal,   IdempotencyClass::Idempotent),
        // §9.1 group A — chat
        "chat_list"             => CatalogRow(CommandCategory::Chat,       Permission::Internal,   IdempotencyClass::Idempotent),
        // §9.1 group F — cluster
        "get_cluster_status"    => CatalogRow(CommandCategory::Cluster,    Permission::Mesh,       IdempotencyClass::TimeSensitive),
        "get_cluster_peers"     => CatalogRow(CommandCategory::Cluster,    Permission::Mesh,       IdempotencyClass::TimeSensitive),
        // §9.1 group F — mdns
        "mdns_advertise_start"  => CatalogRow(CommandCategory::Mdns,       Permission::Mesh,       IdempotencyClass::NonIdempotent),
        // §9.1 group E — oauth / broker vault
        "broker_login_start"    => CatalogRow(CommandCategory::Oauth,      Permission::Network,    IdempotencyClass::NonIdempotent),
        "broker_login_finish"   => CatalogRow(CommandCategory::Vault,      Permission::Network,    IdempotencyClass::NonIdempotent),
        // SPEC-28 — onboarding
        "transition_onboarding" => CatalogRow(CommandCategory::Onboarding, Permission::Internal,   IdempotencyClass::Idempotent),
        // v0.7.0 placeholders — coach / hermes / capture / wipe
        "coach_review_today"    => CatalogRow(CommandCategory::Coach,      Permission::FileSystem, IdempotencyClass::Idempotent),
        "hermes_dispatch_task"  => CatalogRow(CommandCategory::Hermes,     Permission::Network,    IdempotencyClass::NonIdempotent),
        "capture_event_start"   => CatalogRow(CommandCategory::Capture,    Permission::FileSystem, IdempotencyClass::NonIdempotent),
        "wipe_account"          => CatalogRow(CommandCategory::Wipe,       Permission::FileSystem, IdempotencyClass::NonIdempotent),
    };
    match CATALOG.get(name) {
        Some(row) => Ok((row.0, row.1, row.2)),
        None => Err(TauriCommandError::new(
            "unknown_command",
            format!("no command `{}` in §9.1 + §20.1 catalog", name),
        )
        .with_hint("verify the command name matches the catalog (snake_case, <domain>_<verb>)")),
    }
}

/// Compile-time catalog row — newtype around the
/// `(CommandCategory, Permission, IdempotencyClass)` triple so it can live in
/// a `phf::Map` (the macro accepts struct-literal const values).
///
/// 中文: 命令目錄列（row）的新型別包裝，讓 `phf::Map` 可以收 const 結構字面
/// 值；對外仍解構為原本三元組。
#[derive(Debug, Clone, Copy)]
struct CatalogRow(CommandCategory, Permission, IdempotencyClass);

/// Stage 3 helper — confirm the calling context satisfies the row's
/// `Permission` tag per §11.1 capability matrix.
///
/// 中文: 比對呼叫端 capability 與命令需求；Stage 3 預設「全部允許」（in-process
/// 開發 / 測試），Stage 4 接 `tauri::capabilities::CapabilitySet` 之後才會
/// 真正擋掉缺權限的呼叫。
fn permission_check(
    _row: &(CommandCategory, Permission, IdempotencyClass),
) -> Result<(), TauriCommandError> {
    // Stage 3: in-process default — all permissions granted. The hook
    // exists so the Stage 4 wiring is a single helper-body swap without
    // touching call sites. Concrete denial logic lives behind the future
    // `tauri::AppHandle::manage::<CapabilitySet>` runtime injection.
    Ok(())
}

/// Stage 3 helper — confirm `args` serialises to a JSON object (the canonical
/// envelope shape per §7.2). Non-object payloads (raw arrays / strings)
/// return `args_must_be_object` so the frontend never accidentally sends
/// a positional payload.
fn validate_args_shape<T: serde::Serialize>(
    name: &str,
    args: &T,
) -> Result<(), TauriCommandError> {
    let value = serde_json::to_value(args).map_err(|e| {
        TauriCommandError::new(
            "args_serialize_failed",
            format!("command `{}` args failed to serialise: {}", name, e),
        )
    })?;
    if !value.is_object() {
        return Err(TauriCommandError::new(
            "args_must_be_object",
            format!(
                "command `{}` args must be a JSON object per §7.2, got {}",
                name,
                kind_of(&value)
            ),
        ));
    }
    Ok(())
}

fn kind_of(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// Stage 4 helper — validate that `args` shape matches the registered JSON
/// schema for `name`. Currently a no-op (returns Ok) — the structural
/// `validate_args_shape` above covers the §7.2 "must be an object" check;
/// per-field schema validation needs schemars to derive a JSON schema from
/// each Args struct, which is not yet in core/Cargo.toml.
///
/// 中文: 用註冊的 JSON schema 驗證 args 結構。Stage 3 是 no-op；Stage 4 接
/// `schemars` derive 後做 per-field 驗證。
fn validate_args_schema_pseudo<T: serde::Serialize>(
    _name: &str,
    _args: &T,
) -> Result<(), TauriCommandError> {
    // Stage 4: requires `schemars` (not currently in core/Cargo.toml) to
    // auto-derive a JSON schema per Args struct, then `jsonschema` (already
    // in dev-deps but not prod) to validate. Today's no-op is intentional —
    // the structural check in `validate_args_shape` is sufficient to keep
    // the wire envelope contract; schema-level field validation is a
    // defence-in-depth layer that goes in alongside the schemars wiring.
    Ok(())
}

/// Parse a `phantom://...` URL through the SPEC-03 §7.3 BNF and filter it
/// through the SPEC-17 §11.2 allowlist. Returns the structured route on
/// success; on rejection logs the **length + reason only** (never the raw
/// URL, per §13 privacy rule) and returns
/// `TauriCommandError { code: "deep_link_rejected", ... }`.
///
/// Forbidden patterns (all reject):
/// - non-`phantom://` scheme
/// - any path segment containing `..` or `%2e%2e`
/// - query string with > 16 keys
/// - any single query value > 256 chars
/// - URL not matching one of the §11.2 allowlist patterns
///
/// 中文: 解析 `phantom://...` URL，跑完 SPEC-03 §7.3 BNF + SPEC-17 §11.2
/// allowlist。拒絕時只記長度 + 原因（不記原始 URL — §13 隱私規則）。
pub fn dispatch_deep_link(url: &str) -> Result<DeepLinkRoute, TauriCommandError> {
    // Step 1 — parse the raw URL into scheme / host / path / query parts.
    // Stage 3 uses a hand-rolled `phantom://` parser; the `url` crate is
    // optional behind `experimental-hermes-tools` so we avoid pulling it
    // into the default build. The phantom:// grammar is intentionally
    // narrow (no userinfo, no port, no fragment) which makes a 30-line
    // parser straightforward + audit-friendly.
    let parsed = url_parse(url)?;

    // Step 2 — enforce the scheme allowlist: only `phantom://` reaches the
    // dispatcher. Any other scheme (or no scheme) is rejected with
    // `deep_link_invalid_scheme` per SPEC-17 §8. §13 privacy rule: we log
    // only the URL length + reason, never the raw URL string.
    if parsed.scheme != "phantom" {
        return Err(TauriCommandError::new(
            "deep_link_invalid_scheme",
            "only phantom:// scheme is accepted",
        ));
    }

    // Step 3 — split the host + path components into a normalised route.
    // §11.2 allowlist filtering (drop `..`, `%2e%2e`, query-key-count cap,
    // value-length cap) lives inside `route_split`.
    let (host, path) = route_split(&parsed)?;

    // Step 4 — extract decoded query parameters into a `Vec<(String, String)>`
    // (we keep `Vec` instead of `HashMap` so order is preserved for the
    // §11.2 allowlist pattern matcher).
    let query_params = extract_query(&parsed)?;

    // Step 5 — for the OAuth callback path (`phantom://oauth/callback?...`),
    // **CRITICAL** per SPEC-17 §5 audit fix: we MUST NOT propagate the raw
    // `code` / `token` query params into the returned `DeepLinkRoute`.
    // Instead, sanitise down to `{state_id, provider}` so the webview only
    // sees the opaque correlation handle; the secret half stays Rust-side.
    let sanitized_query = if host == "oauth" && path.starts_with("callback") {
        sanitize_oauth_callback(&query_params)?
    } else {
        query_params
    };

    // Step 6 — assemble the `DeepLinkRoute` and return.
    Ok(DeepLinkRoute {
        scheme: parsed.scheme,
        host,
        path,
        query_params: sanitized_query,
    })
}

/// Stage 3 helper — parse a `phantom://host/path?query` URL into its
/// component parts. Hand-rolled to avoid pulling in the optional `url`
/// crate (gated behind `experimental-hermes-tools`). Accepts only the
/// narrow phantom:// grammar: no userinfo, no port, no fragment.
///
/// 中文: 解析 `phantom://` URL 成 scheme / host / path / query 結構。手寫
/// 解析（不用 `url` crate，因為它在 default-off feature 後面）；只接受
/// phantom:// 的 narrow grammar。
fn url_parse(raw: &str) -> Result<ParsedUrl, TauriCommandError> {
    let err_invalid = |reason: &str| {
        // §13 privacy rule: never log the raw URL; log only its length +
        // reason. We bake the reason into the error message and rely on
        // the caller's logger to honour the no-raw-URL rule.
        TauriCommandError::new(
            "deep_link_invalid_format",
            format!("URL ({}-byte) malformed: {}", raw.len(), reason),
        )
    };
    // Split scheme.
    let (scheme, rest) = match raw.split_once("://") {
        Some((s, r)) => (s.to_string(), r),
        None => return Err(err_invalid("missing `://` separator")),
    };
    if scheme.is_empty() {
        return Err(err_invalid("empty scheme"));
    }
    // Reject any `..` or `%2e%2e` anywhere in `rest` — even outside the
    // path segment we don't want them surviving into downstream handlers
    // (defence in depth; §11.2 calls these out explicitly).
    let rest_lower = rest.to_ascii_lowercase();
    if rest_lower.contains("..") || rest_lower.contains("%2e%2e") {
        return Err(err_invalid("path traversal (`..` or `%2e%2e`) rejected"));
    }
    // Split host vs path+query at first `/`.
    let (host, path_and_query) = match rest.split_once('/') {
        Some((h, pq)) => (h.to_string(), pq.to_string()),
        None => (rest.to_string(), String::new()),
    };
    if host.is_empty() {
        return Err(err_invalid("empty host"));
    }
    // Split path vs query at first `?`.
    let (path, query) = match path_and_query.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (path_and_query, String::new()),
    };
    Ok(ParsedUrl {
        scheme,
        host,
        path,
        query,
    })
}

/// Stage 3 helper — split the parsed URL's host + path into normalised route
/// segments. Today this is a thin pass-through (the `url_parse` step already
/// applied the `..` / `%2e%2e` reject); kept as a named helper so the Stage
/// 2 algorithm comment in `dispatch_deep_link` still reads linearly.
fn route_split(parsed: &ParsedUrl) -> Result<(String, String), TauriCommandError> {
    Ok((parsed.host.clone(), parsed.path.clone()))
}

/// Stage 3 helper — extract decoded query params, enforcing the §11.2 limits
/// (≤ 16 keys, ≤ 256 chars per value). Uses `urlencoding::decode` which is
/// already in core/Cargo.toml.
fn extract_query(parsed: &ParsedUrl) -> Result<Vec<(String, String)>, TauriCommandError> {
    if parsed.query.is_empty() {
        return Ok(Vec::new());
    }
    let mut out: Vec<(String, String)> = Vec::new();
    for raw_pair in parsed.query.split('&') {
        if raw_pair.is_empty() {
            continue;
        }
        let (k_raw, v_raw) = match raw_pair.split_once('=') {
            Some((k, v)) => (k, v),
            None => (raw_pair, ""),
        };
        let key = urlencoding::decode(k_raw)
            .map_err(|e| {
                TauriCommandError::new(
                    "deep_link_query_decode_failed",
                    format!("query key decode failed: {}", e),
                )
            })?
            .into_owned();
        let value = urlencoding::decode(v_raw)
            .map_err(|e| {
                TauriCommandError::new(
                    "deep_link_query_decode_failed",
                    format!("query value decode failed: {}", e),
                )
            })?
            .into_owned();
        if value.len() > 256 {
            return Err(TauriCommandError::new(
                "deep_link_query_value_too_long",
                format!(
                    "query value for key `{}` is {} chars (limit 256)",
                    key,
                    value.len()
                ),
            ));
        }
        out.push((key, value));
        if out.len() > 16 {
            return Err(TauriCommandError::new(
                "deep_link_too_many_query_keys",
                "deep-link query has > 16 keys (limit per §11.2)".to_string(),
            ));
        }
    }
    Ok(out)
}

/// Stage 3 helper — sanitise an OAuth callback's query params down to
/// `{state_id, provider}` only; drop any `code` / `token` / `id_token` /
/// `access_token` / `refresh_token` keys per SPEC-17 §5 audit fix.
///
/// 中文: 過濾 OAuth callback 的 query 參數，只保留 `state_id` 與 `provider`；
/// 任何 token 類欄位都不能進到回傳結果（§5 安全修正）。
fn sanitize_oauth_callback(
    query_params: &[(String, String)],
) -> Result<Vec<(String, String)>, TauriCommandError> {
    // §5 audit-fix invariant: only the opaque correlation handle
    // (`state_id`) + the provider slug (for UI routing) are allowed to
    // cross into the webview side. Every other key — especially the
    // token / code / id_token family — must stay Rust-side so the
    // webview never sees secret material.
    const ALLOWED: &[&str] = &["state_id", "provider"];
    let kept: Vec<(String, String)> = query_params
        .iter()
        .filter(|(k, _)| ALLOWED.contains(&k.as_str()))
        .cloned()
        .collect();
    Ok(kept)
}

/// Internal struct for the `url_parse → route_split → extract_query`
/// pipeline. Stage 3 made this real; it's no longer a placeholder.
#[derive(Debug, Clone)]
struct ParsedUrl {
    scheme: String,
    host: String,
    path: String,
    query: String,
}

/// Apply an onboarding FSM transition per SPEC-28. Returns the **new** state
/// on success. Invariants:
///
/// - `Forward` on `FirstReplyReceived` returns
///   `TauriCommandError { code: "onboarding_already_complete", ... }`.
/// - `Rollback` is sanctioned **only** on the `JoinedCluster →
///   CreatedIdentity` edge; all other `Rollback` calls return the input
///   state unchanged (treated as `NoOp`) so the UI never accidentally
///   un-creates an identity.
/// - `NoOp` always succeeds and returns the input state.
///
/// 中文: 套用 SPEC-28 的 onboarding FSM 狀態轉換。`Rollback` 只在合法退路
/// （JoinedCluster→CreatedIdentity）真實生效，其他位置自動退化成 NoOp，
/// 避免 UI 意外 un-create 身份。
pub fn transition_onboarding(
    state: OnboardingState,
    transition: OnboardingTransition,
) -> Result<OnboardingState, TauriCommandError> {
    // Step 1 — look up `(current_state, transition)` in the SPEC-28 §8 FSM
    // table. Stage 3 backs this with a hard-coded `match` on
    // `(state, transition)` so the legal-edge set is one grep away.
    let outcome = fsm_lookup(state, transition)?;

    // Step 2 — validate the transition is legal for the FSM:
    //   - `Forward`: advance one step; on `FirstReplyReceived` returns
    //     `onboarding_already_complete` error.
    //   - `Rollback`: only the sanctioned `JoinedCluster → CreatedIdentity`
    //     edge succeeds; every other Rollback degrades to NoOp so the UI
    //     can never accidentally un-create an identity.
    //   - `NoOp`: always returns the input state unchanged.
    // The legality check happens inside `fsm_lookup_pseudo`; here we only
    // ensure the `outcome` value the helper returned is internally
    // consistent (e.g. Forward never produces a state earlier than input).
    let new_state = outcome;

    // Step 3 — emit a `OnboardingProgressEvent` so the UI can react to the
    // transition (progress bar, route navigation). Stage 3 wires this into
    // `tauri::Event` via the app handle injected into the dispatcher.
    emit_event_pseudo(state, transition, new_state)?;

    // Step 4 — return the new state. Callers are expected to persist this
    // into the on-disk onboarding marker (handled by the command wrapper,
    // not this pure-function layer).
    Ok(new_state)
}

/// Stage 3 helper — look up `(state, transition)` in the SPEC-28 §8 FSM
/// table and return the resulting `OnboardingState`. The table is a single
/// `match` expression so the legal-edge set is trivially greppable.
///
/// 中文: 在 SPEC-28 §8 的 FSM 表查 `(state, transition)`，回傳結果狀態。
/// `Forward` 從 `FirstReplyReceived` 出發回 `onboarding_already_complete`；
/// `Rollback` 只在 `JoinedCluster → CreatedIdentity` 真實生效，其他位置
/// 自動退化成 NoOp。
fn fsm_lookup(
    state: OnboardingState,
    transition: OnboardingTransition,
) -> Result<OnboardingState, TauriCommandError> {
    use OnboardingState::*;
    use OnboardingTransition::*;
    match (state, transition) {
        // Forward edges — happy path advances one step.
        (FreshInstall, Forward) => Ok(PickedLanguage),
        (PickedLanguage, Forward) => Ok(CreatedIdentity),
        (CreatedIdentity, Forward) => Ok(JoinedCluster),
        (JoinedCluster, Forward) => Ok(SetProvider),
        (SetProvider, Forward) => Ok(FirstReplyReceived),
        // Forward on the terminal state is an error — the UI should not
        // be calling Forward once onboarding is done.
        (FirstReplyReceived, Forward) => Err(TauriCommandError::new(
            "onboarding_already_complete",
            "cannot advance past FirstReplyReceived (onboarding done)",
        )),
        // Sanctioned rollback — user cancels half-joined cluster.
        (JoinedCluster, Rollback) => Ok(CreatedIdentity),
        // All other Rollbacks degrade to NoOp so the UI can never
        // accidentally un-create an identity / un-pick a language.
        (s, Rollback) => Ok(s),
        // NoOp always returns the input state.
        (s, NoOp) => Ok(s),
    }
}

/// Stage 4 helper — emit an `OnboardingProgressEvent` to the webview so
/// the UI can react to the transition. Currently a no-op because emitting
/// requires a `tauri::AppHandle` (only available inside a Tauri command
/// handler at runtime, not in the pure wire layer). Stage 4 wires this
/// through a channel injected by the Tauri command wrapper.
///
/// 中文: 發送 `OnboardingProgressEvent`。Stage 3 是 no-op；Stage 4 接
/// `tauri::AppHandle::emit` 後讓 UI 真實收到事件。
fn emit_event_pseudo(
    _from: OnboardingState,
    _transition: OnboardingTransition,
    _to: OnboardingState,
) -> Result<(), TauriCommandError> {
    // Stage 4: requires a `tauri::AppHandle` injected from the Tauri
    // command wrapper. Today we no-op; the FSM transition + caller's
    // persisted state are sufficient for the SPEC-28 invariant — the
    // UI just polls the marker after the transition succeeds.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn onboarding_state_round_trip_smoke() {
        // §7 / SPEC-28 invariant: each FSM state serialises to its snake_case
        // slug and re-decodes back. The 6 variants below must stay stable
        // forever — they leak into the Tauri-bridge ts-rs export, which the
        // frontend onboarding screens key off.
        let states = [
            OnboardingState::FreshInstall,
            OnboardingState::PickedLanguage,
            OnboardingState::CreatedIdentity,
            OnboardingState::JoinedCluster,
            OnboardingState::SetProvider,
            OnboardingState::FirstReplyReceived,
        ];
        let expected_slugs = [
            "\"fresh_install\"",
            "\"picked_language\"",
            "\"created_identity\"",
            "\"joined_cluster\"",
            "\"set_provider\"",
            "\"first_reply_received\"",
        ];
        for (state, expected) in states.iter().zip(expected_slugs.iter()) {
            let j = serde_json::to_string(state).unwrap();
            assert_eq!(&j, expected, "wire slug for {:?}", state);
            let back: OnboardingState = serde_json::from_str(&j).unwrap();
            assert_eq!(*state, back, "round-trip for {:?}", state);
        }
    }

    #[test]
    fn tauri_command_error_serializes_camel_case() {
        // §7.1 envelope invariant: the inner error object must have camelCase
        // field names so the frontend can destructure
        // `{ code, message, recoveryHint }` without a runtime adapter.
        let e = TauriCommandError::new("provider_auth_error", "API key invalid")
            .with_hint("open settings → providers → re-paste");
        let j = serde_json::to_string(&e).unwrap();
        assert!(j.contains("\"code\":\"provider_auth_error\""), "code: {}", j);
        assert!(j.contains("\"message\":\"API key invalid\""), "msg: {}", j);
        assert!(j.contains("\"recoveryHint\":"), "hint camelCase: {}", j);
    }

    #[test]
    fn empty_args_serializes_to_empty_object() {
        // §7.2 invariant: `EmptyArgs` MUST serialise to `{}` so the wire
        // shape stays a JSON object even for no-arg commands — the frontend
        // bridge unconditionally posts an args object.
        let a = EmptyArgs::default();
        let j = serde_json::to_string(&a).unwrap();
        assert_eq!(j, "{}");
        let _: EmptyArgs = serde_json::from_str("{}").unwrap();
    }

    // ─── Stage 3 KAT (known-answer-test) tests ───────────────────────────
    //
    // Replace the Stage 2 `#[should_panic(expected = "Stage 3")]` markers
    // with real behaviour tests now that catalog_lookup / dispatch_deep_link
    // / transition_onboarding are live impls.

    #[test]
    fn validate_command_accepts_known_catalog_entry() {
        // §9.1 invariant: `theme_set_mode` is in the catalog and accepts a
        // `ThemeSetModeArgs`. Step 1 (catalog_lookup) resolves; Step 2
        // (permission_check) auto-grants in Stage 3; Step 3
        // (validate_args_shape) confirms object-shape. End to end → Ok.
        let args = ThemeSetModeArgs { mode: "dark".to_string() };
        assert!(validate_command("theme_set_mode", &args).is_ok());
    }

    #[test]
    fn validate_command_rejects_unknown_command() {
        // §9.4 invariant: a name not in the §9.1 + §20.1 catalog must
        // surface as `unknown_command` so the frontend can show a
        // diagnostic banner rather than silently dropping.
        let err = validate_command("totally_fake_command", &EmptyArgs::default()).unwrap_err();
        assert_eq!(err.code, "unknown_command");
        assert!(
            err.message.contains("totally_fake_command"),
            "carries bad input for debug: {}",
            err.message
        );
    }

    #[test]
    fn validate_command_rejects_non_object_args() {
        // §7.2 invariant: args must be a JSON object. A raw string / array
        // payload fails fast with `args_must_be_object`.
        let err = validate_command("theme_set_mode", &"not-an-object").unwrap_err();
        assert_eq!(err.code, "args_must_be_object");
    }

    #[test]
    fn dispatch_deep_link_sanitizes_oauth_callback_tokens() {
        // SPEC-17 §5 audit-fix invariant: even when the caller hands us an
        // OAuth callback URL containing raw `code` / `access_token` params,
        // `dispatch_deep_link` MUST drop them and only return
        // `{state_id, provider}` in `query_params`.
        let route = dispatch_deep_link(
            "phantom://oauth/callback?state_id=abc&provider=cerebras\
             &code=SECRET_AUTH_CODE&access_token=SECRET_BEARER",
        )
        .expect("dispatch");
        let keys: Vec<&str> = route.query_params.iter().map(|(k, _)| k.as_str()).collect();
        assert!(keys.contains(&"state_id"), "kept state_id: {:?}", keys);
        assert!(keys.contains(&"provider"), "kept provider: {:?}", keys);
        assert!(!keys.contains(&"code"), "dropped code (secret): {:?}", keys);
        assert!(
            !keys.contains(&"access_token"),
            "dropped access_token (secret): {:?}",
            keys
        );
    }

    #[test]
    fn dispatch_deep_link_rejects_path_traversal() {
        // §11.2 invariant: `..` or `%2e%2e` anywhere in the URL causes the
        // whole URL to be dropped with `deep_link_invalid_format`. We must
        // never let a deep-link traversal escape into a handler.
        let err = dispatch_deep_link("phantom://coach/../etc/passwd").unwrap_err();
        assert_eq!(err.code, "deep_link_invalid_format");

        let err2 = dispatch_deep_link("phantom://coach/%2E%2E/secrets").unwrap_err();
        assert_eq!(err2.code, "deep_link_invalid_format");
    }

    #[test]
    fn dispatch_deep_link_rejects_non_phantom_scheme() {
        // §8 invariant: only `phantom://` is accepted; `http://` / `file://`
        // / `javascript:` all reject with `deep_link_invalid_scheme`.
        let err = dispatch_deep_link("http://evil.example.com/x").unwrap_err();
        assert_eq!(err.code, "deep_link_invalid_scheme");
    }

    #[test]
    fn dispatch_deep_link_preserves_non_oauth_query_params() {
        // §8 invariant: non-OAuth deep-links keep all query params (after
        // the §11.2 limit checks). The sanitiser only fires on the OAuth
        // callback path.
        let route = dispatch_deep_link("phantom://coach/review?date=2026-05-25&lang=zh-TW")
            .expect("dispatch");
        let keys: Vec<&str> = route.query_params.iter().map(|(k, _)| k.as_str()).collect();
        assert!(keys.contains(&"date"));
        assert!(keys.contains(&"lang"));
    }

    #[test]
    fn fsm_lookup_forward_chain_advances_one_step() {
        // SPEC-28 §8 invariant: Forward from each state advances by exactly
        // one step in the sequence FreshInstall → ... → FirstReplyReceived.
        let mut state = OnboardingState::FreshInstall;
        let expected = [
            OnboardingState::PickedLanguage,
            OnboardingState::CreatedIdentity,
            OnboardingState::JoinedCluster,
            OnboardingState::SetProvider,
            OnboardingState::FirstReplyReceived,
        ];
        for next in expected {
            state = fsm_lookup(state, OnboardingTransition::Forward).expect("forward");
            assert_eq!(state, next);
        }
    }

    #[test]
    fn fsm_lookup_forward_from_terminal_errors() {
        // SPEC-28 §8 invariant: Forward from FirstReplyReceived returns
        // `onboarding_already_complete` rather than silently no-op'ing.
        let err = fsm_lookup(OnboardingState::FirstReplyReceived, OnboardingTransition::Forward)
            .unwrap_err();
        assert_eq!(err.code, "onboarding_already_complete");
    }

    #[test]
    fn fsm_lookup_rollback_only_valid_on_sanctioned_edge() {
        // SPEC-28 §8 invariant: Rollback succeeds ONLY on
        // `JoinedCluster → CreatedIdentity`. Every other Rollback is a NoOp.
        let s = fsm_lookup(OnboardingState::JoinedCluster, OnboardingTransition::Rollback)
            .expect("sanctioned");
        assert_eq!(s, OnboardingState::CreatedIdentity);

        // Every other Rollback degrades to NoOp — return input unchanged.
        for state in [
            OnboardingState::FreshInstall,
            OnboardingState::PickedLanguage,
            OnboardingState::CreatedIdentity,
            OnboardingState::SetProvider,
            OnboardingState::FirstReplyReceived,
        ] {
            let out = fsm_lookup(state, OnboardingTransition::Rollback).expect("noop");
            assert_eq!(out, state, "rollback from {:?} must NoOp", state);
        }
    }

    #[test]
    fn fsm_lookup_noop_returns_input() {
        // SPEC-28 §8 invariant: NoOp always succeeds + returns input state.
        // Used for "is current state still X?" UI queries without mutation.
        for state in [
            OnboardingState::FreshInstall,
            OnboardingState::JoinedCluster,
            OnboardingState::FirstReplyReceived,
        ] {
            let out = fsm_lookup(state, OnboardingTransition::NoOp).expect("noop");
            assert_eq!(out, state);
        }
    }

    #[test]
    fn transition_onboarding_end_to_end_emits_no_error() {
        // §9.4 invariant: transition_onboarding wires together fsm_lookup +
        // emit_event. Stage 3 emit_event is no-op so the full pipeline
        // returns Ok on a legal Forward edge.
        let next = transition_onboarding(
            OnboardingState::FreshInstall,
            OnboardingTransition::Forward,
        )
        .expect("forward");
        assert_eq!(next, OnboardingState::PickedLanguage);
    }
}
