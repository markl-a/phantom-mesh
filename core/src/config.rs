use serde::Deserialize;
use std::collections::HashMap;

// ── Valid tool names (mirrors tools/mod.rs execute() match arms) ───────────
pub const VALID_TOOLS: &[&str] = &[
    "shell",
    "file_read",
    "file_write",
    "file_edit",
    "content_search",
    "glob_search",
    "web_search",
    "memory_store",
    "memory_recall",
    "git_status",
    "git_diff",
    "git_log",
    "git_commit",
    // Multi-agent / multi-machine orchestration. Both `task` and
    // `parallel_tasks` accept a `node:` parameter that dispatches to a
    // peer in the cluster, so a master agent can plan work across the
    // whole mesh from natural language. The `cluster_*` tools are the
    // read-only "what's reachable" queries that pair with them.
    "task",
    "parallel_tasks",
    "cluster_status",
    "cluster_sessions",
    "cluster_peers",
];

// ── Default value helpers ─────────────────────────────────────────────────

fn default_host() -> String { "0.0.0.0".into() }
fn default_port() -> u16 { 7878 }

fn default_model() -> String { "claude-sonnet-4-5-20251022".into() }
fn default_max_rounds() -> usize { 25 }
fn default_token_budget() -> usize { 100_000 }

fn default_system_prompt() -> String {
    "\
You are a capable AI coding assistant operating inside a software project workspace.
You have access to a set of tools and you should use them proactively — never describe
what you *would* do; just call the tool.

## Available tool categories
- **File I/O**: `file_read`, `file_write`, `file_edit` — read before editing; use exact
  strings for edits; prefer atomic, minimal diffs.
- **Search**: `content_search` (ripgrep), `glob_search` — locate symbols, usages, or
  files before changing them.
- **Shell**: `shell` — run build/test/lint commands; verify changes with `cargo check`,
  `npm run build`, or equivalent after editing.
- **Git**: `git_status`, `git_diff`, `git_log`, `git_commit` — inspect repo state and
  make atomic, well-described commits.
- **Memory**: `memory_store`, `memory_recall` — persist context across rounds.
- **Web**: `web_search` — look up docs, crates, packages, or error messages.
- **Multi-machine**: `cluster_status` / `cluster_peers` / `cluster_sessions` to discover
  what's reachable; then `task({agent:'coder', prompt:'...', node:'ayaneo'})` or
  `parallel_tasks({tasks:[{node:'mac1',...},{node:'ayaneo',...}]})` to actually delegate.
  Use this when the user says things like 'have ayaneo run the tests while mac1 builds the
  iOS target' — you don't need them to specify exact URLs, just match the peer name.

## Coding workflow
1. **Understand first** — read relevant files with `file_read` before making changes.
2. **Edit atomically** — make the smallest correct change; use `file_edit` with exact
   `old_string` / `new_string` pairs.
3. **Verify after changes** — run the project's build/test command via `shell` to catch
   errors immediately.
4. **Commit coherently** — stage related changes together; write a descriptive commit
   message in the imperative mood.
5. **Search before assuming** — use `content_search` or `glob_search` to locate
   definitions, usages, or configuration rather than guessing paths.
"
    .into()
}

fn default_tools_list() -> Vec<String> {
    VALID_TOOLS.iter().map(|s| s.to_string()).collect()
}

