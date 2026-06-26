//! Remote-control bot persona — TOML schema + per-channel binding.
//!
//! The *voice* half of a Remote Control (BIG-GOAL §P3): a `Channel` is the
//! wire-protocol transport, a `Persona` is the identity the cluster wears
//! when it talks back through that transport. The same Telegram remote
//! can host `phantom-helper` for the operator's personal chat and
//! `support-bot` for a separate group; both still command the same
//! underlying cluster.
//!
//! A *persona* is the user-visible identity a bot wears on a given channel:
//! its name, intro message, conversational style, and the set of phantom
//! tools it is permitted to invoke. This module is the parser + lookup layer;
//! the dispatcher (and individual `Channel` impls) consume the resulting
//! [`Persona`] value to render greetings and gate tool execution.
//!
//! # Schema (canonical example)
//!
//! ```toml
//! [persona]
//! name = "phantom-helper"
//! description = "General-purpose phantom assistant"
//! intro_message = "Hi! I'm phantom-helper. Ask me anything."
//!
//! [persona.style]
//! tone = "professional"
//! verbosity = "concise"
//!
//! [persona.tools]
//! allowed = ["shell", "file_read", "file_edit"]
//! denied = ["bash_bg"]
//!
//! [persona.channels.telegram]
//! intro_message = "Hi! I'm phantom-helper bot. Type /help."
//! ```
//!
//! `[persona.channels.<name>]` blocks override the top-level fields they
//! mention, and only those fields. A channel block that omits
//! `intro_message` falls back to the top-level intro.
//!
//! # V3 prep notes
//!
//! - This module is **data-only**: it does not invoke channels, spawn tasks,
//!   or call into the LLM layer. It is safe to load eagerly at startup.
//! - Tool gating is exposed via [`Persona::is_tool_allowed`], which encodes
//!   the precedence rule "explicit deny beats explicit allow; otherwise
//!   default to allow when allowlist is empty, deny when allowlist is
//!   non-empty". This matches skill tool catalog semantics so the V3
//!   migration is mechanical.
//! - All public types are `Serialize + Deserialize` so the same struct can be
//!   round-tripped to disk for `phantom personas show` (planned) without a
//!   second schema definition.
//!
//! # Feature gating
//!
//! Compiled only with `--features experimental-remote-control`. Default `cargo
//! build` produces a byte-identical baseline binary.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Parser / IO errors surfaced from the persona module.
#[derive(Debug, thiserror::Error)]
pub enum PersonaError {
    /// Failed to read the persona file from disk.
    #[error("failed to read persona file at {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },

    /// TOML failed to parse or did not match the [`Persona`] schema.
    #[error("failed to parse persona TOML: {0}")]
    Parse(#[from] toml::de::Error),

    /// Schema validation that TOML alone can't enforce (empty name etc.).
    #[error("invalid persona: {0}")]
    Invalid(String),
}

/// Top-level wrapper matching `[persona]` table in the TOML file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PersonaFile {
    persona: Persona,
}

/// A bot persona definition.
///
/// Construct via [`Persona::parse_str`] or [`Persona::load_from`]; do not
/// build by hand in production code (the parser also runs validation).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Persona {
    /// Stable identifier used in logs and dispatch routing. Must be non-empty.
    pub name: String,

    /// Human-readable description (one-liner). Optional.
    #[serde(default)]
    pub description: String,

    /// Default greeting sent on first interaction. Optional; channel-specific
    /// overrides take precedence (see [`Persona::channel_intro`]).
    #[serde(default)]
    pub intro_message: String,

    /// Conversational style hints. Optional with field-level defaults.
    #[serde(default)]
    pub style: PersonaStyle,

    /// Tool allow/deny lists. Optional; empty allow list = "no restriction".
    #[serde(default)]
    pub tools: PersonaTools,

    /// Per-channel overrides keyed by short channel name (e.g. `"telegram"`,
    /// `"slack"`). Unknown channels are simply ignored at lookup time.
    ///
    /// Stored as `BTreeMap` for deterministic serialization (round-trip tests
    /// rely on stable key order).
    #[serde(default)]
    pub channels: BTreeMap<String, PersonaChannelOverride>,
}

/// Conversational style controls. All fields default to empty strings so a
/// minimal persona file (just `name`) parses cleanly.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PersonaStyle {
    /// Free-form tone hint, e.g. `"professional"`, `"casual"`, `"playful"`.
    #[serde(default)]
    pub tone: String,

    /// Verbosity hint, e.g. `"concise"`, `"detailed"`.
    #[serde(default)]
    pub verbosity: String,
}

