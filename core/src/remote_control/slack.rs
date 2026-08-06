//! Slack channel — your team's Remote Control for the cluster (BIG-GOAL
//! §P3), real Bolt-style implementation (B5 / T86).
//!
//! A Slack DM or `@spectyn` mention is a cluster command, not a chat
//! turn: the reply is whatever the agent / runtime produced for that
//! one command. This module is the wire-protocol half of the remote —
//! signing-secret verification, outbound `chat.postMessage`, and a
//! replay-windowed webhook. The voice (persona) and the work (agent)
//! both live elsewhere; see [`crate::remote_control`] module docs for the
//! full Remote Control framing.
//!
//! Outbound: POSTs to `https://slack.com/api/chat.postMessage` with a Bearer
//! `xoxb-…` bot token. Parses Slack's `{ok: bool, error: "…"}` JSON envelope
//! and maps the documented error codes (`channel_not_found`, `not_in_channel`,
//! `rate_limited`) to typed [`ChannelError`] variants so the dispatcher can
//! decide drop-vs-retry without grepping prose strings.
//!
//! Inbound: an axum `Router` exposing `POST /webhook/slack` that:
//!   1. Verifies the request was signed by Slack's signing secret using the
//!      v0 HMAC-SHA256 scheme documented at
//!      <https://api.slack.com/authentication/verifying-requests-from-slack>.
//!   2. Rejects requests whose `X-Slack-Request-Timestamp` is older than five
//!      minutes (replay-attack defense, per Slack docs).
//!   3. Extracts inbound `message` (DM) and `app_mention` (channel mention)
//!      events and pushes them onto an `mpsc::Sender<SlackInboundEvent>` the
//!      caller drains.
//!
//! ## Security
//!
//! * The bot token and signing secret never appear in `Debug` output or logs.
//! * HMAC comparison uses [`subtle::ConstantTimeEq`] to avoid timing oracles.
//! * The replay window matches Slack's own server-side recommendation (5 min).
//!
//! ## Feature gating
//!
//! Built only with `--features experimental-remote-control-slack`. Default `cargo
//! build` does not compile this file at all.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::Router;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use super::channel_trait::{Channel, ChannelError};
use super::dispatch::PersonaDispatcher;
use super::inbound_auth::{AuthError, ChannelInboundAuth, SlackInboundAuth};
use super::persona::Persona;
use super::rate_limit::PerChannelLimiter;

const CHANNEL_NAME: &str = "slack";
const POST_MESSAGE_URL: &str = "https://slack.com/api/chat.postMessage";
// NOTE: Slack's documented replay-defense window (±5 min) used to live here
// as `MAX_TIMESTAMP_SKEW_SECS`. After V3 gap 2 the inbound-auth path runs
// through `SlackInboundAuth` (see `super::inbound_auth`), which owns the
// constant as `SlackInboundAuth::MAX_TIMESTAMP_SKEW_SECS`. Removing the
// duplicate avoids drift if the window is ever tuned.

// ── Config ────────────────────────────────────────────────────────────────

/// Configuration for the Slack adapter. All three values are secrets; the
/// `Debug` impl redacts them.
#[derive(Clone)]
pub struct SlackConfig {
    /// `xoxb-…` bot user OAuth token. Required for `chat.postMessage`.
    pub bot_token: String,
    /// HMAC key used to verify inbound webhook signatures.
    pub signing_secret: String,
    /// App ID (`A012ABCDEF`). Cosmetic — used only for log lines so multiple
    /// Slack apps in one process can be told apart. Not security-critical.
    pub app_id: String,
    /// Optional per-user allowlist (Slack `U…` ids). Empty = allow all.
    pub allowed_users: Vec<String>,
    /// Override the `chat.postMessage` base URL — used by wiremock tests so
    /// requests don't actually hit api.slack.com. Production code leaves it
    /// `None`, which falls back to [`POST_MESSAGE_URL`].
    pub api_base_url: Option<String>,
}

impl std::fmt::Debug for SlackConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlackConfig")
            .field("bot_token", &"<redacted>")
            .field("signing_secret", &"<redacted>")
            .field("app_id", &self.app_id)
            .field("allowed_users", &self.allowed_users)
            .field("api_base_url", &self.api_base_url)
            .finish()
    }
}

impl SlackConfig {
    pub fn new(
        bot_token: impl Into<String>,
        signing_secret: impl Into<String>,
        app_id: impl Into<String>,
    ) -> Self {
        Self {
            bot_token: bot_token.into(),
            signing_secret: signing_secret.into(),
            app_id: app_id.into(),
            allowed_users: Vec::new(),
            api_base_url: None,
        }
    }

    /// URL used for `chat.postMessage`. Tests override this with a wiremock
    /// `MockServer::uri()`.
    fn post_message_url(&self) -> &str {
        self.api_base_url.as_deref().unwrap_or(POST_MESSAGE_URL)
    }
}