// ── Top-level config ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct AgentsConfig {
    #[serde(default)]
    pub core: CoreConfig,
    #[serde(default)]
    pub providers: HashMap<String, ProviderEntry>,
    #[serde(default)]
    pub agent: HashMap<String, AgentEntry>,
    #[serde(default)]
    pub tools: ToolsConfig,
    #[serde(default)]
    pub cluster: crate::mesh::ClusterConfig,
    /// Per-machine workspace pin — when set, `phantom` (no args) auto-cd
    /// to default_dir before launching the TUI, pre-selects pinned_agent,
    /// and the resulting conversation history lives under that path's
    /// cwd-hash. Lets you keep one Windows box dedicated to one project
    /// without remembering which dir to be in.
    #[serde(default)]
    pub workspace: WorkspaceConfig,
    /// `[permissions]` block — Tool(specifier) DSL rules. See
    /// `permission` module. Empty/missing means "no rules", which
    /// preserves legacy `PHANTOM_PERM=allow` behaviour (allow all).
    #[serde(default)]
    pub permissions: PermissionsConfig,
    #[serde(default)]
    pub telegram: Option<crate::TelegramConfig>,
    /// External MCP servers to launch as children at startup. Their tools are
    /// re-exposed to the agent under a `<server_name>_<tool>` namespace.
    #[serde(default, rename = "mcp_servers")]
    pub mcp_servers: Vec<crate::mcp_client::McpServerConfig>,
    /// Default model used when an agent does not specify one.
    #[serde(default = "default_model")]
    pub default_model: String,
    /// Maximum agentic rounds per run.
    #[serde(default = "default_max_rounds")]
    pub max_rounds: usize,
    /// Context token budget before compaction.
    #[serde(default = "default_token_budget")]
    pub token_budget: usize,
    /// Default system prompt injected when an agent has no instructions.
    #[serde(default = "default_system_prompt")]
    pub system_prompt: String,
}

impl Default for AgentsConfig {
    fn default() -> Self {
        Self::with_defaults()
    }
}

impl AgentsConfig {
    /// Return a fully-populated default config ready for use without a TOML file.
    pub fn with_defaults() -> Self {
        let mut agent = HashMap::new();
        agent.insert(
            "master".into(),
            AgentEntry {
                provider: "anthropic".into(),
                providers: None,
                model: default_model(),
                tools: default_tools_list(),
                instructions: default_system_prompt(),
            },
        );

        let mut providers = HashMap::new();
        providers.insert(
            "anthropic".into(),
            ProviderEntry {
                provider_type: "anthropic".into(),
                url: None,
                api_key: None,
                api_key_env: Some("ANTHROPIC_API_KEY".into()),
                default_model: Some(default_model()),
                tier: None,
            },
        );

        Self {
            core: CoreConfig::default(),
            providers,
            agent,
            tools: ToolsConfig::default(),
            cluster: crate::mesh::ClusterConfig::default(),
            workspace: WorkspaceConfig::default(),
            permissions: PermissionsConfig::default(),
            telegram: None,
            mcp_servers: Vec::new(),
            default_model: default_model(),
            max_rounds: default_max_rounds(),
            token_budget: default_token_budget(),
            system_prompt: default_system_prompt(),
        }
    }

    // ── Validation ────────────────────────────────────────────────────────

    /// Validate the configuration and return all errors at once (not just the first).
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors: Vec<String> = Vec::new();

        // At least one provider must have an api_key configured (directly or via env var).
        let has_key = self.providers.values().any(|p| {
            p.api_key.is_some()
                || p.api_key_env
                    .as_deref()
                    .and_then(|env| std::env::var(env).ok())
                    .map(|v| !v.is_empty())
                    .unwrap_or(false)
        });
        if !has_key && !self.providers.is_empty() {
            errors.push(
                "No provider has an api_key or a resolvable api_key_env configured".into(),
            );
        }

        // Each agent must reference a valid provider name.
        for (agent_name, entry) in &self.agent {
            if !entry.provider.is_empty() && !self.providers.contains_key(&entry.provider) {
                errors.push(format!(
                    "Agent '{}' references unknown provider '{}'",
                    agent_name, entry.provider
                ));
            }
        }

