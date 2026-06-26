// SPEC-24 §7 + §9 — Coach delivery wire types (single source of truth for the
// 3-channel fan-out: Markdown / Telegram / Email — DeliveryConfig / per-channel
// configs / DeliveryReceipt + DeliveryAttempt + DeliveryError catalog).
//
// Stage 3 (real impl — pure + serde_json + std::fs + SystemTime helpers live):
// the helpers that depend only on the Rust stdlib + `serde_json` (already in
// core/Cargo.toml) are now backed by real code. Helpers that delegate to
// wires still routed through Stage 2 (encryption_wire / broker_vault_wire)
// or require a crate / feature NOT yet in core/Cargo.toml (`reqwest`'s
// `blocking` feature for sync HTTP POST) stay as Stage 4 panicking markers
// so the audit grep still finds the boundary. The rusqlite ledger and the
// TOML settings reader are now self-contained inside this wire (Phase F-2)
// using the lazy-open pattern shared with `capture_habit_wire`.
//
// 中文: 本檔對應 SPEC-24 §7（資料模型）與 §9（API contracts，應用程式介面合
// 約）。教練（coach，每日複盤引擎）emit `coach.review.ready` 後，
// `NotificationDispatcher`（通知派送中介）查 `DeliveryConfig` → 並行送 3 個
// channel（通道）：本機 markdown 檔（同時觸發 OS notification，作業系統通
// 知）/ Telegram bot push（機器人推播）/ user-owned SMTP email（使用者自帶的
// 簡單郵件傳輸協定 email）。Stage 1 只把 wire（線上格式）型別 + stub（樁函
// 式）排好；Stage 2 把真實邏輯接進 `core/src/notifications/channels/*.rs`。
//
// 縮寫對照表（acronym + 中文意譯）：
// > - `SMTP`（簡單郵件傳輸協定，Simple Mail Transfer Protocol）— 寄信用的
// >   通訊協定，本檔 EmailConfig 用它送 review email。
// > - `TLS`（傳輸層安全協定，Transport Layer Security）— SMTP 上加密通道，
// >   v0.6.0 預設走 STARTTLS（port 587）模式。
// > - `UUIDv7`（通用唯一識別碼第 7 版，Universally Unique Identifier v7）—
// >   時間有序的 128-bit ID，coach review_id 用它。
// > - `OS`（作業系統，Operating System）— 觸發桌面 / 行動端原生通知。
// > - `MarkdownV2`（Markdown 第 2 版）— Telegram Bot API 的嚴格 Markdown
// >   方言，跟 GitHub-flavored 不一樣，須 escape 特殊字元。
// > - `vault`（保險庫）— SPEC-15 broker（中介伺服器）的 secret 儲存區，本
// >   檔只引用 `_ref`（引用字串，e.g. `vault://telegram/bot_token`），絕不
// >   存明文 token / password。
// > - `ledger`（帳本）— sqlite 表 `coach_delivery_ledger`，dedup（去重）+
// >   retry 狀態追蹤用，schema 見 SPEC-24 §7.4。
//
// **Cycle-break note**（跨 spec 循環依賴打破說明，per SPEC-23/24 polish pass）:
// `CoachReviewReadyPayload`（在 `coach_wire.rs`）只帶 `markdown_path` 欄位，
// **不帶** `markdown_body` 明文。本檔 Stage 2 實作必須 **先讀+解密** 該路徑
// 指向的 `.md.age` 檔案（age v1 ciphertext，密文）才拿得到 markdown 明文；
// 永遠不要從 event payload 直接拿明文 — 避免明文飄散到 EventBus（事件匯流
// 排）subscriber log（訂閱者日誌）。`write_markdown_file` stub 同樣強制只
// 寫 `.md.age` 加密檔，不寫明文 `.md` 到 disk。
//
// **Push channel reservation**: `DeliveryChannel` 預留 `Push` variant（變體）
// for v0.6.x mobile background notification（行動端背景推播），但 v0.6.0 **不
// 啟用** — 標 `#[serde(skip)]` 讓 wire 不序列化、UI / settings 也看不到該選
// 項。`deliver()` Stage 2 收到 `Push` 必須 reject 為 `DeliveryError::ConfigMissing`。
//
// TODO Stage 2:
//   - 把 `deliver()` 接進 `NotificationDispatcher::subscribe_coach_reviews`
//     (per SPEC-24 §8.1 channel router 訂閱規則)，fan-out 3 channel `tokio::spawn`。
//   - `send_telegram()` 用 `reqwest` POST `https://api.telegram.org/bot<token>/
//     sendMessage`，body `{chat_id, text, parse_mode}`；401 → map 為
//     `DeliveryError::TelegramBotTokenInvalid`。
//   - `send_email()` 用 `lettre` crate（SMTP client lib）建 `SmtpTransport`
//     STARTTLS / TLS 雙模式；5xx auth → `DeliveryError::EmailSmtpFailed`；
//     5.7.x recipient reject → `DeliveryError::EmailRecipientRejected`。
//   - `write_markdown_file()` 走 SPEC-13 age encrypt → 寫
//     `~/.phantom-mesh/coach/YYYY-MM-DD.md.age`；同時觸發 OS notification
//     (per §10.2)。
//   - `dedup_check()` 查 `coach_delivery_ledger` PK `(review_id, channel,
//     attempted_at_ms)`，24 小時內已有 `status='sent'` row → 回 `true`。

use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

// ─── §7.1 DeliveryChannel — 3 active channels + reserved Push ────────────────

/// Active delivery channels for a coach review. v0.6.0 ships **only 3
/// channels**: `Markdown` (write to local file + OS notification),
/// `Telegram` (bot push), `Email` (user-owned SMTP). `Push` is reserved for
/// v0.6.x mobile background notification but `#[serde(skip)]` — it does
/// **not** appear on the wire and Stage 2 `deliver()` must reject it as
/// `DeliveryError::ConfigMissing`.
///
/// 中文: 教練 review 派送通道。v0.6.0 只開 3 條：`Markdown`（本機 md 檔 + OS
/// 通知）/ `Telegram`（機器人推播）/ `Email`（使用者自帶 SMTP）。`Push`
/// 預留給 v0.6.x 行動端背景推播但本版 **不啟用**，serde 序列化跳過、UI 也看
/// 不到該選項。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/coach_delivery/")]
#[serde(rename_all = "snake_case")]
pub enum DeliveryChannel {
    /// Write age-encrypted `.md.age` to `~/.phantom-mesh/coach/` and fire
    /// an OS notification with a 80-char snippet. Always-on default.
    Markdown,
    /// Telegram Bot API `sendMessage` to user-configured `chat_id`. Bot
    /// token is fetched from SPEC-15 vault per `TelegramConfig.bot_token_ref`.
    Telegram,
    /// SMTP send via user-owned mail server (lettre crate). Password is
    /// fetched from SPEC-15 vault per `EmailConfig.smtp_password_ref`.
    Email,
    /// **Reserved for v0.6.x — not yet active**. Mobile push via APNs (Apple
    /// Push Notification service) / FCM (Firebase Cloud Messaging). Skipped
    /// from serde so it never appears on the wire (per cycle-break fix).
    #[serde(skip)]
    Push,
}

// ─── §7.1 DeliveryStatus — terminal status of one delivery attempt ───────────

/// Terminal status of a single channel delivery attempt. Mirrors SPEC-24
/// §7.1 with `Pending` (still in-flight) + `Sent` (channel confirmed OK)
/// + `Failed` (permanently failed, retries exhausted) + `Suppressed`
/// (dedup ledger hit — already sent within 24h, did not actually send).
///
/// 中文: 一次 channel 派送的終態。`Pending`（仍進行中）/ `Sent`（通道確認
/// 成功）/ `Failed`（永久失敗，重試已用盡）/ `Suppressed`（dedup ledger 已
/// 命中 — 24 小時內已送過，沒實際發）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/coach_delivery/")]
#[serde(rename_all = "snake_case")]
pub enum DeliveryStatus {
    /// Emit 已派發、通道尚未回報結果。In-flight 中介態 (< 7 s typically).
    Pending,
    /// Channel 確認 OK — 例如 Telegram API 回 200 / SMTP 250 / 檔案 fsync 成功。
    Sent,
    /// Channel 永久失敗 — 已 retry exhausted 或 hard error (401 / 535 / disk full)。
    Failed,
    /// Dedup ledger hit — review_id + channel 在 24 小時 window 內已有
    /// `Sent` row，本次跳過不實際發 (per SPEC-24 §8 dedup 規則)。
    Suppressed,
}

// ─── §7.1 TelegramParseMode — Bot API parse_mode field ───────────────────────

/// Telegram Bot API `parse_mode` enum. `MarkdownV2` is the recommended
/// v2 strict mode (须 escape 特殊字元); `Markdown` is legacy v1 (deprecated
/// by Telegram but still works); `Html` is HTML subset; `Plain` sends raw
/// text with no parsing (no `parse_mode` field on the wire).
///
/// 中文: Telegram Bot API 的 `parse_mode` 欄位。`MarkdownV2` 是推薦的嚴格
/// v2 模式（特殊字元如 `_` `*` `[` 都要 escape）；`Markdown` 是 Telegram 自
/// 己 deprecate 的 legacy v1；`Html` 是 HTML 子集；`Plain` 是純文字（API
/// 端不帶 parse_mode 欄位）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/coach_delivery/")]
#[serde(rename_all = "snake_case")]
pub enum TelegramParseMode {
    /// Legacy v1 Markdown — deprecated by Telegram but still parses.
    Markdown,
    /// Recommended strict v2 — special chars `_*[]()~\`>#+-=|{}.!` must escape.
    MarkdownV2,
    /// HTML subset (`<b>`, `<i>`, `<code>`, `<a>`, `<pre>`).
    Html,
    /// Plain text — no `parse_mode` field on the wire, raw text only.
    Plain,
}

// ─── §7.1 TelegramConfig — bot push channel config ───────────────────────────

/// Telegram channel configuration. **Crucially `bot_token_ref` is a vault
/// reference string (e.g. `"vault://telegram/bot_token"`), NEVER the plaintext
/// token** — Stage 2 resolves it via SPEC-15 broker vault GET. The plaintext
/// token never touches `config.toml`, the wire, or any log.
///
/// 中文: Telegram 通道設定。**`bot_token_ref` 是 vault 引用字串（例如
/// `"vault://telegram/bot_token"`），絕對 NOT 明文 token** — Stage 2 透過
/// SPEC-15 broker vault GET 解出真實 token。明文 token 永遠不會進入
/// `config.toml`、wire 或任何 log。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/coach_delivery/")]
#[serde(rename_all = "camelCase")]
pub struct TelegramConfig {
    /// Vault reference for the bot token (`"vault://telegram/bot_token"`).
    /// **NEVER plaintext** — Stage 2 calls SPEC-15 `vault.get(ref)` to resolve.
    pub bot_token_ref: String,
    /// Target chat — user private chat / group / channel id (Telegram allows
    /// negative ids for groups; stored as string for cross-platform safety).
    pub chat_id: String,
    /// `parse_mode` passed on every `sendMessage` call. Default
    /// `MarkdownV2` for proper escape handling.
    pub parse_mode: TelegramParseMode,
}

// ─── §7.1 EmailConfig — SMTP channel config ──────────────────────────────────