// ── Outbound: chat.postMessage ────────────────────────────────────────────

#[derive(Serialize)]
struct PostMessageRequest<'a> {
    channel: &'a str,
    text: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    thread_ts: Option<&'a str>,
}

#[derive(Deserialize)]
struct PostMessageResponse {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
}

/// Real Slack bot. Holds an HTTP client + config. Cheap to clone (the
/// reqwest client is internally `Arc`-shared).
#[derive(Clone)]
pub struct SlackBot {
    cfg: SlackConfig,
    http: reqwest::Client,
}

impl std::fmt::Debug for SlackBot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlackBot").field("cfg", &self.cfg).finish()
    }
}

impl SlackBot {
    pub fn new(cfg: SlackConfig) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("build reqwest client for SlackBot");
        Self { cfg, http }
    }

    /// Test seam — construct with a caller-provided reqwest client so tests
    /// can dial wiremock without TLS.
    pub fn with_client(cfg: SlackConfig, http: reqwest::Client) -> Self {
        Self { cfg, http }
    }

    pub fn config(&self) -> &SlackConfig {
        &self.cfg
    }

    /// POST `chat.postMessage`. Maps Slack's documented error codes onto
    /// typed [`ChannelError`] variants:
    ///   * `channel_not_found` → [`ChannelError::NotFound`]
    ///   * `not_in_channel`    → [`ChannelError::NotInChannel`]
    ///   * `rate_limited` (also surfaced as HTTP 429) → [`ChannelError::UpstreamRateLimited`]
    ///   * anything else with `ok: false` → [`ChannelError::Upstream`]
    pub async fn send_message(&self, channel: &str, text: &str) -> Result<(), ChannelError> {
        let body = PostMessageRequest {
            channel,
            text,
            thread_ts: None,
        };
        let resp = self
            .http
            .post(self.cfg.post_message_url())
            .bearer_auth(&self.cfg.bot_token)
            .json(&body)
            .send()
            .await
            .map_err(|e| ChannelError::Transport {
                channel: CHANNEL_NAME,
                message: e.to_string(),
            })?;

        // Slack returns HTTP 429 with a `Retry-After` header when the workspace
        // exceeds tier limits — handle this before parsing JSON since the
        // body may not be the usual envelope on 429.
        if resp.status() == StatusCode::TOO_MANY_REQUESTS {
            let retry_after_secs = resp
                .headers()
                .get(axum::http::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.trim().parse::<u64>().ok());
            return Err(ChannelError::UpstreamRateLimited {
                channel: CHANNEL_NAME,
                retry_after_secs,
            });
        }

        let status = resp.status();
        let bytes = resp.bytes().await.map_err(|e| ChannelError::Transport {
            channel: CHANNEL_NAME,
            message: format!("read body: {}", e),
        })?;

        let parsed: PostMessageResponse =
            serde_json::from_slice(&bytes).map_err(|e| ChannelError::Upstream {
                channel: CHANNEL_NAME,
                message: format!("non-JSON response (HTTP {}): {}", status, e),
            })?;

        if parsed.ok {
            return Ok(());
        }

        let code = parsed.error.unwrap_or_else(|| "unknown_error".to_string());
        Err(classify_slack_error(&code, channel))
    }
}

/// Map a Slack `error` code (with the request's target channel for context)
/// onto the appropriate [`ChannelError`] variant. Public for unit-testing.
pub fn classify_slack_error(code: &str, target_channel: &str) -> ChannelError {
    match code {
        "channel_not_found" => ChannelError::NotFound {
            channel: CHANNEL_NAME,
            target: target_channel.to_string(),
        },
        "not_in_channel" => ChannelError::NotInChannel {
            channel: CHANNEL_NAME,
            target: target_channel.to_string(),
        },
        "rate_limited" | "ratelimited" => ChannelError::UpstreamRateLimited {
            channel: CHANNEL_NAME,
            retry_after_secs: None,
        },
        other => ChannelError::Upstream {
            channel: CHANNEL_NAME,
            message: other.to_string(),
        },
    }
}

// ── `Channel` trait wiring ─────────────────────────────────────────────────
//
// The trait API is integer-id-based (legacy from the Telegram-first design).
// Slack channel IDs are strings, so we accept the i64 as a placeholder and
// surface a `NotImplemented` error if a caller routes through the trait
// without using the Slack-native `SlackBot::send_message(&str, &str)` API.
// Real callers (the dispatcher) will call the native method directly once
// the channel-id mapping work in V3 lands.
#[async_trait]
impl Channel for SlackBot {
    fn name(&self) -> &str {
        CHANNEL_NAME
    }

    fn is_user_allowed(&self, _user_id: i64) -> bool {
        // Slack user IDs are strings — there's nothing meaningful to check
        // against here. Always allow at the trait level; real auth happens
        // in `webhook_router` via `is_slack_user_allowed`.
        true
    }

