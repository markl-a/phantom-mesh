//! Cross-cutting inbound-auth trait (DEMO-1 gap 2 / V3 gap) — the
//! "is this remote actually paired to me?" check that gates every
//! Remote Control (BIG-GOAL §P3) command before it reaches the cluster.
//!
//! Every channel adapter (Telegram / Slack / WhatsApp) is a remote into
//! the mesh; this trait is the *one* shape every adapter must implement
//! to prove an inbound webhook came from the real upstream and not an
//! attacker who guessed the URL. Wrong remote → drop on the floor;
//! correct remote → forward to the dispatcher.
//!
//! Before this module, every channel had its own bespoke webhook-auth path:
//!   * Telegram → constant-time string compare on the
//!     `X-Telegram-Bot-Api-Secret-Token` header
//!     (`remote_control::webhook_auth::validate_telegram_secret_token`, B3/T84).
//!   * Slack    → HMAC-SHA256 over `v0:<ts>:<body>` plus a ±5-min replay
//!     window (`remote_control::slack::verify_slack_signature`, B5/T86).
//!   * WhatsApp → nothing — the channel is a compile-only stub today.
//!
//! Each lived behind a channel-specific call site, so a future fourth
//! channel had no consistent surface to plug into. DEMO-1 PR #115's
//! "cross-cutting channel-native auth trait is missing" disclaimer was
//! about exactly this. This module closes it by defining
//! [`ChannelInboundAuth`] — one async-free, header+body-shaped trait
//! that every channel implements, while keeping the underlying
//! constant-time / HMAC math byte-identical (no wire-format change).
//!
//! ## Scope of this PR (V3 gap 2)
//!
//! * Define the trait + a single [`AuthError`] enum.
//! * Provide one impl per channel:
//!   - [`TelegramInboundAuth`] wraps the existing constant-time check.
//!   - [`SlackInboundAuth`]    wraps the existing HMAC + replay check.
//!   - [`WhatsappInboundAuth`] returns [`AuthError::NotImplemented`] until
//!     the Meta Business verification track lands.
//! * Migrate `slack::handle_webhook` (the *only* inbound dispatcher entry
//!   point that exists today — Telegram still uses long-poll, WhatsApp
//!   has no handler) to verify via the trait.
//!
//! ## Out of scope (deferred to follow-ups)
//!
//! * Adding an inbound HTTPS webhook to Telegram (tracked separately —
//!   the B3 validator was prepared in advance for that future handler).
//! * Real WhatsApp Cloud API auth (Meta Business verification, multi-week).
//! * Changing what headers each channel reads or the signature scheme.
//!   This PR is a **trait-surface** change only.
//!
//! ## Why `&HeaderMap` + `&[u8]`?
//!
//! Telegram only needs one header; Slack needs two headers *and* the raw
//! request body (the HMAC basestring includes the body bytes). Modelling
//! the trait at the union of those two requirements (`HeaderMap` +
//! `body: &[u8]`) keeps every impl honest: it can read whatever headers
//! it cares about, can read the body when required, and the dispatcher
//! never has to know which channel it's talking to.
//!
//! ## Feature gating
//!
//! Compiled under any of the remote-control sub-features so umbrella +
//! channel-only builds all see the trait. Default `cargo build` (no
//! flags) still compiles zero new code (mirrors the
//! `experimental-remote-control` gating convention used by every other module
//! in this directory).

use axum::http::HeaderMap;

/// Errors a channel-native inbound auth check can return.
///
/// All variants are mappable to an HTTP 401/403/501 response, so the
/// existing axum handlers don't lose any expressiveness when they
/// migrate to the trait.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AuthError {
    /// A required header was missing from the inbound request
    /// (e.g. `X-Slack-Signature`, `X-Telegram-Bot-Api-Secret-Token`).
    /// Maps to HTTP 401 in every existing handler.
    #[error("{channel}: missing required header {header}")]
    MissingHeader {
        channel: &'static str,
        header: &'static str,
    },

    /// A required header was present but malformed (e.g. a non-integer
    /// timestamp, or a Slack signature without the `v0=` prefix).
    /// Maps to HTTP 401.
    #[error("{channel}: malformed header {header}: {reason}")]
    MalformedHeader {
        channel: &'static str,
        header: &'static str,
        reason: &'static str,
    },

    /// The signature / token check itself failed in constant time.
    /// Maps to HTTP 401. NB: the variant carries no detail about *which*
    /// byte mismatched, by design — we never want to leak prefix info
    /// back to a caller.
    #[error("{channel}: signature mismatch")]
    BadSignature { channel: &'static str },

    /// Slack's replay-defense window was violated (request timestamp is
    /// > MAX_SKEW seconds away from local clock, in either direction).
    /// Maps to HTTP 401. Carries the observed skew so the *server-side*
    /// logger can record it; the response body never echoes it.
    #[error("{channel}: timestamp outside replay window (skew {skew_secs}s)")]
    ReplayWindow {
        channel: &'static str,
        skew_secs: i64,
    },

    /// The channel adapter does not implement inbound auth yet
    /// (WhatsApp today). Maps to HTTP 501 by convention so an operator
    /// who accidentally points Meta's webhook at us gets a loud signal
    /// rather than a silent 200 OK that would be misread as success.
    #[error("{channel}: inbound auth not implemented ({reason})")]
    NotImplemented {
        channel: &'static str,
        reason: &'static str,
    },
}

