// Channel trait — inspired by ZeroClaw/OpenCrust's Channel abstraction
// Supports multiple messaging platforms: Telegram, Discord, Slack, etc.

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

/// Inbound message from any channel
#[derive(Debug, Clone)]
pub struct ChannelMessage {
    pub sender: String,
    pub sender_id: String,
    pub text: String,
    pub chat_id: String,
    pub timestamp: u64,
    /// Which channel this message came from (e.g., "telegram", "discord")
    pub channel: String,
    /// Optional reply-to message ID
    pub reply_to: Option<String>,
    /// Optional message ID (for editing/replying)
    pub message_id: Option<String>,
}

/// Supported channel types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChannelType {
    Telegram,
    Discord,
    Slack,
    Matrix,
    Http,
    Cli,
}

impl std::fmt::Display for ChannelType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Telegram => write!(f, "telegram"),
            Self::Discord => write!(f, "discord"),
            Self::Slack => write!(f, "slack"),
            Self::Matrix => write!(f, "matrix"),
            Self::Http => write!(f, "http"),
            Self::Cli => write!(f, "cli"),
        }
    }
}

/// Channel trait — any messaging platform implements this
#[async_trait]
pub trait Channel: Send + Sync {
    /// Unique name for this channel instance
    fn name(&self) -> &str;

    /// Channel type (telegram, discord, etc.)
    fn channel_type(&self) -> ChannelType;

    /// Send a text message to a recipient (chat_id)
    async fn send(&self, chat_id: &str, text: &str) -> Result<()>;

    /// Send a message as a reply to another message
    async fn send_reply(&self, chat_id: &str, text: &str, _reply_to: &str) -> Result<()> {
        // Default: just send without reply context
        self.send(chat_id, text).await
    }

    /// Edit an existing message
    async fn edit_message(&self, chat_id: &str, message_id: &str, text: &str) -> Result<()> {
        // Default: send a new message (not all platforms support editing)
        let _ = message_id;
        self.send(chat_id, text).await
    }

    /// Start listening for inbound messages, forwarding them via the sender
    async fn listen(&self, tx: mpsc::Sender<ChannelMessage>) -> Result<()>;

    /// Check if this channel is connected/ready
    fn is_connected(&self) -> bool {
        true
    }
}

/// Registry for managing multiple channels
pub struct ChannelRegistry {
    channels: Vec<Box<dyn Channel>>,
}

impl ChannelRegistry {
    pub fn new() -> Self {
        Self { channels: Vec::new() }
    }

    pub fn register(&mut self, channel: Box<dyn Channel>) {
        self.channels.push(channel);
    }

    pub fn get(&self, name: &str) -> Option<&dyn Channel> {
        self.channels.iter()
            .find(|c| c.name() == name)
            .map(|c| c.as_ref())
    }

    pub fn list(&self) -> Vec<&str> {
        self.channels.iter().map(|c| c.name()).collect()
    }

    /// Send a message to all channels matching a type
    pub async fn broadcast(&self, channel_type: ChannelType, chat_id: &str, text: &str) -> Result<()> {
        for ch in &self.channels {
            if ch.channel_type() == channel_type {
                ch.send(chat_id, text).await?;
            }
        }
        Ok(())
    }
}

/// Mock channel for testing — captures outbound sends and allows inbound message injection.
#[derive(Clone)]
pub struct MockChannel {
    replies: std::sync::Arc<std::sync::Mutex<Vec<(String, String)>>>,
    injected: std::sync::Arc<std::sync::Mutex<Vec<ChannelMessage>>>,
}

impl MockChannel {
    pub fn new() -> Self {
        Self {
            replies: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            injected: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    /// Inject a message that will be forwarded when listen() is called.
    pub fn inject_message(&self, sender: &str, chat_id: &str, text: &str) {
        self.injected.lock().unwrap().push(ChannelMessage {
            sender: sender.to_string(),
            sender_id: sender.to_string(),
            text: text.to_string(),
            chat_id: chat_id.to_string(),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            channel: "mock".to_string(),
            reply_to: None,
            message_id: None,
        });
    }

    /// Drain and return all captured outbound replies as (chat_id, text) pairs.
    pub fn drain_replies(&self) -> Vec<(String, String)> {
        let mut replies = self.replies.lock().unwrap();
        let drained = replies.clone();
        replies.clear();
        drained
    }

    /// Get replies without draining (for assertions that don't consume).
    pub fn replies(&self) -> Vec<(String, String)> {
        self.replies.lock().unwrap().clone()
    }
}

#[async_trait]
impl Channel for MockChannel {
    fn name(&self) -> &str { "mock" }
    fn channel_type(&self) -> ChannelType { ChannelType::Telegram }

    async fn send(&self, chat_id: &str, text: &str) -> Result<()> {
        self.replies.lock().unwrap().push((chat_id.to_string(), text.to_string()));
        Ok(())
    }

    async fn listen(&self, tx: mpsc::Sender<ChannelMessage>) -> Result<()> {
        let messages = {
            let mut injected = self.injected.lock().unwrap();
            let msgs = injected.clone();
            injected.clear();
            msgs
        };
        for msg in messages {
            tx.send(msg).await.map_err(|e| anyhow::anyhow!("send error: {}", e))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_channel_type_display() {
        assert_eq!(ChannelType::Telegram.to_string(), "telegram");
        assert_eq!(ChannelType::Discord.to_string(), "discord");
        assert_eq!(ChannelType::Slack.to_string(), "slack");
        assert_eq!(ChannelType::Matrix.to_string(), "matrix");
    }

    #[test]
    fn test_channel_registry_empty() {
        let registry = ChannelRegistry::new();
        assert!(registry.list().is_empty());
        assert!(registry.get("test").is_none());
    }

    #[test]
    fn test_channel_type_serde() {
        let ct = ChannelType::Discord;
        let json = serde_json::to_string(&ct).unwrap();
        assert_eq!(json, "\"discord\"");
        let parsed: ChannelType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ChannelType::Discord);
    }

    #[tokio::test]
    async fn test_mock_channel_captures_sends() {
        let mock = MockChannel::new();
        mock.send("chat1", "hello").await.unwrap();
        mock.send("chat1", "world").await.unwrap();

        let replies = mock.drain_replies();
        assert_eq!(replies.len(), 2);
        assert_eq!(replies[0], ("chat1".to_string(), "hello".to_string()));
        assert_eq!(replies[1], ("chat1".to_string(), "world".to_string()));
    }

    #[tokio::test]
    async fn test_mock_channel_drain_clears() {
        let mock = MockChannel::new();
        mock.send("chat1", "first").await.unwrap();
        let _ = mock.drain_replies();
        let replies = mock.drain_replies();
        assert!(replies.is_empty());
    }

    #[tokio::test]
    async fn test_mock_channel_injects_messages() {
        let mock = MockChannel::new();
        let (tx, mut rx) = tokio::sync::mpsc::channel(10);

        mock.inject_message("user1", "chat1", "hello bot");

        // listen() forwards injected messages to the tx channel
        let mock_clone = mock.clone();
        tokio::spawn(async move {
            mock_clone.listen(tx).await.unwrap();
        });

        let msg = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            rx.recv(),
        ).await;
        assert!(msg.is_ok());
        let msg = msg.unwrap().unwrap();
        assert_eq!(msg.text, "hello bot");
        assert_eq!(msg.chat_id, "chat1");
    }
}