    async fn send_message(&self, _chat_id: i64, _text: &str) -> Result<(), ChannelError> {
        Err(ChannelError::NotImplemented {
            channel: CHANNEL_NAME,
            reason: "use SlackBot::send_message(&str channel_id, &str text) — \
                     the i64-based Channel trait does not carry Slack channel IDs",
        })
    }
}

/// True iff `user_id` (a Slack `U…` string) is permitted by this bot's
/// allowlist. Empty allowlist = allow everyone.
pub fn is_slack_user_allowed(cfg: &SlackConfig, user_id: &str) -> bool {
    cfg.allowed_users.is_empty() || cfg.allowed_users.iter().any(|u| u == user_id)
}

// ── Inbound: HMAC-verified webhook router ─────────────────────────────────

/// One inbound event the host loop cares about. Limited to the two surfaces
/// the spec calls out (bot DMs + channel mentions); other event types are
/// silently acknowledged with 200 OK so Slack stops retrying them, but we
/// do not enqueue them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlackInboundEvent {
    pub kind: SlackEventKind,
    pub user_id: String,
    pub channel_id: String,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlackEventKind {
    /// `event.type == "message"` — typically a DM to the bot.
    Message,
    /// `event.type == "app_mention"` — `@bot` in a channel.
    AppMention,
}

#[derive(Clone)]
struct WebhookState {
    cfg: Arc<SlackConfig>,
    tx: mpsc::Sender<SlackInboundEvent>,
    /// Injectable clock — tests pin `now()` so the replay-defense math is
    /// deterministic. Production passes `system_now`.
    now_secs: fn() -> u64,
}

fn system_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Build the axum router exposing `POST /webhook/slack`. Events that pass
/// HMAC + freshness verification are forwarded onto `tx`.
pub fn webhook_router(cfg: SlackConfig, tx: mpsc::Sender<SlackInboundEvent>) -> Router {
    webhook_router_with_clock(cfg, tx, system_now)
}

/// Test-only constructor that takes a clock function.
pub fn webhook_router_with_clock(
    cfg: SlackConfig,
    tx: mpsc::Sender<SlackInboundEvent>,
    now_secs: fn() -> u64,
) -> Router {
    let state = WebhookState {
        cfg: Arc::new(cfg),
        tx,
        now_secs,
    };
    Router::new()
        .route("/webhook/slack", post(handle_webhook))
        .with_state(state)
}

/// `incoming_messages_stream` — convenience wrapper that returns both the
/// router (to mount on the host's axum app) and the `Receiver` side of the
/// channel. The caller drives the receiver in a `tokio::spawn` loop.
///
/// Buffer size of 64 mirrors the Telegram round-trip handler — Slack bursts
/// rarely exceed a handful of concurrent messages per tenant.
pub fn incoming_messages_stream(cfg: SlackConfig) -> (Router, mpsc::Receiver<SlackInboundEvent>) {
    let (tx, rx) = mpsc::channel(64);
    (webhook_router(cfg, tx), rx)
}

