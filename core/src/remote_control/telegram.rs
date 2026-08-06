//! Telegram channel — your phone's Remote Control for the cluster
//! (BIG-GOAL §P3).
//!
//! One Telegram message in, one cluster command out: the bot is the wire
//! protocol, not the brain. Incoming user text is funnelled through a
//! [`Dispatcher`] (the agent / runtime / skill loop) and the textual
//! return value becomes the reply to that same chat. Nothing about this
//! module makes the bot "conversational" in the chatbot sense — replies
//! are the cluster reporting back on work it just did.
//!
//! See `docs/superpowers/BIG-GOAL.md` §P3 and the remote-control epic
//! spec for the locked framing, and
//! `docs/superpowers/specs/2026-05-15-weekend-multi-agent-push-design.md`
//! §4 row [O1] for this module's original track.
//!
//! Build with:
//!   cargo build --features experimental-remote-control-telegram
//!
//! ## Architecture
//!
//! `RemoteTelegramBot` wraps a `teloxide::Bot` plus an injected
//! `Dispatcher` trait object so the module is unit-testable without
//! pulling in the full `AgentRuntime`. The host (e.g. `core/examples/
//! remote_telegram_dispatch.rs` or, eventually, `main.rs`) constructs
//! the bot and passes a `Dispatcher` impl that knows how to turn an
//! incoming user message into a response string.
//!
//! ## Token handling
//!
//! The bot token is NEVER logged or printed. Acquired from the env var
//! that `spectyn keys set telegram_bot <token>` writes
//! (`TELEGRAM_BOT_API_KEY`); the constructor takes the raw string and
//! does not retain it anywhere except inside `teloxide::Bot`'s internal
//! HTTP client.
//!
//! ## Allowlist
//!
//! `allowed_user_ids` defaults empty (allow all); when non-empty, only
//! messages from `from.id` matching one of the entries are dispatched.

use std::sync::Arc;

use async_trait::async_trait;

/// Dispatch a user-typed text message to whatever the host wants
/// (echo / spectyn AgentRuntime / pure function for tests). The
/// response string is sent verbatim back to the same chat.
///
/// Returning `Err` causes the bot to send a generic "internal error"
/// reply so the user sees something happened.
///
/// Named `RemoteTelegramDispatcher` (not just `Dispatcher`) so it cannot
/// collide with `teloxide::dispatching::Dispatcher` inside
/// `run_round_trip`'s function-local `use teloxide::prelude::*;`.
#[async_trait]
pub trait RemoteTelegramDispatcher: Send + Sync {
    /// Legacy one-arg dispatch — kept for backward compatibility with
    /// `EchoDispatcher` and other minimal impls that do not need
    /// per-chat context.
    async fn dispatch(&self, user_text: String) -> Result<String, String>;

    /// Chat-aware dispatch. Defaults to ignoring `chat_id` and calling
    /// `dispatch`. Implementations that maintain per-chat conversation
    /// state (e.g. `SpectynAgentDispatcher`) override this method.
    ///
    /// Telegram chat_ids are i64; both user chats (positive) and group
    /// chats (negative) round-trip cleanly. We do NOT use `u64` here.
    async fn dispatch_with_chat(&self, _chat_id: i64, user_text: String) -> Result<String, String> {
        self.dispatch(user_text).await
    }
}

/// In-memory echo dispatcher — only used by unit tests + the demo
/// example binary. NOT a production path.
pub struct EchoDispatcher;

#[async_trait]
impl RemoteTelegramDispatcher for EchoDispatcher {
    async fn dispatch(&self, user_text: String) -> Result<String, String> {
        Ok(format!("spectyn-mesh echo: {}", user_text))
    }
}

/// Configuration for the remote-control Telegram bot.
///
/// `bot_token` is sensitive — never log it. The `Debug` impl below
/// redacts it to "<redacted>".
#[derive(Clone)]
pub struct RemoteTelegramConfig {
    pub bot_token: String,
    pub allowed_user_ids: Vec<i64>,
}

