// TelegramChannel — Telegram Bot API via reqwest (no SDK)
// Inspired by ZeroClaw src/channels/telegram.rs
//
// - Long-polling via getUpdates
// - Deny-by-default user allowlist
// - 4096-char message chunking
// - Markdown formatting support

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error, info, warn};

use crate::channel::{Channel, ChannelMessage};
use crate::providers::StreamChunk;
use crate::telegram_menu::{
    self, CallbackAction, InlineKeyboard,
};

const TELEGRAM_API: &str = "https://api.telegram.org";
const MAX_MESSAGE_LEN: usize = 4096;
const POLL_TIMEOUT: u64 = 30;

// ---------------------------------------------------------------------------
// MarkdownV2 escaping utilities
// ---------------------------------------------------------------------------

/// Characters that must be escaped in Telegram MarkdownV2 outside of
/// pre/code spans: _ * [ ] ( ) ~ ` > # + - = | { } . !
const MARKDOWNV2_SPECIAL: &[char] = &[
    '_', '*', '[', ']', '(', ')', '~', '`', '>', '#', '+', '-', '=', '|', '{', '}', '.', '!',
];

/// Escape **all** MarkdownV2 special characters in `text`.
///
/// Use this when you want the text rendered verbatim (no formatting at all).
/// Every special char is prefixed with a backslash.
pub fn escape_markdown_v2(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 4);
    for ch in text.chars() {
        if MARKDOWNV2_SPECIAL.contains(&ch) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Convert common LLM Markdown output to Telegram MarkdownV2.
///
/// Strategy:
/// 1. Protect fenced code blocks (```...```) — content inside is not escaped
///    except for `` ` `` and `\`.
/// 2. Protect inline code (`...`) — same treatment.
/// 3. Outside code spans, escape all MarkdownV2 special characters that are
///    **not** part of a recognised formatting pair (bold, italic, strike, etc.).
///
/// This handles CJK text adjacent to formatting markers by working at the
/// character level rather than relying on word-boundary heuristics, which
/// break for scripts without spaces (Chinese, Japanese, Korean).
pub fn sanitize_for_telegram(text: &str) -> String {
    let mut result = String::with_capacity(text.len() + text.len() / 4);
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // --- Fenced code block: ```lang\n...\n``` ---
        if i + 2 < len && chars[i] == '`' && chars[i + 1] == '`' && chars[i + 2] == '`' {
            // Find the closing ```
            if let Some(close) = find_fenced_close(&chars, i + 3) {
                // Emit opening ```
                result.push_str("```");
                // Emit content between the fences (escape only ` and \ inside pre)
                for &ch in &chars[i + 3..close] {
                    if ch == '\\' {
                        result.push_str("\\\\");
                    } else {
                        result.push(ch);
                    }
                }
                // Emit closing ```
                result.push_str("```");
                i = close + 3;
                continue;
            }
            // No closing fence found — escape the backticks and continue
            result.push_str("\\`\\`\\`");
            i += 3;
            continue;
        }

        // --- Inline code: `...` ---
        if chars[i] == '`' {
            if let Some(close) = find_inline_code_close(&chars, i + 1) {
                result.push('`');
                for &ch in &chars[i + 1..close] {
                    if ch == '\\' {
                        result.push_str("\\\\");
                    } else if ch == '`' {
                        result.push_str("\\`");
                    } else {
                        result.push(ch);
                    }
                }
                result.push('`');
                i = close + 1;
                continue;
            }
            // Unmatched backtick — escape it
            result.push_str("\\`");
            i += 1;
            continue;
        }

        // --- Bold markers: ** or __ ---
        // We convert **text** to *text* for MarkdownV2 bold.
        if i + 1 < len && chars[i] == '*' && chars[i + 1] == '*' {
            if let Some(close) = find_double_close(&chars, i + 2, '*') {
                result.push('*');
                let inner: String = chars[i + 2..close].iter().collect();
                result.push_str(&escape_markdown_v2_inline(&inner));
                result.push('*');
                i = close + 2;
                continue;
            }
        }

        // --- Italic: single * ---
        if chars[i] == '*' && (i + 1 >= len || chars[i + 1] != '*') {
            if let Some(close) = find_single_close(&chars, i + 1, '*') {
                result.push_str("_");
                let inner: String = chars[i + 1..close].iter().collect();
                result.push_str(&escape_markdown_v2_inline(&inner));
                result.push_str("_");
                i = close + 1;
                continue;
            }
        }

        // --- Strikethrough: ~~ ---
        if i + 1 < len && chars[i] == '~' && chars[i + 1] == '~' {
            if let Some(close) = find_double_close(&chars, i + 2, '~') {
                result.push('~');
                let inner: String = chars[i + 2..close].iter().collect();
                result.push_str(&escape_markdown_v2_inline(&inner));
                result.push('~');
                i = close + 2;
                continue;
            }
        }

        // --- Everything else: escape MarkdownV2 specials ---
        let ch = chars[i];
        if MARKDOWNV2_SPECIAL.contains(&ch) {
            result.push('\\');
        }
        result.push(ch);
        i += 1;
    }

    result
}