async fn handle_webhook(
    State(state): State<WebhookState>,
    headers: HeaderMap,
    body: Bytes,
) -> axum::response::Response {
    // 1-3. Channel-native inbound auth, routed through the cross-cutting
    // `ChannelInboundAuth` trait (V3 gap 2). The Slack impl wraps the
    // exact same `verify_slack_signature` + ±5-min replay check that
    // used to be inlined here — no wire-format change, only the call
    // surface is uniform across channels now.
    let auth: &dyn ChannelInboundAuth =
        &SlackInboundAuth::with_clock(state.cfg.signing_secret.clone(), state.now_secs);
    if let Err(e) = auth.verify_request(&headers, &body) {
        match &e {
            AuthError::MissingHeader { header, .. } => {
                tracing::warn!(app_id = %state.cfg.app_id, header, "slack-webhook: missing header");
            }
            AuthError::MalformedHeader { header, reason, .. } => {
                tracing::warn!(app_id = %state.cfg.app_id, header, reason, "slack-webhook: malformed header");
            }
            AuthError::ReplayWindow { skew_secs, .. } => {
                tracing::warn!(
                    app_id = %state.cfg.app_id,
                    skew_secs = *skew_secs,
                    "slack-webhook: timestamp outside replay window"
                );
            }
            AuthError::BadSignature { .. } => {
                tracing::warn!(app_id = %state.cfg.app_id, "slack-webhook: bad HMAC signature");
            }
            AuthError::NotImplemented { .. } => {
                // Slack always has a real impl — this branch is
                // unreachable today. Keep it explicit so a future
                // refactor that drops a stub in for Slack would still
                // produce a meaningful log line instead of silently
                // 401-ing every request.
                tracing::error!(app_id = %state.cfg.app_id, "slack-webhook: inbound auth not implemented");
            }
        }
        return StatusCode::UNAUTHORIZED.into_response();
    }

    // 4. Handle Slack's URL-verification handshake (initial endpoint setup
    //    requires echoing back the `challenge` field). This runs after auth
    //    so unauthenticated callers cannot probe the endpoint.
    if let Ok(envelope) = serde_json::from_slice::<UrlVerification>(&body) {
        if envelope.kind.as_deref() == Some("url_verification") {
            return (
                StatusCode::OK,
                [("content-type", "application/json")],
                format!(
                    r#"{{"challenge":"{}"}}"#,
                    envelope.challenge.unwrap_or_default()
                ),
            )
                .into_response();
        }
    }

    // 5. Decode an event-callback envelope; only `message` and `app_mention`
    //    types are enqueued. Other event types are 200 OK'd so Slack doesn't
    //    retry, but ignored.
    if let Ok(envelope) = serde_json::from_slice::<EventCallback>(&body) {
        if let Some(ev) = envelope.event {
            let kind = match ev.kind.as_deref() {
                Some("message") => Some(SlackEventKind::Message),
                Some("app_mention") => Some(SlackEventKind::AppMention),
                _ => None,
            };
            if let Some(kind) = kind {
                // Slack message-changed / bot_message subtypes don't carry a
                // user field we care about — skip those (subtype is present).
                if ev.subtype.is_none() {
                    let evt = SlackInboundEvent {
                        kind,
                        user_id: ev.user.unwrap_or_default(),
                        channel_id: ev.channel.unwrap_or_default(),
                        text: ev.text.unwrap_or_default(),
                    };
                    if !is_slack_user_allowed(&state.cfg, &evt.user_id) {
                        tracing::warn!(
                            app_id = %state.cfg.app_id,
                            user = %evt.user_id,
                            "slack-webhook: dropping non-allowlisted user"
                        );
                        return StatusCode::OK.into_response();
                    }
                    // `try_send` so a stalled host loop returns 503 to Slack
                    // (which will retry) rather than blocking the axum task.
                    if let Err(e) = state.tx.try_send(evt) {
                        tracing::error!(
                            app_id = %state.cfg.app_id,
                            error = %e,
                            "slack-webhook: dispatch channel full or closed"
                        );
                        return StatusCode::SERVICE_UNAVAILABLE.into_response();
                    }
                }
            }
        }
    }

    StatusCode::OK.into_response()
}

#[derive(Deserialize)]
struct UrlVerification {
    #[serde(rename = "type")]
    kind: Option<String>,
    challenge: Option<String>,
}

#[derive(Deserialize)]
struct EventCallback {
    event: Option<SlackEvent>,
}

#[derive(Deserialize)]
struct SlackEvent {
    #[serde(rename = "type")]
    kind: Option<String>,
    user: Option<String>,
    channel: Option<String>,
    text: Option<String>,
    subtype: Option<String>,
}

// ── HMAC helpers (separated for unit testing) ─────────────────────────────