impl std::fmt::Debug for RemoteTelegramConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteTelegramConfig")
            .field("bot_token", &"<redacted>")
            .field("allowed_user_ids", &self.allowed_user_ids)
            .finish()
    }
}

impl RemoteTelegramConfig {
    /// Returns true iff the given user_id is permitted to interact
    /// with this bot. Empty allowlist = allow everyone.
    pub fn is_user_allowed(&self, user_id: i64) -> bool {
        self.allowed_user_ids.is_empty() || self.allowed_user_ids.contains(&user_id)
    }
}

/// Wraps a `teloxide::Bot` plus the host's dispatcher.
///
/// Construct with `new`; run with `run_round_trip` (long-poll loop).
/// `run_round_trip` does NOT return on success — it loops until the
/// process is killed or teloxide returns a fatal error.
pub struct RemoteTelegramBot {
    config: RemoteTelegramConfig,
    dispatcher: Arc<dyn RemoteTelegramDispatcher>,
}

impl RemoteTelegramBot {
    pub fn new(config: RemoteTelegramConfig, dispatcher: Arc<dyn RemoteTelegramDispatcher>) -> Self {
        Self { config, dispatcher }
    }

    pub fn config(&self) -> &RemoteTelegramConfig {
        &self.config
    }

    /// Handle one incoming text message. `chat_id` is threaded through
    /// to the dispatcher so chat-aware impls (e.g. `SpectynAgentDispatcher`)
    /// can scope conversation history per chat. `user_id` is used only
    /// for the allowlist check — it is intentionally NOT passed to the
    /// dispatcher (chat_id is the conversation-scope key, user_id is the
    /// per-person access-control key).
    pub async fn handle_text(&self, user_id: i64, chat_id: i64, text: String) -> Option<String> {
        if !self.config.is_user_allowed(user_id) {
            tracing::warn!(
                user_id,
                "remote-tg: rejecting message from non-allowlisted user"
            );
            return None;
        }
        match self.dispatcher.dispatch_with_chat(chat_id, text).await {
            Ok(reply) => Some(reply),
            Err(e) => {
                // Do NOT log `e` at info+ levels with chat content — the
                // error string might contain echoed user text. `error!`
                // at trace+ is the structured channel; the user gets a
                // generic reply so internal details never leak.
                tracing::error!(error = %e, "remote-tg: dispatcher returned error");
                Some("spectyn-mesh: internal error handling your message.".to_string())
            }
        }
    }
}

// ── Long-poll loop using teloxide ─────────────────────────────────────────

/// Run the long-poll round-trip loop. Spawns a teloxide message handler
/// that forwards each text message to `bot.handle_text` and replies with
/// the dispatcher's response.
///
/// Never returns Ok in normal operation — loops until the process is
/// killed or teloxide returns a fatal error (e.g. bad token rejected by
/// `/getMe` on startup).
///
/// This function is the only place teloxide's runtime types appear; the
/// rest of the module is unit-testable without teloxide.
pub async fn run_round_trip(bot: Arc<RemoteTelegramBot>) -> Result<(), String> {
    use teloxide::prelude::*;
    use teloxide::types::Message;

    let tg = Bot::new(bot.config().bot_token.clone());

    // Sanity: probe getMe up front so a bad token fails loud rather than
    // silently long-polling forever.
    match tg.get_me().await {
        Ok(me) => {
            tracing::info!(
                bot_username = %me.username.as_deref().unwrap_or("<no-username>"),
                "remote-tg: bot identity verified"
            );
        }
        Err(e) => {
            // Strip any token that might be in the URL inside the error.
            let safe = redact_token(&e.to_string(), &bot.config().bot_token);
            return Err(format!("getMe failed: {}", safe));
        }
    }

    let bot_ref = bot.clone();
    let handler = Update::filter_message().endpoint(move |update_msg: Message, tg: Bot| {
        let bot_ref = bot_ref.clone();
        async move {
            let text = match update_msg.text() {
                Some(t) => t.to_string(),
                None => return Ok::<_, teloxide::RequestError>(()),
            };
            let user_id = update_msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);
            let chat_id = update_msg.chat.id;
            if let Some(reply) = bot_ref.handle_text(user_id, chat_id.0, text).await {
                // Telegram has a 4096-char message limit; chunk if needed.
                for chunk in chunk_message(&reply, 4000) {
                    if let Err(e) = tg.send_message(chat_id, chunk).await {
                        tracing::warn!(error = %e, "remote-tg: send_message failed");
                    }
                }
            }
            Ok(())
        }
    });

    Dispatcher::builder(tg, handler)
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;

    Ok(())
}

