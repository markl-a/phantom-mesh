//! Agent / runtime configuration loading and validation.
//!
//! [`AgentsConfig`] is the deserialized form of a `agents.toml` /
//! `SPECTYN.toml` file plus a set of built-in defaults. It holds providers,
//! agents, tools, cluster, workspace, and permission settings.
//!
//! # Loading precedence (lowest → highest)
//!
//! Effective config is built by layering three sources, where each later
//! layer overrides the earlier one:
//!
//! 1. **Built-in defaults** — [`AgentsConfig::with_defaults`] yields a usable
//!    config even with no file present (one `master` agent on the `anthropic`
//!    provider, the full default tool list, etc.).
//! 2. **Config file** — [`AgentsConfig::find_and_load`] searches standard
//!    locations and parses the first file found, in this order:
//!    `./agents.toml` → `./SPECTYN.toml` →
//!    `~/.spectyn-mesh/agents.toml` → `~/.config/spectyn-mesh/config.toml`.
//!    After parsing, `${ENV_VAR}` references inside provider string fields are
//!    resolved by [`AgentsConfig::resolve_env_vars`] /
//!    [`interpolate_env_vars`].
//! 3. **Environment variable overrides** —
//!    [`AgentsConfig::apply_env_overrides`] applies well-known vars on top of
//!    the loaded file. Recognized: `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`,
//!    `SPECTYN_MODEL`, `SPECTYN_MAX_ROUNDS`, `SPECTYN_TOKEN_BUDGET`.
//!    [`default_max_tokens`] separately honors `SPECTYN_MAX_TOKENS` at call
//!    time.
//!
//! Invalid or empty override values are ignored and fall back to the prior
//! layer rather than erroring, so a typo in the environment never breaks a
//! run. Call [`AgentsConfig::validate`] to surface configuration mistakes
//! (missing API key, unknown provider/tool references) all at once.

use serde::Deserialize;
use std::collections::HashMap;

/// All built-in tool names the agent runtime can execute. Mirrors the match
/// arms in `tools/mod.rs::execute()`; [`AgentsConfig::validate`] uses this set
/// to reject unknown tool references (external MCP tools are allowed via their
/// `<server>_` prefix instead).
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

fn default_host() -> String {
    "0.0.0.0".into()
}
fn default_port() -> u16 {
    7878
}

fn default_model() -> String {
    "claude-sonnet-4-5-20251022".into()
}
fn default_max_rounds() -> usize {
    25
}
fn default_token_budget() -> usize {
    100_000
}

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
  what's reachable; then `task({agent:'coder', prompt:'...', node:'host-a'})` or
  `parallel_tasks({tasks:[{node:'host-b',...},{node:'host-a',...}]})` to actually delegate.
  Use this when the user says things like 'have host-a run the tests while host-b builds the
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

