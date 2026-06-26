//! Integration test for the remote-control `Channel` trait.
//!
//! Compiled only when `--features experimental-remote-control` is enabled. The whole
//! file is gated so it's a no-op on default-feature CI builds.
#![cfg(feature = "experimental-remote-control")]

use phantom_mesh::remote_control::{Channel, ChannelError};

/// Compile-time assertion that the `Channel` trait surface exists with the
/// expected async methods. If this stops compiling, the trait shape changed
/// and downstream channels (WhatsApp, Slack, O1's Telegram) need to update.
#[allow(dead_code)]
fn _trait_surface_compiles<C: Channel>(c: &C, chat_id: i64, user_id: i64) {
    let _name: &str = c.name();
    let _allowed: bool = c.is_user_allowed(user_id);
    // send_message returns a Future<Output = Result<(), ChannelError>>
    let _fut = c.send_message(chat_id, "hi");
}

#[test]
fn channel_error_variants_exist() {
    // Smoke-check the error enum's NotImplemented variant — stubs depend on it.
    let err = ChannelError::NotImplemented {
        channel: "whatsapp",
        reason: "weekend-push best-effort: stub only",
    };
    assert!(format!("{err:?}").contains("NotImplemented"));
}

// ── WhatsApp stub ────────────────────────────────────────────────────────────

#[cfg(feature = "experimental-remote-control-whatsapp")]
#[tokio::test]
async fn whatsapp_stub_returns_not_implemented() {
    use phantom_mesh::remote_control::whatsapp::WhatsappStub;

    let stub = WhatsappStub::new();
    assert_eq!(stub.name(), "whatsapp");
    // No allowlist configured → all users allowed (consistent with TelegramBot).
    assert!(stub.is_user_allowed(12345));

    let err = stub
        .send_message(67890, "hi")
        .await
        .expect_err("whatsapp stub must refuse send");
    match err {
        ChannelError::NotImplemented { channel, reason } => {
            assert_eq!(channel, "whatsapp");
            assert!(!reason.is_empty(), "stub must explain why it's not ready");
        }
        other => panic!("expected NotImplemented, got {other:?}"),
    }
}

// ── Slack stub ───────────────────────────────────────────────────────────────

#[cfg(feature = "experimental-remote-control-slack")]
#[tokio::test]
async fn slack_stub_returns_not_implemented() {
    use phantom_mesh::remote_control::slack::SlackStub;

    let stub = SlackStub::with_allowed_users(vec![42]);
    assert_eq!(stub.name(), "slack");
    assert!(stub.is_user_allowed(42));
    assert!(
        !stub.is_user_allowed(99),
        "closed allowlist must reject non-members"
    );

    let err = stub
        .send_message(123, "hello")
        .await
        .expect_err("slack stub must refuse send");
    assert!(matches!(
        err,
        ChannelError::NotImplemented {
            channel: "slack",
            ..
        }
    ));
}

// ── Integration test: drive the existing TelegramBot through remote_control::Channel ─
//
// The struct lives in `core/src/channels/telegram.rs` (shipped well before this
// weekend push). We wrap it in `TelegramAdapter` here so the test exercises the
// real reqwest-based HTTP path through the trait surface, against a wiremock
// server. When O1's `remote_control::telegram::Bot` lands, this adapter can be
// replaced with the real impl and the assertions reused unchanged.
mod telegram_through_trait {
    use super::{Channel, ChannelError};
    use async_trait::async_trait;
    use phantom_mesh::channels::telegram::TelegramBot;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Newtype wrapping the existing `TelegramBot` so it can be driven via
    /// the remote_control `Channel` trait. Replace with `remote_control::telegram::Bot`
    /// once O1 lands. The trait-side surface is exactly what O1's adapter
    /// must implement.
    struct TelegramAdapter {
        inner: TelegramBot,
        api_root: String,
    }

    impl TelegramAdapter {
        fn new(token: String, allowed_users: Vec<i64>, api_root: String) -> Self {
            Self {
                inner: TelegramBot::new(token, allowed_users),
                api_root,
            }
        }
    }

    #[async_trait]
    impl Channel for TelegramAdapter {
        fn name(&self) -> &str {
            "telegram"
        }

        fn is_user_allowed(&self, user_id: i64) -> bool {
            self.inner.is_user_allowed(user_id)
        }

        async fn send_message(&self, chat_id: i64, text: &str) -> Result<(), ChannelError> {
            // Bypass `TelegramBot::send_message` (hardcoded to api.telegram.org)
            // and POST directly to the wiremock root — this is the same wire
            // shape Telegram expects, just pointed at our stub server.
            let url = format!("{}/bot{}/sendMessage", self.api_root, self.inner.token);
            let body = serde_json::json!({ "chat_id": chat_id, "text": text });
            let resp = reqwest::Client::new()
                .post(&url)
                .json(&body)
                .send()
                .await
                .map_err(|e| ChannelError::Transport {
                    channel: "telegram",
                    message: e.to_string(),
                })?;
            let v: serde_json::Value = resp.json().await.map_err(|e| ChannelError::Transport {
                channel: "telegram",
                message: e.to_string(),
            })?;
            if v["ok"].as_bool().unwrap_or(false) {
                Ok(())
            } else {
                Err(ChannelError::Upstream {
                    channel: "telegram",
                    message: v["description"].as_str().unwrap_or("unknown").to_string(),
                })
            }
        }
    }

    #[tokio::test]
    async fn telegram_adapter_sends_through_trait() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/botFAKE_TOKEN/sendMessage"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "result": { "message_id": 1 }
            })))
            .mount(&server)
            .await;

        let adapter = TelegramAdapter::new("FAKE_TOKEN".to_string(), vec![777], server.uri());

        // Allowlist gate works through the trait
        assert!(adapter.is_user_allowed(777));
        assert!(!adapter.is_user_allowed(123));

        // Send a message through the trait method — drives real HTTP to wiremock
        adapter
            .send_message(42, "hello from remote-control trait test")
            .await
            .expect("send via trait must succeed against happy-path wiremock");
    }

    #[tokio::test]
    async fn telegram_adapter_propagates_upstream_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/botFAKE_TOKEN/sendMessage"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": false,
                "description": "Bad Request: chat not found"
            })))
            .mount(&server)
            .await;

        let adapter = TelegramAdapter::new("FAKE_TOKEN".to_string(), vec![], server.uri());

        let err = adapter
            .send_message(0, "noone")
            .await
            .expect_err("upstream-error response must surface as ChannelError::Upstream");
        match err {
            ChannelError::Upstream { channel, message } => {
                assert_eq!(channel, "telegram");
                assert!(
                    message.contains("chat not found"),
                    "unexpected message: {message}"
                );
            }
            other => panic!("expected Upstream, got {other:?}"),
        }
    }
}