/// Strip occurrences of `token` from a string so error logs never leak it.
fn redact_token(s: &str, token: &str) -> String {
    if token.is_empty() {
        return s.to_string();
    }
    s.replace(token, "<redacted>")
}

/// Split a long message into chunks of at most `max_len` bytes,
/// never cutting a multi-byte UTF-8 char.
fn chunk_message(text: &str, max_len: usize) -> Vec<String> {
    if text.len() <= max_len {
        return vec![text.to_string()];
    }
    let mut chunks = Vec::new();
    let mut remaining = text;
    while remaining.len() > max_len {
        let mut boundary = max_len;
        while boundary > 0 && !remaining.is_char_boundary(boundary) {
            boundary -= 1;
        }
        chunks.push(remaining[..boundary].to_string());
        remaining = &remaining[boundary..];
    }
    if !remaining.is_empty() {
        chunks.push(remaining.to_string());
    }
    chunks
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_debug_redacts_token() {
        let cfg = RemoteTelegramConfig {
            bot_token: "REAL_LOOKING_TOKEN:abc-def".into(),
            allowed_user_ids: vec![123],
        };
        let s = format!("{:?}", cfg);
        assert!(
            !s.contains("REAL_LOOKING_TOKEN"),
            "token must NEVER appear in Debug output"
        );
        assert!(s.contains("<redacted>"));
    }

    #[test]
    fn allowlist_empty_permits_anyone() {
        let cfg = RemoteTelegramConfig {
            bot_token: "x".into(),
            allowed_user_ids: vec![],
        };
        assert!(cfg.is_user_allowed(1));
        assert!(cfg.is_user_allowed(99999));
    }

    #[test]
    fn allowlist_blocks_non_matching_users() {
        let cfg = RemoteTelegramConfig {
            bot_token: "x".into(),
            allowed_user_ids: vec![100, 200],
        };
        assert!(cfg.is_user_allowed(100));
        assert!(cfg.is_user_allowed(200));
        assert!(!cfg.is_user_allowed(101));
        assert!(!cfg.is_user_allowed(0));
    }

    #[tokio::test]
    async fn echo_dispatcher_round_trip() {
        let d = EchoDispatcher;
        let r = d.dispatch("hello world".into()).await.unwrap();
        assert_eq!(r, "spectyn-mesh echo: hello world");
    }

    #[tokio::test]
    async fn handle_text_forwards_to_dispatcher_when_allowed() {
        let bot = RemoteTelegramBot::new(
            RemoteTelegramConfig {
                bot_token: "x".into(),
                allowed_user_ids: vec![42],
            },
            Arc::new(EchoDispatcher),
        );
        let r = bot.handle_text(42, 999_001, "ping".into()).await;
        assert_eq!(r, Some("spectyn-mesh echo: ping".into()));
    }

    #[tokio::test]
    async fn handle_text_drops_non_allowlisted_user() {
        let bot = RemoteTelegramBot::new(
            RemoteTelegramConfig {
                bot_token: "x".into(),
                allowed_user_ids: vec![42],
            },
            Arc::new(EchoDispatcher),
        );
        let r = bot.handle_text(7, 999_002, "ping".into()).await;
        assert_eq!(r, None);
    }

    /// Failing dispatcher → user gets the generic error message,
    /// NOT the underlying error string (which may leak detail).
    #[tokio::test]
    async fn handle_text_translates_dispatcher_error_to_generic_reply() {
        struct FailingDispatcher;
        #[async_trait]
        impl RemoteTelegramDispatcher for FailingDispatcher {
            async fn dispatch(&self, _text: String) -> Result<String, String> {
                Err("internal panic with secret stack trace".into())
            }
        }
        let bot = RemoteTelegramBot::new(
            RemoteTelegramConfig {
                bot_token: "x".into(),
                allowed_user_ids: vec![],
            },
            Arc::new(FailingDispatcher),
        );
        let r = bot.handle_text(1, 999_003, "x".into()).await.unwrap();
        assert!(!r.contains("secret stack trace"));
        assert!(r.contains("internal error"));
    }

    #[test]
    fn redact_token_strips_token_from_error_string() {
        let token = "1234:abc-def-ghi";
        let leaky = format!(
            "HTTP 401: bad token https://api.telegram.org/bot{}/getMe",
            token
        );
        let redacted = redact_token(&leaky, token);
        assert!(!redacted.contains(token));
        assert!(redacted.contains("<redacted>"));
    }

    #[test]
    fn redact_token_no_op_on_empty_token() {
        assert_eq!(redact_token("hello", ""), "hello");
    }

    #[test]
    fn chunk_message_short_returns_single_chunk() {
        let chunks = chunk_message("hi", 100);
        assert_eq!(chunks, vec!["hi".to_string()]);
    }

    #[test]
    fn chunk_message_splits_long_text_at_byte_boundary() {
        let text = "a".repeat(10_000);
        let chunks = chunk_message(&text, 4000);
        assert_eq!(chunks.len(), 3);
        assert!(chunks.iter().all(|c| c.len() <= 4000));
        assert_eq!(chunks.iter().map(|c| c.len()).sum::<usize>(), 10_000);
    }

    #[test]
    fn chunk_message_does_not_split_multibyte_char() {
        // Each '中' is 3 bytes in UTF-8 → 4 chars = 12 bytes; max_len = 7
        // would naively cut a char, so chunker must back off to byte 6.
        let text = "中中中中"; // 12 bytes
        let chunks = chunk_message(text, 7);
        for c in &chunks {
            // Round-trip parse must succeed; if a multi-byte char was cut,
            // .chars() would error or yield U+FFFD replacement.
            assert!(c.is_char_boundary(c.len()));
            assert_eq!(c.chars().count() * 3, c.len(), "chunk={:?}", c);
        }
    }

    /// EchoDispatcher must keep working through the new chat-aware method
    /// because it implements only the legacy `dispatch`. The default body
    /// of `dispatch_with_chat` must delegate to `dispatch` so existing
    /// impls don't break.
    #[tokio::test]
    async fn dispatch_with_chat_default_delegates_to_dispatch() {
        let d = EchoDispatcher;
        let r = d.dispatch_with_chat(123, "hello".into()).await.unwrap();
        assert_eq!(r, "spectyn-mesh echo: hello");
    }
}