/// Top-level deserialized configuration. See the module docs for the
/// defaults → file → env loading precedence.
#[derive(Debug, Clone, Deserialize)]
pub struct AgentsConfig {
    /// `[core]` block — server host/port and optional hub API key.
    #[serde(default)]
    pub core: CoreConfig,
    /// `[providers.*]` blocks — keyed by provider name (e.g. `anthropic`).
    #[serde(default)]
    pub providers: HashMap<String, ProviderEntry>,
    /// `[agent.*]` blocks — keyed by agent name (e.g. `master`).
    #[serde(default)]
    pub agent: HashMap<String, AgentEntry>,
    /// `[tools]` block — tool-specific settings (e.g. search API keys).
    #[serde(default)]
    pub tools: ToolsConfig,
    /// `[cluster]` block — multi-machine mesh peer configuration.
    #[serde(default)]
    pub cluster: crate::mesh::ClusterConfig,
    /// Per-machine workspace pin — when set, `spectyn` (no args) auto-cd
    /// to default_dir before launching the TUI, pre-selects pinned_agent,
    /// and the resulting conversation history lives under that path's
    /// cwd-hash. Lets you keep one Windows box dedicated to one project
    /// without remembering which dir to be in.
    #[serde(default)]
    pub workspace: WorkspaceConfig,
    /// `[permissions]` block — Tool(specifier) DSL rules. See
    /// `permission` module. Empty/missing means "no rules", which
    /// preserves legacy `SPECTYN_PERM=allow` behaviour (allow all).
    #[serde(default)]
    pub permissions: PermissionsConfig,
    /// `[trust]` block — Project Trust enforcement (the 4-layer onboarding
    /// model's project layer). Default off; see [`crate::project_trust`].
    #[serde(default)]
    pub trust: TrustConfig,
    /// `[telegram]` block — optional bot token/chat for notifications.
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
            trust: TrustConfig::default(),
            telegram: None,
            mcp_servers: Vec::new(),
            default_model: default_model(),
            max_rounds: default_max_rounds(),
            token_budget: default_token_budget(),
            system_prompt: default_system_prompt(),
        }
    }

    // ── Validation ────────────────────────────────────────────────────────

    /// True if at least one configured provider has a usable api_key — either
    /// inline (`api_key = "…"`) or resolvable from its `api_key_env` env var.
    /// Used by `validate()` and by the TUI first-run hint (so a keyless user
    /// gets pointed at `/login` instead of a blank "all providers failed").
    pub fn has_usable_provider_key(&self) -> bool {
        self.providers.values().any(|p| {
            p.api_key.as_deref().map(|s| !s.is_empty()).unwrap_or(false)
                || p.api_key_env
                    .as_deref()
                    .and_then(|env| std::env::var(env).ok())
                    .map(|v| !v.is_empty())
                    .unwrap_or(false)
        })
    }

    /// Validate the configuration and return all errors at once (not just the first).
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors: Vec<String> = Vec::new();

        // At least one provider must have an api_key configured (directly or via env var).
        let has_key = self.has_usable_provider_key();
        if !has_key && !self.providers.is_empty() {
            errors.push("No provider has an api_key or a resolvable api_key_env configured".into());
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
        let valid_set: std::collections::HashSet<&str> = VALID_TOOLS.iter().copied().collect();
        let mcp_prefixes: Vec<String> = self
            .mcp_servers
            .iter()
            .map(|s| format!("{}_", s.name))
            .collect();
        for (agent_name, entry) in &self.agent {
            for tool in &entry.tools {
                if valid_set.contains(tool.as_str()) {
                    continue;
                }
                if mcp_prefixes.iter().any(|p| tool.starts_with(p)) {
                    continue;
                }
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
                let entry =
                    self.providers
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
                let entry =
                    self.providers
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

        // SPECTYN_MODEL → override default_model.
        if let Ok(model) = std::env::var("SPECTYN_MODEL") {
            if !model.is_empty() {
                self.default_model = model;
            }
        }

        // SPECTYN_MAX_ROUNDS → override max_rounds.
        if let Ok(val) = std::env::var("SPECTYN_MAX_ROUNDS") {
            if let Ok(n) = val.trim().parse::<usize>() {
                self.max_rounds = n;
            }
        }

        // SPECTYN_TOKEN_BUDGET → override token_budget.
        if let Ok(val) = std::env::var("SPECTYN_TOKEN_BUDGET") {
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
    /// 2. `./SPECTYN.toml`
    /// 3. `~/.spectyn-mesh/agents.toml`
    /// 4. `~/.config/spectyn-mesh/config.toml`
    ///
    /// A candidate that exists but fails to read or parse emits a warning to
    /// stderr (so a corrupted config doesn't silently look like a fresh
    /// install) and is then skipped, preserving the historical fallback to
    /// the next candidate and ultimately the caller's built-in defaults.
    /// The ordered config-file candidates spectyn searches, first match wins:
    /// `<cwd>/agents.toml` → `<cwd>/SPECTYN.toml` → `<home>/.spectyn-mesh/agents.toml`
    /// → `<home>/.config/spectyn-mesh/config.toml`.
    ///
    /// Single source of truth for "where does spectyn look for its config",
    /// shared by the runtime loader ([`find_and_load`](Self::find_and_load)) and
    /// `diagnostics::diagnose`, so `spectyn doctor` can never report a different
    /// file than the runtime actually loads. Hermetic (takes explicit paths) so
    /// both callers — and unit tests — agree by construction.
    pub fn candidate_config_paths(
        home: &std::path::Path,
        cwd: &std::path::Path,
    ) -> Vec<std::path::PathBuf> {
        vec![
            cwd.join("agents.toml"),
            cwd.join("SPECTYN.toml"),
            home.join(".spectyn-mesh").join("agents.toml"),
            home.join(".config").join("spectyn-mesh").join("config.toml"),
        ]
    }

    /// Load config from the HOME tier ONLY — `~/.spectyn-mesh/agents.toml` then
    /// `~/.config/spectyn-mesh/config.toml` — deliberately skipping the cwd
    /// candidates that [`find_and_load`](Self::find_and_load) walks first.
    ///
    /// Security knobs (project-trust enforcement, the permission profile/ceiling)
    /// MUST be read from here, not from `find_and_load`: a malicious project's
    /// own `cwd/agents.toml` is cwd-first, so reading the policy via the normal
    /// loader would let an untrusted directory disable the very protection meant
    /// to contain it.
    /// True if a HOME-tier config file EXISTS (regardless of whether it parses).
    /// Lets the security gate distinguish "no config → legacy allow-all is fine"
    /// from "config present but malformed → fail closed".
    pub fn home_config_present() -> bool {
        dirs::home_dir().is_some_and(|home| {
            home.join(".spectyn-mesh").join("agents.toml").exists()
                || home
                    .join(".config")
                    .join("spectyn-mesh")
                    .join("config.toml")
                    .exists()
        })
    }

    pub fn load_home_only() -> Option<Self> {
        let home = dirs::home_dir()?;
        let candidates = [
            home.join(".spectyn-mesh").join("agents.toml"),
            home.join(".config").join("spectyn-mesh").join("config.toml"),
        ];
        for path in candidates {
            if path.exists() {
                match Self::load_path(&path) {
                    Ok(mut cfg) => {
                        cfg.resolve_env_vars();
                        cfg.apply_env_overrides();
                        return Some(cfg);
                    }
                    // A malformed HOME config must NOT silently disable the
                    // security gate (the caller falls back to allow-all). Warn
                    // loudly so the user knows their profile/trust isn't applied.
                    Err(err) => {
                        if warn_once_for_path(&path) {
                            eprintln!(
                                "warning: {err}; HOME security config ignored — \
                                 permission profile / trust enforcement NOT applied. \
                                 Fix the TOML or run `spectyn doctor`."
                            );
                        }
                    }
                }
            }
        }
        None
    }

    pub fn find_and_load() -> Option<Self> {
        let candidates: Vec<std::path::PathBuf> = {
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            match dirs::home_dir() {
                // Same candidate order as `candidate_config_paths` — that is the
                // shared source of truth `diagnostics::diagnose` reads too.
                Some(home) => Self::candidate_config_paths(&home, &cwd),
                // No home → only the cwd-relative candidates are reachable.
                None => vec![cwd.join("agents.toml"), cwd.join("SPECTYN.toml")],
            }
        };

        for path in candidates {
            if path.exists() {
                match Self::load_path(&path) {
                    Ok(mut cfg) => {
                        cfg.resolve_env_vars();
                        cfg.apply_env_overrides();
                        return Some(cfg);
                    }
                    Err(err) => {
                        // A corrupted config must not masquerade as a fresh
                        // install (providers silently vanish with no clue
                        // why). Say what went wrong, then keep the old
                        // fallback behavior: try the next candidate, and
                        // ultimately the caller's built-in defaults.
                        //
                        // find_and_load runs many times per CLI invocation, so
                        // gate the warning behind a once-per-process per-path
                        // guard — otherwise a single broken file spams the same
                        // line on every call. First sighting of a given path
                        // warns; later sightings stay silent.
                        if warn_once_for_path(&path) {
                            eprintln!(
                                "warning: {err}; ignoring this config file \
                                 (falling back to next config location or built-in defaults)"
                            );
                        }
                    }
                }
            }
        }
        None
    }

    /// Read and parse a single config file. On failure returns a descriptive
    /// error naming the path and the underlying read/parse error so
    /// [`AgentsConfig::find_and_load`] can surface it instead of silently
    /// falling back to defaults.
    /// Canonical single-file loader (used by [`AgentsConfig::find_and_load`] and
    /// by tests). Public so the at-rest-sealing round-trip can be exercised
    /// against the real load path.
    pub fn load_path(path: &std::path::Path) -> Result<Self, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read config {}: {}", path.display(), e))?;
        let mut cfg = toml::from_str::<AgentsConfig>(&content)
            .map_err(|e| format!("failed to parse config {}: {}", path.display(), e))?;
        // apex P4: decrypt any at-rest-sealed provider API keys. No-op unless
        // `SPECTYN_ENCRYPT_AGENTS` is on (so OFF stays byte-identical to today).
        // Fail closed — a sealed-but-undecryptable key surfaces as a load error
        // rather than handing back ciphertext as if it were the key.
        crate::skillbank::agents_seal::unseal_on_load(&mut cfg)
            .map_err(|e| format!("failed to decrypt sealed keys in {}: {}", path.display(), e))?;
        Ok(cfg)
    }

    // ── Display summary ───────────────────────────────────────────────────
    // (warn-once dedup helper lives at module scope below, see
    // `warn_once_for_path` / `should_warn_in`.)

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
            format!("{}, ... ({} total)", tool_names[..5].join(", "), tool_count)
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

/// Process-wide set of config paths whose load-failure warning has already
/// been printed. `find_and_load` is called many times per CLI invocation, so
/// without this guard a single corrupted config spams the same warning on
/// every call. Keyed by path so distinct broken files each still get one line.
///
/// `OnceLock<Mutex<HashSet<PathBuf>>>` matches the crate's existing
/// process-local-table idiom (cf. `capture_focus_wire::session_table`).
fn warned_config_paths() -> &'static std::sync::Mutex<std::collections::HashSet<std::path::PathBuf>>
{
    static WARNED: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<std::path::PathBuf>>> =
        std::sync::OnceLock::new();
    WARNED.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()))
}

/// True the first time a given path's warning should be emitted, false on every
/// subsequent call for the same path in this process. Records the path as seen
/// as a side effect. Wraps [`should_warn_in`] over the process-global set.
fn warn_once_for_path(path: &std::path::Path) -> bool {
    let mut seen = warned_config_paths()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    should_warn_in(&mut seen, path)
}

/// Pure dedup decision, extracted so it is testable without touching the
/// process-global `OnceLock`: returns true (and inserts) only the first time
/// `path` is seen in `seen`, false afterwards.
fn should_warn_in(
    seen: &mut std::collections::HashSet<std::path::PathBuf>,
    path: &std::path::Path,
) -> bool {
    seen.insert(path.to_path_buf())
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

/// `[tools]` block — settings consumed by individual tools.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ToolsConfig {
    /// API key for the Brave Search backend of the `web_search` tool.
    #[serde(default)]
    pub brave_search_api_key: Option<String>,
    /// Todoist API token (Settings → Integrations → Developer) for the
    /// `todoist_*` tools and the partner's goal model. Falls back to the
    /// `TODOIST_API_TOKEN` env var when unset (see `crate::todoist::resolve_token`).
    #[serde(default)]
    pub todoist_api_token: Option<String>,
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
/// preserving legacy `SPECTYN_PERM=allow` behaviour. Once any rule is
/// present, unmatched calls fall through to `Ask`.
/// `[trust]` — Project Trust enforcement policy. The trusted-directory *set*
/// lives in `~/.spectyn-mesh/trust.json` (CLI-managed); this only holds the
/// enforcement knob so a malicious cwd config can't trust itself.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TrustConfig {
    /// "off" (default) / "prompt" / "observe" — see [`crate::project_trust::TrustPolicy`].
    #[serde(default)]
    pub enforcement: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PermissionsConfig {
    /// A named preset (observe / suggest / workspace-write / developer-full) —
    /// see [`crate::permission_profiles`]. Used as the base rule set when no
    /// explicit deny/ask/allow rules are given; explicit rules take precedence.
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub deny: Vec<String>,
    #[serde(default)]
    pub ask: Vec<String>,
    #[serde(default)]
    pub allow: Vec<String>,
}

/// `[core]` block — the `spectyn serve` listener and hub auth settings.
#[derive(Debug, Clone, Deserialize)]
pub struct CoreConfig {
    /// Bind address for the server (default `0.0.0.0`).
    #[serde(default = "default_host")]
    pub host: String,
    /// Listener port (default `7878`).
    #[serde(default = "default_port")]
    pub port: u16,
    /// Optional shared secret required to reach hub endpoints.
    #[serde(default)]
    pub hub_api_key: Option<String>,
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            hub_api_key: None,
        }
    }
}

