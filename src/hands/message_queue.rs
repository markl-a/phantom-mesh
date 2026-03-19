//! Hand Message Queue — per-chat message buffering for hand phase transitions.
//!
//! When a hand is running across multiple phases, incoming Telegram messages
//! are queued instead of interrupting. After each phase completes, the queue
//! is drained and messages can be injected into the next phase's context.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::Instant;
use tracing::{debug, info};

/// A queued message with metadata
#[derive(Debug, Clone)]
pub struct QueuedMessage {
    /// Chat ID this message belongs to
    pub chat_id: i64,
    /// Message text
    pub text: String,
    /// When the message was enqueued
    pub timestamp: Instant,
    /// Optional sender name
    pub sender: Option<String>,
}

/// Per-chat message queue for hand workflows.
///
/// While a hand is running for a given chat, new messages are buffered here.
/// Between phases (or after completion), the queue is drained.
pub struct HandMessageQueue {
    /// Per-chat message queues
    queues: Mutex<HashMap<i64, VecDeque<QueuedMessage>>>,
    /// Tracks which chats have an active hand running
    active_hands: Mutex<HashMap<i64, String>>,
    /// Maximum messages per chat queue (prevents unbounded growth)
    max_per_chat: usize,
}

impl HandMessageQueue {
    /// Create a new HandMessageQueue with default max (50 messages per chat).
    pub fn new() -> Self {
        Self {
            queues: Mutex::new(HashMap::new()),
            active_hands: Mutex::new(HashMap::new()),
            max_per_chat: 50,
        }
    }

    /// Create with a custom max messages per chat.
    pub fn with_max(max_per_chat: usize) -> Self {
        Self {
            queues: Mutex::new(HashMap::new()),
            active_hands: Mutex::new(HashMap::new()),
            max_per_chat,
        }
    }

    /// Mark a hand as started for a given chat. New messages will be queued.
    pub fn start_hand(&self, chat_id: i64, hand_name: &str) {
        let mut active = self.active_hands.lock().unwrap();
        active.insert(chat_id, hand_name.to_string());
        info!(
            "HandMessageQueue: hand '{}' started for chat {}",
            hand_name, chat_id
        );
    }

    /// Mark a hand as finished for a given chat.
    pub fn finish_hand(&self, chat_id: i64) {
        let mut active = self.active_hands.lock().unwrap();
        if let Some(hand_name) = active.remove(&chat_id) {
            info!(
                "HandMessageQueue: hand '{}' finished for chat {}",
                hand_name, chat_id
            );
        }
    }

    /// Check if a hand is currently active for a given chat.
    pub fn is_hand_active(&self, chat_id: i64) -> bool {
        let active = self.active_hands.lock().unwrap();
        active.contains_key(&chat_id)
    }

    /// Get the name of the active hand for a chat (if any).
    pub fn active_hand(&self, chat_id: i64) -> Option<String> {
        let active = self.active_hands.lock().unwrap();
        active.get(&chat_id).cloned()
    }

    /// Enqueue a message for a chat. Returns true if queued, false if no active hand.
    pub fn enqueue(&self, chat_id: i64, text: String, sender: Option<String>) -> bool {
        if !self.is_hand_active(chat_id) {
            return false;
        }

        let mut queues = self.queues.lock().unwrap();
        let queue = queues.entry(chat_id).or_insert_with(VecDeque::new);

        // Enforce max queue size — drop oldest if full
        if queue.len() >= self.max_per_chat {
            queue.pop_front();
            debug!(
                "HandMessageQueue: chat {} queue full, dropped oldest message",
                chat_id
            );
        }

        queue.push_back(QueuedMessage {
            chat_id,
            text,
            timestamp: Instant::now(),
            sender,
        });

        debug!(
            "HandMessageQueue: enqueued message for chat {} (queue size: {})",
            chat_id,
            queue.len()
        );
        true
    }

    /// Drain all queued messages for a chat. Returns them in order.
    pub fn drain(&self, chat_id: i64) -> Vec<QueuedMessage> {
        let mut queues = self.queues.lock().unwrap();
        if let Some(queue) = queues.get_mut(&chat_id) {
            let messages: Vec<QueuedMessage> = queue.drain(..).collect();
            if !messages.is_empty() {
                debug!(
                    "HandMessageQueue: drained {} messages for chat {}",
                    messages.len(),
                    chat_id
                );
            }
            messages
        } else {
            Vec::new()
        }
    }

