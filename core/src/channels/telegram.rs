/// Minimal Telegram long-poll bot using plain reqwest.
/// No extra dependencies — reuses the `reqwest` crate already in Cargo.toml.
use serde_json::Value;

pub struct TelegramBot {
    pub token: String,
    /// Telegram user IDs that are allowed to interact.
    /// Empty means allow everyone.
    pub allowed_users: Vec<i64>,
    client: reqwest::Client,
}

impl TelegramBot {
    pub fn new(token: String, allowed_users: Vec<i64>) -> Self {
        Self {
            token,
            allowed_users,
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(35))
                .build()
                .unwrap_or_default(),
        }
    }

    fn api_url(&self, method: &str) -> String {
        format!("https://api.telegram.org/bot{}/{}", self.token, method)
    }

    /// Long-poll for new updates.
    /// Returns a list of `(chat_id, user_id, text, update_id)` tuples.
    pub async fn poll_updates(&self, offset: i64) -> Result<Vec<(i64, i64, String, i64)>, String> {
        let url = format!(
            "{}?offset={}&timeout=30&allowed_updates=[\"message\"]",
            self.api_url("getUpdates"),
            offset
        );

        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("getUpdates request failed: {}", e))?;

        let body: Value = resp
            .json()
            .await
            .map_err(|e| format!("getUpdates JSON parse failed: {}", e))?;

        if !body["ok"].as_bool().unwrap_or(false) {
            return Err(format!(
                "Telegram API error: {}",
                body["description"].as_str().unwrap_or("unknown")
            ));
        }

        let mut results = Vec::new();
        if let Some(updates) = body["result"].as_array() {
            for update in updates {
                let update_id = update["update_id"].as_i64().unwrap_or(0);
                if let Some(msg) = update.get("message") {
                    let chat_id = msg["chat"]["id"].as_i64().unwrap_or(0);
                    let user_id = msg["from"]["id"].as_i64().unwrap_or(0);
                    if let Some(text) = msg["text"].as_str() {
                        results.push((chat_id, user_id, text.to_string(), update_id));
                    }
                }
            }
        }

        Ok(results)
    }

    /// Send a plain-text message to the given chat_id.
    pub async fn send_message(&self, chat_id: i64, text: &str) -> Result<(), String> {
        // Telegram has a 4096 character limit per message
        let chunks = split_message(text, 4000);
        for chunk in chunks {
            let body = serde_json::json!({
                "chat_id": chat_id,
                "text": chunk,
                "parse_mode": "HTML",
            });

            let resp = self
                .client
                .post(self.api_url("sendMessage"))
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("sendMessage request failed: {}", e))?;

            let result: Value = resp
                .json()
                .await
                .map_err(|e| format!("sendMessage JSON parse failed: {}", e))?;

            if !result["ok"].as_bool().unwrap_or(false) {
                // If HTML parse failed, retry as plain text
                let desc = result["description"].as_str().unwrap_or("").to_string();
                if desc.contains("can't parse") {
                    let plain = serde_json::json!({
                        "chat_id": chat_id,
                        "text": chunk,
                    });
                    let _ = self
                        .client
                        .post(&self.api_url("sendMessage"))
                        .json(&plain)
                        .send()
                        .await;
                } else {
                    tracing::warn!("sendMessage failed: {}", desc);
                }
            }
        }
        Ok(())
    }

    /// Returns true if `user_id` is in the allowlist (or the allowlist is empty).
    pub fn is_allowed(&self, user_id: i64) -> bool {
        self.allowed_users.is_empty() || self.allowed_users.contains(&user_id)
    }

    /// Alias kept for backwards compatibility.
    #[inline]
    pub fn is_user_allowed(&self, user_id: i64) -> bool {
        self.is_allowed(user_id)
    }
}

/// Standalone async event loop — intended to be spawned with `tokio::spawn`.
///
/// `_placeholder` is reserved for the real agent-runtime integration that
/// `main.rs` will wire up.  Keeping the signature here lets the compiler
/// enforce the boundary without creating a circular dependency during parallel
/// development.
pub async fn run_bot_loop(bot: TelegramBot, _placeholder: ()) {
    // Placeholder — main.rs will implement the actual dispatch loop using
    // `bot.poll_updates` / `bot.send_message`.
    let _ = bot;
}

/// Find the largest byte index <= max_bytes that falls on a UTF-8 char boundary.
fn split_at_char_boundary(s: &str, max_bytes: usize) -> (&str, &str) {
    if s.len() <= max_bytes {
        return (s, "");
    }
    let mut boundary = max_bytes;
    while boundary > 0 && !s.is_char_boundary(boundary) {
        boundary -= 1;
    }
    s.split_at(boundary)
}

/// Split a long message into chunks of at most `max_len` bytes,
/// splitting on newlines where possible and never cutting a multi-byte char.
fn split_message(text: &str, max_len: usize) -> Vec<String> {
    if text.len() <= max_len {
        return vec![text.to_string()];
    }
    let mut chunks = Vec::new();
    let mut remaining = text;
    while remaining.len() > max_len {
        // Find the split boundary (guaranteed to be a char boundary)
        let (head, _) = split_at_char_boundary(remaining, max_len);
        // Try to split at a newline near the boundary
        let split_at = head
            .rfind('\n')
            .map(|p| p + 1)
            .unwrap_or(head.len());
        chunks.push(remaining[..split_at].to_string());
        remaining = &remaining[split_at..];
    }
    if !remaining.is_empty() {
        chunks.push(remaining.to_string());
    }
    chunks
}
