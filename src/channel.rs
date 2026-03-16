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
}