/// Cross-cutting inbound-auth contract.
///
/// `verify_request` MUST be constant-time with respect to any secret it
/// compares (token / HMAC digest). Implementations should delegate to
/// the existing `subtle::ConstantTimeEq`-based primitives rather than
/// reimplementing the math.
///
/// `Send + Sync` so impls can live in `Arc<dyn ChannelInboundAuth>` and
/// be shared across the axum router's tokio tasks.
pub trait ChannelInboundAuth: Send + Sync {
    /// Stable short channel name (e.g. `"telegram"`, `"slack"`,
    /// `"whatsapp"`). Used only for logging + error formatting; the
    /// auth decision must not depend on it.
    fn channel(&self) -> &'static str;

    /// Verify the inbound request. `headers` is the parsed axum HeaderMap
    /// from the incoming POST. `body` is the *raw* request body bytes —
    /// Slack's HMAC includes the body, so the caller MUST pass the
    /// unparsed bytes (no JSON round-trip, no whitespace normalisation).
    ///
    /// `Ok(())` on success. On failure, return the appropriate
    /// [`AuthError`]; the caller decides the HTTP status code (every
    /// existing handler uses 401 for everything except `NotImplemented`
    /// which maps to 501).
    fn verify_request(&self, headers: &HeaderMap, body: &[u8]) -> Result<(), AuthError>;
}

// ── Telegram impl ─────────────────────────────────────────────────────────

/// `X-Telegram-Bot-Api-Secret-Token` validator wrapping B3/T84's
/// constant-time check. The body bytes are ignored — Telegram's
/// inbound auth scheme is header-only.
///
/// Construct with the same secret string the operator passed to
/// Telegram's `setWebhook` call.
#[cfg(feature = "experimental-remote-control-telegram")]
pub struct TelegramInboundAuth {
    expected_secret: String,
}

#[cfg(feature = "experimental-remote-control-telegram")]
impl TelegramInboundAuth {
    /// Build a verifier from the secret token registered with Telegram.
    /// Fail-closed semantics live inside
    /// [`super::webhook_auth::validate_telegram_secret_token`] — passing
    /// an empty string here yields a validator that rejects every request.
    pub fn new(expected_secret: impl Into<String>) -> Self {
        Self {
            expected_secret: expected_secret.into(),
        }
    }
}

#[cfg(feature = "experimental-remote-control-telegram")]
impl ChannelInboundAuth for TelegramInboundAuth {
    fn channel(&self) -> &'static str {
        "telegram"
    }

    fn verify_request(&self, headers: &HeaderMap, _body: &[u8]) -> Result<(), AuthError> {
        const HEADER: &str = "X-Telegram-Bot-Api-Secret-Token";
        let provided =
            headers
                .get(HEADER)
                .and_then(|v| v.to_str().ok())
                .ok_or(AuthError::MissingHeader {
                    channel: "telegram",
                    header: HEADER,
                })?;
        if super::webhook_auth::validate_telegram_secret_token(provided, &self.expected_secret) {
            Ok(())
        } else {
            Err(AuthError::BadSignature {
                channel: "telegram",
            })
        }
    }
}

// ── Slack impl ────────────────────────────────────────────────────────────

/// HMAC-SHA256 + replay-window validator wrapping B5/T86's
/// `verify_slack_signature`. Reads `X-Slack-Signature` and
/// `X-Slack-Request-Timestamp` from the headers and the raw body bytes.
///
/// Clock injection is supported via [`SlackInboundAuth::with_clock`] so
/// tests can pin "now" without poking process-global state. Production
/// callers should use [`SlackInboundAuth::new`] which uses the system
/// clock.
#[cfg(feature = "experimental-remote-control-slack")]
pub struct SlackInboundAuth {
    signing_secret: String,
    /// ±MAX_SKEW seconds — Slack's documented replay window is 5 min.
    max_skew_secs: u64,
    /// Injectable clock; production = `system_now`, tests pin it.
    now_secs: fn() -> u64,
}

