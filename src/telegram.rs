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

const TELEGRAM_API: &str = "https://api.telegram.org";
const MAX_MESSAGE_LEN: usize = 4096;
const POLL_TIMEOUT: u64 = 30;

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

    /// Send a message and return the message_id for later editing
    pub async fn send_message_get_id(&self, chat_id: &str, text: &str) -> Result<i64> {
        let body = json!({
            "chat_id": chat_id,
            "text": text,
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

    /// Edit an existing message's text
    pub async fn edit_message(&self, chat_id: &str, message_id: i64, text: &str) -> Result<()> {
        // Telegram requires the text to be different from the current text
        // and has a minimum length of 1 character
        if text.is_empty() {
            return Ok(());
        }

        let body = json!({
            "chat_id": chat_id,
            "message_id": message_id,
            "text": text,
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

            let body = json!({
                "chat_id": chat_id,
                "text": format!("{}{}", chunk, suffix),
                "parse_mode": "Markdown",
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
                // If Markdown fails, retry without parse_mode
                if err_text.contains("can't parse entities") {
                    debug!("Markdown parse failed, retrying as plain text");
                    let plain_body = json!({
                        "chat_id": chat_id,
                        "text": format!("{}{}", chunk, suffix),
                        "disable_web_page_preview": true
                    });
                    self.client
                        .post(&self.api_url("sendMessage"))
                        .json(&plain_body)
                        .send()
                        .await?;
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
                "{}?offset={}&timeout={}&allowed_updates=[\"message\"]",
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

                    // Process message
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
}
