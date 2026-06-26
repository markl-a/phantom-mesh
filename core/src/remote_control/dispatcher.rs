//! Multi-bot dispatcher — keyed `HashMap<bot_id, Arc<dyn Channel>>`.
//!
//! In Remote Control terms (BIG-GOAL §P3) this is the "many remotes, one
//! cluster" piece: one phantom process can host every remote the operator
//! pairs to the mesh — `telegram:phantom_test_bot`, `slack:T01ABC123`,
//! `whatsapp:+15551234567` — and route each inbound command to the right
//! handler without the per-channel code knowing the others exist.
//!
//! Track **B7 / T88** (2026-05-16). Promotes remote control from "one bot per
//! process" to "one process can host multiple bots across multiple channel
//! adapters" — v0.6.0 V3 prerequisite for per-channel personas.
//!
//! # Design
//!
//! Each registered channel is keyed by a stable `bot_id` string such as:
//!
//! - `"telegram:phantom_test_bot"`
//! - `"slack:T01ABC123"`
//! - `"whatsapp:+15551234567"`
//!
//! Convention is `"<channel_name>:<channel_specific_handle>"`, but the
//! dispatcher does NOT parse the key — it is opaque.
//!
//! # Back-compat
//!
//! The single-bot constructor [`RemoteDispatcher::single`] builds a
//! one-entry HashMap, so existing wiring (single bot per process) can
//! migrate by wrapping the bot's `Channel` impl into the dispatcher with
//! a single line. `RemoteTelegramBot::handle_text(user_id, text)` from
//! O1 is intentionally NOT touched — the dispatcher routes at the
//! outbound `send_message` surface, not at the inbound handler surface.
//!
//! # Concurrency
//!
//! [`RemoteDispatcher`] is `Send + Sync` and cheap to clone — the
//! `HashMap` lives behind `Arc<RwLock<…>>` so concurrent `send_to` reads
//! never block each other. Register/unregister briefly hold the write
//! lock (operator-initiated hot-reload).

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

use super::channel_trait::{Channel, ChannelError};

/// Errors specific to multi-bot routing, layered on top of [`ChannelError`].
#[derive(Debug, thiserror::Error)]
pub enum DispatchError {
    /// Caller passed a `bot_id` that is not registered. Routing/config
    /// error, distinct from a transport failure against a real bot.
    #[error("unknown bot_id: {0}")]
    UnknownBot(String),

