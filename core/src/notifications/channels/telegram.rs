use std::sync::Arc;

use async_trait::async_trait;
use pm_types::{Notification, NotificationPriority};

use crate::channels::telegram::TelegramBot;

use super::NotificationChannel;

/// Telegram notification channel. Wraps the daemon's existing `TelegramBot`
/// (poll-based long-poll client) as a one-way delivery transport. Messages are
/// sent with HTML-escaped title + body; the 4096-character chunking already
/// lives inside `TelegramBot::send_message`.
pub struct TelegramChannel {
    bot: Arc<TelegramBot>,
    chat_id: i64,
}

impl TelegramChannel {
    pub fn new(bot: Arc<TelegramBot>, chat_id: i64) -> Self {
        Self { bot, chat_id }
    }
}

#[async_trait]
impl NotificationChannel for TelegramChannel {
    fn name(&self) -> &str {
        "telegram"
    }

    async fn send(&self, n: &Notification) -> anyhow::Result<()> {
        let emoji = match n.priority {
            NotificationPriority::P0 => "⚠️",
            NotificationPriority::P1 => "✅",
            NotificationPriority::P2 => "·",
        };
        let title_html = html_escape(&n.title);
        let body_html = html_escape(&n.body);
        let text = format!("{} <b>{}</b>\n{}", emoji, title_html, body_html);
        self.bot
            .send_message(self.chat_id, &text)
            .await
            .map_err(|e| anyhow::anyhow!("telegram send: {}", e))?;
        Ok(())
    }

    async fn send_batch(&self, ns: &[Notification]) -> anyhow::Result<()> {
        if ns.is_empty() {
            return Ok(());
        }
        let mut text = format!("📋 <b>完成 {} 個任務</b>\n", ns.len());
        for n in ns.iter().take(10) {
            text.push_str(&format!("• {}\n", html_escape(&n.title)));
        }
        if ns.len() > 10 {
            text.push_str(&format!("…還有 {} 筆\n", ns.len() - 10));
        }
        self.bot
            .send_message(self.chat_id, &text)
            .await
            .map_err(|e| anyhow::anyhow!("telegram batch send: {}", e))?;
        Ok(())
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_handles_specials() {
        assert_eq!(html_escape("a<b&c>"), "a&lt;b&amp;c&gt;");
    }
}