/// Email channel configuration (user-owned SMTP — phantom does NOT relay
/// through any phantom-operated mail server). **`smtp_password_ref` is a
/// vault reference, NEVER plaintext password**. `use_tls = true` → port 465
/// implicit TLS; `use_tls = false` + port 587 → STARTTLS (default
/// recommendation per SPEC-24 §7).
///
/// 中文: Email 通道設定（使用者自帶 SMTP — phantom 自己不營運郵件伺服器）。
/// **`smtp_password_ref` 是 vault 引用字串，絕對 NOT 明文密碼**。
/// `use_tls = true` 走 port 465 implicit TLS；`false` + port 587 走 STARTTLS
/// (預設推薦，相容性最高)。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/coach_delivery/")]
#[serde(rename_all = "camelCase")]
pub struct EmailConfig {
    /// SMTP server hostname (`"smtp.gmail.com"` / `"mail.fastmail.com"` / ...).
    pub smtp_host: String,
    /// SMTP server port. 587 = STARTTLS (recommended); 465 = implicit TLS;
    /// 25 = plaintext (NOT supported by Stage 2).
    pub smtp_port: u16,
    /// SMTP username (typically the same as `from_address`).
    pub smtp_user: String,
    /// Vault reference for SMTP password (`"vault://email/smtp_pass"`).
    /// **NEVER plaintext** — Stage 2 resolves via SPEC-15 vault GET.
    pub smtp_password_ref: String,
    /// `From:` header — sender address shown to recipient.
    pub from_address: String,
    /// `To:` header — recipient (typically user's own inbox).
    pub to_address: String,
    /// `true` = implicit TLS (port 465); `false` = STARTTLS (port 587).
    pub use_tls: bool,
}

// ─── §7.1 DeliveryConfig — top-level user config ─────────────────────────────

/// Top-level delivery configuration stored under `[coach.delivery]` in
/// `~/.phantom-mesh/config.toml`. Each channel is opt-in — `markdown_enabled`
/// defaults `true` (offline-safe baseline), Telegram + Email default disabled
/// (require explicit user setup), Push is reserved for v0.6.x and `false`.
///
/// 中文: 教練派送的最上層使用者設定，存在 `~/.phantom-mesh/config.toml` 的
/// `[coach.delivery]` section。每個通道都 opt-in — `markdown_enabled` 預
/// 設 `true`（離線安全 baseline），Telegram / Email 預設關閉（要 user 自己
/// 設定才開），`push_enabled` 預留 v0.6.x 用、本版固定 `false`。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/coach_delivery/")]
#[serde(rename_all = "camelCase")]
pub struct DeliveryConfig {
    /// Local markdown file + OS notification channel. Default `true`.
    pub markdown_enabled: bool,
    /// Telegram bot config — `None` = channel disabled.
    pub telegram_config: Option<TelegramConfig>,
    /// Email SMTP config — `None` = channel disabled.
    pub email_config: Option<EmailConfig>,
    /// Push channel toggle. **Reserved for v0.6.x — must stay `false` in
    /// v0.6.0**. Stage 2 `deliver()` rejects with `ConfigMissing` if `true`.
    #[serde(default)]
    pub push_enabled: bool,
}

// ─── §7.1 DeliveryReceipt — single-channel outcome ───────────────────────────

/// Receipt of one channel's delivery attempt for one review. Persisted to
/// the `coach_delivery_ledger` sqlite table (PK `(review_id, channel,
/// attempted_at_ms)`) so the UI history view + dedup check can both query it.
///
/// 中文: 一次 channel 派送結果的收據，落地寫入 sqlite 表
/// `coach_delivery_ledger`（主鍵 `(review_id, channel, attempted_at_ms)`），
/// UI 歷史頁 + dedup check 都從此表讀。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/coach_delivery/")]
#[serde(rename_all = "camelCase")]
pub struct DeliveryReceipt {
    /// UUIDv7 of the coach review (same as `CoachReviewReadyPayload.review_id`).
    pub review_id: String,
    /// Channel this receipt is for.
    pub channel: DeliveryChannel,
    /// Unix epoch milliseconds when the attempt started.
    pub attempted_at_ms: u64,
    /// Terminal status of the attempt.
    pub status: DeliveryStatus,
    /// Human-readable error message — `None` when `status == Sent` or
    /// `Suppressed`; populated when `Failed`.
    pub error_message: Option<String>,
}

// ─── §7.1 DeliveryAttempt — retry-tracking variant ───────────────────────────

/// Retry-tracking record for one channel attempt. Distinct from
/// `DeliveryReceipt` in that it carries `retry_count` (how many times this
/// review-channel pair has been retried); ledger query merges attempts by
/// `(review_id, channel)` to surface the latest retry to the UI.
///
/// 中文: 一次 channel 嘗試的重試追蹤記錄，跟 `DeliveryReceipt` 不同處在於多
/// 帶一個 `retry_count`（同 review × channel 被重試的次數）；ledger 查詢時
/// 用 `(review_id, channel)` 做合併、UI 只顯示最新一筆。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/coach_delivery/")]
#[serde(rename_all = "camelCase")]
pub struct DeliveryAttempt {
    /// UUIDv7 of the coach review.
    pub review_id: String,
    /// Channel this attempt is for.
    pub channel: DeliveryChannel,
    /// Unix epoch milliseconds when this attempt started.
    pub attempted_at_ms: u64,
    /// How many times this review-channel pair has been retried so far
    /// (0 = first attempt). Stage 2 caps at 3 retries per channel.
    pub retry_count: u8,
    /// Terminal status of this attempt.
    pub status: DeliveryStatus,
    /// Human-readable error message when `status == Failed`.
    pub error_message: Option<String>,
}

// ─── §11.1 DeliveryError — error catalog mirror ──────────────────────────────

/// Wire-facing error variants for the coach delivery subsystem. Mirrors the
/// SPEC-24 §11.1 error catalog one-to-one (subset focused on per-channel
/// failure modes the UI + CLI dispatch on). Sent back via Tauri command
/// failure path; CLI maps via `phantom_error::Error::user_message`.
///
/// 中文: SPEC-24 §11.1 error catalog 的 wire-facing 鏡像（聚焦在 per-channel
/// 失敗模式，UI + CLI 用機器可讀 code 做 dispatch）。`#[serde(tag = "code")]`
/// 讓前端可以 `switch (err.code)` 分流處理。
#[derive(Debug, Clone, Serialize, Deserialize, TS, thiserror::Error)]
#[ts(export, export_to = "../../app/src/lib/generated/coach_delivery/")]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum DeliveryError {
    /// Telegram API returned 401 — bot token invalid / revoked. Recovery:
    /// user regenerates token via BotFather and re-saves.
    #[error("delivery.telegram_bot_token_invalid")]
    TelegramBotTokenInvalid,
    /// SMTP server returned 5xx during connect / EHLO / AUTH. Recovery:
    /// verify smtp_host / smtp_user / vault password.
    #[error("delivery.email_smtp_failed: {detail}")]
    EmailSmtpFailed { detail: String },
    /// SMTP server returned 5.7.x permanent reject for recipient (e.g.
    /// mailbox does not exist, address blacklisted). Recovery: user
    /// fixes `to_address`.
    #[error("delivery.email_recipient_rejected: {address}")]
    EmailRecipientRejected { address: String },
    /// OS denied notification permission (macOS UserNotifications, iOS
    /// UNUserNotificationCenter, Android POST_NOTIFICATIONS). Recovery:
    /// user opens system Settings → allow phantom notifications.
    #[error("delivery.os_notification_denied: {os}")]
    OsNotificationDenied { os: String },
    /// Channel selected but its config is missing (`None`) or required
    /// secret unresolvable. Also returned when caller passes `Push`
    /// channel in v0.6.0 (reserved variant, not yet active).
    #[error("delivery.config_missing: {channel}")]
    ConfigMissing { channel: String },
    /// Per-channel rate limit hit (e.g. Telegram 30 msg/sec global, our
    /// own 60 set/hr config-write cap). Recovery: wait + retry.
    #[error("delivery.rate_limited: retry_after_ms={retry_after_ms}")]
    RateLimited { retry_after_ms: u64 },
}

// ─── §9.6 / §8 Stage-1 stub helpers (Stage 2 implements) ─────────────────────

/// Fan out one coach review to the requested channels in parallel and
/// return a `DeliveryReceipt` for each. Stage 2 spawns one `tokio::task`
/// per channel; Markdown writes locally, Telegram + Email do network I/O.
/// Per-channel failures do **not** abort the fan-out — each receipt is
/// independent.
///
/// `review_id` is the UUIDv7 from `CoachReviewReadyPayload.review_id`;
/// `markdown_path` is the path to the `.md.age` file (Stage 2 reads +
/// decrypts before passing plaintext to Telegram / Email senders — markdown
/// content is **never** passed via event payload, see cycle-break note at
/// top of file).
///
/// 中文: 把一份 coach review 並行 fan-out 到指定通道，每通道一份
/// `DeliveryReceipt` 回傳。Stage 2 每通道開一個 `tokio::task`；Markdown 寫
/// 本機，Telegram / Email 走網路。單通道失敗不終止 fan-out — 各 receipt 獨
/// 立。`markdown_path` 指向 `.md.age` 密文檔，Stage 2 必須先讀+解密才能把
/// 明文丟給 Telegram / Email sender（明文絕不放 event payload，見檔頭
/// cycle-break 註解）。
pub fn deliver(
    review_id: &str,
    markdown_path: &Path,
    channels: &[DeliveryChannel],
) -> Result<Vec<DeliveryReceipt>, DeliveryError> {
    // Step 1: read + decrypt the .md.age ciphertext once (single I/O hit; reuse
    //         plaintext across all network channels). Stage 3 wires
    //         encryption_wire so the plaintext only ever lives in this stack
    //         frame, never on disk and never in event payloads.
    let plaintext: String = read_md_age_pseudo(markdown_path)?;

    // Step 2: walk each requested channel and dispatch. Per-channel failure
    //         does NOT abort the fan-out — each receipt is independent so the
    //         UI can show "telegram OK, email failed" side-by-side.
    let mut receipts: Vec<DeliveryReceipt> = Vec::with_capacity(channels.len());
    let now_ms: u64 = now_ms_pseudo();

    for channel in channels {
        // Step 2a: dedup_check first — skip the actual send and write a
        //          `Suppressed` receipt when a Sent row already exists in
        //          the 24-hour ledger window for (review_id, channel).
        let already_sent: bool = dedup_check(review_id, *channel)?;
        if already_sent {
            receipts.push(DeliveryReceipt {
                review_id: review_id.to_string(),
                channel: *channel,
                attempted_at_ms: now_ms,
                status: DeliveryStatus::Suppressed,
                error_message: None,
            });
            continue;
        }

        // Step 2b: dispatch by channel kind. Markdown writes locally,
        //          Telegram + Email do network I/O. Push is reserved and
        //          maps to ConfigMissing per cycle-break rule.
        let outcome: Result<(), DeliveryError> = match channel {
            DeliveryChannel::Markdown => {
                write_markdown_file(review_id, markdown_path)
            }
            DeliveryChannel::Telegram => {
                // Caller wiring (Stage 3) injects TelegramConfig from
                // DeliveryConfig.telegram_config — pseudo stub stands in.
                let cfg: TelegramConfig = load_telegram_config_pseudo()?;
                send_telegram(&cfg, &plaintext)
            }
            DeliveryChannel::Email => {
                let cfg: EmailConfig = load_email_config_pseudo()?;
                send_email(&cfg, "Phantom coach review", &plaintext)
            }
            DeliveryChannel::Push => Err(DeliveryError::ConfigMissing {
                channel: "push".to_string(),
            }),
        };

        // Step 3: translate outcome into a DeliveryReceipt row — Sent on Ok,
        //         Failed on Err with the error rendered as the human-readable
        //         message_en already supplied by thiserror Display.
        let (status, error_message) = match outcome {
            Ok(()) => (DeliveryStatus::Sent, None),
            Err(e) => (DeliveryStatus::Failed, Some(e.to_string())),
        };
        receipts.push(DeliveryReceipt {
            review_id: review_id.to_string(),
            channel: *channel,
            attempted_at_ms: now_ms,
            status,
            error_message,
        });
    }

    // Step 4: return the vector. Caller (NotificationDispatcher) persists each
    //         row into coach_delivery_ledger and emits coach.delivery.done.
    Ok(receipts)
}

