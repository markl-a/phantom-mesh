//! Track [T1] — Phantom agent dispatcher for the OpenClaw Telegram channel.
//!
//! The cluster-side of Telegram Remote Control (BIG-GOAL §P3): wires the
//! Telegram remote up to the actual `AgentRuntime` so a chat message
//! becomes a real cluster task, with the agent's textual output flowing
//! straight back to the same chat. Without this bridge the bot would
//! merely echo; with it, the bot *is* the cluster speaking back.
//!
//! Bridges `OpenclawTelegramBot` to the phantom `AgentRuntime`:
//! incoming text → `runtime.run(agent_name, text, history, None)` →
//! `result.output` → response back to the same Telegram chat.
//!
//! See `docs/superpowers/plans/2026-05-15-track-t1-telegram-dispatch.md`.
//!
//! ## v1 scope
//!
//! - In-memory `HashMap<chat_id, Vec<ChatMessage>>` for per-chat history.
//! - Bounded ring (default 40 messages ≈ 20 user/assistant turns) so a
//!   long-lived chat does not balloon RSS.
//! - **No persistence.** A process restart resets every chat to turn 0.
//!   That's an explicit non-goal for the weekend cut; persistence is a
//!   follow-up issue.
//!
//! ## Concurrency
//!
//! The history map is wrapped in `tokio::sync::Mutex` because the
//! dispatcher must drop the lock across `runtime.run(...).await` —
//! holding it would serialise all chats. The lock is only held while
//! cloning the current history snapshot OUT and while pushing the new
//! turn IN; the agent call runs lock-free in between.
//!
//! ## Token safety
//!
//! This file never sees the Telegram bot token. No new redaction
//! surface is added; we only handle user-typed text + chat_ids.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::agent::AgentRuntime;
use crate::openclaw::telegram::OpenclawDispatcher;
use crate::providers::traits::ChatMessage;

/// Default per-chat history bound: 40 messages ≈ 20 round-trips.
/// Tuned conservatively to keep memory bounded; LLM context-window
/// pressure also caps useful retention well before this.
pub const DEFAULT_HISTORY_LIMIT: usize = 40;

/// Dispatcher that invokes the real phantom `AgentRuntime` for each
/// incoming Telegram message and maintains per-chat conversation
/// history in-memory.
pub struct PhantomAgentDispatcher {
    runtime: Arc<AgentRuntime>,
    agent_name: String,
    history_limit: usize,
    history: Arc<Mutex<HashMap<i64, Vec<ChatMessage>>>>,
}

impl PhantomAgentDispatcher {
    /// Construct with the default per-chat history bound
    /// ([`DEFAULT_HISTORY_LIMIT`]).
    pub fn new(runtime: Arc<AgentRuntime>, agent_name: String) -> Self {
        Self::new_with_limit(runtime, agent_name, DEFAULT_HISTORY_LIMIT)
    }

    /// Construct with an explicit per-chat history bound. Use 0 to
    /// disable history (every message is treated as a fresh turn-0
    /// conversation) — useful for tests and stateless deployments.
    pub fn new_with_limit(
        runtime: Arc<AgentRuntime>,
        agent_name: String,
        history_limit: usize,
    ) -> Self {
        Self {
            runtime,
            agent_name,
            history_limit,
            history: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Test-only inspector: returns a copy of the current history for
    /// `chat_id` (empty `Vec` if the chat is unknown).
    pub async fn history_for(&self, chat_id: i64) -> Vec<ChatMessage> {
        self.history
            .lock()
            .await
            .get(&chat_id)
            .cloned()
            .unwrap_or_default()
    }
}

#[async_trait]
impl OpenclawDispatcher for PhantomAgentDispatcher {
    /// The legacy single-arg form is unused by this dispatcher — chat
    /// context is keyed on chat_id, which only `dispatch_with_chat`
    /// receives. Return a clear error if the trait's default method is
    /// ever bypassed.
    async fn dispatch(&self, _user_text: String) -> Result<String, String> {
        Err(
            "PhantomAgentDispatcher requires chat_id context — call dispatch_with_chat instead"
                .to_string(),
        )
    }

    async fn dispatch_with_chat(&self, chat_id: i64, user_text: String) -> Result<String, String> {
        // 1. Snapshot the chat's prior history WITHOUT holding the lock
        //    across the agent call.
        let history_snapshot: Vec<ChatMessage> = if self.history_limit == 0 {
            Vec::new()
        } else {
            self.history
                .lock()
                .await
                .get(&chat_id)
                .cloned()
                .unwrap_or_default()
        };

        // 2. Invoke the agent runtime with (current message, prior history).
        let result = self
            .runtime
            .run(&self.agent_name, &user_text, &history_snapshot, None)
            .await
            .map_err(|e| e.to_string())?;

        // 3. Push the new (user, assistant) turn into the chat's history,
        //    bounded by `history_limit`.
        if self.history_limit > 0 {
            let mut guard = self.history.lock().await;
            let chat_hist = guard.entry(chat_id).or_default();
            chat_hist.push(ChatMessage {
                role: "user".into(),
                content: user_text,
                tool_calls: None,
            });
            chat_hist.push(ChatMessage {
                role: "assistant".into(),
                content: result.output.clone(),
                tool_calls: None,
            });
            // Trim oldest entries while exceeding the bound. Pop from the
            // front in pairs to keep the user/assistant alignment so the
            // LLM never sees a dangling assistant message.
            while chat_hist.len() > self.history_limit {
                // Drop the head user message; then if a stray assistant
                // is still at the front (i.e. we had a leftover pair),
                // drop it too to keep alignment.
                chat_hist.remove(0);
                if chat_hist.len() > self.history_limit
                    && chat_hist.first().map(|m| m.role.as_str()) == Some("assistant")
                {
                    chat_hist.remove(0);
                }
            }
        }

        Ok(result.output)
    }
}

// ── In-file unit tests (no network) ───────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AgentsConfig;

    #[test]
    fn default_history_limit_is_40() {
        assert_eq!(DEFAULT_HISTORY_LIMIT, 40);
    }

    /// `dispatch` (the legacy single-arg path) must NOT be the entry
    /// point — it returns an explanatory error so a misconfigured
    /// caller doesn't silently bypass per-chat context.
    #[tokio::test]
    async fn legacy_dispatch_returns_explanatory_error() {
        let rt = Arc::new(AgentRuntime::new(AgentsConfig::default()));
        let d = PhantomAgentDispatcher::new(rt, "master".into());
        let err = d.dispatch("hi".into()).await.unwrap_err();
        assert!(err.contains("dispatch_with_chat"), "unexpected: {}", err);
    }
}