        // Tool names referenced by agents must be known. External MCP tools
        // are accepted if they start with a configured `<server>_` prefix.
        let valid_set: std::collections::HashSet<&str> =
            VALID_TOOLS.iter().copied().collect();
        let mcp_prefixes: Vec<String> = self.mcp_servers.iter()
            .map(|s| format!("{}_", s.name)).collect();
        for (agent_name, entry) in &self.agent {
            for tool in &entry.tools {
                if valid_set.contains(tool.as_str()) { continue; }
                if mcp_prefixes.iter().any(|p| tool.starts_with(p)) { continue; }
                errors.push(format!(
                    "Agent '{}' references unknown tool '{}'",
                    agent_name, tool
                ));
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    // ── Environment variable overrides ────────────────────────────────────

    /// Apply well-known environment variables on top of the loaded config.
    pub fn apply_env_overrides(&mut self) {
        // ANTHROPIC_API_KEY → set on the "anthropic" provider (create if absent).
        if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
            if !key.is_empty() {
                let entry = self
                    .providers
                    .entry("anthropic".into())
                    .or_insert_with(|| ProviderEntry {
                        provider_type: "anthropic".into(),
                        url: None,
                        api_key: None,
                        api_key_env: None,
                        default_model: Some(self.default_model.clone()),
                        tier: None,
                    });
                entry.api_key = Some(key);
            }
        }

        // OPENAI_API_KEY → set on the "openai" provider (create if absent).
        if let Ok(key) = std::env::var("OPENAI_API_KEY") {
            if !key.is_empty() {
                let entry = self
                    .providers
                    .entry("openai".into())
                    .or_insert_with(|| ProviderEntry {
                        provider_type: "openai".into(),
                        url: None,
                        api_key: None,
                        api_key_env: None,
                        default_model: Some("gpt-4o".into()),
                        tier: None,
                    });
                entry.api_key = Some(key);
            }
        }

        // PHANTOM_MODEL → override default_model.
        if let Ok(model) = std::env::var("PHANTOM_MODEL") {
            if !model.is_empty() {
                self.default_model = model;
            }
        }

        // PHANTOM_MAX_ROUNDS → override max_rounds.
        if let Ok(val) = std::env::var("PHANTOM_MAX_ROUNDS") {
            if let Ok(n) = val.trim().parse::<usize>() {
                self.max_rounds = n;
            }
        }

        // PHANTOM_TOKEN_BUDGET → override token_budget.
        if let Ok(val) = std::env::var("PHANTOM_TOKEN_BUDGET") {
            if let Ok(n) = val.trim().parse::<usize>() {
                self.token_budget = n;
            }
        }
    }

    // ── Auto-discovery ────────────────────────────────────────────────────

    /// Search standard locations for a config file and parse the first one found.
    ///
    /// Search order:
    /// 1. `./agents.toml`
    /// 2. `./PHANTOM.toml`
    /// 3. `~/.phantom-mesh/agents.toml`
    /// 4. `~/.config/phantom-mesh/config.toml`
    pub fn find_and_load() -> Option<Self> {
        let candidates: Vec<std::path::PathBuf> = {
            let mut v: Vec<std::path::PathBuf> = vec![
                "./agents.toml".into(),
                "./PHANTOM.toml".into(),
            ];
            if let Some(home) = dirs::home_dir() {
                v.push(home.join(".phantom-mesh").join("agents.toml"));
                v.push(home.join(".config").join("phantom-mesh").join("config.toml"));
            }
            v
        };

        for path in candidates {
            if path.exists() {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(mut cfg) = toml::from_str::<AgentsConfig>(&content) {
                        cfg.resolve_env_vars();
                        cfg.apply_env_overrides();
                        return Some(cfg);
                    }
                }
            }
        }
        None
    }

    // ── Display summary ───────────────────────────────────────────────────