/// Per-machine workspace pin. All fields optional — empty/missing block
/// means "no pin, spectyn uses caller's cwd as before".
///
/// When `default_dir` is set, the bare `spectyn` command (no args, no
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

/// A single `[providers.<name>]` block describing one LLM backend.
///
/// Local servers (Ollama / LM Studio / Lemonade) can be *probed* at request
/// time on their standard localhost ports (see
/// [`crate::providers::local_servers::detect_local_servers`]).
///
/// IMPORTANT (corrected doc, fix #3): detection results are **not** synthesized
/// into config blocks. The `SPECTYN_LOCAL_FIRST` reorder only promotes providers
/// that *already exist in the resolved chain*, i.e. that have an explicit
/// `[providers.NAME]` block. So **local-first only takes effect when you add a
/// `[providers.<name>]` block** for the local server. By codebase convention
/// that name is `local-`prefixed — e.g. `[providers.local-ollama]` — and the
/// reorder bridges the bare detected slug (`ollama`) to it. Without such a block
/// the flag is a no-op and the default cloud order stands.
///
/// When `SPECTYN_LOCAL_FIRST` is set (`1`/`true`/`yes`), a detected local server
/// that *also* has a configured block is moved to the front of the provider
/// chain — cloud providers stay in the chain for graceful fallback.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ProviderEntry {
    /// Provider kind, e.g. `anthropic`, `openai`, `groq` (TOML key `type`).
    #[serde(rename = "type", default)]
    pub provider_type: String,
    /// Base URL override for the API (TOML key `url` or `base_url`).
    #[serde(default, alias = "base_url")]
    pub url: Option<String>,
    /// Inline API key. Supports `${ENV_VAR}` interpolation at load time.
    #[serde(default)]
    pub api_key: Option<String>,
    /// Name of an env var to read the API key from when `api_key` is unset.
    #[serde(default)]
    pub api_key_env: Option<String>,
    /// Default model for this provider when an agent does not name one.
    #[serde(default)]
    pub default_model: Option<String>,
    /// Optional free-form tier label (e.g. `free`, `paid`) for routing hints.
    #[serde(default)]
    pub tier: Option<String>,
}