// ── B2/T83 — CLI launcher glue ─────────────────────────────────────────────

/// Helpers used by `spectyn serve --remote-telegram` to read the bot token
/// from the operator's environment and build an [`RemoteTelegramConfig`].
///
/// Kept in its own submodule so the env-var-name validation + allowlist
/// parsing can be unit-tested without spawning the long-poll loop or
/// touching the real `TELEGRAM_BOT_API_KEY`.
pub mod cli {
    use super::RemoteTelegramConfig;

    /// The default env-var name written by `spectyn keys set telegram_bot`.
    pub const DEFAULT_BOT_TOKEN_ENV: &str = "TELEGRAM_BOT_API_KEY";

    /// The (optional) env-var holding a comma-separated allowlist of
    /// numeric Telegram user IDs. Empty list = allow everyone.
    pub const ALLOWED_USERS_ENV: &str = "TELEGRAM_ALLOWED_USERS";

    /// Errors that can occur while resolving the operator's CLI flags into
    /// a usable bot configuration.
    #[derive(Debug, PartialEq, Eq)]
    pub enum CliError {
        /// The operator-provided env var name was invalid (empty, or
        /// contained non-ASCII / non-shell-safe chars). We reject these
        /// rather than passing them to `std::env::var` because a stray
        /// `--bot-token-env "TOKEN=secret"` would otherwise look like a
        /// successful lookup of a spectyn variable.
        InvalidEnvName(String),
        /// The env var named by `--bot-token-env` was not set at process
        /// start. Operators should `spectyn keys set telegram_bot <token>`
        /// and source the env file (or export the var) before launching.
        EnvMissing(String),
        /// The env var was set but its value was empty / whitespace-only.
        /// We treat this as missing because passing it to teloxide would
        /// fail at `/getMe` time anyway, but with a less actionable error.
        EnvEmpty(String),
    }

