//! Data-driven registry for external one-shot gateway CLIs.
//!
//! Brand command strings are consolidated in REGISTRY and nowhere else — no
//! other core module hardcodes gateway program names. To add a new gateway,
//! edit REGISTRY. To remove all brand names from the build, replace REGISTRY
//! with an empty slice.

/// Spec for an external gateway CLI driven via the cli_session substrate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalGatewaySpec {
    /// Provider key (matches [providers.KEY] in agents.toml).
    pub key: &'static str,
    /// Executable name to spawn.
    pub program: &'static str,
    /// Fixed args prepended before the user prompt.
    pub args: &'static [&'static str],
    /// How to parse the program's stdout into events.
    pub output_style: ExternalOutputStyle,
}

/// Stdout parsing strategy for external gateway CLIs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExternalOutputStyle {
    /// JSON payload array — reply is in `payloads[].text`.
    JsonPayload,
    /// Plain text / oneshot — entire stdout is the answer.
    PlainText,
}

/// Built-in external gateway CLIs. Brand command strings live only here.
pub static REGISTRY: &[ExternalGatewaySpec] = &[
    ExternalGatewaySpec {
        key: "openclaw_agent",
        program: "openclaw",
        args: &["agent", "--local", "--agent", "main", "--json", "--message"],
        output_style: ExternalOutputStyle::JsonPayload,
    },
    ExternalGatewaySpec {
        key: "hermes_agent",
        program: "hermes",
        args: &["-z"],
        output_style: ExternalOutputStyle::PlainText,
    },
];

/// Look up a gateway spec by its provider key (e.g. `"openclaw_agent"`).
pub fn lookup(key: &str) -> Option<&'static ExternalGatewaySpec> {
    REGISTRY.iter().find(|s| s.key == key)
}

/// Look up by program name (e.g. `"openclaw"`) or provider key.
pub fn lookup_by_name(name: &str) -> Option<&'static ExternalGatewaySpec> {
    REGISTRY
        .iter()
        .find(|s| s.program == name || s.key == name || s.key == format!("{}_agent", name))
}

/// All registered external gateway CLIs.
pub fn all() -> &'static [ExternalGatewaySpec] {
    REGISTRY
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_well_formed_lookups_resolve_and_unknown_fails_closed() {
        // Registry integrity: keys are unique, and every entry is resolvable by
        // BOTH lookup(key) and lookup_by_name(program). An entry that resolves
        // one way but not the other — or a duplicate key shadowing another —
        // would silently fail to run when configured.
        let mut seen = std::collections::HashSet::new();
        for spec in all() {
            assert!(seen.insert(spec.key), "duplicate registry key: {}", spec.key);
            assert_eq!(lookup(spec.key), Some(spec), "lookup(key) must find {}", spec.key);
            assert_eq!(
                lookup_by_name(spec.program),
                Some(spec),
                "lookup_by_name(program) must find {}",
                spec.program
            );
            // lookup_by_name also accepts the exact provider key.
            assert_eq!(lookup_by_name(spec.key), Some(spec), "by key {}", spec.key);
        }
        // Fail-closed: an unknown key/name must never accidentally match.
        assert!(lookup("nonexistent").is_none());
        assert!(lookup_by_name("nonexistent-gateway").is_none());
    }
}