#[cfg(feature = "experimental-remote-control-slack")]
impl SlackInboundAuth {
    /// Slack's documented replay-defense window: ±5 minutes.
    /// Mirrors [`super::slack::MAX_TIMESTAMP_SKEW_SECS`] but is re-exported
    /// here so the trait module is standalone-testable.
    pub const MAX_TIMESTAMP_SKEW_SECS: u64 = 60 * 5;

    /// Build a Slack verifier using the system clock + Slack's
    /// recommended 5-min replay window.
    pub fn new(signing_secret: impl Into<String>) -> Self {
        Self {
            signing_secret: signing_secret.into(),
            max_skew_secs: Self::MAX_TIMESTAMP_SKEW_SECS,
            now_secs: system_now,
        }
    }

    /// Test-only constructor that takes a pinned clock function.
    pub fn with_clock(signing_secret: impl Into<String>, now_secs: fn() -> u64) -> Self {
        Self {
            signing_secret: signing_secret.into(),
            max_skew_secs: Self::MAX_TIMESTAMP_SKEW_SECS,
            now_secs,
        }
    }
}

#[cfg(feature = "experimental-remote-control-slack")]
fn system_now() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(feature = "experimental-remote-control-slack")]
impl ChannelInboundAuth for SlackInboundAuth {
    fn channel(&self) -> &'static str {
        "slack"
    }

    fn verify_request(&self, headers: &HeaderMap, body: &[u8]) -> Result<(), AuthError> {
        const SIG_HEADER: &str = "X-Slack-Signature";
        const TS_HEADER: &str = "X-Slack-Request-Timestamp";

        let provided_sig = headers
            .get(SIG_HEADER)
            .and_then(|v| v.to_str().ok())
            .ok_or(AuthError::MissingHeader {
                channel: "slack",
                header: SIG_HEADER,
            })?;
        let ts_header = headers.get(TS_HEADER).and_then(|v| v.to_str().ok()).ok_or(
            AuthError::MissingHeader {
                channel: "slack",
                header: TS_HEADER,
            },
        )?;

        let ts: i64 = ts_header.parse().map_err(|_| AuthError::MalformedHeader {
            channel: "slack",
            header: TS_HEADER,
            reason: "expected integer unix-seconds",
        })?;
        let now = (self.now_secs)() as i64;
        let skew = now - ts;
        if skew.unsigned_abs() > self.max_skew_secs {
            return Err(AuthError::ReplayWindow {
                channel: "slack",
                skew_secs: skew,
            });
        }

        if super::slack::verify_slack_signature(&self.signing_secret, ts_header, body, provided_sig)
        {
            Ok(())
        } else {
            Err(AuthError::BadSignature { channel: "slack" })
        }
    }
}

// ── WhatsApp stub ─────────────────────────────────────────────────────────

/// WhatsApp inbound auth stub — Meta's Cloud API uses an
/// `X-Hub-Signature-256` HMAC of the body plus an `hub.verify_token`
/// handshake at endpoint registration. Both require the Meta Business
/// verification dance, which is the same multi-week gate that keeps
/// `WhatsappStub` from sending. Until that lands, every inbound request
/// fails with [`AuthError::NotImplemented`] so a misconfigured operator
/// gets a loud HTTP 501 rather than a silent 200.
#[cfg(feature = "experimental-remote-control-whatsapp")]
pub struct WhatsappInboundAuth;

