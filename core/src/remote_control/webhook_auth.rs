//! Telegram webhook secret-token validation (B3 / T84) — the bottom
//! half of "is this Remote Control (BIG-GOAL §P3) actually paired to
//! my cluster?" for the Telegram channel.
//!
//! Without this check, a webhook URL is a public RPC endpoint anyone
//! who guesses it can use to issue commands at the cluster. With it,
//! the bot only acts on POSTs carrying the secret token registered at
//! `setWebhook` time. See [`crate::remote_control::inbound_auth`] for the
//! cross-channel trait this validator slots into.
//!
//! Telegram lets bot owners register a `secret_token` when calling
//! `setWebhook`; every subsequent inbound HTTPS POST then includes the
//! header `X-Telegram-Bot-Api-Secret-Token: <token>`. Servers MUST drop
//! requests whose header does not match the configured value, otherwise
//! the webhook URL is a public RPC endpoint for anyone who can guess /
//! discover it.
//!
//! Reference: <https://core.telegram.org/bots/api#setwebhook>
//!
//! ## Why constant-time compare
//!
//! Naive `==` on `&str` short-circuits on the first byte mismatch, which
//! leaks token length and prefix info via response-time side channels.
//! Telegram allows tokens up to 256 chars containing `A-Z a-z 0-9 _ -`,
//! so the realistic search space is small enough that a timing oracle
//! could feasibly recover it. We use [`subtle::ConstantTimeEq`] (already
//! a top-level dep — same crate the OAuth code uses for state compare).
//!
//! This module deliberately lives outside `telegram.rs` so it can land
//! before PR #28 (the bot runtime) — the `Channel` impl on PR #28 will
//! call [`validate_telegram_secret_token`] in its axum handler once both
//! are on `main`. See the integration note at the bottom of the PR
//! description.
//!
//! Built only with `--features experimental-remote-control-telegram`; the
//! umbrella `experimental-remote-control` feature deliberately does NOT pull
//! it in yet (matches the comment in `core/Cargo.toml`).

use subtle::ConstantTimeEq;

/// Returns `true` iff `provided` and `expected` are byte-identical, in
/// constant time relative to the longer of the two slices.
///
/// Length-mismatch is handled with an early `false` return — but only
/// AFTER the length comparison, which is itself O(1). We deliberately
/// do NOT pad/extend the shorter input to compare-against the longer
/// one because Telegram tokens have a hard `>=1, <=256` ASCII bound and
/// the length itself is not the secret; the *contents* are.
///
/// Returns `false` for empty `expected` to avoid the degenerate case
/// where a misconfigured server (e.g. `TELEGRAM_WEBHOOK_SECRET=""`)
/// would accept any request whose header is also empty. A server that
/// did not configure a secret should not be calling this function at
/// all; failing closed is the safer default.
pub fn validate_telegram_secret_token(provided: &str, expected: &str) -> bool {
    if expected.is_empty() {
        return false;
    }
    if provided.len() != expected.len() {
        return false;
    }
    provided.as_bytes().ct_eq(expected.as_bytes()).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_tokens_validate() {
        let expected = "my-secret-token-123";
        assert!(validate_telegram_secret_token(expected, expected));
    }

    #[test]
    fn mismatched_tokens_rejected() {
        assert!(!validate_telegram_secret_token(
            "wrong-token-1234567",
            "my-secret-token-123",
        ));
    }

    #[test]
    fn different_lengths_rejected() {
        // Shorter provided.
        assert!(!validate_telegram_secret_token(
            "short",
            "my-secret-token-123"
        ));
        // Longer provided.
        assert!(!validate_telegram_secret_token(
            "my-secret-token-123-extra",
            "my-secret-token-123",
        ));
    }

    #[test]
    fn empty_provided_rejected() {
        assert!(!validate_telegram_secret_token("", "my-secret-token-123"));
    }

    #[test]
    fn empty_expected_rejected_even_if_provided_empty() {
        // Fail-closed: missing server-side config must never grant access.
        assert!(!validate_telegram_secret_token("", ""));
        assert!(!validate_telegram_secret_token("anything", ""));
    }

    #[test]
    fn telegram_max_length_token_matches() {
        // Telegram caps secret_token at 256 chars, A-Z a-z 0-9 _ -.
        // 32 chars is the typical minimum a sane operator would pick.
        let token: String = std::iter::repeat('A').take(32).collect();
        assert!(validate_telegram_secret_token(&token, &token));

        // Also exercise the documented maximum (256 chars).
        let max_token: String = std::iter::repeat('Z').take(256).collect();
        assert!(validate_telegram_secret_token(&max_token, &max_token));
    }

    #[test]
    fn telegram_max_length_token_single_byte_diff_rejected() {
        // Last byte differs — must fail (would catch a naive
        // `expected.starts_with(provided)` regression).
        let expected: String = std::iter::repeat('A').take(32).collect();
        let mut provided = expected.clone();
        provided.pop();
        provided.push('B');
        assert!(!validate_telegram_secret_token(&provided, &expected));
    }

    #[test]
    fn first_byte_diff_rejected() {
        // Catches a naive "endswith" regression and mirrors the
        // timing-attack scenario this module exists to prevent.
        let expected = "Asecret-token-32-chars-of-stuff!";
        let provided = "Zsecret-token-32-chars-of-stuff!";
        assert_eq!(expected.len(), provided.len());
        assert!(!validate_telegram_secret_token(provided, expected));
    }
}