/// Send `markdown` text to Telegram via Bot API `sendMessage`. Stage 2
/// resolves `config.bot_token_ref` through SPEC-15 vault GET, builds the
/// POST body `{chat_id, text, parse_mode}`, and maps a 401 response to
/// `DeliveryError::TelegramBotTokenInvalid`.
///
/// 中文: 把 `markdown` 透過 Telegram Bot API `sendMessage` 發出去。Stage 2
/// 先用 SPEC-15 vault GET 解 `config.bot_token_ref` 拿 token，組 POST body
/// `{chat_id, text, parse_mode}`；401 回應 map 為
/// `DeliveryError::TelegramBotTokenInvalid`。
pub fn send_telegram(
    config: &TelegramConfig,
    markdown: &str,
) -> Result<(), DeliveryError> {
    // Step 1: resolve the bot token from the SPEC-15 broker vault. We never
    //         store / log the plaintext token — vault_read_pseudo returns it
    //         in a String that drops at the end of this call.
    let bot_token: String = vault_read_pseudo(&config.bot_token_ref)?;

    // Step 2: build the Bot API URL + JSON body. Telegram parse_mode goes on
    //         the wire as the lower-case snake string Telegram expects
    //         (markdown / markdown_v2 / html); Plain omits the field entirely.
    let url: String =
        format!("https://api.telegram.org/bot{}/sendMessage", bot_token);
    let parse_mode_str: Option<&'static str> = match config.parse_mode {
        TelegramParseMode::Markdown => Some("Markdown"),
        TelegramParseMode::MarkdownV2 => Some("MarkdownV2"),
        TelegramParseMode::Html => Some("HTML"),
        TelegramParseMode::Plain => None,
    };
    let body_json: String = build_telegram_body_pseudo(
        &config.chat_id,
        markdown,
        parse_mode_str,
    );

    // Step 3: POST the request. https_post_pseudo returns (status_code, body)
    //         so we can map the Bot API error envelope deterministically.
    let (status_code, response_body): (u16, String) =
        https_post_pseudo(&url, &body_json)?;

    // Step 4: parse the Bot API response shape `{ok: bool, description: str,
    //         parameters: {retry_after?: u64}}` and map known failure modes.
    let parsed: TelegramBotResponse = parse_telegram_response_pseudo(&response_body)?;

    // Step 5: dispatch on status code first (401 always wins — invalid token),
    //         then on the parsed `ok` flag for non-2xx + retry_after.
    if status_code == 401 {
        return Err(DeliveryError::TelegramBotTokenInvalid);
    }
    if status_code == 429 {
        let retry_after_ms: u64 =
            parsed.retry_after.unwrap_or(1) * 1000;
        return Err(DeliveryError::RateLimited { retry_after_ms });
    }
    if !parsed.ok || status_code >= 400 {
        return Err(DeliveryError::ConfigMissing {
            channel: "telegram".to_string(),
        });
    }
    Ok(())
}

/// Send an email with `subject` and `markdown_or_html` body via the
/// user-owned SMTP server in `config`. Stage 2 uses the `lettre` crate
/// (Rust SMTP client lib) with STARTTLS (port 587) or implicit TLS (port
/// 465) per `config.use_tls`; resolves `config.smtp_password_ref` via
/// SPEC-15 vault GET; maps SMTP 535 auth fail to `EmailSmtpFailed` and
/// 5.7.x recipient reject to `EmailRecipientRejected`.
///
/// 中文: 透過使用者自帶的 SMTP server 寄信。Stage 2 用 `lettre` crate（Rust
/// SMTP client 套件）走 STARTTLS（port 587）或 implicit TLS（port 465）—
/// 看 `config.use_tls`；用 SPEC-15 vault GET 解 `smtp_password_ref` 拿密
/// 碼；SMTP 535 auth fail 對應 `EmailSmtpFailed`，5.7.x 收件人 reject 對應
/// `EmailRecipientRejected`。
pub fn send_email(
    config: &EmailConfig,
    subject: &str,
    markdown_or_html: &str,
) -> Result<(), DeliveryError> {
    // Step 1: resolve the SMTP password from the SPEC-15 broker vault. As
    //         with the Telegram bot token, the plaintext password is never
    //         persisted — it lives in this stack frame only.
    let smtp_password: String = vault_read_pseudo(&config.smtp_password_ref)?;

    // Step 2: build a lettre Envelope wrapping `from_address`, `to_address`,
    //         subject and body. lettre_envelope_pseudo enforces RFC 5322
    //         header escape rules so the subject can't smuggle a CRLF
    //         injection into adjacent headers.
    let envelope: LettreEnvelope = lettre_envelope_pseudo(
        &config.from_address,
        &config.to_address,
        subject,
        markdown_or_html,
    )?;

    // Step 3: connect to the SMTP server and send. smtp_send_pseudo selects
    //         implicit TLS (port 465 when use_tls=true) or STARTTLS (port 587
    //         when use_tls=false) and maps server replies to DeliveryError.
    let outcome: SmtpOutcome = smtp_send_pseudo(
        &config.smtp_host,
        config.smtp_port,
        &config.smtp_user,
        &smtp_password,
        config.use_tls,
        &envelope,
    )?;

    // Step 4: map the SMTP server reply. 535 = auth fail → EmailSmtpFailed.
    //         5.7.x recipient policy reject → EmailRecipientRejected with
    //         the offending address for the UI to surface. 250 = OK.
    match outcome {
        SmtpOutcome::Accepted => Ok(()),
        SmtpOutcome::AuthFailed { code, detail } => {
            Err(DeliveryError::EmailSmtpFailed {
                detail: format!("{code} {detail}"),
            })
        }
        SmtpOutcome::RecipientRejected { address } => {
            Err(DeliveryError::EmailRecipientRejected { address })
        }
        SmtpOutcome::ServerError { code, detail } => {
            Err(DeliveryError::EmailSmtpFailed {
                detail: format!("{code} {detail}"),
            })
        }
    }
}

/// Write the age-encrypted markdown file for one coach review to disk
/// at `markdown_path`. **The canonical on-disk artifact is the
/// `.md.age` ciphertext**, never plaintext `.md` — Stage 2 must call
/// `crypto::age::encrypt(&plaintext, &event_key)` before writing.
/// Plaintext markdown is only ever an in-memory UI view, never disk.
///
/// 中文: 把 age 加密過的 markdown 寫到 `markdown_path`。**落地檔永遠是
/// `.md.age` 密文**，絕不是明文 `.md` — Stage 2 必須先呼
/// `crypto::age::encrypt(&plaintext, &event_key)` 加密再寫。明文 markdown
/// 只有 in-memory UI 顯示時短暫存在，絕不寫 disk。
pub fn write_markdown_file(
    _review_id: &str,
    markdown_path: &Path,
) -> Result<(), DeliveryError> {
    // Step 1: ensure the parent directory exists (~/.phantom-mesh/coach/ by
    //         default). Creating it idempotently means the very first review
    //         on a fresh install still lands somewhere — no panic on missing dir.
    ensure_parent_dir_pseudo(markdown_path)?;

    // Step 2: read the existing ciphertext file. The coach pipeline already
    //         wrote the .md.age artifact before emitting coach.review.ready
    //         (see SPEC-24 §10.2) — the Markdown "channel" here is really a
    //         "make sure the canonical artifact landed + OS-notify" step.
    let cipher_bytes: Vec<u8> = read_file_bytes_pseudo(markdown_path)?;

    // Step 3: confirm what we just read is still an age v1 ciphertext header,
    //         NEVER plaintext. If somebody upstream regressed to writing
    //         plaintext .md, fail loudly here so the leak is caught before
    //         OS notification can copy the snippet anywhere.
    confirm_age_ciphertext_pseudo(&cipher_bytes)?;

    // Step 4: the file is already canonical — no decrypt + rewrite needed.
    //         (Decrypting + re-writing plaintext would defeat the entire
    //         encryption-at-rest invariant.) Caller's responsibility is to
    //         trigger the OS notification with the 80-char snippet, which
    //         happens one layer up in NotificationDispatcher per SPEC-24 §10.2.
    Ok(())
}

/// Check the dedup ledger for an existing `Sent` row for this
/// `(review_id, channel)` pair within the 24-hour dedup window. Returns
/// `Ok(true)` when a sent row exists (caller should skip + write
/// `Suppressed` receipt); `Ok(false)` when no row exists (caller proceeds
/// with actual send).
///
/// 中文: 查 dedup ledger 看 `(review_id, channel)` 在 24 小時 window 內是否
/// 已有 `Sent` 紀錄。`Ok(true)` = 已送過（呼叫端跳過 + 寫 `Suppressed`
/// receipt）；`Ok(false)` = 沒紀錄（呼叫端實際送）。
pub fn dedup_check(
    review_id: &str,
    channel: DeliveryChannel,
) -> Result<bool, DeliveryError> {
    // Step 1: render the channel enum to its snake_case wire string so the
    //         SQL parameter exactly matches what the ledger writer stored
    //         (e.g. "markdown" / "telegram" / "email"). Push is reserved so
    //         it never reaches this path. Shared slug fn with the writer
    //         (`persist_receipts`) so reader + writer can never drift.
    let channel_str: &'static str = delivery_channel_slug(channel)?;

    // Step 2: run a count query against the dedup ledger. SQL is intentionally
    //         a count(*) instead of EXISTS so the bound params stay positional
    //         identical to the prepared-statement cache key Stage 3 will reuse
    //         across all dedup calls in a single review fan-out.
    let sql: &'static str = "SELECT count(*) FROM coach_delivery_ledger \
        WHERE review_id = ? AND channel = ? AND status = 'sent' \
        AND attempted_at_ms > ?";
    let twenty_four_h_ago_ms: u64 = now_ms_pseudo().saturating_sub(24 * 60 * 60 * 1000);
    let count: u64 = sqlite_query_pseudo(
        sql,
        &[review_id, channel_str, &twenty_four_h_ago_ms.to_string()],
    )?;

    // Step 3: any prior Sent row in the window means we suppress this attempt.
    Ok(count > 0)
}

/// Channel → ledger/wire snake_case slug. **Single source of truth** shared by
/// the ledger WRITER ([`persist_receipts`]) and the READER ([`dedup_check`]) so
/// the `channel` column value they compare on can never drift (a drift would
/// silently break dedup — the reader would look for `"telegram"` while the
/// writer stored `"Telegram"`). `Push` is reserved/inactive in v0.6.0 → maps to
/// `ConfigMissing` per the cycle-break rule, so it never reaches the ledger.
fn delivery_channel_slug(channel: DeliveryChannel) -> Result<&'static str, DeliveryError> {
    match channel {
        DeliveryChannel::Markdown => Ok("markdown"),
        DeliveryChannel::Telegram => Ok("telegram"),
        DeliveryChannel::Email => Ok("email"),
        DeliveryChannel::Push => Err(DeliveryError::ConfigMissing {
            channel: "push".to_string(),
        }),
    }
}