/// A single `[agent.<name>]` block — one named agent's provider, model,
/// tool allow-list, and system instructions.
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

    /// Model ID for this agent; empty falls back to the provider/global default.
    #[serde(default)]
    pub model: String,
    /// Tool names this agent may call (validated against [`VALID_TOOLS`]).
    #[serde(default)]
    pub tools: Vec<String>,
    /// System prompt / instructions for this agent; empty uses the default.
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
        // SPECTYN_MAX_TOKENS first because cargo test runs tests in
        // parallel within one process and another test might be using it
        // — but since this whole module is the only writer and we set
        // then unset within this test, ordering is fine when run alone.
        std::env::remove_var("SPECTYN_MAX_TOKENS");
        assert_eq!(default_max_tokens(), 8192);

        // Valid override above the sanity floor (256) is honored.
        std::env::set_var("SPECTYN_MAX_TOKENS", "16384");
        assert_eq!(default_max_tokens(), 16384);

        // Below sanity floor falls back to default — protects against
        // a typo like `SPECTYN_MAX_TOKENS=10` silently destroying every
        // chat reply.
        std::env::set_var("SPECTYN_MAX_TOKENS", "10");
        assert_eq!(default_max_tokens(), 8192);

        // Garbage falls back to default.
        std::env::set_var("SPECTYN_MAX_TOKENS", "not-a-number");
        assert_eq!(default_max_tokens(), 8192);

        std::env::remove_var("SPECTYN_MAX_TOKENS");
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
    fn has_usable_provider_key_detects_inline_and_empty() {
        let mut cfg = AgentsConfig::with_defaults();
        // Point every default provider at a guaranteed-unset env var so the
        // result is deterministic regardless of the ambient test environment.
        for p in cfg.providers.values_mut() {
            p.api_key = None;
            p.api_key_env = Some("SPECTYN_TEST_DEFINITELY_UNSET_KEY".to_string());
        }
        assert!(!cfg.has_usable_provider_key(), "all key sources cleared → none usable");

        // An empty inline api_key must NOT count as usable.
        if let Some(p) = cfg.providers.values_mut().next() {
            p.api_key = Some(String::new());
        }
        assert!(!cfg.has_usable_provider_key(), "empty inline api_key is not usable");

        // A non-empty inline api_key is usable.
        if let Some(p) = cfg.providers.values_mut().next() {
            p.api_key = Some("sk-real".to_string());
        }
        assert!(cfg.has_usable_provider_key(), "non-empty inline api_key is usable");
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
        cfg.agent
            .get_mut("master")
            .unwrap()
            .tools
            .push("not_a_real_tool".into());
        let errs = cfg.validate().unwrap_err();
        assert!(errs.iter().any(|e| e.contains("unknown tool")));
    }

    #[test]
    fn env_override_model() {
        std::env::set_var("SPECTYN_MODEL", "gpt-4o");
        let mut cfg = AgentsConfig::with_defaults();
        cfg.apply_env_overrides();
        assert_eq!(cfg.default_model, "gpt-4o");
        std::env::remove_var("SPECTYN_MODEL");
    }

    #[test]
    fn env_override_max_rounds() {
        std::env::set_var("SPECTYN_MAX_ROUNDS", "50");
        let mut cfg = AgentsConfig::with_defaults();
        cfg.apply_env_overrides();
        assert_eq!(cfg.max_rounds, 50);
        std::env::remove_var("SPECTYN_MAX_ROUNDS");
    }

    #[test]
    fn env_override_token_budget() {
        std::env::set_var("SPECTYN_TOKEN_BUDGET", "200000");
        let mut cfg = AgentsConfig::with_defaults();
        cfg.apply_env_overrides();
        assert_eq!(cfg.token_budget, 200_000);
        std::env::remove_var("SPECTYN_TOKEN_BUDGET");
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
        assert_eq!(
            short_model_label("claude-sonnet-4-5-20251022"),
            "claude-sonnet-4-5"
        );
        assert_eq!(short_model_label("gpt-4o"), "gpt-4o");
    }

    #[test]
    fn interpolate_env_resolves_set_var() {
        std::env::set_var("SPECTYN_TEST_KEY_K1", "secret-abc");
        assert_eq!(interpolate_env_vars("${SPECTYN_TEST_KEY_K1}"), "secret-abc");
        assert_eq!(
            interpolate_env_vars("prefix-${SPECTYN_TEST_KEY_K1}-suffix"),
            "prefix-secret-abc-suffix"
        );
        std::env::remove_var("SPECTYN_TEST_KEY_K1");
    }

    #[test]
    fn interpolate_env_unset_var_becomes_empty() {
        std::env::remove_var("SPECTYN_TEST_NEVER_SET_K2");
        assert_eq!(interpolate_env_vars("${SPECTYN_TEST_NEVER_SET_K2}"), "");
        assert_eq!(
            interpolate_env_vars("a-${SPECTYN_TEST_NEVER_SET_K2}-b"),
            "a--b"
        );
    }

    #[test]
    fn interpolate_env_no_braces_passes_through() {
        // `$FOO` (no braces) and plain `$` are left alone — only `${...}` is resolved.
        std::env::set_var("SPECTYN_TEST_K3", "value");
        assert_eq!(interpolate_env_vars("$SPECTYN_TEST_K3"), "$SPECTYN_TEST_K3");
        assert_eq!(interpolate_env_vars("plain string"), "plain string");
        assert_eq!(interpolate_env_vars("$"), "$");
        std::env::remove_var("SPECTYN_TEST_K3");
    }

    #[test]
    fn interpolate_env_handles_unclosed_brace() {
        // `${UNCLOSED` (no `}`) should pass through unchanged, not eat the rest.
        assert_eq!(
            interpolate_env_vars("${UNCLOSED and more"),
            "${UNCLOSED and more"
        );
    }

    #[test]
    fn interpolate_env_provider_entry_resolves_api_key() {
        std::env::set_var("SPECTYN_TEST_GROQ_K4", "gsk_real");
        let toml_str = r#"
            [providers.groq]
            type = "groq"
            api_key = "${SPECTYN_TEST_GROQ_K4}"
        "#;
        let mut cfg: AgentsConfig = toml::from_str(toml_str).unwrap();
        cfg.resolve_env_vars();
        assert_eq!(cfg.providers["groq"].api_key.as_deref(), Some("gsk_real"));
        std::env::remove_var("SPECTYN_TEST_GROQ_K4");
    }

    // ── Load robustness ───────────────────────────────────────────────────
    //
    // `AgentsConfig` is intentionally load-only: it derives `Deserialize`
    // (and so do its sub-structs and the external `ClusterConfig` /
    // `TelegramConfig` / `McpServerConfig` it embeds) but NOT `Serialize`,
    // and there is no `save` / `to_toml` writer. There is therefore no
    // serialize→re-parse round-trip to exercise. What we *can* and should
    // pin down is that the parse side is robust: a representative file
    // parses into the expected fields, missing optional blocks fall back to
    // defaults, and malformed input fails cleanly (Err, not panic).

    /// A representative `agents.toml` exercising every top-level block.
    const REPRESENTATIVE_TOML: &str = r#"