/// Compute the Slack-spec basestring `v0:<timestamp>:<raw_body>` and HMAC-
/// SHA256 it with `signing_secret`, returning the lowercase hex digest
/// (without the `v0=` prefix Slack puts in the header).
pub fn compute_slack_signature(signing_secret: &str, timestamp: &str, body: &[u8]) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;

    let mut mac =
        HmacSha256::new_from_slice(signing_secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(b"v0:");
    mac.update(timestamp.as_bytes());
    mac.update(b":");
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

/// Constant-time verify the `X-Slack-Signature` header value (format
/// `v0=<hex>`) against the locally-computed digest. Returns `false` for
/// malformed headers or an empty signing secret (fail-closed).
pub fn verify_slack_signature(
    signing_secret: &str,
    timestamp: &str,
    body: &[u8],
    provided_header: &str,
) -> bool {
    use subtle::ConstantTimeEq;
    if signing_secret.is_empty() {
        return false;
    }
    let Some(provided_hex) = provided_header.strip_prefix("v0=") else {
        return false;
    };
    let expected_hex = compute_slack_signature(signing_secret, timestamp, body);
    // Length compare first (constant in body length, not in header content).
    if expected_hex.len() != provided_hex.len() {
        return false;
    }
    expected_hex
        .as_bytes()
        .ct_eq(provided_hex.as_bytes())
        .into()
}

// ── Legacy `SlackStub` (back-compat for the umbrella example) ─────────────
//
// Before B5 (T86) the Slack adapter was a compile-only stub keyed on i64 user
// IDs. The `experimental_remote_control_example` binary still imports `SlackStub`
// to exercise the i64-based `Channel` trait against a Slack-shaped type that
// returns `NotImplemented` (the example deliberately does NOT hit the real
// Slack API). Keep the type alive so the umbrella `experimental-remote-control`
// feature continues to build, while the real implementation lives in
// `SlackBot` above.

const STUB_REASON: &str =
    "SlackStub retained for legacy i64-based Channel trait; use SlackBot for real Slack traffic";

pub struct SlackStub {
    allowed_users: Vec<i64>,
    /// Optional local rate limiter (B9/T90). When set, `send_message` calls
    /// `limiter.check(CHANNEL_NAME)` *before* the (currently stubbed) network
    /// call, so the wiring contract is in place for when the real Slack impl
    /// lands.
    limiter: Option<Arc<PerChannelLimiter>>,
    /// B6 / T87: optional persona applied to dispatched agent. `None` means
    /// "no persona configured" — every dispatcher helper is a no-op so
    /// minimal-config back-compat is preserved.
    persona: Option<Persona>,
}

impl SlackStub {
    pub fn new() -> Self {
        Self {
            allowed_users: Vec::new(),
            limiter: None,
            persona: None,
        }
    }

    pub fn with_allowed_users(allowed_users: Vec<i64>) -> Self {
        Self {
            allowed_users,
            limiter: None,
            persona: None,
        }
    }

    /// Attach a shared per-channel rate limiter. Returns `self` for chaining.
    pub fn with_limiter(mut self, limiter: Arc<PerChannelLimiter>) -> Self {
        self.limiter = Some(limiter);
        self
    }

    /// Attach a persona to the stub. Channel-adapter code MUST consume the
    /// persona via [`SlackStub::dispatcher`] — see `remote_control::dispatch`
    /// for the empty-override fallback contract and the CI grep-check
    /// that enforces the anti-pattern guard.
    pub fn with_persona(mut self, persona: Persona) -> Self {
        self.persona = Some(persona);
        self
    }

    /// Borrow the persona (None if not attached).
    pub fn persona(&self) -> Option<&Persona> {
        self.persona.as_ref()
    }

    /// Build a [`PersonaDispatcher`] for this stub. With no persona the
    /// dispatcher is a pass-through (no behavior change).
    pub fn dispatcher(&self) -> PersonaDispatcher<'_> {
        PersonaDispatcher::new(self.persona.as_ref())
    }
}

impl Default for SlackStub {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Channel for SlackStub {
    fn name(&self) -> &str {
        CHANNEL_NAME
    }

    fn is_user_allowed(&self, user_id: i64) -> bool {
        self.allowed_users.is_empty() || self.allowed_users.contains(&user_id)
    }