/// Tool allow/deny lists.
///
/// Semantics (matches the skill tool catalog gating, see V3 prep notes in module
/// docs):
///
/// - If `allowed` is empty: every tool is permitted *unless* listed in `denied`.
/// - If `allowed` is non-empty: only tools in `allowed` are permitted, and a
///   listing in `denied` overrides any listing in `allowed` (explicit deny
///   wins).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PersonaTools {
    #[serde(default)]
    pub allowed: Vec<String>,
    #[serde(default)]
    pub denied: Vec<String>,
}

/// Per-channel override block. Only the fields the operator wants to change
/// need to be present; missing fields fall back to the top-level [`Persona`].
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PersonaChannelOverride {
    /// Override greeting for this channel. `None` means "use top-level".
    #[serde(default)]
    pub intro_message: Option<String>,
}

impl Persona {
    /// Parse a persona definition from a TOML string.
    pub fn parse_str(s: &str) -> Result<Self, PersonaError> {
        let file: PersonaFile = toml::from_str(s)?;
        let persona = file.persona;
        persona.validate()?;
        Ok(persona)
    }

    /// Load a persona from a TOML file on disk.
    pub fn load_from(path: &Path) -> Result<Self, PersonaError> {
        let bytes = fs::read_to_string(path).map_err(|source| PersonaError::Io {
            path: path.display().to_string(),
            source,
        })?;
        Self::parse_str(&bytes)
    }

    /// Serialize the persona back to TOML (round-trip support for
    /// `phantom personas show` and tests).
    pub fn to_toml_string(&self) -> Result<String, toml::ser::Error> {
        let wrapper = PersonaFile {
            persona: self.clone(),
        };
        toml::to_string(&wrapper)
    }

    /// Resolve the intro message for a given channel name.
    ///
    /// Returns the channel-specific override if present (and non-empty);
    /// otherwise the top-level `intro_message` if non-empty; otherwise `None`.
    pub fn channel_intro(&self, channel: &str) -> Option<&str> {
        if let Some(over) = self.channels.get(channel) {
            if let Some(text) = over.intro_message.as_deref() {
                if !text.is_empty() {
                    return Some(text);
                }
            }
        }
        if self.intro_message.is_empty() {
            None
        } else {
            Some(self.intro_message.as_str())
        }
    }

    /// Returns true if `tool` is permitted under this persona.
    ///
    /// See [`PersonaTools`] for full precedence rules.
    pub fn is_tool_allowed(&self, tool: &str) -> bool {
        if self.tools.denied.iter().any(|t| t == tool) {
            return false;
        }
        if self.tools.allowed.is_empty() {
            return true;
        }
        self.tools.allowed.iter().any(|t| t == tool)
    }

    /// Build the system-prompt prefix string the dispatched agent should
    /// receive *in addition* to phantom's built-in system prompt.
    ///
    /// The prefix is empty when neither tone nor verbosity is set, so it can
    /// be unconditionally prepended without growing the prompt for the
    /// minimal-persona case.
    ///
    /// Returned shape (lines joined with `\n`, trailing newline only when
    /// non-empty):
    ///
    /// ```text
    /// Persona: <name>
    /// Tone: <tone>
    /// Verbosity: <verbosity>
    /// ```
    ///
    /// Empty fields are skipped (the "Tone:" / "Verbosity:" lines disappear
    /// rather than emitting `Tone: ` with a trailing space).
    pub fn system_prompt_prefix(&self) -> String {
        let mut lines: Vec<String> = Vec::new();
        // Always include the name line when *either* style field is set
        // (provides identity context for tone-only adjustments), or when
        // an intro_message exists (so the agent knows who it's playing).
        let has_style = !self.style.tone.is_empty() || !self.style.verbosity.is_empty();
        if has_style || !self.intro_message.is_empty() {
            lines.push(format!("Persona: {}", self.name));
        }
        if !self.style.tone.is_empty() {
            lines.push(format!("Tone: {}", self.style.tone));
        }
        if !self.style.verbosity.is_empty() {
            lines.push(format!("Verbosity: {}", self.style.verbosity));
        }
        if lines.is_empty() {
            String::new()
        } else {
            let mut out = lines.join("\n");
            out.push('\n');
            out
        }
    }