/// Status → ledger snake_case slug, matching the `#[serde(rename_all =
/// "snake_case")]` wire form (`"sent"` is the exact value [`dedup_check`]'s
/// `WHERE status = 'sent'` predicate compares on — keep them in lockstep).
fn delivery_status_slug(status: DeliveryStatus) -> &'static str {
    match status {
        DeliveryStatus::Pending => "pending",
        DeliveryStatus::Sent => "sent",
        DeliveryStatus::Failed => "failed",
        DeliveryStatus::Suppressed => "suppressed",
    }
}

/// Persist a batch of [`DeliveryReceipt`]s into the `coach_delivery_ledger`
/// sqlite table — the WRITE half that [`deliver`] deliberately leaves to its
/// caller (see `deliver` step 4). Until this exists the ledger is **write-never**,
/// so [`dedup_check`] can never observe a `Sent` row and the 24-hour dedup
/// window is dead. `INSERT OR REPLACE` keyed on the PK `(review_id, channel,
/// attempted_at_ms)` makes re-persisting the same attempt idempotent.
///
/// Returns the number of rows written. Any rusqlite / I/O failure maps to
/// `DeliveryError::ConfigMissing { channel: "ledger" }` (never panics) so a
/// caller fanning out a review surfaces a clean error rather than aborting. A
/// `Push` receipt (reserved channel) is rejected via [`delivery_channel_slug`].
///
/// 中文: 把一批 `DeliveryReceipt` 寫進 `coach_delivery_ledger`(deliver 刻意
/// 留給呼叫端的「寫」那半 — 沒有它 ledger 永遠是空的、dedup 形同虛設)。以
/// PK `INSERT OR REPLACE` 保證同一次嘗試重寫具冪等性。回傳寫入列數;任何
/// sqlite/I-O 失敗收斂為 `ConfigMissing{channel:"ledger"}`,永不 panic。
pub fn persist_receipts(receipts: &[DeliveryReceipt]) -> Result<usize, DeliveryError> {
    if receipts.is_empty() {
        return Ok(0);
    }
    let ledger_err = || DeliveryError::ConfigMissing {
        channel: "ledger".to_string(),
    };
    let mut conn = open_delivery_ledger_db()?;
    // Wrap the whole batch in one transaction so a mid-batch failure leaves the
    // ledger UNCHANGED (all-or-nothing) instead of partially written. On any
    // `?` below `tx` drops un-committed → automatic rollback.
    let tx = conn.transaction().map_err(|_| ledger_err())?;
    let mut written = 0usize;
    for r in receipts {
        // `Push` is a reserved/inactive channel. `deliver()` emits a *Failed*
        // receipt carrying `channel: Push` for it, which must NOT be persisted —
        // and crucially must NOT abort the batch (else one Push receipt would
        // drop the valid Markdown/Telegram/Email rows alongside it). Skip it.
        let channel_str = match delivery_channel_slug(r.channel) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let status_str = delivery_status_slug(r.status);
        tx.execute(
            "INSERT OR REPLACE INTO coach_delivery_ledger \
             (review_id, channel, attempted_at_ms, status, error_message, retry_count) \
             VALUES (?, ?, ?, ?, ?, 0)",
            rusqlite::params![
                r.review_id,
                channel_str,
                r.attempted_at_ms as i64,
                status_str,
                r.error_message,
            ],
        )
        .map_err(|_| ledger_err())?;
        written += 1;
    }
    tx.commit().map_err(|_| ledger_err())?;
    Ok(written)
}

/// Caller-facing entry point: run [`deliver`] then [`persist_receipts`] in one
/// call so the dedup ledger actually records what was sent. This is the wiring
/// a scheduler / dispatcher uses — calling bare `deliver` leaves the ledger
/// empty, so `Suppressed` could never trigger and the same review would re-send
/// on every fire. Returns the receipts `deliver` produced (the ledger is a
/// side-effect).
///
/// **Best-effort persistence**: the sends in `deliver` have ALREADY happened by
/// the time we persist, so a ledger-write failure must NOT (a) discard the
/// receipts — the caller needs to know which channels were delivered — nor (b)
/// be reported as a delivery failure. The worst case of a dropped ledger write
/// is one possible duplicate on the next fan-out, not a correctness break, so we
/// log a warning and still return the receipts. Callers that need the persist
/// outcome explicitly should call [`deliver`] + [`persist_receipts`] separately.
///
/// 中文: 對外單一入口 — 一次跑完 `deliver` + `persist_receipts`,讓 dedup
/// ledger 真的記下送了什麼(只呼 `deliver` 會讓 ledger 空著、`Suppressed`
/// 永不觸發、同一份 review 每次都重送)。**盡力持久化**:send 早已發生,所以
/// ledger 寫入失敗既不丟棄 receipts、也不算派送失敗(最壞只是下次可能重送一
/// 次),記一筆 warn 後照常回傳 receipts。需要明確得知持久化結果者請改分開呼
/// `deliver` + `persist_receipts`。
pub fn deliver_and_persist(
    review_id: &str,
    markdown_path: &Path,
    channels: &[DeliveryChannel],
) -> Result<Vec<DeliveryReceipt>, DeliveryError> {
    let receipts = deliver(review_id, markdown_path, channels)?;
    if let Err(e) = persist_receipts(&receipts) {
        tracing::warn!(
            review_id,
            error = %e,
            "coach delivery: receipts sent but ledger persist failed \
             (dedup may re-send next cycle)"
        );
    }
    Ok(receipts)
}

// ─── Stage 2 inner pseudocode helpers (Stage 3+4 progressively replace bodies) ─
//
// Per docs/superpowers/SPEC-TO-CODE-PLAYBOOK.md (mirroring rpc_wire.rs Stage 2):
//   Stage 2 = function body shows what it WILL do via comments + nested
//   unimplemented!() inner helpers. A reviewer can audit the algorithm flow
//   without trusting any network / crypto / sqlite implementation. Stage 3
//   commit swaps each `_pseudo` for the real crate call indicated in the
//   panic message hint.
//
// Wave 13 Stage 4 promotions (lettre 0.11 now in core/Cargo.toml):
//   • `lettre_envelope_pseudo` → real `lettre::Message::builder()` with
//     `ContentType::TEXT_PLAIN`; address parse failures surface as
//     `EmailSmtpFailed` (from) / `EmailRecipientRejected` (to) so the
//     receipt carries actionable detail rather than panicking.
//   • `smtp_send_pseudo` → real blocking `lettre::SmtpTransport` selecting
//     `::relay()` (port 465 implicit TLS) or `::starttls_relay()` (port 587
//     STARTTLS). SMTP error string is probed for 535 / 5xx codes to map
//     onto the SPEC-24 §11.1 error catalog.
//
// Still Stage 4 (waiting on cross-wire plumbing):
//   • `vault_read_pseudo` — needs broker_vault_wire's `vault::get()` surface
//     (Stage 2 only exposes seal helpers; secret retrieval isn't wired yet).
//   • `https_post_pseudo` — needs reqwest's `blocking` feature or async refactor.
//
// Phase F-4 bonus Stage 4 promotion (unblocked by F-1 encryption_wire bridge):
//   • `read_md_age_pseudo` — delegates to
//     `crate::encryption_wire::decrypt_raw_age_blob` against the per-process
//     `EventKey` cache; I/O / decrypt / UTF-8 failures all collapse to
//     `DeliveryError::ConfigMissing { channel: "markdown" }` per SPEC-13 §12.1
//     STRIDE-Tampering oracle-leak rule.
//
// Phase F-2 Stage 4 promotions (this wire is now self-contained for ledger
// + settings I/O — no NotificationDispatcher field plumbing required):
//   • `sqlite_query_pseudo` — lazy-opens `~/.phantom-mesh/coach_delivery.sqlite`,
//     runs `CREATE TABLE IF NOT EXISTS coach_delivery_ledger`, then executes
//     the count query (positional `?` params). Mirrors the pattern in
//     `capture_habit_wire::open_habits_db`.
//   • `load_telegram_config_pseudo` / `load_email_config_pseudo` — read
//     `~/.phantom-mesh/coach/delivery_config.toml` (camelCase keys to match
//     the wire `DeliveryConfig` shape), return the per-channel sub-config
//     or `DeliveryError::ConfigMissing` when the file / channel is absent.

/// Parsed shape of the Telegram Bot API response envelope. Only the fields
/// we actually branch on are extracted — Stage 3 picks an actual JSON
/// deserializer crate (serde_json) to populate this.
struct TelegramBotResponse {
    ok: bool,
    /// `parameters.retry_after` in seconds when Telegram returns 429.
    retry_after: Option<u64>,
}

/// Envelope produced by lettre crate. Stage 4 = real `lettre::Message`
/// built via `Message::builder()` with RFC 5322 header escaping enforced
/// by lettre itself (subject CRLF injection rejected at parse time).
#[derive(Debug)]
struct LettreEnvelope {
    message: lettre::Message,
}

/// Result variants returned by the pseudo SMTP send so `send_email` can map
/// them to the exact `DeliveryError` variants the spec requires.
enum SmtpOutcome {
    Accepted,
    AuthFailed { code: u16, detail: String },
    RecipientRejected { address: String },
    ServerError { code: u16, detail: String },
}

fn read_md_age_pseudo(path: &Path) -> Result<String, DeliveryError> {
    let blob = std::fs::read(path).map_err(|_| DeliveryError::ConfigMissing {
        channel: "markdown".into(),
    })?;
    let plaintext = crate::encryption_wire::decrypt_raw_age_blob(&blob).map_err(|_| {
        DeliveryError::ConfigMissing {
            channel: "markdown".into(),
        }
    })?;
    String::from_utf8(plaintext).map_err(|_| DeliveryError::ConfigMissing {
        channel: "markdown".into(),
    })
}

/// Parse a SPEC-15 vault reference `"vault://<service>/<key>"` into its
/// `(service, key)` parts. Anything not matching the scheme (or with an empty
/// component) maps to `ConfigMissing` so the caller surfaces a clean config
/// error rather than attempting a malformed broker GET.
fn parse_vault_ref(ref_str: &str) -> Result<(String, String), DeliveryError> {
    let rest = ref_str
        .strip_prefix("vault://")
        .ok_or_else(|| DeliveryError::ConfigMissing {
            channel: "vault".to_string(),
        })?;
    let (service, key) = rest
        .split_once('/')
        .ok_or_else(|| DeliveryError::ConfigMissing {
            channel: "vault".to_string(),
        })?;
    if service.is_empty() || key.is_empty() {
        return Err(DeliveryError::ConfigMissing {
            channel: "vault".to_string(),
        });
    }
    Ok((service.to_string(), key.to_string()))
}