    /// Peek at queued messages without removing them.
    pub fn peek(&self, chat_id: i64) -> Vec<QueuedMessage> {
        let queues = self.queues.lock().unwrap();
        queues
            .get(&chat_id)
            .map(|q| q.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Get the number of queued messages for a chat.
    pub fn queue_len(&self, chat_id: i64) -> usize {
        let queues = self.queues.lock().unwrap();
        queues.get(&chat_id).map(|q| q.len()).unwrap_or(0)
    }

    /// Get the total number of active hands across all chats.
    pub fn active_count(&self) -> usize {
        let active = self.active_hands.lock().unwrap();
        active.len()
    }

    /// Format queued messages as context string for injection into phase prompt.
    pub fn format_as_context(messages: &[QueuedMessage]) -> String {
        if messages.is_empty() {
            return String::new();
        }
        let mut ctx = String::from("\n[Queued messages received during execution]\n");
        for msg in messages {
            let sender = msg.sender.as_deref().unwrap_or("user");
            ctx.push_str(&format!("- {}: {}\n", sender, msg.text));
        }
        ctx
    }
}

impl Default for HandMessageQueue {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_queue_empty() {
        let q = HandMessageQueue::new();
        assert_eq!(q.active_count(), 0);
        assert!(!q.is_hand_active(123));
        assert_eq!(q.queue_len(123), 0);
    }

    #[test]
    fn test_start_and_finish_hand() {
        let q = HandMessageQueue::new();
        q.start_hand(123, "lead");
        assert!(q.is_hand_active(123));
        assert_eq!(q.active_hand(123), Some("lead".to_string()));
        assert_eq!(q.active_count(), 1);

        q.finish_hand(123);
        assert!(!q.is_hand_active(123));
        assert_eq!(q.active_hand(123), None);
        assert_eq!(q.active_count(), 0);
    }

    #[test]
    fn test_enqueue_requires_active_hand() {
        let q = HandMessageQueue::new();
        // No active hand — should not queue
        assert!(!q.enqueue(123, "hello".to_string(), None));
        assert_eq!(q.queue_len(123), 0);
    }

    #[test]
    fn test_enqueue_and_drain() {
        let q = HandMessageQueue::new();
        q.start_hand(123, "researcher");

        assert!(q.enqueue(123, "msg 1".to_string(), Some("Alice".to_string())));
        assert!(q.enqueue(123, "msg 2".to_string(), None));
        assert_eq!(q.queue_len(123), 2);

        let messages = q.drain(123);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].text, "msg 1");
        assert_eq!(messages[0].sender, Some("Alice".to_string()));
        assert_eq!(messages[1].text, "msg 2");

        // Queue should be empty after drain
        assert_eq!(q.queue_len(123), 0);
    }

    #[test]
    fn test_peek_does_not_remove() {
        let q = HandMessageQueue::new();
        q.start_hand(123, "content");
        q.enqueue(123, "hello".to_string(), None);

        let peeked = q.peek(123);
        assert_eq!(peeked.len(), 1);
        assert_eq!(q.queue_len(123), 1); // still there
    }

    #[test]
    fn test_max_queue_size() {
        let q = HandMessageQueue::with_max(3);
        q.start_hand(123, "test");

        q.enqueue(123, "msg 1".to_string(), None);
        q.enqueue(123, "msg 2".to_string(), None);
        q.enqueue(123, "msg 3".to_string(), None);
        assert_eq!(q.queue_len(123), 3);

        // 4th message should drop oldest
        q.enqueue(123, "msg 4".to_string(), None);
        assert_eq!(q.queue_len(123), 3);

        let messages = q.drain(123);
        assert_eq!(messages[0].text, "msg 2"); // "msg 1" was dropped
        assert_eq!(messages[2].text, "msg 4");
    }

    #[test]
    fn test_multiple_chats_independent() {
        let q = HandMessageQueue::new();
        q.start_hand(100, "hand_a");
        q.start_hand(200, "hand_b");

        q.enqueue(100, "chat 100 msg".to_string(), None);
        q.enqueue(200, "chat 200 msg 1".to_string(), None);
        q.enqueue(200, "chat 200 msg 2".to_string(), None);

        assert_eq!(q.queue_len(100), 1);
        assert_eq!(q.queue_len(200), 2);

        let messages_100 = q.drain(100);
        assert_eq!(messages_100.len(), 1);
        assert_eq!(q.queue_len(200), 2); // unaffected
    }

    #[test]
    fn test_drain_empty_queue() {
        let q = HandMessageQueue::new();
        let messages = q.drain(999);
        assert!(messages.is_empty());
    }

    #[test]
    fn test_format_as_context_empty() {
        assert_eq!(HandMessageQueue::format_as_context(&[]), "");
    }

    #[test]
    fn test_format_as_context() {
        let messages = vec![
            QueuedMessage {
                chat_id: 123,
                text: "What about X?".to_string(),
                timestamp: Instant::now(),
                sender: Some("Alice".to_string()),
            },
            QueuedMessage {
                chat_id: 123,
                text: "Also check Y".to_string(),
                timestamp: Instant::now(),
                sender: None,
            },
        ];
        let ctx = HandMessageQueue::format_as_context(&messages);
        assert!(ctx.contains("[Queued messages"));
        assert!(ctx.contains("Alice: What about X?"));
        assert!(ctx.contains("user: Also check Y"));
    }

    #[test]
    fn test_finish_hand_without_start() {
        let q = HandMessageQueue::new();
        q.finish_hand(999); // should not panic
        assert_eq!(q.active_count(), 0);
    }
}