    /// Filter a candidate tool registry down to the persona-permitted subset,
    /// preserving the *input* order. Strings are compared as-is (no case
    /// folding).
    ///
    /// This is the helper channel adapters should pass their tool registry
    /// through before handing the registry to the dispatched agent.
    pub fn filter_tool_registry<'a, S: AsRef<str>>(&self, tools: &'a [S]) -> Vec<&'a str> {
        tools
            .iter()
            .map(AsRef::as_ref)
            .filter(|t| self.is_tool_allowed(t))
            .collect()
    }

    fn validate(&self) -> Result<(), PersonaError> {
        if self.name.trim().is_empty() {
            return Err(PersonaError::Invalid(
                "persona.name must be a non-empty string".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"
        [persona]
        name = "min"
    "#;

    const FULL: &str = r#"
        [persona]
        name = "phantom-helper"
        description = "General-purpose phantom assistant"
        intro_message = "Hi! I'm phantom-helper."

        [persona.style]
        tone = "professional"
        verbosity = "concise"

        [persona.tools]
        allowed = ["shell", "file_read", "file_edit"]
        denied = ["bash_bg"]

        [persona.channels.telegram]
        intro_message = "Hi! I'm phantom-helper bot. Type /help."

        [persona.channels.slack]
        intro_message = ""
    "#;

    #[test]
    fn parse_minimal_persona_uses_defaults() {
        let p = Persona::parse_str(MINIMAL).expect("minimal persona parses");
        assert_eq!(p.name, "min");
        assert_eq!(p.description, "");
        assert_eq!(p.intro_message, "");
        assert_eq!(p.style, PersonaStyle::default());
        assert_eq!(p.tools, PersonaTools::default());
        assert!(p.channels.is_empty());
    }

    #[test]
    fn parse_full_persona_round_trips_through_toml() {
        let p = Persona::parse_str(FULL).expect("full persona parses");
        let serialized = p.to_toml_string().expect("serialize");
        let p2 = Persona::parse_str(&serialized).expect("re-parse serialized");
        assert_eq!(p, p2);
    }

    #[test]
    fn channel_intro_prefers_channel_override() {
        let p = Persona::parse_str(FULL).unwrap();
        assert_eq!(
            p.channel_intro("telegram"),
            Some("Hi! I'm phantom-helper bot. Type /help.")
        );
    }

    #[test]
    fn channel_intro_falls_back_to_top_level_when_override_empty() {
        // Slack override has an empty string — should NOT be returned;
        // we should fall back to the top-level intro.
        let p = Persona::parse_str(FULL).unwrap();
        assert_eq!(p.channel_intro("slack"), Some("Hi! I'm phantom-helper."));
    }

    #[test]
    fn channel_intro_returns_top_level_for_unknown_channel() {
        let p = Persona::parse_str(FULL).unwrap();
        assert_eq!(p.channel_intro("whatsapp"), Some("Hi! I'm phantom-helper."));
    }

    #[test]
    fn channel_intro_returns_none_when_nothing_defined() {
        let p = Persona::parse_str(MINIMAL).unwrap();
        assert_eq!(p.channel_intro("telegram"), None);
        assert_eq!(p.channel_intro("anything"), None);
    }

    #[test]
    fn tool_gating_empty_allowlist_means_open_except_denied() {
        let p = Persona::parse_str(MINIMAL).unwrap();
        assert!(p.is_tool_allowed("shell"));
        assert!(p.is_tool_allowed("anything_at_all"));
    }

    #[test]
    fn tool_gating_allowlist_restricts_set() {
        let p = Persona::parse_str(FULL).unwrap();
        assert!(p.is_tool_allowed("shell"));
        assert!(p.is_tool_allowed("file_read"));
        assert!(!p.is_tool_allowed("network_fetch"), "not in allowlist");
    }

    #[test]
    fn tool_gating_explicit_deny_beats_allow() {
        // bash_bg is in *neither* allowed (in FULL) nor — but specifically
        // listed in denied, which must override even an open allowlist.
        let p = Persona::parse_str(FULL).unwrap();
        assert!(!p.is_tool_allowed("bash_bg"));

        // Sanity: if we hand-craft a persona where the same tool appears in
        // both lists, deny still wins.
        let p2 = Persona {
            name: "x".into(),
            description: String::new(),
            intro_message: String::new(),
            style: PersonaStyle::default(),
            tools: PersonaTools {
                allowed: vec!["shell".into()],
                denied: vec!["shell".into()],
            },
            channels: BTreeMap::new(),
        };
        assert!(!p2.is_tool_allowed("shell"));
    }

    // ── B6 / T87: persona-application helpers ─────────────────────────────

    #[test]
    fn system_prompt_prefix_is_empty_for_minimal_persona() {
        // Minimal persona (name only, no style, no intro_message) must
        // produce an empty prefix so back-compat is byte-identical: the
        // dispatched agent gets the same prompt as before the persona wiring.
        let p = Persona::parse_str(MINIMAL).unwrap();
        assert_eq!(p.system_prompt_prefix(), "");
    }

    #[test]
    fn system_prompt_prefix_includes_name_and_style_when_set() {
        let p = Persona::parse_str(FULL).unwrap();
        let prefix = p.system_prompt_prefix();
        assert!(
            prefix.starts_with("Persona: phantom-helper\n"),
            "got: {prefix:?}"
        );
        assert!(prefix.contains("Tone: professional\n"));
        assert!(prefix.contains("Verbosity: concise\n"));
        assert!(prefix.ends_with('\n'));
    }

    #[test]
    fn system_prompt_prefix_skips_empty_style_fields() {
        // Tone set, verbosity empty -> "Verbosity:" line must NOT appear.
        let toml_src = r#"
            [persona]
            name = "tone-only"
            [persona.style]
            tone = "playful"
        "#;
        let p = Persona::parse_str(toml_src).unwrap();
        let prefix = p.system_prompt_prefix();
        assert!(prefix.contains("Tone: playful"));
        assert!(!prefix.contains("Verbosity:"), "got: {prefix:?}");
    }

    #[test]
    fn filter_tool_registry_preserves_input_order() {
        let p = Persona::parse_str(FULL).unwrap();
        // Registry includes one denied ("bash_bg"), some allowed, one not in allowlist.
        let registry = [
            "file_edit",
            "bash_bg",
            "shell",
            "network_fetch",
            "file_read",
        ];
        let filtered = p.filter_tool_registry(&registry);
        assert_eq!(filtered, vec!["file_edit", "shell", "file_read"]);
    }

    #[test]
    fn filter_tool_registry_minimal_persona_is_identity() {
        // No allow/deny lists => filter must keep every tool in the same
        // order (back-compat: minimal persona = no change).
        let p = Persona::parse_str(MINIMAL).unwrap();
        let registry = ["a", "b", "c"];
        let filtered = p.filter_tool_registry(&registry);
        assert_eq!(filtered, vec!["a", "b", "c"]);
    }

    #[test]
    fn channel_intro_telegram_override_used() {
        // Spec test #2: persona with [persona.channels.telegram] intro_message = "TG hi"
        // => telegram uses "TG hi".
        let toml_src = r#"
            [persona]
            name = "p"
            intro_message = "top"
            [persona.channels.telegram]
            intro_message = "TG hi"
        "#;
        let p = Persona::parse_str(toml_src).unwrap();
        assert_eq!(p.channel_intro("telegram"), Some("TG hi"));
    }

    #[test]
    fn channel_intro_slack_empty_falls_through() {
        // Spec test #3: persona with [persona.channels.slack] intro_message = ""
        // => slack falls through to top-level intro_message.
        let toml_src = r#"
            [persona]
            name = "p"
            intro_message = "top"
            [persona.channels.slack]
            intro_message = ""
        "#;
        let p = Persona::parse_str(toml_src).unwrap();
        assert_eq!(p.channel_intro("slack"), Some("top"));
    }

    #[test]
    fn denied_tool_is_not_callable_after_filter() {
        // Spec test #4: denied tool not callable.
        let toml_src = r#"
            [persona]
            name = "p"
            [persona.tools]
            denied = ["bash_bg"]
        "#;
        let p = Persona::parse_str(toml_src).unwrap();
        assert!(!p.is_tool_allowed("bash_bg"));
        let registry = ["shell", "bash_bg", "file_edit"];
        let filtered = p.filter_tool_registry(&registry);
        assert!(!filtered.contains(&"bash_bg"), "filtered: {filtered:?}");
    }

    #[test]
    fn empty_name_is_rejected() {
        let bad = r#"[persona]
name = "  ""#;
        let err = Persona::parse_str(bad).unwrap_err();
        match err {
            PersonaError::Invalid(msg) => assert!(msg.contains("name")),
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn missing_persona_table_is_a_parse_error() {
        let bad = r#"name = "no-table""#;
        let err = Persona::parse_str(bad).unwrap_err();
        assert!(matches!(err, PersonaError::Parse(_)));
    }

    #[test]
    fn load_from_reads_file_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.toml");
        std::fs::write(&path, FULL).unwrap();
        let p = Persona::load_from(&path).expect("load_from succeeds");
        assert_eq!(p.name, "phantom-helper");
    }

    #[test]
    fn load_from_missing_file_returns_io_error() {
        let path = Path::new("/definitely/does/not/exist/p.toml");
        let err = Persona::load_from(path).unwrap_err();
        assert!(matches!(err, PersonaError::Io { .. }));
    }

    /// Locks the shipped docs to the schema: if either example file ever
    /// drifts from the parser, this fails loudly instead of users hitting
    /// the bad TOML at startup.
    #[test]
    fn shipped_example_files_parse() {
        // CARGO_MANIFEST_DIR points at .../core, so docs/remote-personas
        // sits one level up.
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("core has parent");
        for name in ["example.toml", "assistant-friendly.toml"] {
            let path = root.join("docs").join("remote-personas").join(name);
            let p = Persona::load_from(&path)
                .unwrap_or_else(|e| panic!("failed to load shipped {name}: {e}"));
            assert!(!p.name.is_empty(), "{name} must have a name");
        }
    }
}