/// Testable core of the SPEC-15 vault GET: fetch the sealed payload for
/// `service`/`key` from the broker's dumb-storage `/vault/get` endpoint,
/// FAIL CLOSED on a missing/mismatched integrity HMAC (so a malicious or buggy
/// broker can't substitute or replay ciphertext — the binding lives in
/// `service‖key‖sealed‖ts_ms`), then unseal locally with the device seal key.
/// Mirrors `cli_config::config_pull_sealed_lines`' per-item read path; the
/// plaintext lives only in the returned String. Any transport / parse / HMAC /
/// unseal failure maps to `ConfigMissing` — never a panic.
fn vault_get_unseal(
    base_url: &str,
    token: &str,
    seal_key: &crate::broker_vault_wire::VaultSealKey,
    service: &str,
    key: &str,
) -> Result<String, DeliveryError> {
    let cfg_missing = || DeliveryError::ConfigMissing {
        channel: "vault".to_string(),
    };
    let url = format!(
        "{}/{}?service={}&key={}",
        base_url.trim_end_matches('/'),
        crate::broker_vault_wire::BrokerEndpoint::VaultGet.path_slug(),
        service,
        key,
    );
    let auth_header = format!("Bearer {}", token);
    let body = crate::providers_wire::block_on_async(async move {
        let resp = reqwest::Client::new()
            .get(&url)
            .header("Authorization", auth_header)
            .send()
            .await
            .map_err(|_| DeliveryError::ConfigMissing {
                channel: "vault".to_string(),
            })?;
        if !resp.status().is_success() {
            return Err(DeliveryError::ConfigMissing {
                channel: "vault".to_string(),
            });
        }
        Ok::<String, DeliveryError>(resp.text().await.unwrap_or_default())
    })?;

    let v: serde_json::Value = serde_json::from_str(&body).map_err(|_| cfg_missing())?;
    let value_sealed = v
        .get("value_sealed")
        .or_else(|| v.get("valueSealed"))
        .and_then(|x| x.as_str())
        .unwrap_or("");
    let ts_ms = v
        .get("ts_ms")
        .or_else(|| v.get("tsMs"))
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    let server_hmac = v
        .get("server_hmac_hex")
        .or_else(|| v.get("serverHmacHex"))
        .and_then(|x| x.as_str())
        .unwrap_or("");
    if value_sealed.is_empty() || server_hmac.trim().is_empty() {
        return Err(cfg_missing());
    }
    // FAIL CLOSED: re-derive the integrity HMAC locally; mismatch ⇒ refuse.
    let local =
        crate::broker_vault_wire::compute_client_hmac(seal_key, service, key, value_sealed, ts_ms);
    if !local.eq_ignore_ascii_case(server_hmac.trim()) {
        return Err(cfg_missing());
    }
    let plaintext =
        crate::broker_vault_wire::unseal_vault_value(value_sealed, seal_key).map_err(|_| cfg_missing())?;
    String::from_utf8(plaintext).map_err(|_| cfg_missing())
}

/// Stage 4 real impl — resolve a `"vault://service/key"` reference to its
/// plaintext secret via the SPEC-15 broker vault. Loads the broker URL + token
/// from `auth.json` (`phantom login broker`) and the device seal key, then
/// delegates to [`vault_get_unseal`]. A missing login / seal key / ref maps to
/// `ConfigMissing` (the channel surfaces a clean "run phantom login" style
/// error) — the plaintext secret never touches disk, config, or logs.
fn vault_read_pseudo(ref_str: &str) -> Result<String, DeliveryError> {
    let (service, key) = parse_vault_ref(ref_str)?;
    let chan_missing = || DeliveryError::ConfigMissing {
        channel: format!("vault:{}", service),
    };
    let auth = crate::auth::load().ok_or_else(chan_missing)?;
    if auth.broker_url.is_empty() || auth.broker_token.is_empty() {
        return Err(chan_missing());
    }
    let seal_key = crate::broker_vault_wire::load_vault_seal_key().map_err(|_| chan_missing())?;
    vault_get_unseal(&auth.broker_url, &auth.broker_token, &seal_key, &service, &key)
}

/// Stage 4 real impl — POST `body_json` (application/json) to `url` and return
/// `(status_code, response_body)`. reqwest 0.12 has no `blocking` feature in
/// `core/Cargo.toml`, and this wire surface is sync, so we bridge to the async
/// client via the crate-wide `block_on_async` helper (same pattern providers_wire
/// uses) rather than adding a blocking dep or making the whole deliver() chain
/// async. A transport failure (could not reach the server / no response) has no
/// dedicated `DeliveryError` variant, so it maps to the channel's generic
/// `ConfigMissing` bucket — consistent with `send_telegram`'s non-ok handling.
fn https_post_pseudo(url: &str, body_json: &str) -> Result<(u16, String), DeliveryError> {
    let url = url.to_string();
    let body = body_json.to_string();
    crate::providers_wire::block_on_async(async move {
        let resp = reqwest::Client::new()
            .post(&url)
            .header("content-type", "application/json")
            .body(body)
            .send()
            .await
            .map_err(|_| DeliveryError::ConfigMissing {
                channel: "telegram".to_string(),
            })?;
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        Ok((status, text))
    })
}

/// Stage 3 real impl — build the Telegram Bot API `sendMessage` JSON body.
/// `parse_mode` is omitted when `None` (Plain mode); otherwise serialised as
/// the exact string Telegram expects (`Markdown` / `MarkdownV2` / `HTML`).
fn build_telegram_body_pseudo(
    chat_id: &str,
    markdown: &str,
    parse_mode: Option<&'static str>,
) -> String {
    let mut obj = serde_json::Map::new();
    obj.insert(
        "chat_id".to_string(),
        serde_json::Value::String(chat_id.to_string()),
    );
    obj.insert(
        "text".to_string(),
        serde_json::Value::String(markdown.to_string()),
    );
    if let Some(pm) = parse_mode {
        obj.insert(
            "parse_mode".to_string(),
            serde_json::Value::String(pm.to_string()),
        );
    }
    serde_json::Value::Object(obj).to_string()
}

/// Stage 3 real impl — parse the Telegram Bot API response envelope. Extracts
/// `ok` (top-level bool) + `parameters.retry_after` (seconds, when present on
/// HTTP 429). Malformed JSON maps to `DeliveryError::ConfigMissing { channel:
/// "telegram" }` so the caller surfaces a clean config error rather than panic.
fn parse_telegram_response_pseudo(
    body: &str,
) -> Result<TelegramBotResponse, DeliveryError> {
    let v: serde_json::Value =
        serde_json::from_str(body).map_err(|_| DeliveryError::ConfigMissing {
            channel: "telegram".to_string(),
        })?;
    let ok = v.get("ok").and_then(|x| x.as_bool()).unwrap_or(false);
    let retry_after = v
        .get("parameters")
        .and_then(|p| p.get("retry_after"))
        .and_then(|r| r.as_u64());
    Ok(TelegramBotResponse { ok, retry_after })
}

/// Stage 4 real impl — build a `lettre::Message` (RFC 5322 envelope) from
/// the user-supplied addresses + subject + body. lettre's `.parse()` on
/// addresses rejects malformed RFC 5322 mailboxes; `.subject()` rejects
/// embedded CRLF so a hostile review body cannot smuggle a header
/// injection into the outgoing email. `ContentType::TEXT_PLAIN` keeps the
/// markdown body shipped as-is (no HTML rendering — the recipient mail
/// client decides how to display markdown text).
///
/// Any parse / build failure maps to `EmailSmtpFailed { detail }` so the
/// caller's receipt carries a clean reason string rather than panicking.
fn lettre_envelope_pseudo(
    from: &str,
    to: &str,
    subject: &str,
    body: &str,
) -> Result<LettreEnvelope, DeliveryError> {
    use lettre::message::header::ContentType;
    use lettre::message::Message;

    let from_addr: lettre::message::Mailbox =
        from.parse().map_err(|e: lettre::address::AddressError| {
            DeliveryError::EmailSmtpFailed {
                detail: format!("invalid from address: {e}"),
            }
        })?;
    let to_addr: lettre::message::Mailbox =
        to.parse().map_err(|_: lettre::address::AddressError| {
            DeliveryError::EmailRecipientRejected {
                address: to.to_string(),
            }
        })?;
    let message = Message::builder()
        .from(from_addr)
        .to(to_addr)
        .subject(subject)
        .header(ContentType::TEXT_PLAIN)
        .body(body.to_string())
        .map_err(|e| DeliveryError::EmailSmtpFailed {
            detail: format!("message build failed: {e}"),
        })?;
    Ok(LettreEnvelope { message })
}

/// Stage 4 real impl — connect to the user-owned SMTP server and send the
/// envelope via lettre's blocking `SmtpTransport`. Picks implicit TLS
/// (port 465, `use_tls=true`) via `::relay()` or STARTTLS (port 587,
/// `use_tls=false`) via `::starttls_relay()`. Credentials are passed as
/// `(user, password)` — lettre takes ownership of the password string at
/// the boundary and zeroizes it inside its internal `Credentials` type.
///
/// Outcome mapping: lettre's `transport::smtp::Error` carries an optional
/// SMTP response code; we map 535 (auth failed) to `AuthFailed`, 550 /
/// 553 (recipient policy reject) to `RecipientRejected`, and any other
/// 4xx / 5xx / transport error to `ServerError` with the upstream message
/// preserved so the UI receipt has a debuggable detail string.
fn smtp_send_pseudo(
    host: &str,
    port: u16,
    user: &str,
    password: &str,
    use_tls: bool,
    envelope: &LettreEnvelope,
) -> Result<SmtpOutcome, DeliveryError> {
    use lettre::transport::smtp::authentication::Credentials;
    use lettre::{SmtpTransport, Transport};

    let creds = Credentials::new(user.to_string(), password.to_string());

    // Pick implicit TLS (relay = wrapped TLS on port 465) vs STARTTLS
    // (relay upgrades plaintext on port 587 — lettre's default for
    // `starttls_relay`). Both arms produce the same builder type so the
    // final `.build()` call is shared below.
    let transport_builder = if use_tls {
        SmtpTransport::relay(host)
    } else {
        SmtpTransport::starttls_relay(host)
    }
    .map_err(|e| DeliveryError::EmailSmtpFailed {
        detail: format!("smtp relay setup failed: {e}"),
    })?;

    let transport = transport_builder
        .port(port)
        .credentials(creds)
        .build();

    match transport.send(&envelope.message) {
        Ok(_response) => Ok(SmtpOutcome::Accepted),
        Err(e) => {
            // lettre's smtp Error exposes a `status()` method that returns
            // the SMTP response code when the server returned one. Mapping
            // is best-effort — when the code is absent (transport-level
            // failure: DNS, TLS handshake, connection refused) we surface
            // it as a generic ServerError with the upstream string.
            let detail = e.to_string();
            // Inspect the upstream error string for known SMTP response
            // code prefixes. lettre exposes `e.status()` but only on the
            // `transport::smtp::Error` concrete type — the `to_string()`
            // shape is the most-stable cross-version probe.
            if detail.contains("535") || detail.to_ascii_lowercase().contains("authentication") {
                Ok(SmtpOutcome::AuthFailed {
                    code: 535,
                    detail,
                })
            } else if detail.contains("550") || detail.contains("553") || detail.contains("5.7.") {
                Ok(SmtpOutcome::RecipientRejected {
                    // Best-effort: surface the configured to_address from
                    // the envelope rather than parsing the SMTP reply.
                    address: envelope
                        .message
                        .envelope()
                        .to()
                        .first()
                        .map(|a| a.to_string())
                        .unwrap_or_else(|| "<unknown>".to_string()),
                })
            } else {
                Ok(SmtpOutcome::ServerError { code: 0, detail })
            }
        }
    }
}