    /// Return a concise human-readable summary of this configuration.
    pub fn display_summary(&self) -> String {
        // Providers line.
        let providers_str = if self.providers.is_empty() {
            "(none)".into()
        } else {
            let mut parts: Vec<String> = self
                .providers
                .iter()
                .map(|(name, entry)| {
                    let model = entry
                        .default_model
                        .as_deref()
                        .unwrap_or(&self.default_model);
                    // Show a short model label (last segment after '-' groups).
                    let short_model = short_model_label(model);
                    format!("{} ({})", name, short_model)
                })
                .collect();
            parts.sort();
            parts.join(", ")
        };

        // Agents line.
        let agents_str = if self.agent.is_empty() {
            "(none)".into()
        } else {
            let mut names: Vec<&str> = self.agent.keys().map(|s| s.as_str()).collect();
            names.sort();
            // Mark "master" as the default.
            let parts: Vec<String> = names
                .iter()
                .map(|n| {
                    if *n == "master" {
                        format!("{} (default)", n)
                    } else {
                        n.to_string()
                    }
                })
                .collect();
            parts.join(", ")
        };

        // Tools line — collect union of all agent tools, fall back to VALID_TOOLS.
        let all_tools: std::collections::BTreeSet<String> = if self.agent.is_empty() {
            VALID_TOOLS.iter().map(|s| s.to_string()).collect()
        } else {
            self.agent
                .values()
                .flat_map(|a| a.tools.iter().cloned())
                .collect()
        };
        let tool_count = all_tools.len();
        let tool_names: Vec<&str> = all_tools.iter().map(|s| s.as_str()).collect();
        let tools_preview = if tool_names.len() > 5 {
            format!(
                "{}, ... ({} total)",
                tool_names[..5].join(", "),
                tool_count
            )
        } else {
            tool_names.join(", ")
        };

        format!(
            "Providers: {}\nAgents: {}\nTools: {}\nToken budget: {}\nMax rounds: {}\nDefault model: {}",
            providers_str,
            agents_str,
            tools_preview,
            format_number(self.token_budget),
            self.max_rounds,
            self.default_model,
        )
    }
}

/// Shorten a model ID to a brief human-readable label, e.g.
/// "claude-sonnet-4-5-20251022" → "claude-sonnet-4-5", "gpt-4o" → "gpt-4o".
fn short_model_label(model: &str) -> String {
    // Strip trailing date suffix like -20251022.
    let re_date = model.rfind('-').and_then(|pos| {
        let suffix = &model[pos + 1..];
        if suffix.len() == 8 && suffix.chars().all(|c| c.is_ascii_digit()) {
            Some(pos)
        } else {
            None
        }
    });
    if let Some(pos) = re_date {
        model[..pos].to_string()
    } else {
        model.to_string()
    }
}

/// Format a usize with thousands separators (e.g. 100000 → "100,000").
fn format_number(n: usize) -> String {
    let s = n.to_string();
    let mut result = String::new();
    for (i, ch) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(ch);
    }
    result.chars().rev().collect()
}

// ── Sub-structs ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ToolsConfig {
    #[serde(default)]
    pub brave_search_api_key: Option<String>,
}

/// `[permissions]` TOML block. Each list is a sequence of rule strings
/// in the `Tool(specifier)` DSL — see `crate::permission` for syntax
/// and evaluation order.
///
/// Example:
/// ```toml
/// [permissions]
/// deny  = ["Read(./.env)", "Read(./secrets/*)", "Bash(rm -rf *)"]
/// ask   = ["WebFetch", "Bash"]
/// allow = ["Bash(git status)", "Bash(cargo check)", "Read(./README.md)"]
/// ```
///
/// Default mode (when the user provides no rules at all) is allow-all,
/// preserving legacy `PHANTOM_PERM=allow` behaviour. Once any rule is
/// present, unmatched calls fall through to `Ask`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PermissionsConfig {
    #[serde(default)]
    pub deny: Vec<String>,
    #[serde(default)]
    pub ask: Vec<String>,
    #[serde(default)]
    pub allow: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CoreConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default)]
    pub hub_api_key: Option<String>,
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self { host: default_host(), port: default_port(), hub_api_key: None }
    }
}

