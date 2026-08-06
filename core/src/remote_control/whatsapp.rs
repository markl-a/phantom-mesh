//! WhatsApp channel — Remote Control surface (BIG-GOAL §P3), **STUB ONLY**.
//!
//! Will eventually let any WhatsApp user issue cluster commands the same
//! way Telegram and Slack do; until then this module exists so the
//! `Channel` trait + dispatcher can be wired today, with operator-facing
//! errors that point at the runbook instead of pretending to be live.
//!
//! Real implementation is deferred (Meta Business verification is 1-3 weeks
//! per spec §3 non-goals — see `docs/superpowers/runbooks/` for the
//! operator's verify-then-enable flow). Calls to `send_message` return
//! [`ChannelError::NotImplemented`] so any caller that mistakenly routes
//! traffic here gets a loud, structured error rather than a fake success.

use std::sync::Arc;

use async_trait::async_trait;

use super::channel_trait::{Channel, ChannelError};
use super::dispatch::PersonaDispatcher;
use super::persona::Persona;
use super::rate_limit::PerChannelLimiter;

const CHANNEL_NAME: &str = "whatsapp";
const STUB_REASON: &str =
    "WhatsApp Cloud API integration deferred — Meta Business verification pending (spec §5 O2 best-effort)";

/// Compile-only stub. Construction never fails; sending always fails with
/// [`ChannelError::NotImplemented`] (unless the local rate limiter rejects
/// the call first, in which case [`ChannelError::RateLimited`] is returned —
/// this proves the limiter wiring is live ahead of the real impl).
pub struct WhatsappStub {
    allowed_users: Vec<i64>,
    /// Optional local rate limiter (B9/T90). See `slack::SlackStub::limiter`
    /// for the design rationale — both stubs share the same opt-in pattern.
    limiter: Option<Arc<PerChannelLimiter>>,
    /// B6 / T87: optional persona applied to dispatched agent. `None` means
    /// "no persona configured" and every persona helper is a no-op
    /// (back-compat: byte-identical greeting behavior).
    persona: Option<Persona>,
}

impl WhatsappStub {
    /// Create a stub with no allowlist (open access — but every send still
    /// fails with NotImplemented, so this is harmless).
    pub fn new() -> Self {
        Self {
            allowed_users: Vec::new(),
            limiter: None,
            persona: None,
        }
    }

    /// Create a stub with a fixed allowlist. Provided so the same wiring
    /// code that configures the real channel later still type-checks.
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
    /// persona via [`WhatsappStub::dispatcher`] (which routes through
    /// `Persona::channel_intro`) rather than indexing `persona.channels`
    /// directly. See `remote_control::dispatch` for the anti-pattern guard.
    pub fn with_persona(mut self, persona: Persona) -> Self {
        self.persona = Some(persona);
        self
    }

    /// Borrow the persona (for callers that need to inspect raw fields,
    /// e.g. tests). Returns `None` when no persona was attached.
    pub fn persona(&self) -> Option<&Persona> {
        self.persona.as_ref()
    }

    /// Build a [`PersonaDispatcher`] bound to this stub's persona (or a
    /// no-op dispatcher when no persona is set). The dispatched-agent
    /// integration point uses this for intro / system prompt / tool gating.
    pub fn dispatcher(&self) -> PersonaDispatcher<'_> {
        PersonaDispatcher::new(self.persona.as_ref())
    }
}

impl Default for WhatsappStub {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Channel for WhatsappStub {
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

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r#"
        [persona]
        name = "spectyn-helper"
        intro_message = "top"
        [persona.style]
        tone = "professional"
        verbosity = "concise"
        [persona.tools]
        allowed = ["shell", "file_read"]
        denied = ["bash_bg"]
        [persona.channels.telegram]
        intro_message = "TG hi"
        [persona.channels.slack]
        intro_message = ""
    "#;

    #[test]
    fn whatsapp_empty_persona_dispatcher_is_noop() {
        // Spec test #1: no persona => dispatcher returns None / "" / unchanged.
        let s = WhatsappStub::new();
        let d = s.dispatcher();
        assert_eq!(d.channel_intro("whatsapp"), None);
        assert_eq!(d.system_prompt_prefix(), "");
        let reg = ["shell", "bash_bg"];
        assert_eq!(d.filter_tools(&reg), vec!["shell", "bash_bg"]);
    }

    #[test]
    fn whatsapp_dispatcher_uses_top_level_intro() {
        // whatsapp has no per-channel override in FULL => top-level intro.
        let p = Persona::parse_str(FULL).unwrap();
        let s = WhatsappStub::new().with_persona(p);
        assert_eq!(s.dispatcher().channel_intro("whatsapp"), Some("top"));
    }

    #[test]
    fn whatsapp_dispatcher_denied_tool_not_callable() {
        // Spec test #4: denied tool must not survive the dispatcher filter.
        let p = Persona::parse_str(FULL).unwrap();
        let s = WhatsappStub::new().with_persona(p);
        let reg = ["bash_bg", "shell", "file_read"];
        let filtered = s.dispatcher().filter_tools(&reg);
        assert!(!filtered.contains(&"bash_bg"), "filtered: {filtered:?}");
    }
}
