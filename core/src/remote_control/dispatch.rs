//! Remote-control persona dispatcher — gives every Remote Control (BIG-GOAL §P3)
//! a consistent voice by applying the loaded [`Persona`] to channel adapters.
//!
//! The channel is the *transport* of the remote; persona is the *voice*
//! that reports back. This module ensures both stay in sync: the same
//! cluster command issued from Telegram, Slack, or WhatsApp gets the
//! same persona-shaped reply, while still allowing per-channel overrides
//! (e.g. "use markdown on Slack, plain text on WhatsApp") through the
//! [`Persona::channel_intro`] fallback chain.
//!
//! **B6 / T87**: this module is the *only* sanctioned path from channel
//! adapters (`telegram.rs`, `whatsapp.rs`, `slack.rs`) to persona state.
//! Channel-adapter code MUST go through [`PersonaDispatcher::channel_intro`]
//! rather than indexing `persona.channels.get(name)` directly — otherwise
//! the per-channel fallback semantics (empty override -> top-level intro)
//! are silently dropped and bots greet users with empty strings.
//!
//! See [`Persona::channel_intro`] for the fallback contract this dispatcher
//! preserves, and `scripts/ci/check-persona-direct-access.sh` for the
//! CI grep-check that enforces the anti-pattern guard.
//!
//! ## Surface
//!
//! Three calls cover the dispatched-agent integration:
//!
//! * [`PersonaDispatcher::channel_intro`] — welcome / reply prefix.
//! * [`PersonaDispatcher::system_prompt_prefix`] — tone + verbosity hints
//!   prepended to phantom's built-in system prompt.
//! * [`PersonaDispatcher::filter_tools`] — allow/deny gating applied to the
//!   tool registry handed to the dispatched agent.
//!
//! All three are pure, allocation-light, and a no-op when the dispatcher
//! holds `None` (i.e. no persona was loaded — back-compat path).

use super::persona::Persona;

/// Holds an `Option<&Persona>` and exposes the three operations channel
/// adapters need. Cheap to construct (just wraps a reference) so adapters
/// can build one per message.
#[derive(Debug, Clone, Copy)]
pub struct PersonaDispatcher<'a> {
    persona: Option<&'a Persona>,
}

impl<'a> PersonaDispatcher<'a> {
    /// Construct from an optional persona. `None` => every operation is a
    /// pass-through (back-compat: empty persona means no behavior change).
    pub fn new(persona: Option<&'a Persona>) -> Self {
        Self { persona }
    }

    /// Resolve the intro / reply prefix for `channel_name` (e.g.
    /// `"telegram"`, `"whatsapp"`, `"slack"`).
    ///
    /// Always goes through [`Persona::channel_intro`] so the per-channel
    /// fallback (empty override -> top-level intro) is honored.
    pub fn channel_intro(&self, channel_name: &str) -> Option<&'a str> {
        self.persona.and_then(|p| p.channel_intro(channel_name))
    }

    /// System-prompt prefix the dispatched agent should receive. Empty
    /// string when no persona, so unconditional prepending stays byte-equal
    /// to baseline.
    pub fn system_prompt_prefix(&self) -> String {
        self.persona
            .map(Persona::system_prompt_prefix)
            .unwrap_or_default()
    }

    /// Filter the candidate tool registry through the persona's
    /// allowed/denied lists. With no persona, returns every tool unchanged
    /// (back-compat).
    pub fn filter_tools<'r, S: AsRef<str>>(&self, tools: &'r [S]) -> Vec<&'r str> {
        match self.persona {
            Some(p) => p.filter_tool_registry(tools),
            None => tools.iter().map(AsRef::as_ref).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::persona::Persona;
    use super::*;

    const FULL: &str = r#"
        [persona]
        name = "phantom-helper"
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
    fn empty_persona_is_no_op_back_compat() {
        // Spec test #1: empty persona (None) => no behavior change.
        let d = PersonaDispatcher::new(None);
        assert_eq!(d.channel_intro("telegram"), None);
        assert_eq!(d.system_prompt_prefix(), "");
        let reg = ["shell", "bash_bg", "anything"];
        assert_eq!(d.filter_tools(&reg), vec!["shell", "bash_bg", "anything"]);
    }

    #[test]
    fn dispatcher_telegram_uses_channel_override() {
        // Spec test #2.
        let p = Persona::parse_str(FULL).unwrap();
        let d = PersonaDispatcher::new(Some(&p));
        assert_eq!(d.channel_intro("telegram"), Some("TG hi"));
    }

    #[test]
    fn dispatcher_slack_empty_override_falls_through_to_top_level() {
        // Spec test #3 — guards the anti-pattern: a direct
        // `persona.channels.get("slack").intro_message.as_deref()` would
        // return Some("") here, sending an empty greeting. The dispatcher
        // routes through `channel_intro` which falls through to top-level.
        let p = Persona::parse_str(FULL).unwrap();
        let d = PersonaDispatcher::new(Some(&p));
        assert_eq!(d.channel_intro("slack"), Some("top"));
    }

    #[test]
    fn dispatcher_whatsapp_unknown_channel_uses_top_level() {
        // No [persona.channels.whatsapp] block -> top-level intro applies.
        let p = Persona::parse_str(FULL).unwrap();
        let d = PersonaDispatcher::new(Some(&p));
        assert_eq!(d.channel_intro("whatsapp"), Some("top"));
    }

    #[test]
    fn dispatcher_denied_tool_filtered_out_of_registry() {
        // Spec test #4: denied tool not callable after dispatcher filter.
        let p = Persona::parse_str(FULL).unwrap();
        let d = PersonaDispatcher::new(Some(&p));
        let reg = ["shell", "bash_bg", "file_read"];
        let filtered = d.filter_tools(&reg);
        assert_eq!(filtered, vec!["shell", "file_read"]);
        assert!(!filtered.contains(&"bash_bg"));
    }

    #[test]
    fn dispatcher_system_prompt_prefix_contains_style_hints() {
        let p = Persona::parse_str(FULL).unwrap();
        let d = PersonaDispatcher::new(Some(&p));
        let prefix = d.system_prompt_prefix();
        assert!(prefix.contains("Persona: phantom-helper"));
        assert!(prefix.contains("Tone: professional"));
        assert!(prefix.contains("Verbosity: concise"));
    }
}
