//! The `Channel` trait — uniform send/receive surface for the
//! remote-control bots (BIG-GOAL §P3).
//!
//! A `Channel` is the **wire-protocol half** of a remote: the part that
//! actually pushes bytes to Telegram / Slack / WhatsApp and validates the
//! human on the other end is allowed to issue cluster commands. The
//! agent's reply is the cluster's response to a remote-control click, not
//! the start of a chatty conversation.
//!
//! Channels are async and fallible. Errors must be classifiable: the
//! dispatcher distinguishes "channel down" (retry) from "user not allowed"
//! (drop) from "stub — not yet implemented" (skip permanently).

use async_trait::async_trait;

/// Errors a channel can return when sending or checking auth.
#[derive(Debug, thiserror::Error)]
pub enum ChannelError {
    /// Network / API error talking to the upstream service.
    #[error("transport error on {channel}: {message}")]
    Transport {
        channel: &'static str,
        message: String,
    },

    /// Upstream returned a structured error (bad token, rate limit, etc.).
    #[error("upstream error on {channel}: {message}")]
    Upstream {
        channel: &'static str,
        message: String,
    },

    /// The channel adapter exists but isn't wired up yet (stub).
    /// Used by WhatsApp/Slack until full implementations land.
    #[error("{channel} channel is not yet implemented: {reason}")]
    NotImplemented {
        channel: &'static str,
        reason: &'static str,
    },

    /// Local token-bucket limiter rejected the send (B9/T90).
    /// `retry_after_sec` is the wall-clock time the caller should wait before
    /// retrying. This is always emitted *before* any network call so it never
    /// counts against upstream API quotas.
    #[error("{channel} channel rate-limited locally; retry in {retry_after_sec:.3}s")]
    RateLimited {
        channel: &'static str,
        retry_after_sec: f64,
    },

    /// Target channel / chat does not exist on the upstream service.
    /// Slack: `channel_not_found`. WhatsApp: invalid phone number.
    /// Distinguished from `Upstream` so the dispatcher can drop (not retry)
    /// the message rather than wedging the channel.
    #[error("channel-not-found on {channel}: {target}")]
    NotFound {
        channel: &'static str,
        target: String,
    },

    /// Bot is not a member of the target channel — applies to Slack public
    /// channels the bot must be invited to before posting. Drop, do not retry.
    /// Slack error code: `not_in_channel`.
    #[error("bot not in channel on {channel}: {target}")]
    NotInChannel {
        channel: &'static str,
        target: String,
    },

    /// Upstream service returned an explicit rate-limit signal (distinct from
    /// our local token bucket above). Carries the upstream-suggested retry-
    /// after duration (seconds) when one was provided.
    /// Slack error code: `rate_limited` (HTTP 429 + `Retry-After` header).
    #[error("upstream rate-limited on {channel}: retry after {retry_after_secs:?}s")]
    UpstreamRateLimited {
        channel: &'static str,
        retry_after_secs: Option<u64>,
    },
}

/// A messaging channel that can deliver text to a chat and gate users.
///
/// Implementations are expected to be `Send + Sync` so they can live in an
/// `Arc<dyn Channel>` shared across the dispatcher's tokio tasks.
#[async_trait]
pub trait Channel: Send + Sync {
    /// Stable short name used in logs + dispatch routing (e.g. `"telegram"`).
    fn name(&self) -> &str;

    /// Returns true if `user_id` is permitted to interact with this channel.
    /// Empty allowlists return true (open access). Closed allowlists return
    /// false for unknown users without making any network calls.
    fn is_user_allowed(&self, user_id: i64) -> bool;

    /// Send `text` to `chat_id`. Implementations must be safe to call
    /// concurrently from multiple tasks.
    async fn send_message(&self, chat_id: i64, text: &str) -> Result<(), ChannelError>;
}