#[cfg(feature = "experimental-remote-control-whatsapp")]
impl WhatsappInboundAuth {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(feature = "experimental-remote-control-whatsapp")]
impl Default for WhatsappInboundAuth {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "experimental-remote-control-whatsapp")]
impl ChannelInboundAuth for WhatsappInboundAuth {
    fn channel(&self) -> &'static str {
        "whatsapp"
    }

    fn verify_request(&self, _headers: &HeaderMap, _body: &[u8]) -> Result<(), AuthError> {
        Err(AuthError::NotImplemented {
            channel: "whatsapp",
            reason: "WhatsApp Cloud API inbound auth deferred — Meta Business verification pending",
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    // ── Telegram dispatch ──────────────────────────────────────────────

    #[cfg(feature = "experimental-remote-control-telegram")]
    #[test]
    fn telegram_trait_dispatch_accepts_valid_secret() {
        let auth = TelegramInboundAuth::new("my-secret-token-123");
        let mut headers = HeaderMap::new();
        headers.insert(
            "X-Telegram-Bot-Api-Secret-Token",
            HeaderValue::from_static("my-secret-token-123"),
        );
        assert_eq!(auth.channel(), "telegram");
        // Body is irrelevant for Telegram — pass empty + arbitrary,
        // both must succeed.
        auth.verify_request(&headers, b"").expect("empty body ok");
        auth.verify_request(&headers, b"{\"update_id\": 1}")
            .expect("json body ok");
    }

    #[cfg(feature = "experimental-remote-control-telegram")]
    #[test]
    fn telegram_trait_dispatch_rejects_bad_secret_with_bad_signature() {
        let auth = TelegramInboundAuth::new("my-secret-token-123");
        let mut headers = HeaderMap::new();
        headers.insert(
            "X-Telegram-Bot-Api-Secret-Token",
            HeaderValue::from_static("wrong-token-1234567"),
        );
        let err = auth.verify_request(&headers, b"").unwrap_err();
        assert_eq!(
            err,
            AuthError::BadSignature {
                channel: "telegram"
            }
        );
    }

    #[cfg(feature = "experimental-remote-control-telegram")]
    #[test]
    fn telegram_trait_dispatch_missing_header_returns_missing_header() {
        let auth = TelegramInboundAuth::new("my-secret-token-123");
        let headers = HeaderMap::new();
        let err = auth.verify_request(&headers, b"").unwrap_err();
        match err {
            AuthError::MissingHeader {
                channel: "telegram",
                header,
            } => assert_eq!(header, "X-Telegram-Bot-Api-Secret-Token"),
            other => panic!("expected MissingHeader, got {other:?}"),
        }
    }

    // ── Slack dispatch ─────────────────────────────────────────────────

    #[cfg(feature = "experimental-remote-control-slack")]
    const FIXED_NOW: u64 = 1_700_000_000;
    #[cfg(feature = "experimental-remote-control-slack")]
    fn fixed_now() -> u64 {
        FIXED_NOW
    }

    #[cfg(feature = "experimental-remote-control-slack")]
    fn slack_signed_headers(secret: &str, ts: u64, body: &[u8]) -> HeaderMap {
        use crate::remote_control::slack::compute_slack_signature;
        let ts_str = ts.to_string();
        let hex = compute_slack_signature(secret, &ts_str, body);
        let mut h = HeaderMap::new();
        h.insert(
            "X-Slack-Signature",
            HeaderValue::from_str(&format!("v0={hex}")).unwrap(),
        );
        h.insert(
            "X-Slack-Request-Timestamp",
            HeaderValue::from_str(&ts_str).unwrap(),
        );
        h
    }

    #[cfg(feature = "experimental-remote-control-slack")]
    #[test]
    fn slack_trait_dispatch_accepts_valid_hmac_within_window() {
        let auth = SlackInboundAuth::with_clock("the-signing-secret", fixed_now);
        let body = br#"{"type":"event_callback"}"#;
        let headers = slack_signed_headers("the-signing-secret", FIXED_NOW, body);
        assert_eq!(auth.channel(), "slack");
        auth.verify_request(&headers, body).expect("valid HMAC ok");
    }

    #[cfg(feature = "experimental-remote-control-slack")]
    #[test]
    fn slack_trait_dispatch_rejects_bad_hmac_with_bad_signature() {
        let auth = SlackInboundAuth::with_clock("the-signing-secret", fixed_now);
        let body = br#"{"type":"event_callback"}"#;
        // Sign with WRONG secret → digest will mismatch.
        let headers = slack_signed_headers("attacker-controlled-secret", FIXED_NOW, body);
        let err = auth.verify_request(&headers, body).unwrap_err();
        assert_eq!(err, AuthError::BadSignature { channel: "slack" });
    }

    #[cfg(feature = "experimental-remote-control-slack")]
    #[test]
    fn slack_trait_dispatch_rejects_stale_timestamp_with_replay_window() {
        let auth = SlackInboundAuth::with_clock("the-signing-secret", fixed_now);
        let body = br#"{"type":"event_callback"}"#;
        // 6 minutes in the past with a fully-valid HMAC — must still fail.
        let stale_ts = FIXED_NOW - (6 * 60);
        let headers = slack_signed_headers("the-signing-secret", stale_ts, body);
        let err = auth.verify_request(&headers, body).unwrap_err();
        match err {
            AuthError::ReplayWindow {
                channel: "slack",
                skew_secs,
            } => {
                assert_eq!(skew_secs, 360, "6 min stale → skew of +360s");
            }
            other => panic!("expected ReplayWindow, got {other:?}"),
        }
    }

    #[cfg(feature = "experimental-remote-control-slack")]
    #[test]
    fn slack_trait_dispatch_missing_signature_header_returns_missing_header() {
        let auth = SlackInboundAuth::with_clock("the-signing-secret", fixed_now);
        let mut headers = HeaderMap::new();
        headers.insert(
            "X-Slack-Request-Timestamp",
            HeaderValue::from_str(&FIXED_NOW.to_string()).unwrap(),
        );
        let err = auth.verify_request(&headers, b"{}").unwrap_err();
        match err {
            AuthError::MissingHeader {
                channel: "slack",
                header,
            } => assert_eq!(header, "X-Slack-Signature"),
            other => panic!("expected MissingHeader, got {other:?}"),
        }
    }

    #[cfg(feature = "experimental-remote-control-slack")]
    #[test]
    fn slack_trait_dispatch_non_integer_timestamp_returns_malformed_header() {
        let auth = SlackInboundAuth::with_clock("the-signing-secret", fixed_now);
        let mut headers = HeaderMap::new();
        headers.insert(
            "X-Slack-Signature",
            HeaderValue::from_str(&format!("v0={}", "0".repeat(64))).unwrap(),
        );
        headers.insert(
            "X-Slack-Request-Timestamp",
            HeaderValue::from_static("not-a-number"),
        );
        let err = auth.verify_request(&headers, b"{}").unwrap_err();
        assert!(
            matches!(
                err,
                AuthError::MalformedHeader {
                    channel: "slack",
                    ..
                }
            ),
            "got {err:?}"
        );
    }

    // ── WhatsApp stub ──────────────────────────────────────────────────

    #[cfg(feature = "experimental-remote-control-whatsapp")]
    #[test]
    fn whatsapp_trait_dispatch_returns_not_implemented() {
        let auth = WhatsappInboundAuth::new();
        assert_eq!(auth.channel(), "whatsapp");
        let headers = HeaderMap::new();
        let err = auth.verify_request(&headers, b"anything").unwrap_err();
        assert!(
            matches!(
                err,
                AuthError::NotImplemented {
                    channel: "whatsapp",
                    ..
                }
            ),
            "got {err:?}"
        );
    }

    // ── Object-safety / trait-dispatch via &dyn ────────────────────────
    //
    // Spec requirement: "dispatcher uses `&dyn ChannelInboundAuth` instead
    // of channel-specific branching". Prove the trait is object-safe and
    // that a single function pointer-typed call site can drive all three
    // impls uniformly.

    #[cfg(all(
        feature = "experimental-remote-control-telegram",
        feature = "experimental-remote-control-slack",
        feature = "experimental-remote-control-whatsapp",
    ))]
    #[test]
    fn trait_is_object_safe_and_dispatches_uniformly_across_channels() {
        fn verify_via_dyn(
            auth: &dyn ChannelInboundAuth,
            headers: &HeaderMap,
            body: &[u8],
        ) -> Result<&'static str, AuthError> {
            auth.verify_request(headers, body)?;
            Ok(auth.channel())
        }

        // Telegram: valid secret → channel name "telegram".
        let tg = TelegramInboundAuth::new("s3cret");
        let mut tg_headers = HeaderMap::new();
        tg_headers.insert(
            "X-Telegram-Bot-Api-Secret-Token",
            HeaderValue::from_static("s3cret"),
        );
        assert_eq!(verify_via_dyn(&tg, &tg_headers, b"").unwrap(), "telegram");

        // Slack: valid HMAC → channel name "slack".
        let sl = SlackInboundAuth::with_clock("ss", fixed_now);
        let body = br#"{}"#;
        let sl_headers = slack_signed_headers("ss", FIXED_NOW, body);
        assert_eq!(verify_via_dyn(&sl, &sl_headers, body).unwrap(), "slack");

        // WhatsApp: stub returns NotImplemented uniformly.
        let wa = WhatsappInboundAuth::new();
        let empty = HeaderMap::new();
        let err = verify_via_dyn(&wa, &empty, b"").unwrap_err();
        assert!(matches!(
            err,
            AuthError::NotImplemented {
                channel: "whatsapp",
                ..
            }
        ));
    }
}