    /// The underlying channel returned an error when sending.
    #[error(transparent)]
    Channel(#[from] ChannelError),
}

/// Multi-bot dispatcher. Holds an internal
/// `HashMap<bot_id, Arc<dyn Channel>>` and routes outbound sends by id.
#[derive(Clone)]
pub struct RemoteDispatcher {
    channels: Arc<RwLock<HashMap<String, Arc<dyn Channel>>>>,
}

impl RemoteDispatcher {
    /// Construct an empty dispatcher.
    pub fn new() -> Self {
        Self {
            channels: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Back-compat thin wrapper: one bot, one dispatcher.
    pub fn single(bot_id: impl Into<String>, channel: Arc<dyn Channel>) -> Self {
        let mut map = HashMap::new();
        map.insert(bot_id.into(), channel);
        Self {
            channels: Arc::new(RwLock::new(map)),
        }
    }

    /// Register `channel` under `bot_id`. If a bot was already registered
    /// under that id, it is replaced and the previous Arc is returned so
    /// the caller can drain / shut it down.
    pub async fn register(
        &self,
        bot_id: impl Into<String>,
        channel: Arc<dyn Channel>,
    ) -> Option<Arc<dyn Channel>> {
        let mut guard = self.channels.write().await;
        guard.insert(bot_id.into(), channel)
    }

    /// Unregister and return the channel previously bound to `bot_id`.
    pub async fn unregister(&self, bot_id: &str) -> Option<Arc<dyn Channel>> {
        let mut guard = self.channels.write().await;
        guard.remove(bot_id)
    }

    /// Returns true if `bot_id` is currently registered.
    pub async fn contains(&self, bot_id: &str) -> bool {
        let guard = self.channels.read().await;
        guard.contains_key(bot_id)
    }

    /// Number of registered bots.
    pub async fn len(&self) -> usize {
        let guard = self.channels.read().await;
        guard.len()
    }

    /// True if no bots are registered.
    pub async fn is_empty(&self) -> bool {
        let guard = self.channels.read().await;
        guard.is_empty()
    }

    /// Sorted snapshot of registered bot ids — for diagnostics + tests.
    pub async fn bot_ids(&self) -> Vec<String> {
        let guard = self.channels.read().await;
        let mut ids: Vec<String> = guard.keys().cloned().collect();
        ids.sort();
        ids
    }

    /// Route an outbound message: look up the channel by `bot_id` and call
    /// its `send_message`. Returns [`DispatchError::UnknownBot`] if not
    /// registered (a config error, NOT a transport error).
    ///
    /// The lookup clones the `Arc<dyn Channel>` and releases the read lock
    /// *before* awaiting `send_message`, so a slow upstream cannot block
    /// concurrent sends to other bots.
    pub async fn send_to(
        &self,
        bot_id: &str,
        chat_id: i64,
        text: &str,
    ) -> Result<(), DispatchError> {
        let channel = {
            let guard = self.channels.read().await;
            guard
                .get(bot_id)
                .cloned()
                .ok_or_else(|| DispatchError::UnknownBot(bot_id.to_string()))?
        };
        channel.send_message(chat_id, text).await?;
        Ok(())
    }
}

impl Default for RemoteDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod multi_bot {
    //! Multi-bot routing tests. Path: `remote_control::dispatcher::multi_bot`.
    //!
    //! Run: cargo test --features experimental-remote-control \
    //!         remote_control::dispatcher::multi_bot

    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;

    /// In-memory test channel that records every `send_message` call.
    struct RecordingChannel {
        name: &'static str,
        allowed_users: Vec<i64>,
        sent: Arc<Mutex<Vec<(i64, String)>>>,
    }

    impl RecordingChannel {
        fn new(name: &'static str) -> (Arc<Self>, Arc<Mutex<Vec<(i64, String)>>>) {
            let sent = Arc::new(Mutex::new(Vec::new()));
            let ch = Arc::new(Self {
                name,
                allowed_users: Vec::new(),
                sent: sent.clone(),
            });
            (ch, sent)
        }

        fn with_allowlist(
            name: &'static str,
            allowed_users: Vec<i64>,
        ) -> (Arc<Self>, Arc<Mutex<Vec<(i64, String)>>>) {
            let sent = Arc::new(Mutex::new(Vec::new()));
            let ch = Arc::new(Self {
                name,
                allowed_users,
                sent: sent.clone(),
            });
            (ch, sent)
        }
    }

    #[async_trait]
    impl Channel for RecordingChannel {
        fn name(&self) -> &str {
            self.name
        }
        fn is_user_allowed(&self, user_id: i64) -> bool {
            self.allowed_users.is_empty() || self.allowed_users.contains(&user_id)
        }
        async fn send_message(&self, chat_id: i64, text: &str) -> Result<(), ChannelError> {
            self.sent.lock().unwrap().push((chat_id, text.to_string()));
            Ok(())
        }
    }

    /// CRITICAL spec test: two Telegram-style bots from one dispatcher;
    /// each receives only its own messages (no cross-talk).
    #[tokio::test]
    async fn two_bots_receive_only_their_own_messages() {
        let (bot_a, sent_a) = RecordingChannel::new("telegram");
        let (bot_b, sent_b) = RecordingChannel::new("telegram");

        let dispatcher = RemoteDispatcher::new();
        dispatcher
            .register("telegram:phantom_test_bot", bot_a)
            .await;
        dispatcher
            .register("telegram:phantom_other_bot", bot_b)
            .await;

        dispatcher
            .send_to("telegram:phantom_test_bot", 100, "hello A")
            .await
            .unwrap();
        dispatcher
            .send_to("telegram:phantom_other_bot", 200, "hello B")
            .await
            .unwrap();
        dispatcher
            .send_to("telegram:phantom_test_bot", 100, "second to A")
            .await
            .unwrap();

        let a = sent_a.lock().unwrap();
        let b = sent_b.lock().unwrap();
        assert_eq!(
            &*a,
            &[(100i64, "hello A".to_string()), (100, "second to A".into())]
        );
        assert_eq!(&*b, &[(200i64, "hello B".to_string())]);
    }

    /// Back-compat: the `single` constructor builds a 1-entry dispatcher
    /// that routes as the pre-B7 "one bot per process" code did.
    #[tokio::test]
    async fn single_bot_constructor_round_trips() {
        let (bot, sent) = RecordingChannel::new("telegram");
        let dispatcher = RemoteDispatcher::single("telegram:only", bot);

        assert_eq!(dispatcher.len().await, 1);
        assert!(dispatcher.contains("telegram:only").await);
        dispatcher.send_to("telegram:only", 7, "hi").await.unwrap();

        let s = sent.lock().unwrap();
        assert_eq!(&*s, &[(7i64, "hi".to_string())]);
    }

    /// Back-compat: per-channel allowlist semantics still work — empty
    /// allowlist on the underlying `Channel` permits anyone. Mirrors the
    /// `allowlist_empty_permits_anyone` invariant baked into every channel
    /// adapter (slack/whatsapp/telegram); the dispatcher delegates auth to
    /// the channel; `is_user_allowed` is unchanged.
    #[tokio::test]
    async fn allowlist_empty_permits_anyone_via_dispatcher() {
        let (bot, _sent) = RecordingChannel::new("telegram");
        let dispatcher = RemoteDispatcher::single("telegram:open", bot.clone());

        // Empty allowlist → every user_id permitted.
        assert!(bot.is_user_allowed(1));
        assert!(bot.is_user_allowed(99_999));

        // Routing works — no allowlist rejection at dispatcher layer.
        dispatcher
            .send_to("telegram:open", 42, "anyone goes")
            .await
            .unwrap();
    }

    /// Unknown bot_id is a config error, NOT a transport error. Distinct
    /// variant so operators can tell "I typo'd the bot id" from
    /// "Telegram is down".
    #[tokio::test]
    async fn send_to_unknown_bot_id_returns_unknown_bot_error() {
        let dispatcher = RemoteDispatcher::new();
        let err = dispatcher
            .send_to("telegram:does_not_exist", 1, "msg")
            .await
            .unwrap_err();
        match err {
            DispatchError::UnknownBot(id) => assert_eq!(id, "telegram:does_not_exist"),
            other => panic!("expected UnknownBot, got {other:?}"),
        }
    }

    /// Runtime hot-removal: unregistering a bot makes subsequent sends
    /// return UnknownBot, without affecting other registered bots.
    #[tokio::test]
    async fn unregister_removes_bot_at_runtime() {
        let (bot_a, sent_a) = RecordingChannel::new("telegram");
        let (bot_b, sent_b) = RecordingChannel::new("telegram");

        let dispatcher = RemoteDispatcher::new();
        dispatcher.register("telegram:a", bot_a).await;
        dispatcher.register("telegram:b", bot_b).await;
        assert_eq!(dispatcher.len().await, 2);

        let removed = dispatcher.unregister("telegram:a").await;
        assert!(removed.is_some(), "unregister must return removed channel");
        assert_eq!(dispatcher.len().await, 1);
        assert!(!dispatcher.contains("telegram:a").await);
        assert!(dispatcher.contains("telegram:b").await);

        // Send to removed bot fails with UnknownBot.
        let err = dispatcher
            .send_to("telegram:a", 1, "ghost")
            .await
            .unwrap_err();
        assert!(matches!(err, DispatchError::UnknownBot(_)));

        // Surviving bot still receives traffic.
        dispatcher
            .send_to("telegram:b", 2, "still here")
            .await
            .unwrap();
        assert_eq!(sent_a.lock().unwrap().len(), 0);
        assert_eq!(
            &*sent_b.lock().unwrap(),
            &[(2i64, "still here".to_string())]
        );

        // Re-unregistering an absent bot is a no-op, not a panic.
        assert!(dispatcher.unregister("telegram:a").await.is_none());
    }

    /// Registering twice under the same id replaces the previous channel
    /// and returns it so the caller can drain/shut it down.
    #[tokio::test]
    async fn register_returns_previous_channel_on_replace() {
        let (bot_v1, sent_v1) = RecordingChannel::new("telegram");
        let (bot_v2, sent_v2) = RecordingChannel::new("telegram");

        let dispatcher = RemoteDispatcher::new();
        let first = dispatcher.register("telegram:foo", bot_v1).await;
        assert!(
            first.is_none(),
            "first register should not displace anything"
        );

        let previous = dispatcher.register("telegram:foo", bot_v2).await;
        assert!(
            previous.is_some(),
            "second register must return prior channel"
        );

        // After replacement, sends go to v2 only.
        dispatcher
            .send_to("telegram:foo", 1, "to v2")
            .await
            .unwrap();
        assert_eq!(sent_v1.lock().unwrap().len(), 0);
        assert_eq!(&*sent_v2.lock().unwrap(), &[(1i64, "to v2".to_string())]);
    }

    /// `bot_ids` returns a sorted snapshot — used by the future
    /// `phantom remote list` command for stable diff-able output.
    #[tokio::test]
    async fn bot_ids_returns_sorted_snapshot() {
        let dispatcher = RemoteDispatcher::new();
        let (a, _) = RecordingChannel::new("telegram");
        let (b, _) = RecordingChannel::new("slack");
        let (c, _) = RecordingChannel::new("whatsapp");
        dispatcher.register("z:last", a).await;
        dispatcher.register("a:first", b).await;
        dispatcher.register("m:middle", c).await;

        assert_eq!(
            dispatcher.bot_ids().await,
            vec!["a:first".to_string(), "m:middle".into(), "z:last".into()]
        );
    }

    /// `Clone` produces a cheap handle sharing the same registry.
    #[tokio::test]
    async fn clone_shares_same_registry() {
        let (bot, _) = RecordingChannel::with_allowlist("telegram", vec![42]);
        let d1 = RemoteDispatcher::new();
        let d2 = d1.clone();

        d1.register("telegram:shared", bot).await;
        assert!(
            d2.contains("telegram:shared").await,
            "registration via d1 must be visible to clone d2"
        );

        d2.unregister("telegram:shared").await;
        assert!(
            !d1.contains("telegram:shared").await,
            "unregistration via d2 must be visible to original d1"
        );
    }
}