/// Stage 3 real impl — create the parent directory tree for `path` if missing.
/// Idempotent (`create_dir_all`); I/O errors (permission / read-only fs) map
/// to `DeliveryError::ConfigMissing { channel: "markdown" }`.
fn ensure_parent_dir_pseudo(path: &Path) -> Result<(), DeliveryError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|_| DeliveryError::ConfigMissing {
                channel: "markdown".to_string(),
            })?;
        }
    }
    Ok(())
}

/// Stage 3 real impl — read the raw bytes of `path` from disk. I/O errors
/// (missing file / permission denied) map to
/// `DeliveryError::ConfigMissing { channel: "markdown" }` so the caller
/// surfaces a clean config error instead of panicking.
fn read_file_bytes_pseudo(path: &Path) -> Result<Vec<u8>, DeliveryError> {
    std::fs::read(path).map_err(|_| DeliveryError::ConfigMissing {
        channel: "markdown".to_string(),
    })
}

/// Stage 3 real impl — assert `bytes` starts with the age v1 magic header
/// (binary `"age-encryption.org/v1\n"` or armored `"-----BEGIN AGE"`).
/// Fails loud as `DeliveryError::ConfigMissing { channel: "markdown" }`
/// when plaintext markdown is detected on disk — this is the regression
/// trip-wire that catches the "someone wrote `.md` instead of `.md.age`"
/// leak before the OS notification copies a snippet anywhere.
fn confirm_age_ciphertext_pseudo(bytes: &[u8]) -> Result<(), DeliveryError> {
    const AGE_BINARY_MAGIC: &[u8] = b"age-encryption.org/v1\n";
    const AGE_ARMOR_MAGIC: &[u8] = b"-----BEGIN AGE";
    if bytes.starts_with(AGE_BINARY_MAGIC) || bytes.starts_with(AGE_ARMOR_MAGIC) {
        Ok(())
    } else {
        Err(DeliveryError::ConfigMissing {
            channel: "markdown".to_string(),
        })
    }
}

/// Stage 4 real impl — run a single-row `count(*)` query against the
/// `coach_delivery_ledger` sqlite table at `~/.phantom-mesh/coach_delivery.sqlite`.
/// Lazy-opens the connection (creating the file + schema on first call) so the
/// wire stays self-contained — no external connection handle is plumbed through
/// the call chain. Mirrors the pattern in `capture_habit_wire::open_habits_db`.
///
/// The `$PHANTOM_MESH_COACH_LEDGER_DIR` environment variable is honoured as a
/// test override (the integration tests point it at a `tempdir` so the
/// production ledger at `~/.phantom-mesh/` is never touched).
///
/// Any rusqlite / I/O failure maps to `DeliveryError::ConfigMissing { channel:
/// "ledger" }` — never panic — so `dedup_check`'s caller surfaces a clean
/// receipt error rather than aborting the whole fan-out.
fn sqlite_query_pseudo(
    sql: &str,
    params: &[&str],
) -> Result<u64, DeliveryError> {
    let conn = open_delivery_ledger_db()?;
    let count: i64 = conn
        .query_row(
            sql,
            rusqlite::params_from_iter(params.iter().copied()),
            |row| row.get(0),
        )
        .map_err(|_| DeliveryError::ConfigMissing {
            channel: "ledger".to_string(),
        })?;
    Ok(count.max(0) as u64)
}

/// Lazy-open the coach delivery ledger sqlite database. Resolves the parent
/// directory from `$PHANTOM_MESH_COACH_LEDGER_DIR` (test hook) or
/// `dirs::home_dir()/.phantom-mesh/` (production), creates the directory tree
/// idempotently, opens / creates `coach_delivery.sqlite`, and ensures the
/// `coach_delivery_ledger` table exists (SPEC-24 §7.4 DDL).
fn open_delivery_ledger_db() -> Result<rusqlite::Connection, DeliveryError> {
    let parent = ledger_parent_dir().ok_or_else(|| DeliveryError::ConfigMissing {
        channel: "ledger".to_string(),
    })?;
    std::fs::create_dir_all(&parent).map_err(|_| DeliveryError::ConfigMissing {
        channel: "ledger".to_string(),
    })?;
    let db_path = parent.join("coach_delivery.sqlite");
    let conn = rusqlite::Connection::open(&db_path).map_err(|_| DeliveryError::ConfigMissing {
        channel: "ledger".to_string(),
    })?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS coach_delivery_ledger (\
            review_id TEXT NOT NULL, \
            channel TEXT NOT NULL, \
            attempted_at_ms INTEGER NOT NULL, \
            status TEXT NOT NULL, \
            error_message TEXT, \
            retry_count INTEGER NOT NULL DEFAULT 0, \
            PRIMARY KEY (review_id, channel, attempted_at_ms)\
         );\
         CREATE INDEX IF NOT EXISTS idx_coach_delivery_ledger_lookup \
            ON coach_delivery_ledger(review_id, channel, status, attempted_at_ms);",
    )
    .map_err(|_| DeliveryError::ConfigMissing {
        channel: "ledger".to_string(),
    })?;
    Ok(conn)
}

/// Resolve the directory that holds `coach_delivery.sqlite` +
/// `coach/delivery_config.toml`. Honours `$PHANTOM_MESH_COACH_LEDGER_DIR` for
/// test isolation; otherwise falls back to `~/.phantom-mesh/`.
fn ledger_parent_dir() -> Option<std::path::PathBuf> {
    if let Ok(override_dir) = std::env::var("PHANTOM_MESH_COACH_LEDGER_DIR") {
        if !override_dir.is_empty() {
            return Some(std::path::PathBuf::from(override_dir));
        }
    }
    crate::cli_config::phantom_data_dir().ok()
}

/// Stage 4 real impl — load `DeliveryConfig.telegram_config` from
/// `~/.phantom-mesh/coach/delivery_config.toml`. Returns `ConfigMissing` when
/// either (a) the settings file is absent / unreadable / malformed or
/// (b) the file parses but `telegramConfig` is `null` (channel disabled).
fn load_telegram_config_pseudo() -> Result<TelegramConfig, DeliveryError> {
    let cfg = read_delivery_config_file()?;
    cfg.telegram_config.ok_or_else(|| DeliveryError::ConfigMissing {
        channel: "telegram".to_string(),
    })
}

/// Stage 4 real impl — load `DeliveryConfig.email_config` from
/// `~/.phantom-mesh/coach/delivery_config.toml`. Same error contract as
/// `load_telegram_config_pseudo` above.
fn load_email_config_pseudo() -> Result<EmailConfig, DeliveryError> {
    let cfg = read_delivery_config_file()?;
    cfg.email_config.ok_or_else(|| DeliveryError::ConfigMissing {
        channel: "email".to_string(),
    })
}

/// Read + parse `~/.phantom-mesh/coach/delivery_config.toml` (or the
/// `$PHANTOM_MESH_COACH_LEDGER_DIR/coach/delivery_config.toml` test override)
/// into a `DeliveryConfig`. The file uses camelCase keys to match the wire
/// (`markdownEnabled` / `telegramConfig` / `emailConfig` / `pushEnabled`).
fn read_delivery_config_file() -> Result<DeliveryConfig, DeliveryError> {
    let parent = ledger_parent_dir().ok_or_else(|| DeliveryError::ConfigMissing {
        channel: "settings".to_string(),
    })?;
    let path = parent.join("coach").join("delivery_config.toml");
    let raw = std::fs::read_to_string(&path).map_err(|_| DeliveryError::ConfigMissing {
        channel: "settings".to_string(),
    })?;
    toml::from_str::<DeliveryConfig>(&raw).map_err(|_| DeliveryError::ConfigMissing {
        channel: "settings".to_string(),
    })
}