default_model = "claude-sonnet-4-5-20251022"
max_rounds = 30
token_budget = 150000

[core]
host = "127.0.0.1"
port = 9000
hub_api_key = "hub-secret"

[providers.anthropic]
type = "anthropic"
api_key = "sk-anthropic"
default_model = "claude-sonnet-4-5-20251022"

[providers.groq]
type = "groq"
api_key_env = "GROQ_API_KEY"
tier = "free"

[agent.master]
provider = "anthropic"
model = "claude-sonnet-4-5-20251022"
tools = ["shell", "file_read", "file_write"]
instructions = "be helpful"

[agent.coder]
providers = ["groq", "anthropic"]
tools = ["shell"]

[tools]
brave_search_api_key = "brave-key"

[workspace]
default_dir = "/work/project"
pinned_agent = "coder"

[permissions]
deny = ["Read(./.env)"]
ask = ["WebFetch"]
allow = ["Bash(git status)"]
"#;

    #[test]
    fn representative_toml_parses_all_blocks() {
        let cfg: AgentsConfig =
            toml::from_str(REPRESENTATIVE_TOML).expect("representative toml must parse");

        // Top-level scalars.
        assert_eq!(cfg.default_model, "claude-sonnet-4-5-20251022");
        assert_eq!(cfg.max_rounds, 30);
        assert_eq!(cfg.token_budget, 150_000);

        // [core]
        assert_eq!(cfg.core.host, "127.0.0.1");
        assert_eq!(cfg.core.port, 9000);
        assert_eq!(cfg.core.hub_api_key.as_deref(), Some("hub-secret"));

        // [providers.*]
        assert_eq!(cfg.providers.len(), 2);
        let anthropic = &cfg.providers["anthropic"];
        assert_eq!(anthropic.provider_type, "anthropic");
        assert_eq!(anthropic.api_key.as_deref(), Some("sk-anthropic"));
        let groq = &cfg.providers["groq"];
        assert_eq!(groq.api_key_env.as_deref(), Some("GROQ_API_KEY"));
        assert_eq!(groq.tier.as_deref(), Some("free"));

        // [agent.*]
        assert_eq!(cfg.agent.len(), 2);
        let master = &cfg.agent["master"];
        assert_eq!(master.provider, "anthropic");
        assert_eq!(master.tools, vec!["shell", "file_read", "file_write"]);
        assert_eq!(master.instructions, "be helpful");
        let coder = &cfg.agent["coder"];
        assert_eq!(
            coder.providers.as_deref(),
            Some(["groq".to_string(), "anthropic".to_string()].as_slice())
        );

        // [tools] / [workspace] / [permissions]
        assert_eq!(cfg.tools.brave_search_api_key.as_deref(), Some("brave-key"));
        assert_eq!(cfg.workspace.default_dir.as_deref(), Some("/work/project"));
        assert_eq!(cfg.workspace.pinned_agent.as_deref(), Some("coder"));
        assert_eq!(cfg.permissions.deny, vec!["Read(./.env)"]);
        assert_eq!(cfg.permissions.ask, vec!["WebFetch"]);
        assert_eq!(cfg.permissions.allow, vec!["Bash(git status)"]);
    }

    #[test]
    fn empty_toml_falls_back_to_defaults() {
        // An empty file is valid: every block is `#[serde(default)]`, so
        // scalars take their `default_*` helpers and collections are empty.
        let cfg: AgentsConfig = toml::from_str("").expect("empty toml must parse");
        assert_eq!(cfg.default_model, default_model());
        assert_eq!(cfg.max_rounds, default_max_rounds());
        assert_eq!(cfg.token_budget, default_token_budget());
        assert_eq!(cfg.core.host, default_host());
        assert_eq!(cfg.core.port, default_port());
        assert!(cfg.core.hub_api_key.is_none());
        assert!(cfg.providers.is_empty());
        assert!(cfg.agent.is_empty());
        assert!(cfg.tools.brave_search_api_key.is_none());
        assert!(cfg.workspace.default_dir.is_none());
        assert!(cfg.permissions.deny.is_empty());
        assert!(cfg.mcp_servers.is_empty());
        assert!(cfg.telegram.is_none());
    }

    #[test]
    fn missing_optional_fields_in_blocks_default() {
        // A provider/agent block that omits every optional field must still
        // parse, with the omitted fields taking their type defaults.
        let toml_str = r#"
            [providers.minimal]
            type = "openai"

            [agent.bare]
            provider = "minimal"
        "#;
        let cfg: AgentsConfig = toml::from_str(toml_str).expect("minimal blocks must parse");
        let p = &cfg.providers["minimal"];
        assert_eq!(p.provider_type, "openai");
        assert!(p.url.is_none());
        assert!(p.api_key.is_none());
        assert!(p.api_key_env.is_none());
        assert!(p.default_model.is_none());
        assert!(p.tier.is_none());

        let a = &cfg.agent["bare"];
        assert_eq!(a.provider, "minimal");
        assert!(a.providers.is_none());
        assert_eq!(a.model, "");
        assert!(a.tools.is_empty());
        assert_eq!(a.instructions, "");
    }

    #[test]
    fn base_url_alias_parses() {
        // `[providers.X] base_url = ...` must populate `url` via the serde alias.
        let toml_str = r#"
            [providers.local]
            type = "openai"
            base_url = "http://localhost:1234/v1"
        "#;
        let cfg: AgentsConfig = toml::from_str(toml_str).expect("base_url alias must parse");
        assert_eq!(
            cfg.providers["local"].url.as_deref(),
            Some("http://localhost:1234/v1")
        );
    }

    #[test]
    fn malformed_toml_errors_cleanly() {
        // Syntactically broken TOML must return Err, never panic.
        let bad = "this is = not valid = toml [[[";
        assert!(toml::from_str::<AgentsConfig>(bad).is_err());
    }

    #[test]
    fn wrong_typed_field_errors_cleanly() {
        // A field with the wrong type (string where a u16 port is expected)
        // must surface as a deserialization error, not a silent default.
        let toml_str = r#"
            [core]
            port = "not-a-number"
        "#;
        assert!(toml::from_str::<AgentsConfig>(toml_str).is_err());
    }

    #[test]
    fn load_path_malformed_toml_errs_naming_path_and_reason() {
        // Regression: find_and_load used to swallow read/parse errors
        // silently, so a corrupted agents.toml looked like a fresh install
        // (providers vanished, no diagnostic). The inner load fn must return
        // a descriptive error naming the path so find_and_load can warn.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agents.toml");
        std::fs::write(&path, "this is = not valid = toml [[[").unwrap();
        let err = AgentsConfig::load_path(&path).unwrap_err();
        assert!(
            err.contains("failed to parse config"),
            "error should say parsing failed: {err}"
        );
        assert!(
            err.contains(&path.display().to_string()),
            "error should name the offending path: {err}"
        );
    }

    #[test]
    fn load_path_unreadable_file_errs_naming_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.toml");
        let err = AgentsConfig::load_path(&path).unwrap_err();
        assert!(
            err.contains("failed to read config"),
            "error should say reading failed: {err}"
        );
        assert!(
            err.contains(&path.display().to_string()),
            "error should name the offending path: {err}"
        );
    }

    #[test]
    fn load_path_valid_toml_parses() {
        // Sanity: the happy path through the extracted helper still loads.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agents.toml");
        std::fs::write(&path, "default_model = \"gpt-4o\"\n").unwrap();
        let cfg = AgentsConfig::load_path(&path).expect("valid toml must load");
        assert_eq!(cfg.default_model, "gpt-4o");
    }

    // ── Warn-once dedup ─────────────────────────────────────────────────────
    //
    // The process-global `warn_once_for_path` guard cannot be exercised
    // deterministically from a parallel test runner (a `OnceLock<Mutex<…>>`
    // shared across all tests in the process has no reset). The dedup *decision*
    // is therefore extracted into the pure `should_warn_in(&mut set, path)`,
    // which takes the set explicitly and is fully testable in isolation.

    #[test]
    fn should_warn_in_emits_once_per_path() {
        let mut seen = std::collections::HashSet::new();
        let a = std::path::Path::new("/tmp/agents.toml");
        let b = std::path::Path::new("/tmp/SPECTYN.toml");

        // First sighting of `a` warns; repeats stay silent.
        assert!(should_warn_in(&mut seen, a), "first sighting must warn");
        assert!(!should_warn_in(&mut seen, a), "second sighting must be silent");
        assert!(!should_warn_in(&mut seen, a), "third sighting must be silent");

        // A different path warns once on its own, independent of `a`.
        assert!(should_warn_in(&mut seen, b), "distinct path warns once");
        assert!(!should_warn_in(&mut seen, b), "distinct path then silent");

        // `a` is still suppressed after `b`'s first warning.
        assert!(!should_warn_in(&mut seen, a), "a stays suppressed");
    }

    #[test]
    fn warn_once_for_path_is_idempotent_for_a_given_path() {
        // Smoke-test the process-global wrapper end-to-end. We can't assert the
        // *first* call returns true (another test in the same process may have
        // already recorded this path), but we can assert that once a path has
        // been observed, every subsequent observation returns false — which is
        // the property that stops the warning from spamming.
        let path = std::path::Path::new(
            "/tmp/spectyn-mesh-warn-once-test-UNIQUE-d8f1/agents.toml",
        );
        // Force it into the seen-set, then confirm it never warns again.
        let _ = warn_once_for_path(path);
        assert!(
            !warn_once_for_path(path),
            "an already-seen path must never warn again in this process"
        );
        assert!(
            !warn_once_for_path(path),
            "still silent on a third observation"
        );
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
/// Override with the `SPECTYN_MAX_TOKENS` env var when a particular model
/// supports more (Claude Opus extended thinking goes to 64k, gpt-4o to 16k).
/// Invalid values fall back to the default rather than erroring — the cap
/// is non-critical and we never want a typo in env to break a chat run.
pub fn default_max_tokens() -> u32 {
    const FLOOR: u32 = 8192;
    std::env::var("SPECTYN_MAX_TOKENS")
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
                        if j > i + 2 + close_off {
                            break;
                        }
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