    impl std::fmt::Display for CliError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                CliError::InvalidEnvName(n) => write!(
                    f,
                    "--bot-token-env must be an ASCII identifier (letters / digits / underscore), got {:?}",
                    n
                ),
                CliError::EnvMissing(n) => write!(
                    f,
                    "env var {} is not set — run `spectyn keys set telegram_bot <token>` first",
                    n
                ),
                CliError::EnvEmpty(n) => write!(
                    f,
                    "env var {} is set but empty — re-run `spectyn keys set telegram_bot <token>`",
                    n
                ),
            }
        }
    }

    impl std::error::Error for CliError {}

    /// Validate an env-var name supplied via `--bot-token-env <VAR>`.
    ///
    /// Conservative rule: must be non-empty, must begin with letter or
    /// underscore, and the remainder must be ASCII alphanumeric or
    /// underscore. This rejects accidental injections like `"X; rm -rf"`
    /// or `"FOO=bar"` that would otherwise silently look up `$FOO=bar`.
    pub fn validate_env_name(name: &str) -> Result<(), CliError> {
        if name.is_empty() {
            return Err(CliError::InvalidEnvName(name.to_string()));
        }
        let mut chars = name.chars();
        let first = chars.next().unwrap();
        if !(first.is_ascii_alphabetic() || first == '_') {
            return Err(CliError::InvalidEnvName(name.to_string()));
        }
        for c in chars {
            if !(c.is_ascii_alphanumeric() || c == '_') {
                return Err(CliError::InvalidEnvName(name.to_string()));
            }
        }
        Ok(())
    }

    /// Parse the optional `TELEGRAM_ALLOWED_USERS` env var into a list of
    /// numeric user IDs. Bad entries are silently dropped (so a stray
    /// comma or comment doesn't disable the bot), and an unset / empty
    /// value yields the empty allowlist (= allow all).
    pub fn parse_allowed_users(raw: Option<&str>) -> Vec<i64> {
        match raw {
            None => Vec::new(),
            Some(s) => s
                .split(',')
                .filter_map(|p| p.trim().parse::<i64>().ok())
                .collect(),
        }
    }

    /// Build an [`RemoteTelegramConfig`] from operator-controlled inputs.
    ///
    /// `lookup` is the env-var reader (usually `|n| std::env::var(n).ok()`);
    /// taking it as a function makes this fully unit-testable without
    /// poking process-global state.
    pub fn resolve_config<F>(
        env_name: &str,
        allowed_raw: Option<String>,
        mut lookup: F,
    ) -> Result<RemoteTelegramConfig, CliError>
    where
        F: FnMut(&str) -> Option<String>,
    {
        validate_env_name(env_name)?;
        let token = lookup(env_name).ok_or_else(|| CliError::EnvMissing(env_name.to_string()))?;
        if token.trim().is_empty() {
            return Err(CliError::EnvEmpty(env_name.to_string()));
        }
        let allowed_user_ids = parse_allowed_users(allowed_raw.as_deref());
        Ok(RemoteTelegramConfig {
            bot_token: token,
            allowed_user_ids,
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn default_env_name_matches_spectyn_keys_convention() {
            // Sanity: if `spectyn keys set telegram_bot` ever changes the
            // var it writes, this constant must change too. Locking the
            // value here forces that PR to update both call sites.
            assert_eq!(DEFAULT_BOT_TOKEN_ENV, "TELEGRAM_BOT_API_KEY");
        }

        #[test]
        fn validate_env_name_accepts_standard_identifiers() {
            assert!(validate_env_name("TELEGRAM_BOT_API_KEY").is_ok());
            assert!(validate_env_name("FOO").is_ok());
            assert!(validate_env_name("_X").is_ok());
            assert!(validate_env_name("A1_B2").is_ok());
        }

        #[test]
        fn validate_env_name_rejects_empty() {
            assert_eq!(
                validate_env_name(""),
                Err(CliError::InvalidEnvName(String::new()))
            );
        }

        #[test]
        fn validate_env_name_rejects_leading_digit() {
            assert!(matches!(
                validate_env_name("1FOO"),
                Err(CliError::InvalidEnvName(_))
            ));
        }

        #[test]
        fn validate_env_name_rejects_shell_metachars() {
            // These would be devastating if forwarded to `std::env::var`
            // or interpreted by a downstream shell — reject loudly.
            for bad in ["FOO=bar", "FOO BAR", "FOO;rm", "FOO$BAR", "FOO\nBAR"] {
                assert!(
                    matches!(validate_env_name(bad), Err(CliError::InvalidEnvName(_))),
                    "expected reject for {:?}",
                    bad
                );
            }
        }

        #[test]
        fn parse_allowed_users_empty_input_yields_empty_list() {
            assert!(parse_allowed_users(None).is_empty());
            assert!(parse_allowed_users(Some("")).is_empty());
        }

        #[test]
        fn parse_allowed_users_handles_csv_with_whitespace() {
            assert_eq!(
                parse_allowed_users(Some("123, 456 ,789")),
                vec![123, 456, 789]
            );
        }

        #[test]
        fn parse_allowed_users_drops_garbage_entries() {
            assert_eq!(parse_allowed_users(Some("123,,abc,456")), vec![123, 456]);
        }

        #[test]
        fn resolve_config_happy_path() {
            let cfg = resolve_config("TELEGRAM_BOT_API_KEY", Some("42,99".into()), |n| {
                if n == "TELEGRAM_BOT_API_KEY" {
                    Some("real-token".into())
                } else {
                    None
                }
            })
            .expect("must succeed");
            assert_eq!(cfg.bot_token, "real-token");
            assert_eq!(cfg.allowed_user_ids, vec![42, 99]);
        }

        #[test]
        fn resolve_config_missing_env_returns_actionable_error() {
            let err = resolve_config("TELEGRAM_BOT_API_KEY", None, |_| None).unwrap_err();
            assert_eq!(err, CliError::EnvMissing("TELEGRAM_BOT_API_KEY".into()));
            // The Display output must name the var so the operator knows
            // what to set; assert that explicitly to lock the contract.
            let msg = err.to_string();
            assert!(msg.contains("TELEGRAM_BOT_API_KEY"));
            assert!(msg.contains("spectyn keys set"));
        }

        #[test]
        fn resolve_config_empty_env_distinguished_from_missing() {
            let err =
                resolve_config("TELEGRAM_BOT_API_KEY", None, |_| Some("   ".into())).unwrap_err();
            assert_eq!(err, CliError::EnvEmpty("TELEGRAM_BOT_API_KEY".into()));
        }

        #[test]
        fn resolve_config_rejects_bad_env_name_before_lookup() {
            // Crucial: the lookup must NEVER fire if the name is bad,
            // otherwise a `--bot-token-env "FOO=bar"` could be observed
            // by a malicious env reader. We assert this by panicking in
            // the lookup closure.
            let err = resolve_config("FOO=bar", None, |_| {
                panic!("lookup must not be called for invalid env name")
            })
            .unwrap_err();
            assert!(matches!(err, CliError::InvalidEnvName(_)));
        }

        #[test]
        fn resolve_config_returns_config_that_redacts_token_in_debug() {
            let cfg = resolve_config("TELEGRAM_BOT_API_KEY", None, |_| {
                Some("SECRET_TOKEN_VALUE".into())
            })
            .unwrap();
            let s = format!("{:?}", cfg);
            assert!(!s.contains("SECRET_TOKEN_VALUE"));
            assert!(s.contains("<redacted>"));
        }
    }
}