/// Stage 3 real impl — current Unix epoch milliseconds. `SystemTime` is the
/// canonical wall-clock source per SPEC-24 §7.4 ledger DDL. The
/// `.duration_since` call only fails when the system clock is before
/// `UNIX_EPOCH` (impossible on any sane machine); we surface 0 in that
/// degenerate case rather than panicking so the function stays infallible.
fn now_ms_pseudo() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_post_pseudo_returns_status_and_body() {
        // Stage 4 https_post_pseudo bridges the sync wire to the async reqwest
        // client via block_on_async. Drive a wiremock server from a setup
        // runtime, then call the sync helper from this (non-runtime) test thread
        // so block_on_async spins its own runtime — exercising the real POST.
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let rt = tokio::runtime::Runtime::new().unwrap();
        let mock = rt.block_on(MockServer::start());
        rt.block_on(
            Mock::given(method("POST"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_string(r#"{"ok":true,"result":{}}"#),
                )
                .mount(&mock),
        );

        let url = format!("{}/bot123/sendMessage", mock.uri());
        let (status, body) =
            https_post_pseudo(&url, r#"{"chat_id":"42","text":"hi"}"#).expect("post should succeed");
        assert_eq!(status, 200);
        assert!(body.contains("\"ok\":true"), "got: {body}");

        // Unreachable host → transport error maps to the channel's generic
        // ConfigMissing bucket (no dedicated transport variant), never a panic.
        let err = https_post_pseudo("http://127.0.0.1:1/bot/x", "{}")
            .expect_err("connection refused must be an error");
        assert!(matches!(err, DeliveryError::ConfigMissing { .. }));
    }

    #[test]
    fn vault_get_unseal_round_trips_and_fails_closed_on_hmac() {
        // Seal a secret with a test seal key, stand up a mock broker that echoes
        // the sealed payload + a valid integrity HMAC, and confirm vault_get_unseal
        // recovers the exact plaintext. Then confirm a key the HMAC wasn't bound to
        // is rejected (fail-closed) — proving the tamper/replay guard.
        use crate::broker_vault_wire::{compute_client_hmac, generate_vault_seal_key, seal_vault_value};
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let seal_key = generate_vault_seal_key();
        let secret = "123456:AA-real-bot-token";
        let sealed = seal_vault_value(secret.as_bytes(), &seal_key).expect("seal");
        let ts_ms: u64 = 1_700_000_000_000;
        let hmac = compute_client_hmac(&seal_key, "telegram", "bot_token", &sealed, ts_ms);

        let rt = tokio::runtime::Runtime::new().unwrap();
        let mock = rt.block_on(MockServer::start());
        let body = serde_json::json!({
            "value_sealed": sealed,
            "ts_ms": ts_ms,
            "server_hmac_hex": hmac,
        })
        .to_string();
        rt.block_on(
            Mock::given(method("GET"))
                .and(path("/vault/get"))
                .respond_with(ResponseTemplate::new(200).set_body_string(body))
                .mount(&mock),
        );

        // Correct service/key → plaintext recovered.
        let got = vault_get_unseal(&mock.uri(), "tok", &seal_key, "telegram", "bot_token")
            .expect("round-trip should recover the secret");
        assert_eq!(got, secret);

        // Same payload, but the HMAC was bound to "bot_token" — reading it as a
        // different key recomputes a non-matching HMAC ⇒ fail closed.
        let tampered = vault_get_unseal(&mock.uri(), "tok", &seal_key, "telegram", "other_key")
            .expect_err("HMAC bound to a different key must be rejected");
        assert!(matches!(tampered, DeliveryError::ConfigMissing { .. }));
    }

    #[test]
    fn delivery_receipt_round_trip_smoke() {
        // §7.2 invariant: DeliveryReceipt survives a Rust → JSON → Rust
        // round-trip byte-identical so the sqlite ledger writer (Rust) and
        // the settings UI history view (TS) agree on the exact shape. Any
        // field rename here is a wire-break that ledger queries will silently
        // mismatch on — so we lock the 5-key schema explicitly.
        let r = DeliveryReceipt {
            review_id: "01923f9c-1b42-7000-9d8a-3c4e2f1a8b75".to_string(),
            channel: DeliveryChannel::Telegram,
            attempted_at_ms: 1_716_563_400_000,
            status: DeliveryStatus::Sent,
            error_message: None,
        };
        let j = serde_json::to_string(&r).unwrap();
        let back: DeliveryReceipt = serde_json::from_str(&j).unwrap();
        assert_eq!(r.review_id, back.review_id);
        assert_eq!(r.channel, back.channel);
        assert_eq!(r.attempted_at_ms, back.attempted_at_ms);
        assert_eq!(r.status, back.status);
        assert_eq!(r.error_message, back.error_message);

        let v: serde_json::Value = serde_json::from_str(&j).unwrap();
        let obj = v.as_object().expect("receipt is JSON object");
        assert_eq!(
            obj.len(),
            5,
            "DeliveryReceipt must stay 5 fields; got keys: {:?}",
            obj.keys().collect::<Vec<_>>()
        );
        assert!(obj.contains_key("reviewId"));
        assert!(obj.contains_key("channel"));
        assert!(obj.contains_key("attemptedAtMs"));
        assert!(obj.contains_key("status"));
        assert!(obj.contains_key("errorMessage"));
    }

    #[test]
    fn delivery_channel_serializes_three_active_variants() {
        // §7.1 invariant: the 3 active v0.6.0 channels serialize to the
        // exact snake_case strings the UI + ledger column compare on.
        // Push variant is `#[serde(skip)]` so attempting to serialize it
        // would fail — we only test the 3 active ones.
        let j = serde_json::to_string(&DeliveryChannel::Markdown).unwrap();
        assert_eq!(j, "\"markdown\"");
        let j = serde_json::to_string(&DeliveryChannel::Telegram).unwrap();
        assert_eq!(j, "\"telegram\"");
        let j = serde_json::to_string(&DeliveryChannel::Email).unwrap();
        assert_eq!(j, "\"email\"");

        // Round-trip back from JSON to verify deserialize agrees.
        let m: DeliveryChannel = serde_json::from_str("\"markdown\"").unwrap();
        assert_eq!(m, DeliveryChannel::Markdown);
        let t: DeliveryChannel = serde_json::from_str("\"telegram\"").unwrap();
        assert_eq!(t, DeliveryChannel::Telegram);
        let e: DeliveryChannel = serde_json::from_str("\"email\"").unwrap();
        assert_eq!(e, DeliveryChannel::Email);
    }

    #[test]
    fn delivery_status_serializes_snake_case() {
        // §7.1 invariant: status strings on the wire must stay snake_case
        // so the sqlite `status` column value comparison stays stable.
        assert_eq!(
            serde_json::to_string(&DeliveryStatus::Pending).unwrap(),
            "\"pending\""
        );
        assert_eq!(
            serde_json::to_string(&DeliveryStatus::Sent).unwrap(),
            "\"sent\""
        );
        assert_eq!(
            serde_json::to_string(&DeliveryStatus::Failed).unwrap(),
            "\"failed\""
        );
        assert_eq!(
            serde_json::to_string(&DeliveryStatus::Suppressed).unwrap(),
            "\"suppressed\""
        );
    }

    #[test]
    fn delivery_error_serializes_with_code_tag() {
        // §11.1 invariant: error wire shape uses `{"code": "..."}` tag so
        // the UI can `switch (err.code)`. Verify a couple of variants
        // round-trip cleanly with the right tag.
        let e = DeliveryError::TelegramBotTokenInvalid;
        let j = serde_json::to_string(&e).unwrap();
        assert!(
            j.contains("telegram_bot_token_invalid"),
            "wire shape: {}",
            j
        );

        let e2 = DeliveryError::EmailRecipientRejected {
            address: "user@example.com".to_string(),
        };
        let j2 = serde_json::to_string(&e2).unwrap();
        assert!(j2.contains("email_recipient_rejected"), "wire shape: {}", j2);
        assert!(j2.contains("user@example.com"), "payload preserved: {}", j2);
    }

    #[test]
    fn telegram_config_keeps_token_ref_not_plaintext() {
        // SPEC-15 invariant: TelegramConfig field is `bot_token_ref` (a
        // vault reference), NOT a plaintext token. Serialized wire shape
        // must surface `botTokenRef` so any reviewer scanning settings JSON
        // immediately sees "this is a reference, not the token itself".
        let c = TelegramConfig {
            bot_token_ref: "vault://telegram/bot_token".to_string(),
            chat_id: "123456789".to_string(),
            parse_mode: TelegramParseMode::MarkdownV2,
        };
        let j = serde_json::to_string(&c).unwrap();
        assert!(j.contains("botTokenRef"), "must surface ref field: {}", j);
        assert!(
            j.contains("vault://telegram/bot_token"),
            "ref string preserved: {}",
            j
        );
        // Sanity: ensure no leaked field name suggesting plaintext token.
        assert!(
            !j.contains("\"botToken\":") && !j.contains("\"token\":"),
            "must NOT have plaintext token field: {}",
            j
        );
    }

    #[test]
    fn email_config_keeps_password_ref_not_plaintext() {
        // SPEC-15 invariant: EmailConfig field is `smtp_password_ref`, NOT
        // a plaintext password. Same reasoning as TelegramConfig above.
        let c = EmailConfig {
            smtp_host: "smtp.example.com".to_string(),
            smtp_port: 587,
            smtp_user: "user@example.com".to_string(),
            smtp_password_ref: "vault://email/smtp_pass".to_string(),
            from_address: "user@example.com".to_string(),
            to_address: "user@example.com".to_string(),
            use_tls: false,
        };
        let j = serde_json::to_string(&c).unwrap();
        assert!(
            j.contains("smtpPasswordRef"),
            "must surface ref field: {}",
            j
        );
        assert!(
            j.contains("vault://email/smtp_pass"),
            "ref string preserved: {}",
            j
        );
        assert!(
            !j.contains("\"smtpPass\":") && !j.contains("\"password\":"),
            "must NOT have plaintext password field: {}",
            j
        );
    }

    #[test]
    fn delivery_config_push_enabled_defaults_false() {
        // Cycle-break invariant: Push channel is reserved for v0.6.x — in
        // v0.6.0 `push_enabled` must default false when missing from JSON.
        // Stage 2 `deliver()` will reject any Push channel as ConfigMissing.
        let j = r#"{
            "markdownEnabled": true,
            "telegramConfig": null,
            "emailConfig": null
        }"#;
        let c: DeliveryConfig = serde_json::from_str(j).unwrap();
        assert!(c.markdown_enabled);
        assert!(c.telegram_config.is_none());
        assert!(c.email_config.is_none());
        assert!(
            !c.push_enabled,
            "push_enabled must default false in v0.6.0 (reserved variant)"
        );
    }

    // ─── Stage 4 KAT — sqlite ledger + TOML settings reader ──────────────

    /// Process-wide mutex serialising the ledger + settings tests. Each test
    /// mutates `$PHANTOM_MESH_COACH_LEDGER_DIR`, and `std::env::set_var` is a
    /// global on the process — cargo's default parallel test runner would
    /// race without this guard. Helpers return the held lock so callers keep
    /// it for the entire test body.
    static LEDGER_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Per-test scratch directory used as the `$PHANTOM_MESH_COACH_LEDGER_DIR`
    /// override so the ledger + settings tests never touch the real
    /// `~/.phantom-mesh/` tree. Returns the held env-lock guard alongside
    /// the path; the guard must live for the rest of the test body so other
    /// parallel tests can't flip the env var underneath.
    fn fresh_ledger_dir(
        tag: &str,
    ) -> (std::sync::MutexGuard<'static, ()>, std::path::PathBuf) {
        let guard = LEDGER_ENV_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let dir = std::env::temp_dir().join(format!(
            "phantom-coach-{}-{}-{}",
            tag,
            std::process::id(),
            now_ms_pseudo()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch ledger dir");
        std::env::set_var("PHANTOM_MESH_COACH_LEDGER_DIR", &dir);
        (guard, dir)
    }

    #[test]
    fn dedup_check_returns_false_on_empty_ledger() {
        // Fresh ledger DB has no rows so dedup_check must report "not yet
        // sent" → caller proceeds with the real send. `ConfigMissing` from
        // a leaked production path would also surface here.
        let (_lock, _scratch) = fresh_ledger_dir("dedup-empty");
        let r = dedup_check(
            "01923f9c-1b42-7000-9d8a-3c4e2f1a8b75",
            DeliveryChannel::Markdown,
        );
        assert!(matches!(r, Ok(false)), "empty ledger must yield Ok(false), got {r:?}");
    }

    #[test]
    fn dedup_check_returns_true_when_recent_sent_row_present() {
        // Insert a `Sent` row inside the 24-hour window and verify
        // dedup_check sees it. Uses the same lazy-open + schema migrate
        // path as the production helper.
        let (_lock, _scratch) = fresh_ledger_dir("dedup-hit");
        let now_ms = now_ms_pseudo();
        let conn = open_delivery_ledger_db().expect("open scratch ledger");
        conn.execute(
            "INSERT INTO coach_delivery_ledger \
             (review_id, channel, attempted_at_ms, status, error_message, retry_count) \
             VALUES (?1, ?2, ?3, 'sent', NULL, 0)",
            rusqlite::params!["review-abc", "telegram", now_ms as i64],
        )
        .expect("insert recent sent row");
        drop(conn);

        let r = dedup_check("review-abc", DeliveryChannel::Telegram);
        assert!(matches!(r, Ok(true)), "recent sent row must yield Ok(true), got {r:?}");
    }

    #[test]
    fn load_telegram_config_reads_camel_case_toml() {
        // Settings file uses camelCase keys to match the wire DeliveryConfig
        // shape — verify the loader pulls TelegramConfig back round-trip.
        let (_lock, scratch) = fresh_ledger_dir("load-telegram");
        let coach_dir = scratch.join("coach");
        std::fs::create_dir_all(&coach_dir).unwrap();
        std::fs::write(
            coach_dir.join("delivery_config.toml"),
            r#"
markdownEnabled = true
pushEnabled = false

[telegramConfig]
botTokenRef = "vault://telegram/bot_token"
chatId = "123456789"
parseMode = "markdown_v2"
"#,
        )
        .unwrap();

        let cfg = load_telegram_config_pseudo().expect("telegram config loads");
        assert_eq!(cfg.bot_token_ref, "vault://telegram/bot_token");
        assert_eq!(cfg.chat_id, "123456789");
        assert_eq!(cfg.parse_mode, TelegramParseMode::MarkdownV2);
    }

    #[test]
    fn load_email_config_reads_camel_case_toml() {
        // Symmetric to the telegram loader test, including the smtpPasswordRef
        // round-trip so the vault ref discipline carries through the toml read.
        let (_lock, scratch) = fresh_ledger_dir("load-email");
        let coach_dir = scratch.join("coach");
        std::fs::create_dir_all(&coach_dir).unwrap();
        std::fs::write(
            coach_dir.join("delivery_config.toml"),
            r#"
markdownEnabled = true
pushEnabled = false

[emailConfig]
smtpHost = "smtp.example.com"
smtpPort = 587
smtpUser = "user42@example.com"
smtpPasswordRef = "vault://email/smtp_pass"
fromAddress = "user42@example.com"
toAddress = "user42@example.com"
useTls = false
"#,
        )
        .unwrap();

        let cfg = load_email_config_pseudo().expect("email config loads");
        assert_eq!(cfg.smtp_host, "smtp.example.com");
        assert_eq!(cfg.smtp_port, 587);
        assert_eq!(cfg.smtp_user, "user42@example.com");
        assert_eq!(cfg.smtp_password_ref, "vault://email/smtp_pass");
        assert!(!cfg.use_tls);
    }

    #[test]
    fn load_telegram_config_missing_file_maps_to_config_missing() {
        // No delivery_config.toml at all → ConfigMissing so the caller's
        // receipt carries a clean settings error instead of a panic.
        let (_lock, _scratch) = fresh_ledger_dir("load-telegram-missing");
        let r = load_telegram_config_pseudo();
        assert!(
            matches!(r, Err(DeliveryError::ConfigMissing { .. })),
            "missing settings file must map to ConfigMissing, got {r:?}"
        );
    }

    #[test]
    fn load_email_config_section_absent_maps_to_config_missing() {
        // Settings file exists but `emailConfig` is omitted (channel
        // disabled) → ConfigMissing{channel:"email"} so the dispatcher
        // surfaces the right per-channel error.
        let (_lock, scratch) = fresh_ledger_dir("load-email-absent");
        let coach_dir = scratch.join("coach");
        std::fs::create_dir_all(&coach_dir).unwrap();
        std::fs::write(
            coach_dir.join("delivery_config.toml"),
            "markdownEnabled = true\npushEnabled = false\n",
        )
        .unwrap();

        let r = load_email_config_pseudo();
        assert!(
            matches!(r, Err(DeliveryError::ConfigMissing { ref channel }) if channel == "email"),
            "absent emailConfig must map to ConfigMissing{{channel:\"email\"}}, got {r:?}"
        );
    }

    // ─── Stage 3 KAT (known-answer-test) vectors ─────────────────────────

    #[test]
    fn build_telegram_body_includes_parse_mode_when_set() {
        // §7.1 invariant: Telegram Bot API JSON body MUST carry chat_id +
        // text + parse_mode (when not Plain). Pin the exact JSON shape so
        // the wire is byte-stable across builds.
        let body = build_telegram_body_pseudo(
            "123456789",
            "hello *world*",
            Some("MarkdownV2"),
        );
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v.get("chat_id").and_then(|x| x.as_str()), Some("123456789"));
        assert_eq!(v.get("text").and_then(|x| x.as_str()), Some("hello *world*"));
        assert_eq!(
            v.get("parse_mode").and_then(|x| x.as_str()),
            Some("MarkdownV2")
        );
    }

    #[test]
    fn build_telegram_body_omits_parse_mode_when_plain() {
        // Plain mode (`None`) must NOT include the `parse_mode` field at
        // all — Telegram interprets its presence as a directive.
        let body = build_telegram_body_pseudo("321", "raw text", None);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(
            v.get("parse_mode").is_none(),
            "parse_mode must be absent in Plain mode: {}",
            body
        );
        assert_eq!(v.get("text").and_then(|x| x.as_str()), Some("raw text"));
    }

    #[test]
    fn parse_telegram_response_extracts_ok_and_retry_after() {
        // Telegram 429 envelope shape: `{"ok": false, "parameters": {"retry_after": 7}}`.
        let body = r#"{"ok":false,"parameters":{"retry_after":7}}"#;
        let parsed = parse_telegram_response_pseudo(body).unwrap();
        assert!(!parsed.ok);
        assert_eq!(parsed.retry_after, Some(7));
    }

    #[test]
    fn parse_telegram_response_handles_success_shape() {
        // Telegram 200 envelope shape: `{"ok": true, "result": {...}}` — no
        // retry_after, ok=true.
        let body = r#"{"ok":true,"result":{"message_id":42}}"#;
        let parsed = parse_telegram_response_pseudo(body).unwrap();
        assert!(parsed.ok);
        assert_eq!(parsed.retry_after, None);
    }

    #[test]
    fn parse_telegram_response_rejects_malformed_json() {
        // Malformed JSON must map to ConfigMissing{channel:"telegram"} so
        // the caller's receipt surfaces a clean config error rather than
        // a parse panic crashing the fan-out.
        let r = parse_telegram_response_pseudo("not json at all");
        assert!(matches!(r, Err(DeliveryError::ConfigMissing { .. })));
    }

    #[test]
    fn confirm_age_ciphertext_accepts_binary_magic() {
        // Real age v1 binary ciphertext starts with the canonical magic
        // string. The confirm helper must pass this byte sequence so
        // legitimate review writes are not falsely rejected as plaintext.
        let bytes = b"age-encryption.org/v1\nsome cipher bytes here";
        assert!(confirm_age_ciphertext_pseudo(bytes).is_ok());
    }

    #[test]
    fn confirm_age_ciphertext_accepts_armored_magic() {
        // ASCII-armored age ciphertext is also valid (e.g. when the
        // markdown writer was invoked with the armor option).
        let bytes = b"-----BEGIN AGE ENCRYPTED FILE-----\nbase64...";
        assert!(confirm_age_ciphertext_pseudo(bytes).is_ok());
    }

    #[test]
    fn confirm_age_ciphertext_rejects_plaintext() {
        // Plaintext markdown leak regression trip-wire: a `.md` file ending
        // up at the `.md.age` path must be rejected so the OS notification
        // never gets a chance to copy the plaintext snippet anywhere.
        let bytes = b"# Daily review\n\nplaintext leak";
        let r = confirm_age_ciphertext_pseudo(bytes);
        assert!(matches!(r, Err(DeliveryError::ConfigMissing { .. })));
    }

    #[test]
    fn now_ms_pseudo_returns_post_2026_epoch() {
        // SPEC-24 §7.4 ledger DDL pins wall-clock milliseconds. Sanity-check
        // the helper returns a plausible value (≥ 2026-01-01 00:00 UTC)
        // rather than a clock-skew zero or a panic.
        let t = now_ms_pseudo();
        // 2026-01-01T00:00:00Z = 1767225600000 ms
        assert!(t > 1_767_225_600_000, "expected post-2026 epoch ms, got {}", t);
    }

    #[test]
    fn ensure_parent_dir_creates_missing_tree() {
        // Idempotent mkdir -p so a fresh install survives the very first
        // review write. Uses a tmpdir under cargo's target/test scratch.
        let tmp_root = std::env::temp_dir().join(format!(
            "phantom-coach-test-{}",
            now_ms_pseudo()
        ));
        let nested = tmp_root.join("a").join("b").join("c").join("review.md.age");
        ensure_parent_dir_pseudo(&nested).unwrap();
        assert!(nested.parent().unwrap().exists(), "parent dir not created");
        let _ = std::fs::remove_dir_all(&tmp_root);
    }

    #[test]
    fn read_file_bytes_missing_path_maps_to_config_missing() {
        // I/O fail (missing file) must surface as DeliveryError::ConfigMissing
        // {channel:"markdown"} — never panic. This is what the caller
        // depends on to attach a clean "review file missing" message
        // to the failed receipt.
        let r = read_file_bytes_pseudo(Path::new("/definitely/does/not/exist.md.age"));
        assert!(matches!(r, Err(DeliveryError::ConfigMissing { .. })));
    }

    #[test]
    fn read_md_age_missing_path_maps_to_config_missing() {
        // Phase F-4 bonus close — read_md_age_pseudo must not panic when the
        // file is absent; per SPEC-13 §12.1 STRIDE-Tampering, all 3 failure
        // modes (I/O / decrypt / UTF-8) collapse to the same ConfigMissing
        // variant so the caller can't oracle-leak key vs file-corrupt state.
        let r = read_md_age_pseudo(Path::new("/definitely/does/not/exist.md.age"));
        match r {
            Err(DeliveryError::ConfigMissing { channel }) => assert_eq!(channel, "markdown"),
            other => panic!("expected ConfigMissing{{markdown}}, got {:?}", other),
        }
    }

    // ─── Stage 4 KAT — lettre envelope construction ──────────────────────

    #[test]
    fn lettre_envelope_builds_well_formed_message() {
        // SPEC-24 §9.6 invariant: a valid (from, to, subject, body) tuple
        // must produce a lettre::Message with the From / To / Subject
        // headers populated and a text/plain body. We DO NOT actually open
        // an SMTP connection here — the test only checks the in-memory
        // RFC 5322 envelope so it runs offline + deterministic.
        let env = lettre_envelope_pseudo(
            "sender@example.com",
            "recipient@example.com",
            "phantom coach review",
            "hello body",
        )
        .expect("valid addresses must produce a Message");
        // formatted() returns the on-the-wire byte string of the envelope;
        // we assert the headers + body all show up so a regression that
        // dropped any of them would fail loudly here.
        let bytes = env.message.formatted();
        let wire = String::from_utf8_lossy(&bytes);
        assert!(wire.contains("From: sender@example.com"), "From missing: {wire}");
        assert!(wire.contains("To: recipient@example.com"), "To missing: {wire}");
        assert!(wire.contains("Subject: phantom coach review"), "Subject missing: {wire}");
        assert!(wire.contains("hello body"), "body missing: {wire}");
        assert!(
            wire.to_ascii_lowercase().contains("text/plain"),
            "ContentType TEXT_PLAIN missing: {wire}"
        );
    }

    #[test]
    fn lettre_envelope_rejects_malformed_from_address() {
        // RFC 5322 invariant: lettre's address parser rejects malformed
        // mailboxes. Surface this as EmailSmtpFailed so the receipt carries
        // a clean reason string ("invalid from address: ...") rather than
        // panicking and aborting the fan-out.
        let r = lettre_envelope_pseudo(
            "not a valid address",
            "recipient@example.com",
            "subject",
            "body",
        );
        assert!(
            matches!(r, Err(DeliveryError::EmailSmtpFailed { .. })),
            "malformed from must map to EmailSmtpFailed, got: {r:?}"
        );
    }

    #[test]
    fn lettre_envelope_rejects_malformed_to_address() {
        // Symmetric to the from-address test but on the recipient leg —
        // surfaces as EmailRecipientRejected so the UI can prompt the user
        // to fix their `to_address` setting rather than dig through smtp
        // server logs.
        let r = lettre_envelope_pseudo(
            "sender@example.com",
            "also not an address",
            "subject",
            "body",
        );
        assert!(
            matches!(r, Err(DeliveryError::EmailRecipientRejected { .. })),
            "malformed to must map to EmailRecipientRejected, got: {r:?}"
        );
    }

    #[test]
    fn delivery_attempt_carries_retry_count() {
        // §7.1 invariant: DeliveryAttempt distinguishes itself from
        // DeliveryReceipt by carrying `retry_count`. Verify the field
        // survives round-trip so ledger merge-by-(review_id, channel)
        // logic in Stage 2 has reliable retry numbering to display in UI.
        let a = DeliveryAttempt {
            review_id: "01923f9c-1b42-7000-9d8a-3c4e2f1a8b75".to_string(),
            channel: DeliveryChannel::Email,
            attempted_at_ms: 1_716_563_400_000,
            retry_count: 2,
            status: DeliveryStatus::Failed,
            error_message: Some("smtp connection timeout".to_string()),
        };
        let j = serde_json::to_string(&a).unwrap();
        let back: DeliveryAttempt = serde_json::from_str(&j).unwrap();
        assert_eq!(back.retry_count, 2);
        assert_eq!(back.status, DeliveryStatus::Failed);
        assert_eq!(
            back.error_message.as_deref(),
            Some("smtp connection timeout")
        );
    }
}
