//! Remote control — chat channels (Telegram/Slack/WhatsApp) as remotes for the mesh (BIG-GOAL pillar P3).
//!
//! Every chat channel you already use — Telegram, Slack, WhatsApp — becomes
//! a remote for the mesh. One Telegram message = one cluster command; the
//! bot's reply is the agent's response to *your cluster doing the work*,
//! not a standalone conversation. Channel adapters here are the transport;
//! [`persona`] supplies the voice; the runtime supplies the work. See
//! `docs/superpowers/BIG-GOAL.md` §P3 for the locked framing and
//! the remote-control epic spec for the epic that this module serves.
//!
//! The adapters below are pluggable Telegram / WhatsApp / Slack
//! implementations that all sit behind the same [`Channel`] trait so the
//! dispatcher can route messages without knowing the wire protocol.
//! WhatsApp and Slack ship as compile-only stubs this weekend (per spec
//! §5 O2 best-effort); their `send_message` impls return
//! [`ChannelError::NotImplemented`].
//!
//! Telegram (track O1, spec §4) ships as the first real adapter, gated
//! behind `experimental-remote-control-telegram`. When the feature is enabled the
//! `remote_control::telegram` submodule pulls in `teloxide` and exposes a
//! round-trip bot loop plus a `cli` helper used by `phantom serve
//! --remote-telegram` (B2/T83).

pub mod channel_trait;

#[cfg(feature = "experimental-remote-control-telegram")]
pub mod telegram;

#[cfg(feature = "experimental-remote-control-telegram")]
pub mod telegram_agent_dispatcher;

#[cfg(feature = "experimental-remote-control-whatsapp")]
pub mod whatsapp;

#[cfg(feature = "experimental-remote-control-slack")]
pub mod slack;

// T54 — bot persona TOML schema + per-channel binding (v0.6.0 V3 prep).
// Compiled whenever any remote-control channel subfeature is on (the umbrella
// pulls them all in). The persona module is data-only (parser + lookup);
// it pulls in no new transitive crates because `toml` and `serde` are
// already top-level dependencies. See
// `docs/remote-personas/example.toml` for the canonical schema.
#[cfg(any(
    feature = "experimental-remote-control",
    feature = "experimental-remote-control-whatsapp",
    feature = "experimental-remote-control-slack",
))]
pub mod persona;

// B7 / T88 — multi-bot HashMap dispatcher (v0.6.0 V3 prep).
// Gated under the umbrella `experimental-remote-control` flag so default `cargo
// build` stays byte-identical to baseline. Pulls in no new transitive crates
// (tokio's `sync::RwLock` and `std::collections::HashMap` are already
// available). See dispatcher.rs module docs for routing semantics.
#[cfg(feature = "experimental-remote-control")]
pub mod dispatcher;

// B9/T90 — per-channel token-bucket rate limiter. Pulls in dashmap + arc-swap
// (declared as transitive deps of every per-channel sub-feature so the slack /
// whatsapp standalone builds can use it too — see Cargo.toml feature graph).
// Default `cargo build` stays byte-identical to baseline because none of those
// flags is on. Channel impls call `PerChannelLimiter::check` from
// `send_message` before any HTTP call.
#[cfg(any(
    feature = "experimental-remote-control",
    feature = "experimental-remote-control-slack",
    feature = "experimental-remote-control-whatsapp",
))]
pub mod rate_limit;

// B8/T89 — channel-agnostic media (photo/voice/document) handler shared by
// telegram/whatsapp/slack. Bridges downloaded bytes into the existing
// `multimodal::prompt_to_content_value` sentinel pipeline. No new transitive
// crates: reqwest/futures/base64 are already top-level deps.
#[cfg(feature = "experimental-remote-control")]
pub mod media;

// B6 / T87 — applies a loaded `Persona` to the dispatched agent (intro +
// system-prompt prefix + tool registry filtering) across all three channel
// adapters. Channel modules MUST funnel persona lookups through
// `PersonaDispatcher` rather than indexing `persona.channels.get(name)`
// directly; see the module docs and the CI grep-check in
// `scripts/ci/check-persona-direct-access.sh` for the anti-pattern guard.
#[cfg(any(
    feature = "experimental-remote-control",
    feature = "experimental-remote-control-whatsapp",
    feature = "experimental-remote-control-slack",
))]
pub mod dispatch;

// B3 / T84 — constant-time validator for Telegram's
// `X-Telegram-Bot-Api-Secret-Token` webhook header. The `telegram.rs`
// runtime (B2/T83) imports `validate_telegram_secret_token` from here.
// Gated under the same feature flag so a `cargo build` (no flags) compiles
// zero new code.
#[cfg(feature = "experimental-remote-control-telegram")]
pub mod webhook_auth;

// V3 gap 2 (DEMO-1 PR #115 disclaimer) — cross-cutting `ChannelInboundAuth`
// trait. Wraps the per-channel auth primitives (B3 constant-time token for
// Telegram, B5 HMAC + replay for Slack, NotImplemented for WhatsApp) under
// one trait surface so the dispatcher can call `&dyn ChannelInboundAuth`
// without channel-specific branching. Compiled under any of the remote-control
// sub-features so umbrella + channel-only builds both see it; default
// `cargo build` still compiles zero new code.
#[cfg(any(
    feature = "experimental-remote-control-telegram",
    feature = "experimental-remote-control-slack",
    feature = "experimental-remote-control-whatsapp",
))]
pub mod inbound_auth;

pub use channel_trait::{Channel, ChannelError};

#[cfg(any(
    feature = "experimental-remote-control-telegram",
    feature = "experimental-remote-control-slack",
    feature = "experimental-remote-control-whatsapp",
))]
pub use inbound_auth::{AuthError, ChannelInboundAuth};

#[cfg(any(
    feature = "experimental-remote-control",
    feature = "experimental-remote-control-whatsapp",
    feature = "experimental-remote-control-slack",
))]
pub use dispatch::PersonaDispatcher;
#[cfg(any(
    feature = "experimental-remote-control",
    feature = "experimental-remote-control-whatsapp",
    feature = "experimental-remote-control-slack",
))]
pub use persona::{Persona, PersonaError};