/// Escape MarkdownV2 specials inside an already-identified formatting span.
/// This escapes everything except the formatting delimiters themselves.
fn escape_markdown_v2_inline(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 4);
    for ch in text.chars() {
        // Inside a bold/italic/strikethrough span we still need to escape
        // special chars that are not the wrapping delimiter.
        if MARKDOWNV2_SPECIAL.contains(&ch) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Find closing ``` starting from position `start`.
/// Returns the index of the first backtick of the closing ```.
fn find_fenced_close(chars: &[char], start: usize) -> Option<usize> {
    let len = chars.len();
    let mut i = start;
    while i + 2 < len {
        if chars[i] == '`' && chars[i + 1] == '`' && chars[i + 2] == '`' {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Find closing single backtick for inline code.
fn find_inline_code_close(chars: &[char], start: usize) -> Option<usize> {
    for i in start..chars.len() {
        if chars[i] == '`' {
            // Make sure it's not part of a triple backtick
            if i + 2 < chars.len() && chars[i + 1] == '`' && chars[i + 2] == '`' {
                continue;
            }
            return Some(i);
        }
        // Inline code cannot span newlines
        if chars[i] == '\n' {
            return None;
        }
    }
    None
}

/// Find closing double delimiter (e.g. ** or ~~).
/// Returns index of the first char of the closing pair.
fn find_double_close(chars: &[char], start: usize, delim: char) -> Option<usize> {
    let len = chars.len();
    let mut i = start;
    while i + 1 < len {
        if chars[i] == delim && chars[i + 1] == delim {
            // Don't match empty spans
            if i > start {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// Find closing single delimiter (e.g. single *).
/// Returns index of the closing delimiter.
fn find_single_close(chars: &[char], start: usize, delim: char) -> Option<usize> {
    for i in start..chars.len() {
        if chars[i] == delim {
            // Skip double delimiters
            if i + 1 < chars.len() && chars[i + 1] == delim {
                continue;
            }
            if i > start {
                return Some(i);
            }
        }
        // Single-line only for inline formatting
        if chars[i] == '\n' {
            return None;
        }
    }
    None
}

/// Telegram Bot API response wrapper
#[derive(Debug, Deserialize)]
struct TelegramResponse<T> {
    ok: bool,
    result: Option<T>,
    description: Option<String>,
}

/// Telegram Update object (simplified)
#[derive(Debug, Deserialize)]
struct Update {
    update_id: i64,
    message: Option<Message>,
    callback_query: Option<CallbackQuery>,
}

/// Telegram CallbackQuery object (from inline keyboard button press)
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct CallbackQuery {
    id: String,
    from: User,
    message: Option<CallbackMessage>,
    data: Option<String>,
}

/// Simplified message inside a callback query (different from top-level Message:
/// Telegram only guarantees `message_id` and `chat`; `text` and `from` may be absent).
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct CallbackMessage {
    message_id: i64,
    chat: Chat,
}

/// Telegram Message object (simplified)
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct Message {
    message_id: i64,
    from: Option<User>,
    chat: Chat,
    date: i64,
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct User {
    id: i64,
    first_name: String,
    username: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct Chat {
    id: i64,
    #[serde(rename = "type")]
    chat_type: String,
}

/// Telegram channel configuration
#[derive(Debug, Clone, Deserialize)]
pub struct TelegramConfig {
    pub bot_token: String,
    #[serde(default)]
    pub allowed_users: Vec<String>,
}

pub struct TelegramChannel {
    bot_token: String,
    allowed_users: Vec<String>,
    client: Client,
    offset: Arc<RwLock<i64>>,
}

impl TelegramChannel {
    pub fn new(config: TelegramConfig) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(POLL_TIMEOUT + 10))
            .build()
            .expect("Failed to build reqwest client");

        info!(
            "TelegramChannel initialized (allowed_users: {})",
            if config.allowed_users.is_empty() {
                "DENY ALL (empty allowlist)".to_string()
            } else {
                config.allowed_users.join(", ")
            }
        );

        Self {
            bot_token: config.bot_token,
            allowed_users: config.allowed_users,
            client,
            offset: Arc::new(RwLock::new(0)),
        }
    }

    fn api_url(&self, method: &str) -> String {
        format!("{}/bot{}/{}", TELEGRAM_API, self.bot_token, method)
    }

    fn is_user_allowed(&self, user_id: i64, username: Option<&str>) -> bool {
        if self.allowed_users.is_empty() {
            return false;
        }
        let uid_str = user_id.to_string();
        for allowed in &self.allowed_users {
            if allowed == "*" || allowed == &uid_str {
                return true;
            }
            if let Some(uname) = username {
                if allowed == uname || allowed == &format!("@{}", uname) {
                    return true;
                }
            }
        }
        false
    }

    /// Send "typing..." indicator (expires after ~5s on Telegram)
    pub async fn send_typing(&self, chat_id: &str) -> Result<()> {
        let body = json!({
            "chat_id": chat_id,
            "action": "typing"
        });
        self.client
            .post(&self.api_url("sendChatAction"))
            .json(&body)
            .send()
            .await?;
        Ok(())
    }

    /// Spawn a background task that keeps sending typing indicator every 4s
    /// until the returned guard is dropped
    pub fn keep_typing(&self, chat_id: String) -> TypingGuard {
        let client = self.client.clone();
        let url = self.api_url("sendChatAction");
        let cancel = Arc::new(tokio::sync::Notify::new());
        let cancel_clone = cancel.clone();

        tokio::spawn(async move {
            loop {
                let body = json!({
                    "chat_id": chat_id,
                    "action": "typing"
                });
                let _ = client.post(&url).json(&body).send().await;

                tokio::select! {
                    _ = cancel_clone.notified() => break,
                    _ = tokio::time::sleep(Duration::from_secs(4)) => {}
                }
            }
        });

        TypingGuard { cancel }
    }

    /// Send a message and return the message_id for later editing.
    /// Text is escaped for MarkdownV2 so CJK and special chars display correctly.
    pub async fn send_message_get_id(&self, chat_id: &str, text: &str) -> Result<i64> {
        let escaped = escape_markdown_v2(text);
        let body = json!({
            "chat_id": chat_id,
            "text": escaped,
            "parse_mode": "MarkdownV2",
            "disable_web_page_preview": true,
        });

        let resp = self.client
            .post(&self.api_url("sendMessage"))
            .json(&body)
            .send()
            .await?;

        let json: Value = resp.json().await?;
        json.get("result")
            .and_then(|r| r.get("message_id"))
            .and_then(|v| v.as_i64())
            .ok_or_else(|| anyhow!("Failed to get message_id from sendMessage response"))
    }

    /// Edit an existing message's text.
    /// Text is escaped for MarkdownV2 so CJK and special chars display correctly.
    pub async fn edit_message(&self, chat_id: &str, message_id: i64, text: &str) -> Result<()> {
        // Telegram requires the text to be different from the current text
        // and has a minimum length of 1 character
        if text.is_empty() {
            return Ok(());
        }

        let escaped = escape_markdown_v2(text);
        let body = json!({
            "chat_id": chat_id,
            "message_id": message_id,
            "text": escaped,
            "parse_mode": "MarkdownV2",
            "disable_web_page_preview": true,
        });

        let resp = self.client
            .post(&self.api_url("editMessageText"))
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let err_text = resp.text().await.unwrap_or_default();
            // "message is not modified" is not a real error
            if !err_text.contains("message is not modified") {
                debug!("editMessageText failed ({}): {}", status, err_text);
            }
        }
        Ok(())
    }

    /// Send a streaming response: initial sendMessage, then progressive editMessageText.
    /// Consumes a stream of StreamChunk and updates the message progressively.
    pub async fn send_streaming(
        &self,
        chat_id: &str,
        mut stream: Pin<Box<dyn futures_util::Stream<Item = Result<StreamChunk>> + Send>>,
    ) -> Result<String> {
        let mut full_text = String::new();
        let mut message_id: Option<i64> = None;
        let mut last_edit = std::time::Instant::now();
        let edit_interval = Duration::from_millis(500);
        let min_chars_delta = 50; // Minimum chars before editing
        let mut last_edit_len = 0;

        while let Some(chunk) = stream.next().await {
            match chunk? {
                StreamChunk::ContentDelta(delta) => {
                    full_text.push_str(&delta);

                    // Send initial message on first content
                    if message_id.is_none() && !full_text.is_empty() {
                        match self.send_message_get_id(chat_id, &format!("{}▌", &full_text)).await {
                            Ok(id) => {
                                message_id = Some(id);
                                last_edit = std::time::Instant::now();
                                last_edit_len = full_text.len();
                            }
                            Err(e) => {
                                warn!("send_streaming: failed to send initial message: {}", e);
                            }
                        }
                        continue;
                    }

                    // Throttle edits: every 500ms or 50 chars
                    let elapsed = last_edit.elapsed();
                    let chars_since_edit = full_text.len() - last_edit_len;
                    if elapsed >= edit_interval || chars_since_edit >= min_chars_delta {
                        if let Some(mid) = message_id {
                            // Add cursor indicator while streaming
                            let display = format!("{}▌", &full_text);
                            let _ = self.edit_message(chat_id, mid, &display).await;
                            last_edit = std::time::Instant::now();
                            last_edit_len = full_text.len();
                        }
                    }
                }
                StreamChunk::Done { .. } => {
                    // Final edit: remove cursor
                    if let Some(mid) = message_id {
                        let _ = self.edit_message(chat_id, mid, &full_text).await;
                    } else if !full_text.is_empty() {
                        // Never sent initial message, send now
                        let _ = self.send(chat_id, &full_text).await;
                    }
                    break;
                }
                StreamChunk::ToolCallStart { name, .. } => {
                    debug!("Stream: tool call started: {}", name);
                }
                StreamChunk::ToolCallArgumentsDelta { .. } => {}
            }
        }

        // Handle case where stream ended without Done
        if let Some(mid) = message_id {
            let _ = self.edit_message(chat_id, mid, &full_text).await;
        }

        Ok(full_text)
    }

    /// Split long messages into chunks of MAX_MESSAGE_LEN
    fn chunk_message(text: &str) -> Vec<String> {
        if text.len() <= MAX_MESSAGE_LEN {
            return vec![text.to_string()];
        }

        let mut chunks = Vec::new();
        let mut remaining = text;

        while !remaining.is_empty() {
            if remaining.len() <= MAX_MESSAGE_LEN {
                chunks.push(remaining.to_string());
                break;
            }

            // Try to split at a newline near the limit
            let split_at = remaining[..MAX_MESSAGE_LEN]
                .rfind('\n')
                .unwrap_or(MAX_MESSAGE_LEN);

            let (chunk, rest) = remaining.split_at(split_at);
            chunks.push(chunk.to_string());
            remaining = rest.trim_start_matches('\n');
        }

        chunks
    }

    // -----------------------------------------------------------------------
    // Inline keyboard support
    // -----------------------------------------------------------------------

    /// Send a message with an inline keyboard attached.
    ///
    /// The `keyboard.to_json()` produces the `reply_markup` value:
    /// `{"inline_keyboard": [[{"text": "...", "callback_data": "..."}]]}`.
    pub async fn send_message_with_keyboard(
        &self,
        chat_id: &str,
        text: &str,
        keyboard: &InlineKeyboard,
    ) -> Result<i64> {
        let escaped = escape_markdown_v2(text);
        let body = json!({
            "chat_id": chat_id,
            "text": escaped,
            "parse_mode": "MarkdownV2",
            "reply_markup": keyboard.to_json(),
            "disable_web_page_preview": true,
        });

        let resp = self
            .client
            .post(&self.api_url("sendMessage"))
            .json(&body)
            .send()
            .await?;

        let json_resp: Value = resp.json().await?;
        json_resp
            .get("result")
            .and_then(|r| r.get("message_id"))
            .and_then(|v| v.as_i64())
            .ok_or_else(|| anyhow!("Failed to get message_id from sendMessage response"))
    }

    /// Build the JSON body for `send_message_with_keyboard` (useful for testing).
    pub fn build_keyboard_message_body(
        chat_id: &str,
        text: &str,
        keyboard: &InlineKeyboard,
    ) -> Value {
        let escaped = escape_markdown_v2(text);
        json!({
            "chat_id": chat_id,
            "text": escaped,
            "parse_mode": "MarkdownV2",
            "reply_markup": keyboard.to_json(),
            "disable_web_page_preview": true,
        })
    }

    /// Answer a callback query to dismiss the loading spinner on the client.
    ///
    /// Telegram requires calling `answerCallbackQuery` within ~30s of a button press
    /// or the button shows a spinning indicator indefinitely.
    pub async fn answer_callback_query(&self, callback_query_id: &str) -> Result<()> {
        let body = json!({
            "callback_query_id": callback_query_id,
        });

        let _resp = self
            .client
            .post(&self.api_url("answerCallbackQuery"))
            .json(&body)
            .send()
            .await?;

        Ok(())
    }

    /// Parse a callback query from a raw Telegram update JSON and return the
    /// typed `CallbackAction` along with context (chat_id, callback_query_id).
    ///
    /// Returns `None` if the update does not contain a callback query or the
    /// user is not allowed.
    pub fn handle_callback_query(
        &self,
        update_json: &Value,
    ) -> Option<(CallbackAction, String, String)> {
        let cq = update_json.get("callback_query")?;
        let callback_query_id = cq.get("id")?.as_str()?.to_string();
        let data = cq.get("data")?.as_str()?;

        // Extract chat_id from callback_query.message.chat.id
        let chat_id = cq
            .get("message")
            .and_then(|m| m.get("chat"))
            .and_then(|c| c.get("id"))
            .and_then(|id| id.as_i64())
            .map(|id| id.to_string())
            .unwrap_or_default();

        // Check user authorization
        let user_id = cq
            .get("from")
            .and_then(|f| f.get("id"))
            .and_then(|id| id.as_i64())
            .unwrap_or(0);
        let username = cq
            .get("from")
            .and_then(|f| f.get("username"))
            .and_then(|u| u.as_str());

        if !self.is_user_allowed(user_id, username) {
            warn!(
                "Denied callback query from user {} (@{})",
                user_id,
                username.unwrap_or("unknown")
            );
            return None;
        }

        let action = telegram_menu::parse_callback(data);
        Some((action, chat_id, callback_query_id))
    }

    /// Process a slash command and return the appropriate keyboard + text
    /// to send back to the user.
    ///
    /// Returns `Some((text, keyboard))` for recognized commands, or `None`
    /// if the text is not a known command.
    pub fn process_command(
        &self,
        text: &str,
        hand_names: &[&str],
    ) -> Option<(String, Option<InlineKeyboard>)> {
        let cmd = text.trim();
        if cmd == "/hands" || cmd == "/menu" {
            let kb = telegram_menu::hand_selector(hand_names);
            let label = if hand_names.is_empty() {
                "No hands available.".to_string()
            } else {
                format!("Select a hand to run ({} available):", hand_names.len())
            };
            Some((label, Some(kb)))
        } else if cmd == "/status" {
            let kb = telegram_menu::status_dashboard();
            Some(("Status Dashboard:".to_string(), Some(kb)))
        } else if cmd == "/help" {
            let help = "\
Available commands:\n\
/hands or /menu — Show available hands\n\
/status — Show status dashboard\n\
/help — Show this help message\n\
\n\
You can also type any message to chat with the AI agent.";
            Some((help.to_string(), None))
        } else {
            None
        }
    }
}

#[async_trait]
impl Channel for TelegramChannel {
    fn name(&self) -> &str {
        "telegram"
    }

    fn channel_type(&self) -> crate::channel::ChannelType {
        crate::channel::ChannelType::Telegram
    }

    async fn send(&self, chat_id: &str, text: &str) -> Result<()> {
        let chunks = Self::chunk_message(text);
        let total = chunks.len();

        for (i, chunk) in chunks.iter().enumerate() {
            let suffix = if total > 1 {
                format!("\n\n({}/{})", i + 1, total)
            } else {
                String::new()
            };

            let raw = format!("{}{}", chunk, suffix);
            let sanitized = sanitize_for_telegram(&raw);

            let body = json!({
                "chat_id": chat_id,
                "text": sanitized,
                "parse_mode": "MarkdownV2",
                "disable_web_page_preview": true
            });

            let resp = self
                .client
                .post(&self.api_url("sendMessage"))
                .json(&body)
                .send()
                .await?;

            let status = resp.status();
            if !status.is_success() {
                let err_text = resp.text().await.unwrap_or_default();
                // If MarkdownV2 fails, retry with fully-escaped text
                if err_text.contains("can't parse entities") {
                    debug!("MarkdownV2 parse failed, retrying with full escaping");
                    let escaped = escape_markdown_v2(&raw);
                    let escaped_body = json!({
                        "chat_id": chat_id,
                        "text": escaped,
                        "parse_mode": "MarkdownV2",
                        "disable_web_page_preview": true
                    });
                    let resp2 = self
                        .client
                        .post(&self.api_url("sendMessage"))
                        .json(&escaped_body)
                        .send()
                        .await?;

                    // If even full escaping fails, fall back to plain text
                    if !resp2.status().is_success() {
                        debug!("Full-escaped MarkdownV2 also failed, falling back to plain text");
                        let plain_body = json!({
                            "chat_id": chat_id,
                            "text": raw,
                            "disable_web_page_preview": true
                        });
                        self.client
                            .post(&self.api_url("sendMessage"))
                            .json(&plain_body)
                            .send()
                            .await?;
                    }
                } else {
                    error!("sendMessage failed ({}): {}", status, err_text);
                }
            }

            // Rate limit: small delay between chunks
            if total > 1 && i < total - 1 {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }

        Ok(())
    }

    async fn listen(&self, tx: mpsc::Sender<ChannelMessage>) -> Result<()> {
        info!("Telegram long-polling started");

        loop {
            let offset = *self.offset.read().await;

            let url = format!(
                "{}?offset={}&timeout={}&allowed_updates=[\"message\",\"callback_query\"]",
                self.api_url("getUpdates"),
                offset,
                POLL_TIMEOUT
            );

            let resp = match self.client.get(&url).send().await {
                Ok(r) => r,
                Err(e) => {
                    warn!("getUpdates failed: {}, retrying in 5s", e);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }
            };

            let body: TelegramResponse<Vec<Update>> = match resp.json().await {
                Ok(b) => b,
                Err(e) => {
                    warn!("Failed to parse getUpdates response: {}, retrying in 5s", e);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }
            };

            if !body.ok {
                error!(
                    "Telegram API error: {}",
                    body.description.unwrap_or_default()
                );
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }

            if let Some(updates) = body.result {
                for update in updates {
                    // Update offset
                    {
                        let mut off = self.offset.write().await;
                        if update.update_id >= *off {
                            *off = update.update_id + 1;
                        }
                    }

                    // --- Handle callback queries from inline keyboard ---
                    if let Some(cq) = update.callback_query {
                        let user_id = cq.from.id;
                        let username = cq.from.username.as_deref();

                        if !self.is_user_allowed(user_id, username) {
                            warn!(
                                "Denied callback query from user {} (@{})",
                                user_id,
                                username.unwrap_or("unknown")
                            );
                            continue;
                        }

                        // Answer the callback query to dismiss the spinner
                        if let Err(e) = self.answer_callback_query(&cq.id).await {
                            warn!("Failed to answer callback query: {}", e);
                        }

                        if let Some(data) = cq.data {
                            let action = telegram_menu::parse_callback(&data);
                            let chat_id = cq
                                .message
                                .as_ref()
                                .map(|m| m.chat.id.to_string())
                                .unwrap_or_default();

                            // Convert callback action into a synthetic ChannelMessage
                            // so the caller can process it uniformly.
                            let synthetic_text = match &action {
                                CallbackAction::RunHand(name) => format!("/run {}", name),
                                CallbackAction::ShowStatus(panel) => {
                                    format!("/status {}", panel)
                                }
                                CallbackAction::Confirm => "/confirm".to_string(),
                                CallbackAction::Cancel => "/cancel".to_string(),
                                CallbackAction::SelectProvider(name) => {
                                    format!("/provider {}", name)
                                }
                                CallbackAction::GoalsMood(m) => {
                                    format!("/goals_mood {}", m)
                                }
                                CallbackAction::GoalsTaskDone(id) => {
                                    format!("/goals_task {}", id)
                                }
                                CallbackAction::Unknown(raw) => raw.clone(),
                            };

                            debug!(
                                "Callback from @{}: {} -> {:?}",
                                username.unwrap_or("unknown"),
                                data,
                                action,
                            );

                            let channel_msg = ChannelMessage {
                                sender: cq
                                    .from
                                    .username
                                    .clone()
                                    .unwrap_or_else(|| cq.from.first_name.clone()),
                                sender_id: cq.from.id.to_string(),
                                text: synthetic_text,
                                chat_id,
                                timestamp: 0, // callback queries don't carry a date
                                channel: "telegram".to_string(),
                                reply_to: None,
                                message_id: cq
                                    .message
                                    .as_ref()
                                    .map(|m| m.message_id.to_string()),
                            };

                            if tx.send(channel_msg).await.is_err() {
                                error!("Message channel closed, stopping listener");
                                return Err(anyhow!("Message channel closed"));
                            }
                        }
                        continue;
                    }

                    // --- Handle regular messages ---
                    let msg = match update.message {
                        Some(m) => m,
                        None => continue,
                    };

                    let text = match msg.text {
                        Some(t) => t,
                        None => continue, // Skip non-text messages
                    };

                    let user = match msg.from {
                        Some(u) => u,
                        None => continue,
                    };

                    // Deny-by-default access control
                    if !self.is_user_allowed(user.id, user.username.as_deref()) {
                        warn!(
                            "Denied message from user {} (@{})",
                            user.id,
                            user.username.as_deref().unwrap_or("unknown")
                        );
                        continue;
                    }

                    debug!(
                        "Received from @{}: {}",
                        user.username.as_deref().unwrap_or("unknown"),
                        &text[..text.len().min(80)]
                    );

                    let channel_msg = ChannelMessage {
                        sender: user.username.clone().unwrap_or_else(|| user.first_name.clone()),
                        sender_id: user.id.to_string(),
                        text,
                        chat_id: msg.chat.id.to_string(),
                        timestamp: msg.date as u64,
                        channel: "telegram".to_string(),
                        reply_to: None,
                        message_id: Some(msg.message_id.to_string()),
                    };

                    if tx.send(channel_msg).await.is_err() {
                        error!("Message channel closed, stopping listener");
                        return Err(anyhow!("Message channel closed"));
                    }
                }
            }
        }
    }
}

/// Guard that stops the typing indicator when dropped
pub struct TypingGuard {
    cancel: Arc<tokio::sync::Notify>,
}

impl Drop for TypingGuard {
    fn drop(&mut self) {
        self.cancel.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_short_message() {
        let chunks = TelegramChannel::chunk_message("hello world");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "hello world");
    }

    #[test]
    fn test_chunk_long_message() {
        let long = "a\n".repeat(3000);
        let chunks = TelegramChannel::chunk_message(&long);
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            assert!(chunk.len() <= MAX_MESSAGE_LEN);
        }
    }

    #[test]
    fn test_user_allowed_by_id() {
        let ch = TelegramChannel::new(TelegramConfig {
            bot_token: "test".to_string(),
            allowed_users: vec!["12345".to_string()],
        });
        assert!(ch.is_user_allowed(12345, None));
        assert!(!ch.is_user_allowed(99999, None));
    }

    #[test]
    fn test_user_allowed_by_username() {
        let ch = TelegramChannel::new(TelegramConfig {
            bot_token: "test".to_string(),
            allowed_users: vec!["markl".to_string()],
        });
        assert!(ch.is_user_allowed(99999, Some("markl")));
        assert!(!ch.is_user_allowed(99999, Some("other")));
    }

    #[test]
    fn test_user_allowed_wildcard() {
        let ch = TelegramChannel::new(TelegramConfig {
            bot_token: "test".to_string(),
            allowed_users: vec!["*".to_string()],
        });
        assert!(ch.is_user_allowed(99999, None));
    }

    #[test]
    fn test_empty_allowlist_denies_all() {
        let ch = TelegramChannel::new(TelegramConfig {
            bot_token: "test".to_string(),
            allowed_users: vec![],
        });
        assert!(!ch.is_user_allowed(12345, Some("anyone")));
    }

    // -----------------------------------------------------------------------
    // Inline keyboard integration tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_send_message_with_keyboard_builds_correct_json_body() {
        use crate::telegram_menu::hand_selector;

        let kb = hand_selector(&["seo_content", "outreach", "lead"]);
        let body = TelegramChannel::build_keyboard_message_body("12345", "Pick a hand:", &kb);

        // Verify top-level fields
        assert_eq!(body["chat_id"], "12345");
        assert_eq!(body["text"], "Pick a hand:");
        assert_eq!(body["parse_mode"], "MarkdownV2");
        assert_eq!(body["disable_web_page_preview"], true);

        // Verify reply_markup contains inline_keyboard
        let reply_markup = &body["reply_markup"];
        let rows = reply_markup["inline_keyboard"].as_array().unwrap();
        assert_eq!(rows.len(), 1); // 3 items = 1 row of 3
        assert_eq!(rows[0].as_array().unwrap().len(), 3);
        assert_eq!(rows[0][0]["text"], "seo_content");
        assert_eq!(rows[0][0]["callback_data"], "hand:seo_content");
        assert_eq!(rows[0][2]["callback_data"], "hand:lead");
    }

    #[test]
    fn test_handle_callback_query_parses_run_hand() {
        let ch = TelegramChannel::new(TelegramConfig {
            bot_token: "test".to_string(),
            allowed_users: vec!["*".to_string()],
        });

        let update = json!({
            "callback_query": {
                "id": "abc123",
                "from": {
                    "id": 42,
                    "first_name": "Mark",
                    "username": "markl"
                },
                "message": {
                    "message_id": 100,
                    "chat": { "id": 9999, "type": "private" }
                },
                "data": "hand:seo_content"
            }
        });

        let result = ch.handle_callback_query(&update);
        assert!(result.is_some());
        let (action, chat_id, cq_id) = result.unwrap();
        assert_eq!(action, CallbackAction::RunHand("seo_content".to_string()));
        assert_eq!(chat_id, "9999");
        assert_eq!(cq_id, "abc123");
    }

    #[test]
    fn test_handle_callback_query_denied_user() {
        let ch = TelegramChannel::new(TelegramConfig {
            bot_token: "test".to_string(),
            allowed_users: vec!["12345".to_string()],
        });

        let update = json!({
            "callback_query": {
                "id": "xyz",
                "from": {
                    "id": 99999,
                    "first_name": "Evil",
                    "username": "hacker"
                },
                "message": {
                    "message_id": 1,
                    "chat": { "id": 1, "type": "private" }
                },
                "data": "hand:outreach"
            }
        });

        let result = ch.handle_callback_query(&update);
        assert!(result.is_none(), "Denied user should get None from handle_callback_query");
    }

    #[test]
    fn test_process_command_hands_returns_keyboard() {
        let ch = TelegramChannel::new(TelegramConfig {
            bot_token: "test".to_string(),
            allowed_users: vec!["*".to_string()],
        });

        let hand_names = &["seo_content", "outreach", "lead", "report"];
        let result = ch.process_command("/hands", hand_names);
        assert!(result.is_some());
        let (text, kb) = result.unwrap();
        assert!(text.contains("4 available"));
        let kb = kb.expect("Should have a keyboard");
        // 4 hands -> 2 rows (3 + 1)
        assert_eq!(kb.rows.len(), 2);
        assert_eq!(kb.rows[0].len(), 3);
        assert_eq!(kb.rows[1].len(), 1);
    }

    #[test]
    fn test_process_command_menu_alias() {
        let ch = TelegramChannel::new(TelegramConfig {
            bot_token: "test".to_string(),
            allowed_users: vec![],
        });

        let result = ch.process_command("/menu", &["seo"]);
        assert!(result.is_some());
        let (text, kb) = result.unwrap();
        assert!(text.contains("1 available"));
        assert!(kb.is_some());
    }

    #[test]
    fn test_process_command_status_returns_dashboard() {
        let ch = TelegramChannel::new(TelegramConfig {
            bot_token: "test".to_string(),
            allowed_users: vec![],
        });

        let result = ch.process_command("/status", &[]);
        assert!(result.is_some());
        let (text, kb) = result.unwrap();
        assert_eq!(text, "Status Dashboard:");
        let kb = kb.expect("Should have status keyboard");
        assert_eq!(kb.rows.len(), 1);
        assert_eq!(kb.rows[0].len(), 4);
        assert_eq!(kb.rows[0][0].callback_data, "status:cost");
    }

    #[test]
    fn test_process_command_help_returns_text_only() {
        let ch = TelegramChannel::new(TelegramConfig {
            bot_token: "test".to_string(),
            allowed_users: vec![],
        });

        let result = ch.process_command("/help", &[]);
        assert!(result.is_some());
        let (text, kb) = result.unwrap();
        assert!(text.contains("/hands"));
        assert!(text.contains("/status"));
        assert!(kb.is_none(), "/help should not include a keyboard");
    }

    #[test]
    fn test_process_command_unknown_returns_none() {
        let ch = TelegramChannel::new(TelegramConfig {
            bot_token: "test".to_string(),
            allowed_users: vec![],
        });

        let result = ch.process_command("hello world", &[]);
        assert!(result.is_none());
    }

    #[test]
    fn test_handle_callback_query_no_callback_returns_none() {
        let ch = TelegramChannel::new(TelegramConfig {
            bot_token: "test".to_string(),
            allowed_users: vec!["*".to_string()],
        });

        // An update with only a message, no callback_query
        let update = json!({
            "message": {
                "message_id": 1,
                "chat": { "id": 1, "type": "private" },
                "text": "hello"
            }
        });

        assert!(ch.handle_callback_query(&update).is_none());
    }

    #[test]
    fn test_handle_callback_query_parses_status() {
        let ch = TelegramChannel::new(TelegramConfig {
            bot_token: "test".to_string(),
            allowed_users: vec!["*".to_string()],
        });

        let update = json!({
            "callback_query": {
                "id": "cq_status",
                "from": { "id": 1, "first_name": "Test" },
                "message": {
                    "message_id": 50,
                    "chat": { "id": 777, "type": "private" }
                },
                "data": "status:cluster"
            }
        });

        let (action, chat_id, _) = ch.handle_callback_query(&update).unwrap();
        assert_eq!(action, CallbackAction::ShowStatus("cluster".to_string()));
        assert_eq!(chat_id, "777");
    }

    #[test]
    fn test_process_command_hands_empty_list() {
        let ch = TelegramChannel::new(TelegramConfig {
            bot_token: "test".to_string(),
            allowed_users: vec![],
        });

        let result = ch.process_command("/hands", &[]);
        assert!(result.is_some());
        let (text, kb) = result.unwrap();
        assert!(text.contains("No hands available"));
        let kb = kb.expect("Should still have a keyboard (empty)");
        assert!(kb.rows.is_empty());
    }

    // -----------------------------------------------------------------------
    // MarkdownV2 escaping tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_escape_markdown_v2_basic_specials() {
        let input = "Hello_world *bold* [link](url) ~strike~";
        let escaped = super::escape_markdown_v2(input);
        assert_eq!(
            escaped,
            "Hello\\_world \\*bold\\* \\[link\\]\\(url\\) \\~strike\\~"
        );
    }

    #[test]
    fn test_escape_markdown_v2_all_specials() {
        let input = "_*[]()~`>#+-=|{}.!";
        let escaped = super::escape_markdown_v2(input);
        assert_eq!(
            escaped,
            "\\_\\*\\[\\]\\(\\)\\~\\`\\>\\#\\+\\-\\=\\|\\{\\}\\.\\!"
        );
    }

    #[test]
    fn test_escape_markdown_v2_no_specials() {
        let input = "Hello world 123";
        let escaped = super::escape_markdown_v2(input);
        assert_eq!(escaped, "Hello world 123");
    }

    #[test]
    fn test_escape_markdown_v2_empty_string() {
        assert_eq!(super::escape_markdown_v2(""), "");
    }

    #[test]
    fn test_escape_markdown_v2_cjk_plain() {
        // CJK characters should pass through unmodified
        let input = "你好世界 こんにちは 안녕하세요";
        let escaped = super::escape_markdown_v2(input);
        assert_eq!(escaped, input);
    }

    #[test]
    fn test_escape_markdown_v2_cjk_with_specials() {
        // CJK adjacent to special chars
        let input = "价格是$100.50! 测试*加粗*文本";
        let escaped = super::escape_markdown_v2(input);
        assert_eq!(escaped, "价格是$100\\.50\\! 测试\\*加粗\\*文本");
    }

    #[test]
    fn test_sanitize_for_telegram_plain_text() {
        let input = "Hello world";
        let result = super::sanitize_for_telegram(input);
        assert_eq!(result, "Hello world");
    }

    #[test]
    fn test_sanitize_for_telegram_bold() {
        // **bold** in Markdown -> *bold* in MarkdownV2
        let input = "This is **bold** text";
        let result = super::sanitize_for_telegram(input);
        assert_eq!(result, "This is *bold* text");
    }

    #[test]
    fn test_sanitize_for_telegram_italic() {
        // *italic* in Markdown -> _italic_ in MarkdownV2
        let input = "This is *italic* text";
        let result = super::sanitize_for_telegram(input);
        assert_eq!(result, "This is _italic_ text");
    }

    #[test]
    fn test_sanitize_for_telegram_strikethrough() {
        let input = "This is ~~deleted~~ text";
        let result = super::sanitize_for_telegram(input);
        assert_eq!(result, "This is ~deleted~ text");
    }

    #[test]
    fn test_sanitize_for_telegram_inline_code() {
        let input = "Use `foo.bar()` here";
        let result = super::sanitize_for_telegram(input);
        // Inside inline code, only ` and \ are escaped; . is not
        assert_eq!(result, "Use `foo.bar()` here");
    }

    #[test]
    fn test_sanitize_for_telegram_fenced_code_block() {
        let input = "```rust\nfn main() { println!(\"hello\"); }\n```";
        let result = super::sanitize_for_telegram(input);
        // Inside fenced code blocks, content is preserved (only \ is escaped)
        assert!(result.starts_with("```"));
        assert!(result.ends_with("```"));
        assert!(result.contains("fn main()"));
    }

    #[test]
    fn test_sanitize_for_telegram_cjk_bold() {
        // CJK directly adjacent to bold markers — no space between
        let input = "**加粗文本**是重要的";
        let result = super::sanitize_for_telegram(input);
        // Should produce *加粗文本* with the CJK remainder escaped
        assert_eq!(result, "*加粗文本*是重要的");
    }

    #[test]
    fn test_sanitize_for_telegram_cjk_mixed_formatting() {
        let input = "你好**世界**！这是*测试*。";
        let result = super::sanitize_for_telegram(input);
        // 你好 -> plain, **世界** -> *世界* (MarkdownV2 bold),
        // ！ (U+FF01 fullwidth exclamation) -> not ASCII '!' so not escaped,
        // 这是 -> plain, *测试* -> _测试_ (MarkdownV2 italic),
        // 。 (U+3002 fullwidth period) -> not ASCII '.' so not escaped
        assert_eq!(result, "你好*世界*！这是_测试_。");
    }

    #[test]
    fn test_sanitize_for_telegram_dots_and_exclamations() {
        let input = "Hello! This costs $5.99. Really!";
        let result = super::sanitize_for_telegram(input);
        assert_eq!(result, "Hello\\! This costs $5\\.99\\. Really\\!");
    }

    #[test]
    fn test_sanitize_for_telegram_unmatched_bold() {
        // Unmatched ** should be escaped rather than breaking the message
        let input = "This is **unmatched bold";
        let result = super::sanitize_for_telegram(input);
        assert!(result.contains("\\*\\*"));
    }

    #[test]
    fn test_sanitize_for_telegram_unmatched_backtick() {
        let input = "Here is a ` backtick";
        let result = super::sanitize_for_telegram(input);
        assert!(result.contains("\\`"));
    }

    #[test]
    fn test_sanitize_for_telegram_parentheses_outside_links() {
        let input = "Result (1 of 3) is ready.";
        let result = super::sanitize_for_telegram(input);
        assert_eq!(result, "Result \\(1 of 3\\) is ready\\.");
    }

    #[test]
    fn test_sanitize_for_telegram_number_sign_and_dash() {
        let input = "# Heading\n- item 1\n- item 2";
        let result = super::sanitize_for_telegram(input);
        assert!(result.contains("\\#"));
        assert!(result.contains("\\-"));
    }
}