/// Per-machine workspace pin. All fields optional — empty/missing block
/// means "no pin, phantom uses caller's cwd as before".
///
/// When `default_dir` is set, the bare `phantom` command (no args, no
/// flags, no prompt) cd's to that dir before launching the TUI so the
/// session, conversation history, and tool actions all happen relative
/// to the pinned project. Saves having to remember which dir to be in
/// on each machine.
///
/// `pinned_agent` overrides the default agent selection (which is
/// "master" otherwise). Useful when a machine is dedicated to e.g. a
/// build-runner role and you always want `[agent.build]` selected.
///
/// `auto_open_tui` is reserved for future use — currently a no-op.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct WorkspaceConfig {
    #[serde(default)]
    pub default_dir: Option<String>,
    #[serde(default)]
    pub pinned_agent: Option<String>,
    #[serde(default)]
    pub auto_open_tui: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ProviderEntry {
    #[serde(rename = "type", default)]
    pub provider_type: String,
    #[serde(default, alias = "base_url")]
    pub url: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub default_model: Option<String>,
    #[serde(default)]
    pub tier: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AgentEntry {
    /// Single primary provider (legacy form, kept for backwards compat).
    /// `[agent.X] provider = "groq"` continues to work exactly as before.
    #[serde(default)]
    pub provider: String,

    /// Optional explicit failover priority list.
    /// `[agent.X] providers = ["groq", "cerebras", "opencode"]` tells the
    /// runtime to try those provider blocks in this exact order. After
    /// exhausting the list, the runtime still falls through to `provider`
    /// (singular) and then alphabetical of any remaining configured providers
    /// — so the user can shorten the list to "things I prefer" without
    /// losing access to the rest as last-resort fallbacks.
    /// See `AgentRuntime::call_with_fallback` for the resolution rule.
    #[serde(default)]
    pub providers: Option<Vec<String>>,

    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub instructions: String,
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_max_tokens_floors_at_8192_and_respects_env_override() {
        // No env var set: floor of 8192. We don't unconditionally remove
        // PHANTOM_MAX_TOKENS first because cargo test runs tests in
        // parallel within one process and another test might be using it
        // — but since this whole module is the only writer and we set
        // then unset within this test, ordering is fine when run alone.
        std::env::remove_var("PHANTOM_MAX_TOKENS");
        assert_eq!(default_max_tokens(), 8192);

        // Valid override above the sanity floor (256) is honored.
        std::env::set_var("PHANTOM_MAX_TOKENS", "16384");
        assert_eq!(default_max_tokens(), 16384);

        // Below sanity floor falls back to default — protects against
        // a typo like `PHANTOM_MAX_TOKENS=10` silently destroying every
        // chat reply.
        std::env::set_var("PHANTOM_MAX_TOKENS", "10");
        assert_eq!(default_max_tokens(), 8192);

        // Garbage falls back to default.
        std::env::set_var("PHANTOM_MAX_TOKENS", "not-a-number");
        assert_eq!(default_max_tokens(), 8192);

        std::env::remove_var("PHANTOM_MAX_TOKENS");
    }

    #[test]
    fn with_defaults_is_valid_under_env_key() {
        // Pretend ANTHROPIC_API_KEY is set so validation passes.
        std::env::set_var("ANTHROPIC_API_KEY", "sk-test");
        let cfg = AgentsConfig::with_defaults();
        assert!(cfg.validate().is_ok(), "{:?}", cfg.validate());
        std::env::remove_var("ANTHROPIC_API_KEY");
    }

    #[test]
    fn validate_catches_bad_provider() {
        let mut cfg = AgentsConfig::with_defaults();
        cfg.providers.get_mut("anthropic").unwrap().api_key = Some("sk-x".into());
        cfg.agent.get_mut("master").unwrap().provider = "nonexistent".into();
        let errs = cfg.validate().unwrap_err();
        assert!(errs.iter().any(|e| e.contains("unknown provider")));
    }

    #[test]
    fn validate_catches_bad_tool() {
        let mut cfg = AgentsConfig::with_defaults();
        cfg.providers.get_mut("anthropic").unwrap().api_key = Some("sk-x".into());
        cfg.agent.get_mut("master").unwrap().tools.push("not_a_real_tool".into());
        let errs = cfg.validate().unwrap_err();
        assert!(errs.iter().any(|e| e.contains("unknown tool")));
    }

    #[test]
    fn env_override_model() {
        std::env::set_var("PHANTOM_MODEL", "gpt-4o");
        let mut cfg = AgentsConfig::with_defaults();
        cfg.apply_env_overrides();
        assert_eq!(cfg.default_model, "gpt-4o");
        std::env::remove_var("PHANTOM_MODEL");
    }

    #[test]
    fn env_override_max_rounds() {
        std::env::set_var("PHANTOM_MAX_ROUNDS", "50");
        let mut cfg = AgentsConfig::with_defaults();
        cfg.apply_env_overrides();
        assert_eq!(cfg.max_rounds, 50);
        std::env::remove_var("PHANTOM_MAX_ROUNDS");
    }

    #[test]
    fn env_override_token_budget() {
        std::env::set_var("PHANTOM_TOKEN_BUDGET", "200000");
        let mut cfg = AgentsConfig::with_defaults();
        cfg.apply_env_overrides();
        assert_eq!(cfg.token_budget, 200_000);
        std::env::remove_var("PHANTOM_TOKEN_BUDGET");
    }

    #[test]
    fn display_summary_contains_key_info() {
        let mut cfg = AgentsConfig::with_defaults();
        cfg.providers.get_mut("anthropic").unwrap().api_key = Some("sk-x".into());
        let summary = cfg.display_summary();
        assert!(summary.contains("Providers:"));
        assert!(summary.contains("Agents:"));
        assert!(summary.contains("Tools:"));
        assert!(summary.contains("Token budget: 100,000"));
        assert!(summary.contains("Max rounds: 25"));
    }

    #[test]
    fn format_number_works() {
        assert_eq!(format_number(100_000), "100,000");
        assert_eq!(format_number(1_000_000), "1,000,000");
        assert_eq!(format_number(999), "999");
    }

    #[test]
    fn short_model_label_strips_date() {
        assert_eq!(short_model_label("claude-sonnet-4-5-20251022"), "claude-sonnet-4-5");
        assert_eq!(short_model_label("gpt-4o"), "gpt-4o");
    }

    #[test]
    fn interpolate_env_resolves_set_var() {
        std::env::set_var("PHANTOM_TEST_KEY_K1", "secret-abc");
        assert_eq!(interpolate_env_vars("${PHANTOM_TEST_KEY_K1}"), "secret-abc");
        assert_eq!(interpolate_env_vars("prefix-${PHANTOM_TEST_KEY_K1}-suffix"),
                   "prefix-secret-abc-suffix");
        std::env::remove_var("PHANTOM_TEST_KEY_K1");
    }

    #[test]
    fn interpolate_env_unset_var_becomes_empty() {
        std::env::remove_var("PHANTOM_TEST_NEVER_SET_K2");
        assert_eq!(interpolate_env_vars("${PHANTOM_TEST_NEVER_SET_K2}"), "");
        assert_eq!(interpolate_env_vars("a-${PHANTOM_TEST_NEVER_SET_K2}-b"), "a--b");
    }

    #[test]
    fn interpolate_env_no_braces_passes_through() {
        // `$FOO` (no braces) and plain `$` are left alone — only `${...}` is resolved.
        std::env::set_var("PHANTOM_TEST_K3", "value");
        assert_eq!(interpolate_env_vars("$PHANTOM_TEST_K3"), "$PHANTOM_TEST_K3");
        assert_eq!(interpolate_env_vars("plain string"), "plain string");
        assert_eq!(interpolate_env_vars("$"), "$");
        std::env::remove_var("PHANTOM_TEST_K3");
    }

    #[test]
    fn interpolate_env_handles_unclosed_brace() {
        // `${UNCLOSED` (no `}`) should pass through unchanged, not eat the rest.
        assert_eq!(interpolate_env_vars("${UNCLOSED and more"), "${UNCLOSED and more");
    }

    #[test]
    fn interpolate_env_provider_entry_resolves_api_key() {
        std::env::set_var("PHANTOM_TEST_GROQ_K4", "gsk_real");
        let toml_str = r#"
            [providers.groq]
            type = "groq"
            api_key = "${PHANTOM_TEST_GROQ_K4}"
        "#;
        let mut cfg: AgentsConfig = toml::from_str(toml_str).unwrap();
        cfg.resolve_env_vars();
        assert_eq!(cfg.providers["groq"].api_key.as_deref(), Some("gsk_real"));
        std::env::remove_var("PHANTOM_TEST_GROQ_K4");
    }
}

/// Resolve `${VAR_NAME}` patterns by reading the process environment.
///
/// - `${SET}` where `SET=foo` → `foo`
/// - `${UNSET}` → empty string
/// - `${UNCLOSED` (no closing brace) → passed through verbatim
/// - `$VAR` (no braces) → passed through verbatim — only `${...}` is recognized
///
/// Single-pass — does NOT recursively resolve, so a value like
/// `${A}` where `A=${B}` resolves to the literal string `${B}`,
/// not `B`'s value.
/// Default `max_tokens` for streaming + non-streaming chat completions across
/// every provider call site (agent.rs, streaming.rs).
///
/// Was previously hardcoded as 4096 in four places, which silently truncated
/// long replies (≈3000 EN words / 1500 CJK chars) — users saw output cut off
/// mid-sentence with no error. 8192 is the floor every common provider
/// supports (Anthropic Sonnet ≥ 8192, OpenAI gpt-4o ≥ 16384, Groq llama
/// 70b ≥ 8192, Cerebras ≥ 8192), so we can lift the cap without breaking
/// any of them.
///
/// Override with the `PHANTOM_MAX_TOKENS` env var when a particular model
/// supports more (Claude Opus extended thinking goes to 64k, gpt-4o to 16k).
/// Invalid values fall back to the default rather than erroring — the cap
/// is non-critical and we never want a typo in env to break a chat run.
pub fn default_max_tokens() -> u32 {
    const FLOOR: u32 = 8192;
    std::env::var("PHANTOM_MAX_TOKENS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|&n| n >= 256) // sanity floor — anything tinier is almost certainly a typo
        .unwrap_or(FLOOR)
}

pub fn interpolate_env_vars(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c == '$' {
            if let Some(&(_, '{')) = chars.peek() {
                // Found `${`, look for closing `}`
                if let Some(close_off) = s[i + 2..].find('}') {
                    let var_name = &s[i + 2..i + 2 + close_off];
                    let val = std::env::var(var_name).unwrap_or_default();
                    out.push_str(&val);
                    // Advance the iterator past the closing `}`
                    while let Some(&(j, _)) = chars.peek() {
                        if j > i + 2 + close_off { break; }
                        chars.next();
                    }
                    continue;
                }
            }
        }
        out.push(c);
    }
    out
}

impl AgentsConfig {
    /// Walk every ProviderEntry and resolve `${ENV_VAR}` references in
    /// string fields (api_key, url, default_model).  Should be called
    /// once at load time, after `toml::from_str`.
    pub fn resolve_env_vars(&mut self) {
        for entry in self.providers.values_mut() {
            if let Some(v) = entry.api_key.as_mut() {
                let resolved = interpolate_env_vars(v);
                *v = resolved;
            }
            if let Some(v) = entry.url.as_mut() {
                let resolved = interpolate_env_vars(v);
                *v = resolved;
            }
            if let Some(v) = entry.default_model.as_mut() {
                let resolved = interpolate_env_vars(v);
                *v = resolved;
            }
        }
    }
}