    async fn send_message(&self, _chat_id: i64, _text: &str) -> Result<(), ChannelError> {
        if let Some(lim) = &self.limiter {
            lim.check(CHANNEL_NAME).map_err(|e| e.into_channel())?;
        }
        Err(ChannelError::NotImplemented {
            channel: CHANNEL_NAME,
            reason: STUB_REASON,
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    const TEST_TOKEN: &str = "xoxb-test-1234567890";
    const TEST_SECRET: &str = "test-signing-secret";
    const TEST_APP_ID: &str = "A0TEST";

    fn cfg() -> SlackConfig {
        SlackConfig::new(TEST_TOKEN, TEST_SECRET, TEST_APP_ID)
    }

    // ── Debug redaction ────────────────────────────────────────────────

    #[test]
    fn config_debug_redacts_secrets() {
        let s = format!("{:?}", cfg());
        assert!(
            !s.contains(TEST_TOKEN),
            "bot_token must NEVER appear in Debug output"
        );
        assert!(
            !s.contains(TEST_SECRET),
            "signing_secret must NEVER appear in Debug output"
        );
        assert!(s.contains("<redacted>"));
        assert!(s.contains(TEST_APP_ID));
    }

    #[test]
    fn bot_debug_redacts_secrets() {
        let bot = SlackBot::new(cfg());
        let s = format!("{:?}", bot);
        assert!(!s.contains(TEST_TOKEN));
        assert!(!s.contains(TEST_SECRET));
    }

    // ── Error classification ───────────────────────────────────────────

    #[test]
    fn classify_channel_not_found() {
        let e = classify_slack_error("channel_not_found", "C12345");
        match e {
            ChannelError::NotFound {
                channel: "slack",
                target,
            } => assert_eq!(target, "C12345"),
            other => panic!("expected NotFound, got {:?}", other),
        }
    }

    #[test]
    fn classify_not_in_channel() {
        let e = classify_slack_error("not_in_channel", "C99999");
        match e {
            ChannelError::NotInChannel {
                channel: "slack",
                target,
            } => assert_eq!(target, "C99999"),
            other => panic!("expected NotInChannel, got {:?}", other),
        }
    }

    #[test]
    fn classify_rate_limited() {
        assert!(matches!(
            classify_slack_error("rate_limited", "Cx"),
            ChannelError::UpstreamRateLimited {
                channel: "slack",
                ..
            }
        ));
        // Slack also occasionally uses the no-underscore spelling.
        assert!(matches!(
            classify_slack_error("ratelimited", "Cx"),
            ChannelError::UpstreamRateLimited { .. }
        ));
    }

    #[test]
    fn classify_unknown_falls_back_to_upstream() {
        match classify_slack_error("invalid_auth", "Cx") {
            ChannelError::Upstream {
                channel: "slack",
                message,
            } => assert_eq!(message, "invalid_auth"),
            other => panic!("expected Upstream, got {:?}", other),
        }
    }

    // ── Allowlist ──────────────────────────────────────────────────────

    #[test]
    fn empty_allowlist_permits_anyone() {
        let c = cfg();
        assert!(is_slack_user_allowed(&c, "U_ANYONE"));
    }

    #[test]
    fn closed_allowlist_blocks_non_matching() {
        let mut c = cfg();
        c.allowed_users = vec!["UALICE".into(), "UBOB".into()];
        assert!(is_slack_user_allowed(&c, "UALICE"));
        assert!(is_slack_user_allowed(&c, "UBOB"));
        assert!(!is_slack_user_allowed(&c, "UCARLA"));
    }

    // ── HMAC math ──────────────────────────────────────────────────────

    /// The official Slack example from
    /// <https://api.slack.com/authentication/verifying-requests-from-slack>
    /// (secret + timestamp + body → expected `v0=` digest). Pinning this
    /// guards against accidental basestring format changes.
    #[test]
    fn compute_signature_matches_known_vector() {
        let secret = "8f742231b10e8888abcd99yyyzzz85a5";
        let ts = "1531420618";
        let body = b"token=xyzz0WbapA4vBCDEFasx0q6G&team_id=T1DC2JH3J&team_domain=testteamnow&channel_id=G8PSS9T3V&channel_name=foobar&user_id=U2CERLKJA&user_name=roadrunner&command=%2Fwebhook-collect&text=&response_url=https%3A%2F%2Fhooks.slack.com%2Fcommands%2FT1DC2JH3J%2F397700885554%2F96rGlfmibIGlgcZRskXaIFfN&trigger_id=398738663015.47445629121.803a0bc887a14d10d2c447fce8b6703c";
        let got = compute_slack_signature(secret, ts, body);
        assert_eq!(
            got, "a2114d57b48eac39b9ad189dd8316235a7b4a8d21a10bd27519666489c69b503",
            "Slack-documented test vector must match"
        );
    }

    #[test]
    fn verify_round_trip_accepts_valid_signature() {
        let body = b"hello world";
        let ts = "1700000000";
        let hex = compute_slack_signature(TEST_SECRET, ts, body);
        let header = format!("v0={}", hex);
        assert!(verify_slack_signature(TEST_SECRET, ts, body, &header));
    }

    #[test]
    fn verify_rejects_wrong_signature() {
        let body = b"hello world";
        let ts = "1700000000";
        // Bad hex of correct length.
        let header = format!("v0={}", "f".repeat(64));
        assert!(!verify_slack_signature(TEST_SECRET, ts, body, &header));
    }

    #[test]
    fn verify_rejects_missing_v0_prefix() {
        let body = b"hello world";
        let ts = "1700000000";
        let hex = compute_slack_signature(TEST_SECRET, ts, body);
        // Header should be `v0=<hex>` — drop the prefix.
        assert!(!verify_slack_signature(TEST_SECRET, ts, body, &hex));
    }

    #[test]
    fn verify_rejects_empty_secret() {
        let body = b"x";
        let ts = "1700000000";
        let header = format!("v0={}", "0".repeat(64));
        assert!(!verify_slack_signature("", ts, body, &header));
    }

    #[test]
    fn verify_rejects_signature_with_mutated_body() {
        let ts = "1700000000";
        let hex = compute_slack_signature(TEST_SECRET, ts, b"original body");
        let header = format!("v0={}", hex);
        // Replay the same header against a different body — must fail.
        assert!(!verify_slack_signature(
            TEST_SECRET,
            ts,
            b"tampered body",
            &header
        ));
    }

    // ── Webhook router: HMAC + replay defense end-to-end ───────────────

    /// Fixed clock so the replay-defense math is deterministic.
    /// All timestamps in webhook tests are relative to NOW_FIXED.
    const NOW_FIXED: u64 = 1_700_000_000;
    fn fixed_now() -> u64 {
        NOW_FIXED
    }

    fn signed_request(body: &[u8], ts: u64) -> axum::http::Request<axum::body::Body> {
        let ts_str = ts.to_string();
        let hex = compute_slack_signature(TEST_SECRET, &ts_str, body);
        axum::http::Request::builder()
            .method("POST")
            .uri("/webhook/slack")
            .header("X-Slack-Signature", format!("v0={}", hex))
            .header("X-Slack-Request-Timestamp", ts_str)
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body.to_vec()))
            .unwrap()
    }

    fn event_body(kind: &str, user: &str, channel: &str, text: &str) -> Vec<u8> {
        serde_json::json!({
            "type": "event_callback",
            "event": {
                "type": kind,
                "user": user,
                "channel": channel,
                "text": text,
            }
        })
        .to_string()
        .into_bytes()
    }

    #[tokio::test]
    async fn webhook_valid_hmac_and_recent_timestamp_dispatches_message() {
        let (tx, mut rx) = mpsc::channel(8);
        let router = webhook_router_with_clock(cfg(), tx, fixed_now);
        let body = event_body("message", "UALICE", "DALICE", "ping");
        let req = signed_request(&body, NOW_FIXED);
        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let ev = rx.try_recv().expect("event should have been dispatched");
        assert_eq!(ev.kind, SlackEventKind::Message);
        assert_eq!(ev.user_id, "UALICE");
        assert_eq!(ev.channel_id, "DALICE");
        assert_eq!(ev.text, "ping");
    }

    #[tokio::test]
    async fn webhook_app_mention_dispatches() {
        let (tx, mut rx) = mpsc::channel(8);
        let router = webhook_router_with_clock(cfg(), tx, fixed_now);
        let body = event_body("app_mention", "UBOB", "CGENERAL", "<@U0BOT> hello");
        let req = signed_request(&body, NOW_FIXED);
        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let ev = rx.try_recv().expect("event should have been dispatched");
        assert_eq!(ev.kind, SlackEventKind::AppMention);
    }

    #[tokio::test]
    async fn webhook_bad_hmac_returns_401_and_does_not_dispatch() {
        let (tx, mut rx) = mpsc::channel(8);
        let router = webhook_router_with_clock(cfg(), tx, fixed_now);
        let body = event_body("message", "UALICE", "DALICE", "ping");
        // Build a request with a hand-crafted, wrong signature.
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/webhook/slack")
            .header("X-Slack-Signature", format!("v0={}", "0".repeat(64)))
            .header("X-Slack-Request-Timestamp", NOW_FIXED.to_string())
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body))
            .unwrap();
        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        assert!(
            rx.try_recv().is_err(),
            "no event must be dispatched on bad HMAC"
        );
    }

