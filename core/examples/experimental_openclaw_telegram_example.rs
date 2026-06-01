//! Example: experimental-openclaw-telegram (O1).
//!
//! Gated behind the `experimental-openclaw-telegram` Cargo feature (declared in
//! core/Cargo.toml) via a `[[example]] required-features` stanza, so it is
//! skipped by the default build/`cargo test` and only compiled when the feature
//! is enabled. Run it with:
//!
//!   cargo run -p phantom-mesh \
//!       --example experimental_openclaw_telegram_example \
//!       --features experimental-openclaw-telegram
//!
//! Expected last line: `experimental-openclaw-telegram OK`. Exit code 0.
//!
//! The example never makes a network call — it exercises only the offline
//! parts of the public API: Debug redaction, allowlist gating, and the
//! `handle_text` dispatcher path with the built-in EchoDispatcher.

#![cfg(feature = "experimental-openclaw-telegram")]

use std::sync::Arc;

use phantom_mesh::openclaw::telegram::{
    EchoDispatcher, OpenclawTelegramBot, OpenclawTelegramConfig,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cfg = OpenclawTelegramConfig {
        bot_token: "FAKE_TOKEN_DO_NOT_LOG".into(),
        allowed_user_ids: vec![42],
    };

    // (1) Allowlist gating
    assert!(cfg.is_user_allowed(42));
    assert!(!cfg.is_user_allowed(7));
    println!("[1] allowlist: 42 allowed, 7 denied");

    // (2) Debug redacts the bot token
    let dbg = format!("{:?}", cfg);
    assert!(
        !dbg.contains("FAKE_TOKEN_DO_NOT_LOG"),
        "token must NEVER appear in Debug output"
    );
    assert!(dbg.contains("<redacted>"));
    println!("[2] Debug redacts the token: {dbg}");

    // (3) Dispatcher round-trip via the EchoDispatcher.
    // handle_text(user_id, chat_id, text): user_id gates the allowlist, chat_id
    // scopes conversation history (unused by EchoDispatcher), text is echoed.
    let bot = OpenclawTelegramBot::new(cfg, Arc::new(EchoDispatcher));
    let reply = bot.handle_text(42, 1000, "hello".into()).await;
    assert_eq!(reply.as_deref(), Some("phantom-mesh echo: hello"));
    println!("[3] handle_text(42, chat 1000, \"hello\") -> {:?}", reply.unwrap());

    // (4) Non-allowlisted user is dropped (returns None) regardless of chat.
    let drop = bot.handle_text(7, 1001, "should-be-dropped".into()).await;
    assert_eq!(drop, None);
    println!("[4] handle_text(7, chat 1001, ...) -> None (dropped)");

    println!("experimental-openclaw-telegram OK");
    Ok(())
}