    #[tokio::test]
    async fn webhook_replay_old_timestamp_returns_401_even_with_valid_hmac() {
        // 6 minutes (>5 min window) in the past with a fully valid HMAC.
        let (tx, mut rx) = mpsc::channel(8);
        let router = webhook_router_with_clock(cfg(), tx, fixed_now);
        let body = event_body("message", "UALICE", "DALICE", "old");
        let stale_ts = NOW_FIXED - (6 * 60);
        let req = signed_request(&body, stale_ts);
        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "stale timestamps must be rejected to defeat replay attacks"
        );
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn webhook_future_timestamp_outside_window_returns_401() {
        // 6 minutes in the *future* — clock-skew defense.
        let (tx, _rx) = mpsc::channel(8);
        let router = webhook_router_with_clock(cfg(), tx, fixed_now);
        let body = event_body("message", "U", "D", "x");
        let future_ts = NOW_FIXED + (6 * 60);
        let req = signed_request(&body, future_ts);
        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn webhook_missing_signature_header_returns_401() {
        let (tx, _rx) = mpsc::channel(8);
        let router = webhook_router_with_clock(cfg(), tx, fixed_now);
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/webhook/slack")
            .header("X-Slack-Request-Timestamp", NOW_FIXED.to_string())
            .body(axum::body::Body::from(b"{}".as_ref()))
            .unwrap();
        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn webhook_url_verification_handshake_returns_challenge() {
        let (tx, _rx) = mpsc::channel(8);
        let router = webhook_router_with_clock(cfg(), tx, fixed_now);
        let body = br#"{"type":"url_verification","challenge":"abc123"}"#;
        let req = signed_request(body, NOW_FIXED);
        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let s = std::str::from_utf8(&bytes).unwrap();
        assert!(
            s.contains("abc123"),
            "url_verification must echo challenge: got {}",
            s
        );
    }

    #[tokio::test]
    async fn webhook_drops_event_from_non_allowlisted_user() {
        let mut c = cfg();
        c.allowed_users = vec!["UALICE".into()];
        let (tx, mut rx) = mpsc::channel(8);
        let router = webhook_router_with_clock(c, tx, fixed_now);
        let body = event_body("message", "UEVE", "D", "intrusion");
        let req = signed_request(&body, NOW_FIXED);
        let resp = router.oneshot(req).await.unwrap();
        // 200 OK so Slack doesn't retry — but no dispatch.
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(rx.try_recv().is_err());
    }

    // ── Outbound chat.postMessage via wiremock ─────────────────────────

    fn bot_pointed_at(uri: String) -> SlackBot {
        let mut c = cfg();
        c.api_base_url = Some(uri);
        SlackBot::new(c)
    }

    #[tokio::test]
    async fn send_message_to_valid_channel_returns_ok() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/"))
            .and(header(
                "authorization",
                format!("Bearer {}", TEST_TOKEN).as_str(),
            ))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(r#"{"ok":true,"channel":"C12345","ts":"1700000000.000100"}"#),
            )
            .mount(&mock)
            .await;
        let bot = bot_pointed_at(mock.uri());
        bot.send_message("C12345", "hello").await.expect("ok path");
    }

    #[tokio::test]
    async fn send_message_channel_not_found_returns_notfound() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(r#"{"ok":false,"error":"channel_not_found"}"#),
            )
            .mount(&mock)
            .await;
        let bot = bot_pointed_at(mock.uri());
        let err = bot.send_message("CDOESNOTEXIST", "x").await.unwrap_err();
        match err {
            ChannelError::NotFound {
                channel: "slack",
                target,
            } => assert_eq!(target, "CDOESNOTEXIST"),
            other => panic!("expected NotFound, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn send_message_not_in_channel_returns_notinchannel() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(r#"{"ok":false,"error":"not_in_channel"}"#),
            )
            .mount(&mock)
            .await;
        let bot = bot_pointed_at(mock.uri());
        let err = bot.send_message("CGENERAL", "x").await.unwrap_err();
        assert!(matches!(err, ChannelError::NotInChannel { .. }));
    }

    #[tokio::test]
    async fn send_message_http_429_returns_ratelimited_with_retry_after() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(429)
                    .insert_header("Retry-After", "30")
                    .set_body_string(r#"{"ok":false,"error":"rate_limited"}"#),
            )
            .mount(&mock)
            .await;
        let bot = bot_pointed_at(mock.uri());
        let err = bot.send_message("C", "x").await.unwrap_err();
        match err {
            ChannelError::UpstreamRateLimited {
                channel: "slack",
                retry_after_secs,
            } => assert_eq!(retry_after_secs, Some(30)),
            other => panic!("expected UpstreamRateLimited, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn send_message_invalid_auth_falls_back_to_upstream() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(r#"{"ok":false,"error":"invalid_auth"}"#),
            )
            .mount(&mock)
            .await;
        let bot = bot_pointed_at(mock.uri());
        let err = bot.send_message("C", "x").await.unwrap_err();
        assert!(matches!(err, ChannelError::Upstream { .. }));
    }

    // ── Channel trait wiring ───────────────────────────────────────────

    #[tokio::test]
    async fn channel_trait_send_message_returns_notimplemented() {
        // The i64-based trait surface is intentionally unimplemented; real
        // callers must use SlackBot::send_message(&str, &str).
        let bot = SlackBot::new(cfg());
        let err = Channel::send_message(&bot, 42, "x").await.unwrap_err();
        assert!(matches!(
            err,
            ChannelError::NotImplemented {
                channel: "slack",
                ..
            }
        ));
    }

    #[test]
    fn channel_trait_name_is_slack() {
        let bot = SlackBot::new(cfg());
        assert_eq!(<SlackBot as Channel>::name(&bot), "slack");
    }

    // ── B6/T87 persona + dispatcher (carried over from main) ──────────

    const PERSONA_FULL: &str = r#"
        [persona]
        name = "spectyn-helper"
        intro_message = "top"
        [persona.style]
        tone = "playful"
        [persona.tools]
        denied = ["bash_bg"]
        [persona.channels.slack]
        intro_message = ""
    "#;

    #[test]
    fn slack_empty_persona_dispatcher_is_noop() {
        // Spec test #1: no persona => dispatcher is a no-op.
        let s = SlackStub::new();
        let d = s.dispatcher();
        assert_eq!(d.channel_intro("slack"), None);
        assert_eq!(d.system_prompt_prefix(), "");
        let reg = ["bash_bg"];
        assert_eq!(d.filter_tools(&reg), vec!["bash_bg"]);
    }

    #[test]
    fn slack_empty_override_falls_through_to_top_level_via_dispatcher() {
        // Spec test #3 — anti-pattern guard from the *channel*-module side:
        // when the slack override is `""`, the dispatcher must fall through
        // to the top-level intro_message ("top"), not return Some("").
        let p = Persona::parse_str(PERSONA_FULL).unwrap();
        let s = SlackStub::new().with_persona(p);
        assert_eq!(s.dispatcher().channel_intro("slack"), Some("top"));
    }

    #[test]
    fn slack_denied_tool_not_callable_via_dispatcher() {
        // Spec test #4.
        let p = Persona::parse_str(PERSONA_FULL).unwrap();
        let s = SlackStub::new().with_persona(p);
        let reg = ["bash_bg", "shell"];
        let filtered = s.dispatcher().filter_tools(&reg);
        assert!(!filtered.contains(&"bash_bg"));
        assert!(filtered.contains(&"shell"));
    }
}
