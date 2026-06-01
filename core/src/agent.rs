//! Agent runtime: the core LLM (large language model) conversation loop.
//!
//! [`AgentRuntime`] drives one agent turn from a user prompt to a final
//! answer, looping over rounds until the model stops requesting tools or a
//! cap is hit. The design centers on four strengths:
//!
//! - **SSE (server-sent events) streaming** — provider responses are read as
//!   an incremental token stream rather than a single blocking reply, so the
//!   UI can render tokens, reasoning traces, and tool activity live via
//!   [`AgentEvent`] as they arrive.
//! - **Tool-call dispatch** — when the model emits a tool call, the loop
//!   parses the arguments, consults the optional permission [`ToolGate`]
//!   ([`ToolGateDecision::Allow`] / [`ToolGateDecision::Deny`]), runs the
//!   tool, and feeds the result back as a new message so the model can
//!   continue. Multiple tool calls in one round are handled before the next
//!   model round.
//! - **Token-budget compaction** — the running message history is kept within
//!   the model's context window by compacting older turns once the estimated
//!   token budget is exceeded, preventing unbounded growth across long runs.
//! - **Cooperative interrupt** — an optional [`crate::interrupt::InterruptHandle`]
//!   is checked between rounds and raced against the SSE reader inside a
//!   `tokio::select!`, so a second Enter from the TUI (terminal user interface)
//!   or a gateway RPC (remote procedure call) can unwind the current turn
//!   without waiting for the model to finish.
//!
//! Provider selection and retry live in [`resolve_provider_order`] and the
//! `call_with_*` paths: transient errors retry the same provider up to
//! [`MAX_RETRIES`] times, while non-retriable HTTP statuses (see
//! [`is_non_retriable_status`]) fall through to the next configured provider.
//! Each subagent invocation can scope its own [`MAX_ROUNDS_OVERRIDE`] without
//! mutating any process-global state.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use futures::future::join_all;
use futures::StreamExt;
use serde_json::Value;

use crate::config::{AgentEntry, AgentsConfig};
use crate::cost::CostTracker;
use crate::providers::traits::ChatMessage;
use crate::providers::{BuildRequestOpts, DefaultProviderResolver};
use crate::providers_wire::{system_placement_for_provider, PromptStyle, SystemPlacement};
use crate::streaming::ResolveProvider;
use crate::tasks::SessionWriter;

tokio::task_local! {
    /// Per-task `max_rounds` override (audit C-3 fix). Each subagent
    /// invocation can wrap its own future with
    /// `MAX_ROUNDS_OVERRIDE.scope(Some(n), …)` so the cap propagates
    /// down to `run_inner` WITHOUT touching process-global state. This
    /// replaces the previous pattern where `subagent::run_one` mutated
    /// `PHANTOM_MAX_ROUNDS` via `std::env::set_var` from concurrent
    /// async tasks — which both raced (last writer wins, restore
    /// stomps on its sibling) AND triggered the `setenv()` thread-
    /// safety hazard on Linux.
    ///
    /// `None` (the default when nothing scoped it) means "use the
    /// runtime's configured `max_rounds`". `try_with` is used so
    /// callers that didn't scope the value get the same behaviour they
    /// always had — no panic, no behavioural change.
    pub static MAX_ROUNDS_OVERRIDE: Option<usize>;
}

/// Read the current task-local `max_rounds` override, returning `None`
/// when no caller scoped it. Public so `tools::subagent` can both set
/// it and verify (via the test) that the propagation works without
/// pulling in tokio's task_local internals at the call site.
pub fn current_max_rounds_override() -> Option<usize> {
    MAX_ROUNDS_OVERRIDE.try_with(|v| *v).ok().flatten()
}

const STALL_THRESHOLD: usize = 2;

/// Maximum per-provider retry attempts for transient errors (network / 429 / 503).
const MAX_RETRIES: u32 = 3;

/// Returns `true` for HTTP status codes that should NOT be retried against the
/// same provider. These indicate permanent client errors (bad key, wrong model,
/// invalid request) — the outer `'providers` loop should `continue` to the next
/// provider immediately.
///
/// Codes covered: 400 (bad request), 401 (unauthorized), 403 (forbidden),
/// 404 (not found), 422 (unprocessable entity).
fn is_non_retriable_status(status: u16) -> bool {
    matches!(status, 400 | 401 | 403 | 404 | 422)
}

/// Default system prompt injected when an agent has no `instructions` configured,
/// or prepended to any existing instructions when tools are active.
const DEFAULT_SYSTEM_PROMPT: &str = "\
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

## Workspace context
The workspace root and relevant project metadata will be injected below. Use this
information to resolve relative paths and understand project structure.
";

#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    Token {
        content: String,
    },
    /// Reasoning / chain-of-thought trace from models that expose it
    /// (Anthropic extended thinking, OpenAI o1, opencode reasoning models).
    /// Captured separately from Token so the UI can render it dimmed/collapsed
    /// above the actual answer.
    Thinking {
        content: String,
    },
    ToolStart {
        name: String,
        args_preview: String,
    },
    ToolDone {
        name: String,
        output_preview: String,
    },
    Done {
        output: String,
        cost_usd: f64,
        elapsed_secs: f64,
    },
    /// Non-fatal heads-up surfaced inline (e.g., the provider truncated the
    /// reply because we hit the max_tokens cap). The stream continues
    /// normally afterward — `Done` still fires. The UI renders this as a
    /// red warning so the user knows the answer is incomplete instead of
    /// thinking phantom hung.
    Notice {
        message: String,
    },
    /// T22 — Anti-hallucination V1. Emitted once per agent run when the
    /// deterministic scanner detects an assistant assertion of a side
    /// effect (file written, script created) without any corroborating
    /// `tool_start` event in this round. Default builds never emit this
    /// variant (the entire feature is `cfg`-stripped).
    ///
    /// The variant exists in the enum unconditionally with `#[cfg]` so
    /// callers that destructure `AgentEvent` (TUI, REPL, gateway) get a
    /// clean exhaustive-match error if they forget to handle it when
    /// the feature is on, while seeing zero diff in default builds.
    #[cfg(feature = "experimental-anti-hallucination")]
    ConsistencyWarning {
        unbacked_claims: Vec<String>,
    },
}

/// Inspect a single streaming frame and return a user-facing warning
/// message iff the frame indicates the response was truncated by hitting
/// the `max_tokens` cap. Handles both wire formats:
///
///  - Anthropic SSE: `{"type":"message_delta","delta":{"stop_reason":"max_tokens",…}}`
///  - OpenAI / Groq / Cerebras SSE: `{"choices":[{"finish_reason":"length",…}]}`
///
/// Pulled out as a free function so it is unit-testable without standing up
/// a fake provider — the streaming loop is otherwise too tangled to drive
/// from a test.
pub(crate) fn detect_truncation_notice(frame: &Value) -> Option<String> {
    let cur_cap = crate::config::default_max_tokens();
    let suggested = cur_cap.saturating_mul(2).max(16384);
    let msg = move || {
        format!(
            "⚠ Response truncated: provider hit max_tokens cap ({}). \
             Set `PHANTOM_MAX_TOKENS={}` and re-run for a larger limit, \
             or split the prompt into smaller pieces.",
            cur_cap, suggested,
        )
    };

    // Anthropic format: stop_reason rides in a message_delta event near the
    // end of the stream. We only flag when stop_reason is explicitly
    // "max_tokens" — other values ("end_turn", "tool_use", "stop_sequence")
    // are normal completions.
    if frame["type"].as_str() == Some("message_delta")
        && frame["delta"]["stop_reason"].as_str() == Some("max_tokens")
    {
        return Some(msg());
    }

    // OpenAI-shaped format: finish_reason="length" lives on the choice, not
    // inside the delta. We accept finish_reason at choices[0] (the standard
    // single-choice shape phantom uses).
    if frame["choices"][0]["finish_reason"].as_str() == Some("length") {
        return Some(msg());
    }

    None
}

/// Decision returned by a tool-permission gate.
///
/// `Allow` runs the tool normally. `Deny(reason)` skips execution and
/// reports `reason` back to the model as if the tool had returned that
/// message — the model can then choose another approach.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolGateDecision {
    Allow,
    Deny(String),
}

/// Function signature for a permission gate. Gets the tool name and the
/// parsed argument JSON, returns Allow or Deny.
pub type ToolGate = dyn Fn(&str, &Value) -> ToolGateDecision + Send + Sync;

/// Drives one agent conversation: streams the model reply, dispatches any
/// tool calls, and loops until the model stops or a round cap is reached.
///
/// Construct with [`AgentRuntime::new`], then optionally attach a resolver,
/// interrupt handle, or Hermes memory via the builder methods. Cloning is
/// cheap — the heavy state (config, HTTP client) is behind `Arc`.
#[derive(Clone)]
pub struct AgentRuntime {
    pub(crate) config: Arc<AgentsConfig>,
    pub(crate) http_client: Arc<reqwest::Client>,
    /// Optional cooperative-interrupt handle. When set, the run loop
    /// checks it before each round and the SSE reader races it inside a
    /// `tokio::select!` so a second Enter from the TUI (or a gateway
    /// RPC) can unwind a turn without waiting for the model to finish.
    /// See [`crate::interrupt::InterruptHandle`].
    pub(crate) interrupt: Option<crate::interrupt::InterruptHandle>,
    /// Optional Hermes runtime. When set, each turn's user prompt is
    /// queried against FTS5 long-term memory and the top-k recall is
    /// prepended into the system prompt as a `[memory]` block (Task 6
    /// of the integration plan). Default builds carry `None` so the
    /// run loop is byte-identical to baseline.
    #[cfg(all(
        feature = "experimental-hermes-curator",
        feature = "experimental-hermes-memory",
        feature = "experimental-hermes-tools",
    ))]
    pub(crate) hermes: Option<Arc<crate::hermes::HermesRuntime>>,
    /// DEMO-1 gap 1 Phase 5 (2026-05-17): optional resolver override.
    /// When `Some`, `call_with_fallback` + `call_with_streaming` route
    /// through this resolver instead of building a fresh
    /// `DefaultProviderResolver::from_config(&self.config)`. Default `None`
    /// preserves Phase 4 behaviour byte-for-byte. Set via
    /// [`AgentRuntime::with_resolver`] — the API gate for DEMO-3
    /// (swap-providers-per-request) and for tests that need a `MockResolver`.
    pub(crate) resolver_override: Option<Arc<dyn ResolveProvider>>,
}

/// Build the LLM HTTP client with sane timeouts.
/// Without these, mobile builds (Android/iOS) hang forever when the network
/// stalls during TLS handshake or DNS resolution.
/// `timeout` covers the whole request (LLM gen can be slow).
/// `connect_timeout` fails fast on dead routes.
/// Load the user's per-agent prompt override, if any. Returns
/// `Some(text)` when `~/.phantom-mesh/extensions/prompts/<agent>.md`
/// exists + reads OK; `None` otherwise (file missing, unreadable, or
/// empty).
///
/// Per CONTRIBUTOR-FUNNEL.md §4 + SPEC-FREEZE-V1.1 §4.1-b. Tier 1
/// extension surface — best-effort, never errors out the agent loop.
fn load_prompt_override(agent_name: &str) -> Option<String> {
    let path = crate::extensions::extensions_dir()
        .join("prompts")
        .join(format!("{}.md", agent_name));
    let text = std::fs::read_to_string(&path).ok()?;
    if text.trim().is_empty() {
        return None;
    }
    Some(text)
}

fn build_llm_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .connect_timeout(std::time::Duration::from_secs(15))
        .pool_idle_timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

impl Default for AgentRuntime {
    fn default() -> Self {
        Self {
            config: Arc::new(AgentsConfig::default()),
            http_client: Arc::new(build_llm_http_client()),
            interrupt: None,
            #[cfg(all(
                feature = "experimental-hermes-curator",
                feature = "experimental-hermes-memory",
                feature = "experimental-hermes-tools",
            ))]
            hermes: None,
            resolver_override: None,
        }
    }
}

/// Outcome of a completed agent run, returned by the non-streaming entry
/// points (streaming callers consume [`AgentEvent`]s instead).
pub struct AgentResult {
    /// Final assistant answer text after the last round.
    pub output: String,
    /// Raw tool-call JSON the model emitted across all rounds, in order.
    pub tool_calls_made: Vec<Value>,
    /// Number of model rounds executed in this run.
    pub turns: u32,
    /// Estimated cost in USD attributable to this run.
    pub cost_delta_usd: f64,
    /// Wall-clock duration of the run, in seconds.
    pub elapsed_secs: f64,
}

impl AgentRuntime {
    /// Build a runtime from agent config with a default HTTP client and no
    /// resolver override, interrupt handle, or Hermes memory attached.
    pub fn new(config: AgentsConfig) -> Self {
        Self {
            config: Arc::new(config),
            http_client: Arc::new(build_llm_http_client()),
            interrupt: None,
            #[cfg(all(
                feature = "experimental-hermes-curator",
                feature = "experimental-hermes-memory",
                feature = "experimental-hermes-tools",
            ))]
            hermes: None,
            resolver_override: None,
        }
    }

    /// DEMO-1 gap 1 Phase 5 (2026-05-17): override the default provider
    /// resolver. Builder-style (mirrors `with_hermes` / `with_interrupt`).
    ///
    /// When set, both `call_with_fallback` and `call_with_streaming` consult
    /// this resolver instead of constructing a fresh
    /// `DefaultProviderResolver::from_config(&self.config)` per call. This
    /// is the API gate that unblocks DEMO-3 — one `AgentRuntime` can swap
    /// providers per request without rebuilding (the resolver decides which
    /// `LlmProvider` to dispatch to). Also lets tests inject a `MockResolver`
    /// without rebuilding the trait object every call.
    ///
    /// Default (i.e. callers that never invoke `with_resolver`) preserves
    /// Phase 4 behaviour byte-for-byte: `resolver_override` stays `None`,
    /// so both call paths fall through to
    /// `DefaultProviderResolver::from_config(&self.config)` exactly as before.
    pub fn with_resolver(mut self, resolver: Arc<dyn ResolveProvider>) -> Self {
        self.resolver_override = Some(resolver);
        self
    }

    /// Return the resolver this runtime will use for the next call.
    ///
    /// Routes through the `with_resolver(...)` override when set; otherwise
    /// returns a fresh `DefaultProviderResolver` snapshot of the current
    /// config. Centralised so the two internal call sites
    /// (`call_with_fallback` + `call_with_streaming`) can't drift apart,
    /// and exposed as `pub` so DEMO-3 (and tests) can interrogate which
    /// provider would be dispatched to for a given configured name without
    /// driving a full run.
    pub fn active_resolver(&self) -> Arc<dyn ResolveProvider> {
        if let Some(r) = &self.resolver_override {
            return r.clone();
        }
        Arc::new(DefaultProviderResolver::from_config(&self.config))
    }

    /// Attach a Hermes runtime so each turn's prompt is augmented with
    /// recalled FTS5 memory rows (Task 6 / A4 / T94). Builder-style: cheap
    /// `Arc` clone, returns the modified runtime so callers can chain
    /// `runtime.with_hermes(rt).run(...)`. Default builds (without the
    /// `experimental-hermes` umbrella feature) lack this method entirely.
    #[cfg(all(
        feature = "experimental-hermes-curator",
        feature = "experimental-hermes-memory",
        feature = "experimental-hermes-tools",
    ))]
    pub fn with_hermes(mut self, hermes: Arc<crate::hermes::HermesRuntime>) -> Self {
        self.hermes = Some(hermes);
        self
    }

    /// Return the shared configuration arc for use by other crate consumers.
    pub fn config(&self) -> Arc<AgentsConfig> {
        self.config.clone()
    }

    /// Attach a cooperative-interrupt handle and return the modified
    /// runtime. Builder-style so callers can chain
    /// `runtime.with_interrupt(h).run_with_callbacks(...)` without
    /// touching the constructor. Cheap: `AgentRuntime` is `Clone` and
    /// internal state is `Arc`-shared.
    pub fn with_interrupt(mut self, handle: crate::interrupt::InterruptHandle) -> Self {
        self.interrupt = Some(handle);
        self
    }

    /// Quick check used at safe points in the run loop. Returns `false`
    /// when no handle is attached, so unaffected callers behave exactly
    /// as before.
    fn is_interrupted(&self) -> bool {
        self.interrupt
            .as_ref()
            .map(|h| h.is_cancelled())
            .unwrap_or(false)
    }

    pub async fn run(
        &self,
        agent_name: &str,
        prompt: &str,
        history: &[ChatMessage],
        extra_context: Option<&str>,
    ) -> anyhow::Result<AgentResult> {
        self.run_inner(
            agent_name,
            prompt,
            history,
            extra_context,
            None,
            None,
            None,
            None,
        )
        .await
    }

    pub async fn run_tracked(
        &self,
        agent_name: &str,
        prompt: &str,
        history: &[ChatMessage],
        extra_context: Option<&str>,
        cost_tracker: &CostTracker,
    ) -> anyhow::Result<AgentResult> {
        self.run_inner(
            agent_name,
            prompt,
            history,
            extra_context,
            Some(cost_tracker),
            None,
            None,
            None,
        )
        .await
    }

    /// Like `run_tracked` but additionally captures every meaningful event
    /// (User prompt, Assistant text, ToolCall, ToolResult) into the supplied
    /// `SessionWriter`. The JSONL log is what `/tasks/:id/resume` replays.
    pub async fn run_tracked_with_session(
        &self,
        agent_name: &str,
        prompt: &str,
        history: &[ChatMessage],
        extra_context: Option<&str>,
        cost_tracker: &CostTracker,
        session: &SessionWriter,
    ) -> anyhow::Result<AgentResult> {
        self.run_inner(
            agent_name,
            prompt,
            history,
            extra_context,
            Some(cost_tracker),
            None,
            Some(session),
            None,
        )
        .await
    }

    pub async fn run_with_callbacks<F>(
        &self,
        agent_name: &str,
        prompt: &str,
        history: &[ChatMessage],
        extra_context: Option<&str>,
        cost_tracker: &CostTracker,
        on_event: F,
    ) -> anyhow::Result<AgentResult>
    where
        F: FnMut(AgentEvent) + Send + Sync + 'static,
    {
        let on_event_cell = std::sync::Mutex::new(on_event);
        let on_event_fn = |ev: AgentEvent| {
            if let Ok(mut f) = on_event_cell.lock() {
                f(ev);
            }
        };
        let result = self
            .run_inner(
                agent_name,
                prompt,
                history,
                extra_context,
                Some(cost_tracker),
                Some(&on_event_fn),
                None,
                None,
            )
            .await?;
        on_event_fn(AgentEvent::Done {
            output: result.output.clone(),
            cost_usd: 0.0,
            elapsed_secs: result.elapsed_secs,
        });
        Ok(result)
    }

    /// Like `run_with_callbacks` but additionally takes a `gate` that is
    /// called before every tool execution. The gate decides whether the
    /// tool may run; on `Deny(reason)`, the tool is skipped and the agent
    /// receives `reason` as if it were the tool's output.
    pub async fn run_with_callbacks_gated<F, G>(
        &self,
        agent_name: &str,
        prompt: &str,
        history: &[ChatMessage],
        extra_context: Option<&str>,
        cost_tracker: &CostTracker,
        on_event: F,
        gate: G,
    ) -> anyhow::Result<AgentResult>
    where
        F: FnMut(AgentEvent) + Send + Sync + 'static,
        G: Fn(&str, &Value) -> ToolGateDecision + Send + Sync + 'static,
    {
        let on_event_cell = std::sync::Mutex::new(on_event);
        let on_event_fn = |ev: AgentEvent| {
            if let Ok(mut f) = on_event_cell.lock() {
                f(ev);
            }
        };
        let gate_box: Box<ToolGate> = Box::new(gate);
        let result = self
            .run_inner(
                agent_name,
                prompt,
                history,
                extra_context,
                Some(cost_tracker),
                Some(&on_event_fn),
                None,
                Some(&*gate_box),
            )
            .await?;
        on_event_fn(AgentEvent::Done {
            output: result.output.clone(),
            cost_usd: 0.0,
            elapsed_secs: result.elapsed_secs,
        });
        Ok(result)
    }

    async fn run_inner(
        &self,
        agent_name: &str,
        prompt: &str,
        history: &[ChatMessage],
        extra_context: Option<&str>,
        cost_tracker: Option<&CostTracker>,
        on_event: Option<&(dyn Fn(AgentEvent) + Send + Sync)>,
        session: Option<&SessionWriter>,
        gate: Option<&ToolGate>,
    ) -> anyhow::Result<AgentResult> {
        let start = Instant::now();
        let now_ms = || {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0)
        };

        let agent_cfg = self
            .config
            .agent
            .get(agent_name)
            .or_else(|| self.config.agent.get("master"))
            .cloned();

        let agent_cfg = match agent_cfg {
            Some(cfg) => cfg,
            None => {
                return Err(anyhow::anyhow!(
                    "No agent configuration found (agent '{}'). Check agents.toml.",
                    agent_name
                ));
            }
        };

        // Compute the set of tools the user has blanket-denied via
        // [permissions]. We strip them from the LLM-facing tool list
        // (and from the MCP fan-out below) so the model never proposes
        // a tool it can't run. Mirrors Gemini CLI's
        // `getExcludedTools()`: reduces wasted turns where the model
        // suggests `web_fetch`, gets denied, retries with the same
        // tool, gets denied again, etc. Conditional denies (e.g.
        // `Bash(rm *)`) don't appear here because they only fire on
        // matching args — the gate handles those at call time.
        let denied: std::collections::HashSet<String> = {
            let cfg = &self.config.permissions;
            let deny: Vec<&str> = cfg.deny.iter().map(String::as_str).collect();
            let ask: Vec<&str> = cfg.ask.iter().map(String::as_str).collect();
            let allow: Vec<&str> = cfg.allow.iter().map(String::as_str).collect();
            crate::permission::Engine::from_lists(&deny, &ask, &allow)
                .map(|e| e.statically_denied_tools())
                .unwrap_or_default()
        };

        let mut tool_defs: Vec<Value> = agent_cfg
            .tools
            .iter()
            .filter(|t| !denied.contains(t.as_str()))
            .filter_map(|t| crate::tools::schema(t))
            .collect();
        // Append every tool advertised by external MCP servers under their
        // `<server>_` prefix so the LLM can call them just like built-ins.
        // Same blanket-deny filter applies — a rule of `Deny: ["foo_bar"]`
        // strips `foo_bar` from the MCP slice too.
        if let Some(reg) = crate::mcp_client::global() {
            for def in reg.tool_defs().await {
                let name = def["function"]["name"].as_str().unwrap_or("");
                if !denied.contains(name) {
                    tool_defs.push(def);
                }
            }
        }

        let mut messages: Vec<Value> = Vec::new();

        // Start from the configured instructions, falling back to the built-in prompt.
        let mut system = if agent_cfg.instructions.trim().is_empty() {
            DEFAULT_SYSTEM_PROMPT.to_string()
        } else {
            agent_cfg.instructions.clone()
        };

        // CONTRIBUTOR-FUNNEL §4 — Tier 1 sandbox prompt-override loader.
        // SPEC-FREEZE-V1.1 §4.1-b: ~/.phantom-mesh/extensions/prompts/<agent>.md
        // is the user's per-agent prompt override.
        // Behaviour:
        //   - If the override file's first line is `<!-- replace -->`,
        //     the rest of the file REPLACES the configured instructions.
        //   - Otherwise the override is PREPENDED with a `## User
        //     customisation` separator, leaving built-in safety
        //     instructions intact below.
        // Errors (file unreadable, malformed) silently skip — Tier 1
        // is best-effort; failure here doesn't block agent execution.
        if let Some(override_text) = load_prompt_override(agent_name) {
            const REPLACE_MARKER: &str = "<!-- replace -->";
            if override_text.trim_start().starts_with(REPLACE_MARKER) {
                // Strip the marker line and use the rest verbatim.
                system = override_text
                    .trim_start()
                    .trim_start_matches(REPLACE_MARKER)
                    .trim_start_matches('\n')
                    .to_string();
            } else {
                // Prepend with separator.
                system = format!(
                    "## User customisation\n{}\n\n## Agent instructions\n{}",
                    override_text.trim(),
                    system
                );
            }
        }

        if !tool_defs.is_empty() {
            system.push_str(
                "\n\nCRITICAL RULES:\n\
                - You MUST call the appropriate tool function to perform any action. NEVER describe what you would do — just call the tool.\n\
                - To modify a file: call file_read first, then file_edit with exact old_string and new_string.\n\
                - To run a command: call shell with the exact command string.\n\
                - Never output code blocks as a substitute for calling a tool."
            );
        }
        // A4/T94 — close the loop: when a HermesRuntime is attached, query
        // FTS5 long-term memory for the prompt and inject the top-k hits as
        // a `[memory]` block AFTER the CRITICAL RULES block. The header is a
        // stable cut line that `compact_if_needed` looks for when trimming
        // the system prompt under token pressure.
        #[cfg(all(
            feature = "experimental-hermes-curator",
            feature = "experimental-hermes-memory",
            feature = "experimental-hermes-tools",
        ))]
        if let Some(rt) = self.hermes.as_ref() {
            match rt
                .recall_context_for(prompt, crate::hermes::MEMORY_CONTEXT_MAX_ROWS)
                .await
            {
                Ok(rows) if !rows.is_empty() => {
                    system.push_str("\n\n");
                    system.push_str(crate::hermes::MEMORY_CONTEXT_HEADER);
                    for r in rows {
                        system.push('\n');
                        system.push_str("- ");
                        system.push_str(&r);
                    }
                }
                Ok(_) => {} // no hits — quietly skip
                Err(e) => {
                    // Memory failure must NEVER break the agent loop.
                    tracing::debug!(error = %e, "hermes recall_context_for failed; continuing without memory injection");
                }
            }
        }
        if let Some(extra) = extra_context {
            if !extra.is_empty() {
                system.push_str("\n\n");
                system.push_str(extra);
            }
        }
        // SPEC-14 §9.2 / G5 — frame the assembled system prompt for the
        // primary provider's PromptStyle (Claude XML 標籤 vs GPT JSON 模式 vs
        // Gemini 問答 etc.). The primary provider is the first entry in the
        // resolved attempt order; its `provider:model` parts pick the style
        // AND the SystemPlacement (where the system text physically sits).
        // This is the agent-layer prompt-shaping slice; full SeparateParam
        // extraction (Anthropic out-of-band `system:` param) is adapter-side
        // (see T-PROV-05 blockers).
        let mut placement = SystemPlacement::RoleSystem;
        if !system.is_empty() {
            let order = resolve_provider_order(
                &agent_cfg,
                self.config.providers.keys().map(|s| s.as_str()),
            );
            if let Some(primary) = order.first() {
                let (pname, pmodel) = parse_provider_entry(primary);
                let style = prompt_style_for_provider(pname, pmodel);
                system = frame_system_for_style(&system, style);
                placement = system_placement_for_provider(pname, pmodel);
            }
        }
        // SPEC-14 §7.1 SystemPlacement — branch where the framed system text
        // goes. `RoleSystem` (the default today) + `SeparateParam` both keep a
        // `messages[0].role = "system"` entry at this neutral-messages layer;
        // the per-provider adapter later lifts it out-of-band for Anthropic's
        // separate `system:` param. `EmbedInUserTurn` (on-device / local models
        // with no system role) instead prepends the system text into the first
        // user turn and emits NO system message.
        let embed_system_in_user = !system.is_empty() && placement == SystemPlacement::EmbedInUserTurn;
        if !system.is_empty() && !embed_system_in_user {
            messages.push(serde_json::json!({"role": "system", "content": system}));
        }
        for msg in history {
            messages.push(serde_json::json!({"role": msg.role, "content": msg.content}));
        }
        let user_content = crate::multimodal::prompt_to_content_value(prompt);
        if embed_system_in_user {
            // No system role available — fold the system text into the user
            // turn. Keep multimodal content intact: if the content is a plain
            // string we concatenate; otherwise we lead with a text part so the
            // attachments (image / audio) still ride along unchanged.
            let combined = embed_system_into_user_content(&system, user_content);
            messages.push(serde_json::json!({"role": "user", "content": combined}));
        } else {
            messages.push(serde_json::json!({"role": "user", "content": user_content}));
        }
        if let Some(s) = session {
            let _ = s
                .append(pm_types::SessionEntry::User {
                    content: prompt.to_string(),
                    timestamp: now_ms(),
                })
                .await;
        }

        let mut all_tool_calls: Vec<Value> = Vec::new();
        let mut final_output = String::new();
        let mut stall_rounds: usize = 0;
        let mut last_output = String::new();
        let mut provider_error: Option<anyhow::Error> = None;
        let mut rounds_used: u32 = 0;
        let mut cost_delta_usd: f64 = 0.0;

        // Per-task `max_rounds` override (audit C-3 fix). When a
        // caller (e.g. `tools::subagent::run_one`) wraps the future
        // with `MAX_ROUNDS_OVERRIDE.scope(Some(n), …)`, that value
        // wins over the runtime's global config — but ONLY for this
        // task. Concurrent subagents each see their own scope, no
        // shared mutation, no `setenv()` race.
        let max_rounds = current_max_rounds_override().unwrap_or(self.config.max_rounds);
        let token_budget = self.config.token_budget;

        for round in 0..max_rounds {
            // Cooperative interrupt: bail out at round boundary if the
            // TUI / gateway flipped the cancel flag. We deliberately
            // check *before* compaction + provider dispatch so the
            // unwind is cheap; mid-stream cancellation is handled in
            // `call_with_streaming` via `tokio::select!`.
            if self.is_interrupted() {
                tracing::info!("agent loop interrupted at round {}", round);
                break;
            }

            compact_if_needed(&mut messages, token_budget);

            let (json, model_used) = if let Some(f) = on_event {
                match self
                    .call_with_streaming(&agent_cfg, &messages, &tool_defs, f)
                    .await
                {
                    Ok(pair) => pair,
                    Err(e) => {
                        provider_error = Some(e);
                        break;
                    }
                }
            } else {
                match self
                    .call_with_fallback(&agent_cfg, &messages, &tool_defs)
                    .await
                {
                    Ok(pair) => pair,
                    Err(e) => {
                        provider_error = Some(e);
                        break;
                    }
                }
            };

            rounds_used = rounds_used.saturating_add(1);

            let prompt_tokens = json["usage"]["prompt_tokens"].as_u64().unwrap_or(0);
            let completion_tokens = json["usage"]["completion_tokens"].as_u64().unwrap_or(0);
            if prompt_tokens > 0 || completion_tokens > 0 {
                let (input_p, output_p) = crate::cost::price_per_million(&model_used);
                cost_delta_usd += (prompt_tokens as f64 / 1_000_000.0) * input_p
                    + (completion_tokens as f64 / 1_000_000.0) * output_p;
                if let Some(ct) = cost_tracker {
                    ct.record(&model_used, prompt_tokens, completion_tokens)
                        .await;
                    if ct.is_over_budget().await {
                        tracing::warn!(
                            "Agent budget exceeded after round {} (cost_delta=${:.4}); breaking out.",
                            round, cost_delta_usd
                        );
                        break;
                    }
                }
            }

            let message = &json["choices"][0]["message"];

            if let Some(text) = message["content"].as_str() {
                if !text.is_empty() {
                    final_output = text.to_string();
                    if let Some(s) = session {
                        let _ = s
                            .append(pm_types::SessionEntry::Assistant {
                                content: text.to_string(),
                                timestamp: now_ms(),
                            })
                            .await;
                    }
                }
            }

            let tool_calls = match message["tool_calls"].as_array() {
                Some(tc) if !tc.is_empty() => {
                    stall_rounds = 0;
                    last_output = final_output.clone();
                    tc.clone()
                }
                _ => {
                    let current_output = final_output.clone();
                    if output_unchanged(&last_output, &current_output) {
                        stall_rounds += 1;
                    } else {
                        stall_rounds = 0;
                    }
                    if stall_rounds >= STALL_THRESHOLD {
                        tracing::warn!("Agent stall detected at round {}", round);
                    }
                    break;
                }
            };

            messages.push(message.clone());

            // Build a typed work list so we can fire ToolStart events before
            // launching the concurrent futures, and preserve ordering afterwards.
            // Deduplicate: skip any (tool_name, args) pair that appeared earlier in
            // this same response to prevent infinite-loop traps.
            let mut seen_calls: HashSet<String> = HashSet::new();
            let work: Vec<(String, String, Value, String)> = tool_calls
                .iter()
                .enumerate()
                .filter_map(|(i, tc)| {
                    let tc_id = tc["id"]
                        .as_str()
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| format!("call_{}", i));
                    let fn_name = tc["function"]["name"]
                        .as_str()
                        .unwrap_or("unknown")
                        .to_string();
                    let fn_args: Value = tc["function"]["arguments"]
                        .as_str()
                        .and_then(|s| serde_json::from_str(s).ok())
                        .unwrap_or(Value::Object(serde_json::Map::new()));
                    let preview = serde_json::to_string(&fn_args)
                        .unwrap_or_default()
                        .chars()
                        .take(120)
                        .collect::<String>();

                    // Dedup key: tool name + canonical args JSON.
                    let dedup_key = format!(
                        "{}:{}",
                        fn_name,
                        serde_json::to_string(&fn_args).unwrap_or_default()
                    );
                    if !seen_calls.insert(dedup_key.clone()) {
                        tracing::warn!(
                            "Skipping duplicate tool call: {} (args: {})",
                            fn_name,
                            preview
                        );
                        return None;
                    }

                    Some((tc_id, fn_name, fn_args, preview))
                })
                .collect();

            // Record all tool calls and fire ToolStart for each (sequential,
            // before launching concurrent execution).
            for (tc_id, fn_name, fn_args, preview) in &work {
                all_tool_calls.push(serde_json::json!({
                    "tool": fn_name,
                    "args": fn_args,
                }));
                if let Some(f) = on_event {
                    f(AgentEvent::ToolStart {
                        name: fn_name.clone(),
                        args_preview: preview.clone(),
                    });
                }
                if let Some(s) = session {
                    let _ = s
                        .append(pm_types::SessionEntry::ToolCall {
                            call_id: tc_id.clone(),
                            name: fn_name.clone(),
                            args: fn_args.clone(),
                            timestamp: now_ms(),
                        })
                        .await;
                }
            }

            // Execute all tools concurrently — but consult the optional `gate`
            // first. Denied tools never run; their reason string is returned
            // to the model as if it were the tool's output.
            let tools_config = &self.config.tools;
            let results: Vec<String> = join_all(work.iter().map(|(_, fn_name, fn_args, _)| {
                let decision = match gate {
                    Some(g) => g(fn_name, fn_args),
                    None => ToolGateDecision::Allow,
                };
                async move {
                    match decision {
                        ToolGateDecision::Allow => {
                            crate::tools::execute(fn_name, fn_args, tools_config).await
                        }
                        ToolGateDecision::Deny(reason) => {
                            format!("[denied by user] {}", reason)
                        }
                    }
                }
            }))
            .await;

            // Fire ToolDone and push tool result messages in order.
            for ((tc_id, fn_name, _, _), result) in work.iter().zip(results.iter()) {
                tracing::debug!("tool {} → {} chars", fn_name, result.len());
                if let Some(s) = session {
                    let _ = s
                        .append(pm_types::SessionEntry::ToolResult {
                            call_id: tc_id.clone(),
                            output: result.clone(),
                            synthetic: false,
                            timestamp: now_ms(),
                        })
                        .await;
                }
                if let Some(f) = on_event {
                    // Send the FULL tool output. Consumers (REPL, SSE, evolve)
                    // truncate at the display layer so the full text is still
                    // available for /show <n> expansion in the REPL.
                    f(AgentEvent::ToolDone {
                        name: fn_name.clone(),
                        output_preview: result.clone(),
                    });
                }
                messages.push(serde_json::json!({
                    "role": "tool",
                    "tool_call_id": tc_id,
                    "content": result,
                }));
            }
        }

        // If the loop broke because every provider failed (and no usable output
        // was produced), surface that as an error so callers can mark the task
        // as Failed. Otherwise any non-empty output or successful tool run is
        // considered a completion even if the last round stalled.
        if let Some(err) = provider_error {
            if final_output.is_empty() && all_tool_calls.is_empty() {
                return Err(err);
            }
            tracing::warn!(
                "provider error after partial progress ({} tool calls, {} output chars): {}",
                all_tool_calls.len(),
                final_output.len(),
                err,
            );
        }

        // Empty-response guard (OpenFang-style): no output AND no tool calls =
        // nothing was actually produced — treat as failure rather than lying to
        // the caller with a happy empty string.
        //
        // Exception: when the user interrupted mid-turn, "no output" is the
        // expected outcome (we cancelled before the model wrote anything).
        // Return Ok with an empty result so the TUI's chain-on-finish path
        // can see the JoinHandle complete cleanly and fire the queued
        // follow-up prompt. Without this exception, an early interrupt
        // would surface as a red "agent produced no output" line in
        // transcript and confuse the user.
        if final_output.is_empty() && all_tool_calls.is_empty() {
            if self.is_interrupted() {
                return Ok(AgentResult {
                    output: String::new(),
                    tool_calls_made: Vec::new(),
                    turns: rounds_used,
                    cost_delta_usd,
                    elapsed_secs: start.elapsed().as_secs_f64(),
                });
            }
            return Err(anyhow::anyhow!(
                "agent produced no output and made no tool calls"
            ));
        }

        // ── T22: anti-hallucination V1 hook ──────────────────────────────
        // Pure deterministic scan; no I/O, no LLM. Fires AgentEvent::ConsistencyWarning
        // when the assistant asserted file/script creation in `final_output` while
        // `all_tool_calls` is empty. V1 covers Shape 1 only. See
        // docs/anti-hallucination-v1-design.md.
        #[cfg(feature = "experimental-anti-hallucination")]
        {
            let tool_results: Vec<String> = messages
                .iter()
                .filter(|m| m.get("role").and_then(|r| r.as_str()) == Some("tool"))
                .filter_map(|m| m.get("content").and_then(|c| c.as_str()).map(String::from))
                .collect();
            let claims = crate::hallucination::scan(&final_output, &all_tool_calls, &tool_results);
            if !claims.is_empty() {
                let summaries: Vec<String> = claims
                    .iter()
                    .map(|c| format!("{}: {}", c.rule_id, c.explanation))
                    .collect();
                tracing::warn!(
                    "anti-hallucination: {} unbacked claim(s) — {}",
                    summaries.len(),
                    summaries.join(" | "),
                );
                if let Some(f) = on_event {
                    f(AgentEvent::ConsistencyWarning {
                        unbacked_claims: summaries,
                    });
                }
            }
        }

        Ok(AgentResult {
            output: final_output,
            tool_calls_made: all_tool_calls,
            turns: rounds_used,
            cost_delta_usd,
            elapsed_secs: start.elapsed().as_secs_f64(),
        })
    }

    /// Attempt an HTTP POST with exponential back-off retry for transient errors.
    /// Returns `Ok(response)` on success, or `Err(last_error_string)` if the
    /// provider should be skipped (non-retriable error or retries exhausted).
    async fn streaming_with_retry(
        &self,
        url: &str,
        key: &str,
        body: &Value,
        provider_name: &str,
        // Phase 4: header shape now comes from the `LlmProvider` trait
        // (e.g. Anthropic adds `anthropic-version`, OpenAI-compat doesn't).
        // The trait's headers do NOT include the auth header — we add
        // `x-api-key` for Anthropic-shaped requests and `Bearer …` for the
        // rest, detected by checking the URL path.
        headers: &[(&'static str, String)],
    ) -> Result<reqwest::Response, String> {
        let is_anthropic_messages = url.contains("/v1/messages");
        let mut last_err = String::new();
        for attempt in 0..MAX_RETRIES {
            if attempt > 0 {
                let delay = 1u64 << (attempt - 1);
                tracing::info!(
                    provider = %provider_name,
                    attempt,
                    delay_secs = delay,
                    "Retrying streaming call after transient error"
                );
                tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
            }

            let mut req = self.http_client.post(url);
            // Auth: Anthropic's native /v1/messages takes `x-api-key`;
            // every other path (including Anthropic's OpenAI-compat
            // /v1/chat/completions proxy) takes `Authorization: Bearer`.
            req = if is_anthropic_messages {
                req.header("x-api-key", key)
            } else {
                req.header("Authorization", format!("Bearer {}", key))
            };
            for (k, v) in headers {
                req = req.header(*k, v.clone());
            }

            let r = match req.json(body).send().await {
                Ok(r) => r,
                Err(e) => {
                    last_err = e.to_string();
                    tracing::warn!(provider = %provider_name, attempt, "Network error: {}", e);
                    continue;
                }
            };

            let status_u16 = r.status().as_u16();

            if r.status().is_success() {
                return Ok(r);
            }

            // Non-retriable client errors — skip provider immediately.
            if is_non_retriable_status(status_u16) {
                let text = r.text().await.unwrap_or_default();
                last_err = format!(
                    "[{}] HTTP {} from {}: {}",
                    provider_name,
                    status_u16,
                    url,
                    crate::tools::floor_char_boundary(&text, 200)
                );
                tracing::warn!(
                    provider = %provider_name,
                    status = status_u16,
                    "Non-retriable streaming error, skipping provider"
                );
                return Err(last_err);
            }

            // Retriable: 429 / 503.
            let wait = if status_u16 == 429 {
                r.headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(0)
                    .min(30)
            } else {
                0
            };
            let text = r.text().await.unwrap_or_default();
            last_err = format!(
                "[{}] HTTP {} from {}: {}",
                provider_name,
                status_u16,
                url,
                crate::tools::floor_char_boundary(&text, 200)
            );
            tracing::warn!(provider = %provider_name, status = status_u16, "Streaming provider transient error");
            if matches!(status_u16, 429 | 503) {
                if wait > 0 {
                    tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
                }
                continue; // retry
            }
            // Any other non-success status — skip provider.
            return Err(last_err);
        }
        tracing::warn!(provider = %provider_name, "Exhausted streaming retries, trying next provider");
        Err(last_err)
    }

    async fn call_with_streaming(
        &self,
        agent_cfg: &AgentEntry,
        messages: &[Value],
        tool_defs: &[Value],
        on_token: &(dyn Fn(AgentEvent) + Send + Sync),
    ) -> anyhow::Result<(Value, String)> {
        // Same priority resolution as call_with_fallback + streaming.rs.
        // This is the THIRD code path (repl streaming) — was hardcoded
        // (provider + alphabetical) before, ignoring agent.providers list
        // AND PHANTOM_RUNTIME_OVERRIDE / runtime-override file. So /model
        // X:Y in TUI didn't reach repl-mode chat either. Now consistent.
        let mut provider_names =
            resolve_provider_order(agent_cfg, self.config.providers.keys().map(|s| s.as_str()));
        let runtime_over = std::env::var("PHANTOM_RUNTIME_OVERRIDE")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(crate::cli_config::read_runtime_override);
        if let Some(over) = runtime_over {
            let trimmed = over.trim();
            if !trimmed.is_empty() {
                provider_names.retain(|n| n != trimmed);
                provider_names.insert(0, trimmed.to_string());
            }
        }
        // DEMO-1 gap 1 Phase 4: build the LlmProvider trait resolver once per
        // call so per-provider dispatch goes through `build_stream_request`
        // (which preserves Anthropic-specific cache_control / adaptive
        // thinking / multimodal conversion). The legacy `provider_url`
        // string-switch is gone; the resolver IS the dispatch surface.
        //
        // Phase 5 (2026-05-17): `active_resolver()` returns the
        // `with_resolver(...)` override when set, otherwise falls through to
        // a fresh `DefaultProviderResolver::from_config(&self.config)` —
        // byte-identical to the Phase 4 path when no override is installed.
        let resolver = self.active_resolver();

        let mut last_err = String::new();
        // Collect ALL provider failures so the final error message can
        // explain WHY each one was skipped — was the #1 user complaint
        // pre-open-source: "All providers failed" with no per-provider
        // detail meant users couldn't tell whether to fix a key, change
        // model id, or unblock the network.
        let mut errors: Vec<String> = Vec::new();
        let mut tried_any = false;

        'providers: for entry in provider_names.iter() {
            // Each entry can be bare `<provider>` or `<provider>:<model>`.
            let (provider_name, entry_model) = parse_provider_entry(entry);
            let Some(provider) = self.config.providers.get(provider_name) else {
                let msg = format!(
                    "[{}] not in [providers.*] (no such block in agents.toml)",
                    provider_name
                );
                errors.push(msg.clone());
                crate::diag::record("provider_skip", format!("(streaming) {}", msg));
                continue;
            };
            let api_key = provider.api_key.clone().or_else(|| {
                provider
                    .api_key_env
                    .as_ref()
                    .and_then(|env| std::env::var(env).ok())
            });
            let Some(key) = api_key.filter(|k| !k.is_empty()) else {
                let env_name = provider
                    .api_key_env
                    .as_deref()
                    .unwrap_or("(no api_key_env)");
                let msg = format!(
                    "[{}] no key — env var {} unset (vault sync? `phantom config pull`)",
                    provider_name, env_name
                );
                errors.push(msg.clone());
                crate::diag::record("provider_skip", format!("(streaming) {}", msg));
                continue;
            };
            tried_any = true;
            crate::diag::record(
                "provider_attempt",
                format!(
                    "[{}] (streaming) trying with model={}",
                    provider_name,
                    entry_model.unwrap_or("(default)")
                ),
            );

            // Per-entry model > agent.model > provider.default_model.
            let model = entry_model
                .map(|m| m.to_string())
                .filter(|m| !m.is_empty())
                .or_else(|| (!agent_cfg.model.is_empty()).then(|| agent_cfg.model.clone()))
                .or_else(|| provider.default_model.clone())
                .unwrap_or_default();
            if model.is_empty() {
                if !crate::diag::is_tui_active() {
                    eprintln!(
                        "  [provider {}] skipped: no model configured",
                        provider_name
                    );
                }
                let msg = format!("[{}] no model — entry isn't `provider:model` and provider has no default_model", provider_name);
                errors.push(msg.clone());
                last_err = msg;
                continue 'providers;
            }

            // DEMO-1 gap 1 Phase 4: shape URL + body + headers via the
            // LlmProvider trait. AnthropicProvider/ClaudeCliProvider emit
            // native /v1/messages with cache_control + adaptive thinking;
            // OpenAICompatProvider/GeminiProvider stay on
            // /v1/chat/completions. If a provider isn't registered with the
            // resolver (e.g. config drift), fall back to the OpenAI-compat
            // trait default so behaviour matches the legacy fallthrough.
            let parts_result = match resolver.resolve_by_name(provider_name) {
                Some(p) => p.build_stream_request(&BuildRequestOpts {
                    model: &model,
                    system: "",
                    messages,
                    tools: tool_defs,
                    base_url_override: provider.url.as_deref(),
                    stream: true,
                    max_tokens: crate::config::default_max_tokens(),
                }),
                None => {
                    let msg = format!(
                        "[{}] resolver returned None (no provider type matched)",
                        provider_name
                    );
                    errors.push(msg.clone());
                    last_err = msg;
                    continue 'providers;
                }
            };
            let parts = match parts_result {
                Ok(p) => p,
                Err(e) => {
                    let msg = format!("[{}] build_stream_request failed: {}", provider_name, e);
                    errors.push(msg.clone());
                    last_err = msg;
                    continue 'providers;
                }
            };

            // --- per-provider retry loop for streaming ---
            // Returns Ok(resp) on success, Err(true) to skip provider, Err(false) exhausted.
            let resp_result = self
                .streaming_with_retry(&parts.url, &key, &parts.body, provider_name, &parts.headers)
                .await;
            let resp = match resp_result {
                Ok(r) => r,
                Err(err_msg) => {
                    if !crate::diag::is_tui_active() {
                        eprintln!("  [provider {}] failed: {}", provider_name, err_msg);
                    }
                    crate::diag::record(
                        "provider_fail",
                        format!(
                            "[{}] (streaming) {}",
                            provider_name,
                            err_msg.chars().take(200).collect::<String>()
                        ),
                    );
                    let msg = format!("[{}] {}", provider_name, err_msg);
                    errors.push(msg.clone());
                    last_err = msg;
                    continue 'providers;
                }
            };

            // Stream the response body, parsing SSE frames.
            // Each individual chunk must arrive within 30s to prevent infinite hangs
            // when the server keeps the connection open without sending [DONE].
            let mut stream = resp.bytes_stream();
            let mut line_buf = String::new();
            let mut full_content = String::new();
            // tool_calls accumulator: index → (id, name, accumulated_args)
            let mut tool_calls_map: HashMap<usize, (String, String, String)> = HashMap::new();

            'stream: loop {
                // Race three things: (a) the next SSE chunk, (b) the
                // 30 s no-progress timeout, (c) the cooperative
                // interrupt handle. Without (c), a second Enter from
                // the TUI would have to wait for the model to either
                // finish or stall for 30 s before we noticed.
                let next_chunk =
                    tokio::time::timeout(std::time::Duration::from_secs(30), stream.next());
                let next = if let Some(intr) = self.interrupt.as_ref() {
                    tokio::select! {
                        biased;
                        _ = intr.cancelled() => {
                            tracing::info!(provider = %provider_name, "stream cancelled by interrupt");
                            crate::diag::record(
                                "agent_interrupt",
                                format!("stream cancelled at provider={} content_len={} tool_calls={}",
                                    provider_name, full_content.len(), tool_calls_map.len()),
                            );
                            break 'stream;
                        }
                        r = next_chunk => r,
                    }
                } else {
                    next_chunk.await
                };
                let chunk_opt = match next {
                    Ok(opt) => opt,
                    Err(_) => {
                        tracing::warn!(provider = %provider_name, "Stream chunk timeout (30s), treating as end of stream");
                        break 'stream;
                    }
                };
                let chunk = match chunk_opt {
                    None => break 'stream,
                    Some(Ok(b)) => b,
                    Some(Err(e)) => {
                        tracing::warn!("Streaming chunk error: {}", e);
                        break 'stream;
                    }
                };
                let text = match std::str::from_utf8(&chunk) {
                    Ok(s) => s,
                    Err(e) => {
                        // TODO Stage 2: implement proper UTF-8 boundary buffer
                        // (Vec<u8> carry-over across chunks). For now log + skip
                        // so the corruption is at least traceable instead of
                        // silent. Splits mid-emoji / mid-multibyte will still
                        // drop the chunk, but operator can see frequency in logs.
                        tracing::warn!(chunk_len = chunk.len(), valid_up_to = e.valid_up_to(), "stream chunk failed UTF-8 decode (dropped — multibyte boundary split?): {}", e);
                        continue;
                    }
                };
                // SSE lines may be split across chunks; buffer them.
                for ch in text.chars() {
                    if ch == '\n' {
                        let line = line_buf.trim_end_matches('\r').to_string();
                        line_buf.clear();

                        if !line.starts_with("data: ") {
                            continue;
                        }
                        let data = &line[6..];
                        if data == "[DONE]" {
                            break 'stream;
                        }

                        let json: Value = match serde_json::from_str(data) {
                            Ok(v) => v,
                            Err(e) => {
                                tracing::debug!(
                                    data_len = data.len(),
                                    "SSE frame parse failed, skipping: {}",
                                    e
                                );
                                continue;
                            }
                        };

                        // Surface max_tokens truncation as a non-fatal Notice
                        // BEFORE per-format parsing — the frame carrying the
                        // truncation signal (Anthropic message_delta / OpenAI
                        // finish_reason) wouldn't otherwise reach a branch
                        // that calls on_token, and the user would just see
                        // output stop mid-sentence with no explanation.
                        if let Some(msg) = detect_truncation_notice(&json) {
                            on_token(AgentEvent::Notice { message: msg });
                        }

                        // Anthropic streaming format
                        if json["type"].as_str() == Some("content_block_delta") {
                            let dty = json["delta"]["type"].as_str();
                            if dty == Some("thinking_delta") {
                                if let Some(t) = json["delta"]["thinking"].as_str() {
                                    if !t.is_empty() {
                                        on_token(AgentEvent::Thinking {
                                            content: t.to_string(),
                                        });
                                    }
                                }
                                continue;
                            }
                            if let Some(token) = json["delta"]["text"].as_str() {
                                if !token.is_empty() {
                                    full_content.push_str(token);
                                    on_token(AgentEvent::Token {
                                        content: token.to_string(),
                                    });
                                }
                            }
                            continue;
                        }

                        // OpenAI streaming format
                        if let Some(delta) = json["choices"][0]["delta"].as_object() {
                            // Reasoning trace (opencode/groq/openrouter reasoning models
                            // expose chain-of-thought in `reasoning` or `reasoning_content`).
                            for k in ["reasoning", "reasoning_content"] {
                                if let Some(t) = delta.get(k).and_then(|v| v.as_str()) {
                                    if !t.is_empty() {
                                        on_token(AgentEvent::Thinking {
                                            content: t.to_string(),
                                        });
                                    }
                                }
                            }
                            // Text token
                            if let Some(token) = delta.get("content").and_then(|v| v.as_str()) {
                                if !token.is_empty() {
                                    full_content.push_str(token);
                                    on_token(AgentEvent::Token {
                                        content: token.to_string(),
                                    });
                                }
                            }
                            // Tool call deltas
                            if let Some(tc_array) =
                                delta.get("tool_calls").and_then(|v| v.as_array())
                            {
                                for tc in tc_array {
                                    let index = tc["index"].as_u64().unwrap_or(0) as usize;
                                    let entry = tool_calls_map.entry(index).or_insert_with(|| {
                                        (String::new(), String::new(), String::new())
                                    });
                                    if let Some(id) = tc["id"].as_str() {
                                        if entry.0.is_empty() {
                                            entry.0 = id.to_string();
                                        }
                                    }
                                    if let Some(name) = tc["function"]["name"].as_str() {
                                        if entry.1.is_empty() {
                                            entry.1 = name.to_string();
                                        }
                                    }
                                    if let Some(args) = tc["function"]["arguments"].as_str() {
                                        entry.2.push_str(args);
                                    }
                                }
                            }
                        }
                    } else {
                        line_buf.push(ch);
                    }
                }
            }

            // Build tool_calls array in index order.
            let mut sorted_indices: Vec<usize> = tool_calls_map.keys().cloned().collect();
            sorted_indices.sort();
            let tool_calls_json: Vec<Value> = sorted_indices
                .iter()
                .map(|idx| {
                    let (id, name, args) = &tool_calls_map[idx];
                    serde_json::json!({
                        "id": id,
                        "type": "function",
                        "function": {
                            "name": name,
                            "arguments": args,
                        }
                    })
                })
                .collect();

            // If the user interrupted mid-stream, do NOT fall through
            // to the empty-stream provider fallback below — that
            // would cycle every configured provider with the user's
            // (redirected) prompt and waste tokens / surface noise.
            // Instead, return whatever partial content we have so
            // run_inner can unwind cleanly. The outer loop's pre-
            // round `is_interrupted()` check then breaks the round
            // loop and AgentResult goes back to the TUI, which
            // chains the queued prompt.
            if self.is_interrupted() {
                crate::diag::record(
                    "agent_interrupt",
                    format!("returning partial after interrupt: provider={} content_len={} tool_calls={}",
                        provider_name, full_content.len(), tool_calls_json.len()),
                );
                let synthetic = serde_json::json!({
                    "choices": [{"message": {
                        "role": "assistant",
                        "content": full_content,
                    }}],
                    "usage": {},
                });
                return Ok((synthetic, model));
            }

            // If the stream completed with zero content and no tool calls, treat it
            // as a transient failure so the provider loop tries the next provider
            // rather than returning an empty synthetic response that will hit the
            // empty-output guard in run_inner.
            if full_content.is_empty() && tool_calls_json.is_empty() {
                let msg = format!(
                    "[{}] empty response — no content, no tool calls (model returned blank stream)",
                    provider_name
                );
                errors.push(msg.clone());
                last_err = msg;
                tracing::warn!(provider = %provider_name, "Empty stream result, trying next provider");
                continue 'providers;
            }

            // Synthesise a non-streaming response shape so run_inner works unchanged.
            let synthetic = if tool_calls_json.is_empty() {
                serde_json::json!({
                    "choices": [{"message": {"role": "assistant", "content": full_content}}],
                    "usage": {}
                })
            } else {
                serde_json::json!({
                    "choices": [{"message": {
                        "role": "assistant",
                        "content": full_content,
                        "tool_calls": tool_calls_json,
                    }}],
                    "usage": {}
                })
            };

            return Ok((synthetic, model));
        }

        // Build a multi-line error that lists EVERY provider attempt's
        // outcome so the user can self-diagnose: missing key vs 401 vs
        // model unavailable vs network. Was a single "Last error: X"
        // line that hid the rest of the chain.
        let breakdown = if errors.is_empty() {
            format!("(no providers tried — check [agent.X].providers list and [providers.*] blocks; last_err: {})", last_err)
        } else {
            format!("\n  - {}", errors.join("\n  - "))
        };
        let hint = if !tried_any {
            "\n\nNo provider had a usable key in env. Run `phantom config pull` to refresh \
             vault keys, or set them manually: [Environment]::SetEnvironmentVariable('OPENCODE_API_KEY','<key>','User').\n\
             View / reorder failover chain: /priority   (in TUI)"
        } else {
            "\n\nFix any of the above and the chain will recover. /priority in TUI to reorder, \
             /provider list to check key state."
        };
        Err(anyhow::anyhow!(
            "All providers failed (streaming). Tried {} provider(s):{}{}",
            errors.len(),
            breakdown,
            hint
        ))
    }

    async fn call_with_fallback(
        &self,
        agent_cfg: &AgentEntry,
        messages: &[Value],
        tool_defs: &[Value],
    ) -> anyhow::Result<(Value, String)> {
        // DEMO-1 gap 1 Phase 4: build the LlmProvider trait resolver once per
        // call so non-streaming dispatch goes through `build_stream_request`
        // (same path as call_with_streaming, with `stream: false`). The
        // legacy `provider_url` string-switch is gone.
        //
        // Phase 5 (2026-05-17): see `active_resolver()` doc — override-aware.
        let resolver = self.active_resolver();
        let mut provider_names =
            resolve_provider_order(agent_cfg, self.config.providers.keys().map(|s| s.as_str()));
        // Per-session runtime override. Two sources, env first then file:
        //   1. PHANTOM_RUNTIME_OVERRIDE env (this process)
        //   2. ~/.phantom-mesh/runtime-override (shared across all phantom
        //      processes — so /model X:Y in the TUI also affects the
        //      local `phantom serve` daemon and cluster RPC dispatch.)
        // First non-empty wins. Prepended to provider chain, de-duped.
        let runtime_over = std::env::var("PHANTOM_RUNTIME_OVERRIDE")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(crate::cli_config::read_runtime_override);
        if let Some(over) = runtime_over {
            let trimmed = over.trim();
            if !trimmed.is_empty() {
                provider_names.retain(|n| n != trimmed);
                provider_names.insert(0, trimmed.to_string());
            }
        }

        let mut last_err = String::new();
        let mut errors: Vec<String> = Vec::new();
        let mut tried_any = false;

        'providers: for entry in provider_names.iter() {
            // Each entry can be bare `<provider>` or `<provider>:<model>`.
            // SAME parser as call_with_streaming + streaming.rs so cluster
            // RPC dispatch (which uses this fallback path) honors the
            // priority list's `provider:model` entries identically to
            // local TUI streaming. Without this, "opencode:minimax-m2.5-free"
            // was treated as a literal provider name → not found → skip,
            // chain fell through to the legacy `agent.model` field which
            // might be a totally unrelated model from a different provider
            // (real bug hit on node-b: groq's llama-3.3-70b-versatile was
            // sent to opencode endpoint → ModelError 401).
            let (provider_name, entry_model) = parse_provider_entry(entry);
            let Some(provider) = self.config.providers.get(provider_name) else {
                let msg = format!(
                    "[{}] not in [providers.*] (no such block in agents.toml)",
                    provider_name
                );
                errors.push(msg.clone());
                crate::diag::record("provider_skip", msg);
                continue;
            };
            let api_key = provider.api_key.clone().or_else(|| {
                provider
                    .api_key_env
                    .as_ref()
                    .and_then(|env| std::env::var(env).ok())
            });
            let Some(key) = api_key.filter(|k| !k.is_empty()) else {
                let env_name = provider
                    .api_key_env
                    .as_deref()
                    .unwrap_or("(no api_key_env)");
                let msg = format!(
                    "[{}] no key — env var {} unset (vault sync? `phantom config pull`)",
                    provider_name, env_name
                );
                errors.push(msg.clone());
                crate::diag::record("provider_skip", msg);
                continue;
            };
            tried_any = true;
            crate::diag::record(
                "provider_attempt",
                format!(
                    "[{}] trying with model={}",
                    provider_name,
                    entry_model.unwrap_or("(default)")
                ),
            );

            // Per-entry model from `provider:model` syntax wins over the
            // agent's `model` field, which wins over the provider's
            // `default_model`. Empty everything → bail with helpful error.
            let model = entry_model
                .map(|m| m.to_string())
                .filter(|m| !m.is_empty())
                .or_else(|| (!agent_cfg.model.is_empty()).then(|| agent_cfg.model.clone()))
                .or_else(|| provider.default_model.clone())
                .unwrap_or_default();
            if model.is_empty() {
                if !crate::diag::is_tui_active() {
                    eprintln!(
                        "  [provider {}] skipped: no model configured",
                        provider_name
                    );
                }
                let msg = format!("[{}] no model — entry isn't `provider:model` and provider has no default_model", provider_name);
                errors.push(msg.clone());
                last_err = msg;
                continue 'providers;
            }

            // DEMO-1 gap 1 Phase 4: shape URL + body + headers via the
            // LlmProvider trait (same dispatch as call_with_streaming but
            // with `stream: false`). AnthropicProvider/ClaudeCliProvider
            // emit native /v1/messages with cache_control + adaptive
            // thinking; OpenAI-compat impls stay on /v1/chat/completions.
            let parts_result = match resolver.resolve_by_name(provider_name) {
                Some(p) => p.build_stream_request(&BuildRequestOpts {
                    model: &model,
                    system: "",
                    messages,
                    tools: tool_defs,
                    base_url_override: provider.url.as_deref(),
                    stream: false,
                    max_tokens: crate::config::default_max_tokens(),
                }),
                None => {
                    let msg = format!(
                        "[{}] resolver returned None (no provider type matched)",
                        provider_name
                    );
                    errors.push(msg.clone());
                    last_err = msg;
                    continue 'providers;
                }
            };
            let parts = match parts_result {
                Ok(p) => p,
                Err(e) => {
                    let msg = format!("[{}] build_stream_request failed: {}", provider_name, e);
                    errors.push(msg.clone());
                    last_err = msg;
                    continue 'providers;
                }
            };
            let url = parts.url.clone();
            let body = parts.body.clone();
            // Detect native Anthropic Messages-API for auth header shape.
            let is_anthropic_messages = url.contains("/v1/messages");

            // --- per-provider retry loop: network errors, 429, 503 ---
            for attempt in 0..MAX_RETRIES {
                if attempt > 0 {
                    let delay = 1u64 << (attempt - 1); // 1s, 2s, 4s
                    tracing::info!(
                        provider = %provider_name,
                        attempt,
                        delay_secs = delay,
                        "Retrying after transient error"
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
                }

                let mut req = self.http_client.post(&url);
                req = if is_anthropic_messages {
                    req.header("x-api-key", &key)
                } else {
                    req.header("Authorization", format!("Bearer {}", key))
                };
                for (k, v) in &parts.headers {
                    req = req.header(*k, v.clone());
                }

                let resp = match req.json(&body).send().await {
                    Ok(r) => r,
                    Err(e) => {
                        last_err = e.to_string();
                        tracing::warn!(provider = %provider_name, attempt, "Network error: {}", e);
                        continue; // retry
                    }
                };

                let status = resp.status();
                let status_u16 = status.as_u16();

                if status.is_success() {
                    let json_val = resp.json::<Value>().await.map_err(|e| anyhow::anyhow!(e))?;
                    return Ok((json_val, model));
                }

                // Non-retriable client errors — fail this provider immediately.
                if is_non_retriable_status(status_u16) {
                    let text = resp.text().await.unwrap_or_default();
                    let msg = format!(
                        "[{}] HTTP {} {} ({})",
                        provider_name,
                        status_u16,
                        match status_u16 {
                            401 => "unauthorized — bad/expired key",
                            403 => "forbidden — key lacks model access",
                            404 => "not found — model id wrong?",
                            400 => "bad request — schema mismatch?",
                            _ => "client error",
                        },
                        text.chars().take(120).collect::<String>(),
                    );
                    errors.push(msg.clone());
                    last_err = msg;
                    tracing::warn!(
                        provider = %provider_name,
                        status = status_u16,
                        "Non-retriable error, skipping provider"
                    );
                    continue 'providers;
                }

                // Retriable: 429 (rate-limit) or 503 (overloaded).
                if matches!(status_u16, 429 | 503) {
                    let wait = resp
                        .headers()
                        .get("retry-after")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or(0)
                        .min(30);
                    let text = resp.text().await.unwrap_or_default();
                    last_err = format!(
                        "[{}] HTTP {} from {}: {}",
                        provider_name,
                        status_u16,
                        url,
                        crate::tools::floor_char_boundary(&text, 200)
                    );
                    tracing::warn!(
                        provider = %provider_name,
                        status = status_u16,
                        retry_after = wait,
                        "Rate-limited or overloaded, will retry"
                    );
                    if wait > 0 {
                        tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
                    }
                    continue; // retry
                }

                // Any other error — log and try next provider.
                let text = resp.text().await.unwrap_or_default();
                let msg = format!(
                    "[{}] HTTP {} {}",
                    provider_name,
                    status_u16,
                    text.chars().take(120).collect::<String>(),
                );
                errors.push(msg.clone());
                last_err = msg;
                tracing::warn!(provider = %provider_name, status = status_u16, "Provider failed, trying next");
                continue 'providers;
            }

            // Exhausted retries for this provider — capture if not already.
            tracing::warn!(provider = %provider_name, "Exhausted retries, trying next provider");
            if !last_err.is_empty() && !errors.contains(&last_err) {
                errors.push(last_err.clone());
            }
        }

        let breakdown = if errors.is_empty() {
            format!(
                "(no providers tried — check [agent.X].providers + [providers.*]; last_err: {})",
                last_err
            )
        } else {
            format!("\n  - {}", errors.join("\n  - "))
        };
        let hint = if !tried_any {
            "\n\nNo provider had a usable key. Run `phantom config pull` to refresh vault keys, \
             or set them manually in user env. Open /priority in TUI to reorder failover chain."
        } else {
            "\n\nFix any of the above and the chain recovers automatically. \
             /priority in TUI to reorder · /provider list to check key state."
        };
        Err(anyhow::anyhow!(
            "All providers failed. Tried {} provider(s):{}{}",
            errors.len(),
            breakdown,
            hint
        ))
    }
}

/// Resolve the chat-completions URL for a configured provider.
///
/// Priority:
///   1. Explicit `provider.url` (or `base_url` alias) wins outright — lets
///      users point at on-prem mirrors / proxies / Cerebras / DeepSeek etc.
///   2. Match `provider.type` against the well-known list below.
///   3. **Fall back to `provider_name` itself** — the section key in
///      agents.toml. This is what fixes the silent-misroute bug: a
///      `[providers.opencode]` block without `type =` used to land in the
///      `_ =>` arm and get routed to openrouter.ai, where its OPENCODE
///      key would 401 silently. Now `provider_name="opencode"` matches
///      the explicit arm and goes to the correct host.
///   4. As a last resort, route to openrouter (preserved behavior — but
///      now only triggers for genuinely-unknown provider keys, which is
///      rare and almost certainly a misconfiguration).
/// Split a provider-list entry into its (provider_name, optional_model) parts.
///
/// Bare `"groq"` → `("groq", None)`.
/// Compound `"opencode:claude-sonnet-4-6"` → `("opencode", Some("claude-sonnet-4-6"))`.
/// Edge: empty model after the colon (`"groq:"`) → `("groq", None)` so the
/// resolver falls through to `agent.model` / `provider.default_model` instead
/// of sending an empty model name to the provider.
pub fn parse_provider_entry(entry: &str) -> (&str, Option<&str>) {
    match entry.split_once(':') {
        Some((p, m)) if !m.is_empty() => (p, Some(m)),
        Some((p, _)) => (p, None),
        None => (entry, None),
    }
}

/// Map a provider name (and optional model id) to its [`PromptStyle`].
///
/// SPEC-14 §9.2 / G5: each upstream family expects a different prompt shape —
/// Claude wants `<task>…</task>` XML 標籤（XML tags），GPT-class models want
/// strict JSON-mode（JSON 模式）, Gemini wants `Q:/A:` 問答（Q&A）, small
/// local models want a bare instruction（簡單指令）. Sending an OpenAI-style
/// prompt to Claude measurably drops quality (~15-25%, §17 alt-3), so this is
/// the agent-layer branch that selects the right shape.
///
/// Pure function — provider/model strings in, enum out. The model id is matched
/// first (so a reasoning/o-series model on an OpenAI-compatible endpoint can
/// still pick `JsonMode`), then the provider name, then a sensible default.
///
/// 中文: 把 provider 名（與選用 model id）對應到對應的 prompt 結構風格。
pub fn prompt_style_for_provider(provider_name: &str, model: Option<&str>) -> PromptStyle {
    let p = provider_name.to_ascii_lowercase();
    let m = model.unwrap_or("").to_ascii_lowercase();

    // Anthropic / Claude → XML tags.
    if p.contains("anthropic") || p.contains("claude") || m.contains("claude") {
        return PromptStyle::XmlTags;
    }
    // Gemini → Q&A.
    if p.contains("gemini") || p.contains("google") || m.contains("gemini") {
        return PromptStyle::Qa;
    }
    // Llama / Mistral families (often served via groq / cerebras / together) →
    // few-shot.
    if m.contains("llama") || m.contains("mistral") || m.contains("mixtral") {
        return PromptStyle::FewShot;
    }
    // On-device / local small models → bare simple instruction.
    if p.contains("ollama") || p.contains("local") || p.contains("llamafile") {
        return PromptStyle::Simple;
    }
    // OpenAI / GPT / o-series / openai-compatible default → JSON mode.
    if p.contains("openai") || p.contains("gpt") || m.contains("gpt") || m.starts_with('o') {
        return PromptStyle::JsonMode;
    }
    // Unknown provider → JSON mode is the safest broadly-supported default.
    PromptStyle::JsonMode
}

/// Apply provider-specific structural framing to the assembled `system` prompt
/// according to its [`PromptStyle`].
///
/// This is the SMALLEST correct slice of SPEC-14 §9.2 / G5 prompt shaping: it
/// wraps / prefixes the already-built system text so each upstream family gets
/// a prompt in the shape it scores best on. The user-turn shaping and full
/// `SystemPlacement`（系統提示位置）adapter routing are intentionally left to a
/// follow-up (see blockers) — those touch `LlmRequest` / per-provider adapters,
/// which are out of scope for this slice.
///
/// Pure function — system text in, framed text out. Empty input stays empty.
///
/// 中文: 依 prompt 風格替已組好的 system 文字加上對應的結構框（XML 包裹 / JSON
/// 指示 / 問答前綴 等），是本切片最小可編譯的 prompt 塑形。
pub fn frame_system_for_style(system: &str, style: PromptStyle) -> String {
    if system.is_empty() {
        return String::new();
    }
    match style {
        // Claude — wrap the whole instruction block in an XML tag the model
        // is trained to attend to.
        PromptStyle::XmlTags => format!("<instructions>\n{system}\n</instructions>"),
        // GPT — nudge toward strict-JSON discipline up front.
        PromptStyle::JsonMode => {
            format!("{system}\n\nRespond with valid JSON only when a structured answer is requested.")
        }
        // Gemini — Q&A framing.
        PromptStyle::Qa => format!("System guidance —\nQ: How should you behave?\nA: {system}"),
        // Llama / Mistral — few-shot lead-in (the example slots are filled by
        // history; here we just mark the section so the model expects examples).
        PromptStyle::FewShot => format!("{system}\n\n# Examples follow below."),
        // On-device — leave the instruction bare, no decoration.
        PromptStyle::Simple => system.to_string(),
    }
}

/// Fold a (framed) system prompt into a user-turn content value for providers
/// whose [`SystemPlacement`] is `EmbedInUserTurn` (on-device / local models with
/// no `system` role).
///
/// SPEC-14 §7.1: such models cannot take a `messages[0].role = "system"` entry,
/// so the system instructions must ride inside the first user turn. This helper
/// preserves multimodal content: a plain-string user turn is concatenated as
/// `"{system}\n\n{user}"`; an array (multimodal) user turn gets a leading text
/// part inserted before the existing parts so images / audio stay intact.
///
/// Pure function — system text + user content in, combined content out. Empty
/// system text returns the user content unchanged.
///
/// 中文: 把 system 提示折進使用者 turn（給沒有 system role 的本機 model）；
/// 純文字直接前綴，多模態陣列則在最前面插一段 text part，附件原樣保留。
pub fn embed_system_into_user_content(system: &str, user_content: Value) -> Value {
    if system.is_empty() {
        return user_content;
    }
    match user_content {
        Value::String(user_text) => {
            Value::String(format!("{system}\n\n{user_text}"))
        }
        Value::Array(mut parts) => {
            parts.insert(0, serde_json::json!({"type": "text", "text": system}));
            Value::Array(parts)
        }
        // Any other shape (unexpected) — wrap as a 2-part array so nothing is lost.
        other => Value::Array(vec![
            serde_json::json!({"type": "text", "text": system}),
            other,
        ]),
    }
}

/// Compute the order in which providers are attempted when an agent runs.
///
/// Resolution rule (highest priority first, duplicates removed by first occurrence):
///   1. `agent.providers` (Vec) — explicit user-controlled priority list
///   2. `agent.provider` (String) — legacy single primary
///   3. every other configured provider, alphabetically (deterministic order)
///
/// Steps 2 and 3 always run, so a name missing from the priority list still
/// gets attempted at the end. This means a user can shorten `providers` to
/// just "things I prefer" without losing access to the rest as last-resort
/// fallbacks. Empty strings are dropped.
///
/// Pure function — no I/O, no config lookups beyond the provided iterator
/// of available provider names. Easy to unit-test.
pub fn resolve_provider_order<'a>(
    agent_cfg: &AgentEntry,
    available: impl Iterator<Item = &'a str>,
) -> Vec<String> {
    let mut order: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    // Dedup by PROVIDER NAME (the part before `:`), NOT the full
    // `provider:model` entry. Otherwise `gemini:gemini-2.0-flash` (from the
    // priority list) and a bare `gemini` (from agent.provider or the available
    // [providers.*] keys) are different strings → BOTH get pushed, so gemini is
    // dispatched twice: once with its per-entry model and once bare with
    // agent.model — the latter pairs the wrong model with the provider (e.g. a
    // groq model sent to gemini → HTTP 404). Keep the first (full) entry seen.
    let push =
        |list: &mut Vec<String>, seen: &mut std::collections::HashSet<String>, entry: &str| {
            if entry.is_empty() {
                return;
            }
            let provider_name = entry.split(':').next().unwrap_or(entry).trim();
            // Skip a malformed leading-colon entry (`:model`) — no provider name
            // to dispatch to, and it would otherwise dedup under an empty key.
            if provider_name.is_empty() {
                return;
            }
            if seen.insert(provider_name.to_string()) {
                list.push(entry.to_string());
            }
        };
    // 1. explicit priority list
    if let Some(pri) = agent_cfg.providers.as_ref() {
        for n in pri {
            push(&mut order, &mut seen, n);
        }
    }
    // 2. legacy single primary
    push(&mut order, &mut seen, &agent_cfg.provider);
    // 3. remaining configured providers, alphabetical
    let mut others: Vec<&str> = available.filter(|n| !seen.contains(*n)).collect();
    others.sort();
    for n in others {
        push(&mut order, &mut seen, n);
    }
    order
}

// DEMO-1 gap 1 Phase 4: the `provider_url` string-switch is gone.
// URL/header/body shaping for both `call_with_fallback` and
// `call_with_streaming` now goes through `LlmProvider::build_stream_request`
// via `DefaultProviderResolver`. Equivalent URL-shape coverage lives in
// `core/src/providers/resolver.rs` tests (anthropic_url_matches_legacy_default
// + openai_url_matches_legacy_default + gemini_url_matches_legacy_default
// + openai_compat_honours_explicit_v1) and in
// `core/tests/agent_trait_migration.rs`.

/// Conservative per-image token estimate. Real cost varies by provider
/// (Anthropic ~85 low / ~1500+ high detail; OpenAI ~85 low / ~258+ high), and
/// vision tokens are not directly comparable to text tokens. We use a flat
/// 1000-token overhead per attached image so image-heavy prompts trigger
/// compaction before they overflow the model's context window.
const IMAGE_TOKEN_COST: usize = 1000;

fn estimate_message_tokens(msg: &Value) -> usize {
    let content = &msg["content"];
    // Multipart array (post-parse): count text chars + flat per-image cost.
    if let Some(arr) = content.as_array() {
        let mut total = 0usize;
        for part in arr {
            match part["type"].as_str() {
                Some("image_url") | Some("image") => total += IMAGE_TOKEN_COST,
                _ => {
                    let txt = part["text"].as_str().unwrap_or("");
                    total += txt.len() / 4;
                }
            }
        }
        return total.max(1);
    }
    // Plain string content: detect raw `<phantom-image .../>` sentinels and
    // bill each one at the flat image cost (independent of base64 length, which
    // would otherwise dominate `len()/4` in misleading ways).
    let s = content.as_str().unwrap_or("");
    let images = s.matches("<phantom-image ").count();
    let text_only_len = if images == 0 {
        s.len()
    } else {
        // Approximate text length by stripping each sentinel span.
        let mut remaining = s;
        let mut acc = 0usize;
        while let Some(i) = remaining.find("<phantom-image ") {
            acc += i;
            remaining = &remaining[i..];
            if let Some(j) = remaining.find("/>") {
                remaining = &remaining[j + 2..];
            } else {
                break;
            }
        }
        acc + remaining.len()
    };
    ((text_only_len / 4) + images * IMAGE_TOKEN_COST).max(1)
}

fn estimate_tokens(messages: &[Value]) -> usize {
    messages.iter().map(estimate_message_tokens).sum()
}

fn compact_if_needed(messages: &mut Vec<Value>, budget: usize) {
    if estimate_tokens(messages) <= budget {
        return;
    }

    // A4/T94 — under pressure, the recalled-memory block is the cheapest
    // thing to drop because (a) it's recoverable next turn from FTS5 and
    // (b) it's leaf-knowledge, not load-bearing for the conversation
    // semantics. Strip the `[memory]` section out of every system message
    // BEFORE we start dropping turns, then re-check the budget.
    strip_memory_block(messages);
    if estimate_tokens(messages) <= budget {
        return;
    }

    let system_msgs: Vec<Value> = messages
        .iter()
        .filter(|m| m["role"].as_str() == Some("system"))
        .cloned()
        .collect();
    let conv_msgs: Vec<Value> = messages
        .iter()
        .filter(|m| m["role"].as_str() != Some("system"))
        .cloned()
        .collect();

    let system_tok = estimate_tokens(&system_msgs) + 200;
    let conv_budget = budget.saturating_sub(system_tok);

    let mut kept = conv_msgs.clone();
    while estimate_tokens(&kept) > conv_budget && kept.len() > 2 {
        kept.remove(0);
    }
    kept = strip_orphans(strip_leading_tool(kept));

    let dropped = conv_msgs.len().saturating_sub(kept.len());
    let summary = serde_json::json!({
        "role": "system",
        "content": format!(
            "[Context compacted: {} earlier messages were dropped to fit within context limits. \
             Continue the task based on the remaining conversation history.]",
            dropped
        )
    });

    *messages = system_msgs.clone();
    messages.push(summary);
    messages.extend(kept.clone());

    tracing::info!(
        "Context compacted: dropped {} messages, {} remaining",
        dropped,
        messages.len()
    );

    // Emergency pass
    if estimate_tokens(messages) > budget {
        let conv2: Vec<Value> = messages
            .iter()
            .filter(|m| m["role"].as_str() != Some("system"))
            .cloned()
            .collect();
        let mut kept2 = conv2.clone();
        while estimate_tokens(&kept2) > conv_budget && kept2.len() > 2 {
            kept2.remove(0);
        }
        if kept2.len() > 4 {
            kept2 = kept2[kept2.len() - 4..].to_vec();
        }
        kept2 = strip_orphans(strip_leading_tool(kept2));
        let emergency = serde_json::json!({
            "role": "system",
            "content": "[Emergency compaction: context was still too large; older messages dropped.]"
        });
        *messages = system_msgs;
        messages.push(emergency);
        messages.extend(kept2);
        tracing::warn!(
            "Emergency compaction applied, {} messages remaining",
            messages.len()
        );
    }
}

/// Drop the `[memory]` block (header + bullet lines beneath it) from every
/// system message. The block is recognised by the literal header string and
/// extends until either a blank line, a different bracketed header, or the
/// end of the message — matching the format written by
/// `run_inner` when a HermesRuntime is attached. Messages without a memory
/// block are left exactly as they were.
///
/// Returns nothing; mutates in place. Cheap on the common path because the
/// header probe is the first byte-level check.
fn strip_memory_block(messages: &mut Vec<Value>) {
    // Plain literal — kept identical to MEMORY_CONTEXT_HEADER. We don't
    // import the const here because this function must compile in the
    // default (non-hermes) build too — `compact_if_needed` is unconditional.
    const HEADER: &str = "[memory]";
    for msg in messages.iter_mut() {
        if msg["role"].as_str() != Some("system") {
            continue;
        }
        let Some(content) = msg["content"].as_str() else {
            continue;
        };
        let Some(start) = content.find(HEADER) else {
            continue;
        };
        // Walk forward from `start` until we find a blank line OR a new
        // `\n[` bracketed section that isn't `[memory]`. That marks the
        // end of our block; everything between gets snipped.
        let after = &content[start + HEADER.len()..];
        let mut end_offset = after.len();
        let bytes = after.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'\n' {
                // blank line ends the block
                if i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                    end_offset = i; // include trailing single \n in trimmed prefix below
                    break;
                }
                // a new bracketed header starts the next section
                if i + 1 < bytes.len() && bytes[i + 1] == b'[' {
                    end_offset = i;
                    break;
                }
            }
            i += 1;
        }
        let mut new_content = String::with_capacity(content.len());
        new_content.push_str(&content[..start]);
        // Drop trailing whitespace/newlines we left behind so the prompt
        // doesn't accumulate orphan blank lines on every compaction pass.
        let trimmed_prefix = new_content
            .trim_end_matches(|c: char| c == ' ' || c == '\n')
            .to_string();
        new_content = trimmed_prefix;
        let tail = &after[end_offset..];
        if !tail.is_empty() {
            new_content.push_str("\n\n");
            new_content.push_str(tail.trim_start_matches('\n'));
        }
        msg["content"] = Value::String(new_content);
    }
}

fn strip_leading_tool(mut msgs: Vec<Value>) -> Vec<Value> {
    while msgs
        .first()
        .map(|m| m["role"].as_str() == Some("tool"))
        .unwrap_or(false)
    {
        msgs.remove(0);
    }
    msgs
}

fn strip_orphans(mut msgs: Vec<Value>) -> Vec<Value> {
    loop {
        let mut changed = false;
        let mut i = 0;
        while i < msgs.len() {
            let is_tool_call = msgs[i]["role"].as_str() == Some("assistant")
                && !msgs[i]["tool_calls"].is_null()
                && msgs[i]["tool_calls"]
                    .as_array()
                    .map(|a| !a.is_empty())
                    .unwrap_or(false);
            if is_tool_call {
                let has_result = msgs
                    .get(i + 1)
                    .map(|m| m["role"].as_str() == Some("tool"))
                    .unwrap_or(false);
                if !has_result {
                    msgs.remove(i);
                    changed = true;
                    continue;
                }
            }
            i += 1;
        }
        if !changed {
            break;
        }
    }
    msgs
}

fn output_unchanged(prev: &str, current: &str) -> bool {
    if prev.is_empty() {
        return false;
    }
    let max_len = prev.len().max(current.len());
    if max_len == 0 {
        return true;
    }
    let diff = prev
        .chars()
        .zip(current.chars())
        .filter(|(a, b)| a != b)
        .count()
        + prev.len().abs_diff(current.len());
    diff * 10 < max_len
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── detect_truncation_notice ──────────────────────────────────────────

    // ── provider URL/body shape: covered in `providers::resolver::tests`
    //   (anthropic_url_matches_legacy_default + openai_url_matches_legacy_default
    //    + gemini_url_matches_legacy_default + openai_compat_honours_explicit_v1)
    //   and in `core/tests/agent_trait_migration.rs`. The pre-Phase-4 in-file
    //   `provider_url_*` tests against the deleted `provider_url` string-switch
    //   are gone with the function itself. The `fn p` helper that built
    //   `ProviderEntry`s for those tests went with them.

    fn agent(provider: &str, providers: Option<Vec<&str>>) -> AgentEntry {
        AgentEntry {
            provider: provider.to_string(),
            providers: providers.map(|v| v.into_iter().map(String::from).collect()),
            model: String::new(),
            tools: Vec::new(),
            instructions: String::new(),
        }
    }

    // ── PromptStyle wiring (SPEC-14 §9.2 / G5, T-PROV-04) ─────────────────

    #[test]
    fn prompt_style_maps_each_provider_family() {
        // Anthropic / Claude → XML tags.
        assert_eq!(
            prompt_style_for_provider("anthropic", None),
            PromptStyle::XmlTags
        );
        assert_eq!(
            prompt_style_for_provider("openai", Some("claude-3-5-sonnet")),
            PromptStyle::XmlTags
        );
        // Gemini → Q&A.
        assert_eq!(prompt_style_for_provider("gemini", None), PromptStyle::Qa);
        // Llama / Mistral model id → few-shot (even on a groq endpoint).
        assert_eq!(
            prompt_style_for_provider("groq", Some("llama-3.1-8b-instant")),
            PromptStyle::FewShot
        );
        // Local / on-device → simple.
        assert_eq!(
            prompt_style_for_provider("ollama", None),
            PromptStyle::Simple
        );
        // OpenAI / GPT → JSON mode.
        assert_eq!(
            prompt_style_for_provider("openai", Some("gpt-5.5")),
            PromptStyle::JsonMode
        );
        // Unknown provider → JSON mode default.
        assert_eq!(
            prompt_style_for_provider("some-new-host", None),
            PromptStyle::JsonMode
        );
    }

    #[test]
    fn frame_system_shapes_per_style() {
        let s = "Be helpful.";
        // XML tags wrap.
        let xml = frame_system_for_style(s, PromptStyle::XmlTags);
        assert!(xml.starts_with("<instructions>") && xml.ends_with("</instructions>"));
        assert!(xml.contains(s));
        // JSON mode appends the JSON nudge but preserves the original text.
        let js = frame_system_for_style(s, PromptStyle::JsonMode);
        assert!(js.starts_with(s) && js.contains("valid JSON"));
        // Q&A prefixes.
        let qa = frame_system_for_style(s, PromptStyle::Qa);
        assert!(qa.contains("Q:") && qa.contains(s));
        // FewShot marks an examples section.
        let fs = frame_system_for_style(s, PromptStyle::FewShot);
        assert!(fs.starts_with(s) && fs.contains("Examples"));
        // Simple leaves the text untouched.
        assert_eq!(frame_system_for_style(s, PromptStyle::Simple), s);
        // Empty stays empty regardless of style.
        assert_eq!(frame_system_for_style("", PromptStyle::XmlTags), "");
    }

    // ── SystemPlacement wiring (SPEC-14 §7.1, T-PROV-05) ─────────────────────

    #[test]
    fn embed_system_into_user_content_branches() {
        let sys = "Be terse.";

        // Plain-string user turn → system text prepended, two newlines between.
        let plain = embed_system_into_user_content(sys, Value::String("hi there".into()));
        assert_eq!(plain, Value::String("Be terse.\n\nhi there".into()));

        // Multimodal array user turn → leading text part inserted, parts kept.
        let arr = Value::Array(vec![
            json!({"type": "image_url", "image_url": {"url": "data:image/png;base64,AAA"}}),
        ]);
        let combined = embed_system_into_user_content(sys, arr);
        let parts = combined.as_array().expect("array preserved");
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0], json!({"type": "text", "text": "Be terse."}));
        assert_eq!(parts[1]["type"], "image_url");

        // Empty system text → user content returned unchanged.
        let unchanged = embed_system_into_user_content("", Value::String("hi".into()));
        assert_eq!(unchanged, Value::String("hi".into()));
    }

    #[test]
    fn priority_legacy_provider_only_keeps_old_behavior() {
        // No `providers` list → start with `provider`, then alphabetical of others.
        let cfg = agent("groq", None);
        let order = resolve_provider_order(&cfg, ["opencode", "groq", "cerebras"].into_iter());
        assert_eq!(order, vec!["groq", "cerebras", "opencode"]);
    }

    #[test]
    fn priority_explicit_list_takes_precedence() {
        // Even when `provider` would normally come first, the list wins.
        let cfg = agent("opencode", Some(vec!["groq", "cerebras"]));
        let order = resolve_provider_order(
            &cfg,
            ["opencode", "groq", "cerebras", "anthropic"].into_iter(),
        );
        // groq, cerebras (from list); opencode (from provider field);
        // anthropic (alphabetical of unlisted).
        assert_eq!(order, vec!["groq", "cerebras", "opencode", "anthropic"]);
    }

    #[test]
    fn provider_model_entries_dedup_by_provider_name() {
        // Regression: a `gemini:gemini-2.0-flash` entry in the list must NOT be
        // duplicated by a bare `gemini` (from agent.provider or the available
        // [providers.*] keys). Before the fix, dedup keyed on the full entry
        // string, so gemini was dispatched twice — the 2nd time bare, picking up
        // agent.model (a groq model) → HTTP 404. Mirrors node-a's agents.toml.
        let cfg = agent(
            "groq",
            Some(vec![
                "groq:llama-3.1-8b-instant",
                "gemini:gemini-2.0-flash",
                "mlx-local",
            ]),
        );
        let order = resolve_provider_order(&cfg, ["gemini", "groq", "mlx-local"].into_iter());
        // Each provider exactly once, per-entry model preserved, no bare dup.
        assert_eq!(
            order,
            vec![
                "groq:llama-3.1-8b-instant",
                "gemini:gemini-2.0-flash",
                "mlx-local"
            ]
        );
        assert!(
            !order.iter().any(|e| e == "gemini" || e == "groq"),
            "no bare duplicate of an already-listed provider: {order:?}"
        );
    }

    #[test]
    fn priority_dedupes_overlapping_entries() {
        // Same name appearing in providers list AND as legacy provider — only once.
        let cfg = agent("groq", Some(vec!["groq", "cerebras"]));
        let order = resolve_provider_order(&cfg, ["groq", "cerebras", "opencode"].into_iter());
        assert_eq!(order, vec!["groq", "cerebras", "opencode"]);
        assert_eq!(order.len(), 3);
    }

    #[test]
    fn priority_skips_empty_names() {
        // Defaulted AgentEntry has provider = "" — must not appear as a phantom entry.
        let cfg = agent("", Some(vec!["groq", "", "cerebras"]));
        let order = resolve_provider_order(&cfg, ["groq", "cerebras"].into_iter());
        assert_eq!(order, vec!["groq", "cerebras"]);
    }

    #[test]
    fn parse_provider_entry_bare_and_compound() {
        assert_eq!(parse_provider_entry("groq"), ("groq", None));
        assert_eq!(
            parse_provider_entry("opencode:claude-sonnet-4-6"),
            ("opencode", Some("claude-sonnet-4-6"))
        );
        // Empty after colon falls back to None (resolver uses default).
        assert_eq!(parse_provider_entry("groq:"), ("groq", None));
        // Multi-colon: only first split point is used; the rest is the model id.
        // (Some model ids contain colons in some catalogs, e.g. "qwen3:8b".)
        assert_eq!(
            parse_provider_entry("local-ollama:qwen3:8b"),
            ("local-ollama", Some("qwen3:8b"))
        );
    }

    #[test]
    fn priority_unknown_names_in_list_still_get_added() {
        // If user lists a provider that isn't in [providers.*], it still
        // appears in the order — call site checks for the entry and skips
        // missing ones (so a typo doesn't silently mask the rest of the list).
        let cfg = agent("anthropic", Some(vec!["typo-name", "groq"]));
        let order = resolve_provider_order(&cfg, ["anthropic", "groq"].into_iter());
        assert_eq!(order, vec!["typo-name", "groq", "anthropic"]);
    }

    // ── detect_truncation_notice ──────────────────────────────────────────

    #[test]
    fn detects_anthropic_max_tokens_stop_reason() {
        // The signal frame Anthropic sends just before the run wraps up.
        let frame = json!({
            "type": "message_delta",
            "delta": { "stop_reason": "max_tokens", "stop_sequence": null },
            "usage": { "output_tokens": 8192 }
        });
        let notice = detect_truncation_notice(&frame).expect("should detect");
        assert!(notice.contains("max_tokens"), "got: {notice}");
        assert!(notice.contains("PHANTOM_MAX_TOKENS"), "got: {notice}");
    }

    #[test]
    fn detects_openai_finish_reason_length() {
        // What Groq, Cerebras, OpenAI all send on the final chunk when the
        // cap hit.
        let frame = json!({
            "id": "chatcmpl-abc",
            "choices": [{ "delta": {}, "finish_reason": "length", "index": 0 }],
        });
        let notice = detect_truncation_notice(&frame).expect("should detect");
        assert!(notice.contains("max_tokens"), "got: {notice}");
    }

    #[test]
    fn ignores_normal_completion_signals() {
        // Anthropic end_turn is the happy path — must NOT emit a notice.
        let normal_anthropic = json!({
            "type": "message_delta",
            "delta": { "stop_reason": "end_turn", "stop_sequence": null }
        });
        assert!(detect_truncation_notice(&normal_anthropic).is_none());

        // OpenAI finish_reason="stop" is the happy path.
        let normal_openai = json!({
            "choices": [{ "delta": {}, "finish_reason": "stop" }]
        });
        assert!(detect_truncation_notice(&normal_openai).is_none());

        // tool_use is also not a truncation.
        let tool_anthropic = json!({
            "type": "message_delta",
            "delta": { "stop_reason": "tool_use" }
        });
        assert!(detect_truncation_notice(&tool_anthropic).is_none());
    }

    #[test]
    fn ignores_intermediate_streaming_chunks() {
        // Mid-stream content_block_delta has no stop_reason at all.
        let mid = json!({
            "type": "content_block_delta",
            "delta": { "type": "text_delta", "text": "hello" }
        });
        assert!(detect_truncation_notice(&mid).is_none());

        // OpenAI mid-stream chunk has finish_reason: null.
        let mid_oai = json!({
            "choices": [{ "delta": { "content": "world" }, "finish_reason": null }]
        });
        assert!(detect_truncation_notice(&mid_oai).is_none());
    }

    #[test]
    fn agent_estimate_tokens_accounts_for_image_sentinels() {
        // 100 chars of text + one phantom-image sentinel (raw string form, as
        // it would appear before `prompt_to_content_value` parses it).
        let text_100 = "x".repeat(100);
        let sentinel = r#"<phantom-image mime="image/png" data="AAAA"/>"#;
        let msg = json!({
            "role": "user",
            "content": format!("{} {}", text_100, sentinel),
        });
        let tokens = estimate_tokens(&[msg]);
        // Text alone would be ~25 tokens (100/4). With one image, must be at
        // least IMAGE_TOKEN_COST (1000).
        assert!(
            tokens >= 1000,
            "expected >=1000 tokens for image msg, got {}",
            tokens
        );

        // Sanity: a plain 100-char text message stays cheap.
        let plain = json!({ "role": "user", "content": text_100 });
        assert!(estimate_tokens(&[plain]) < 100);
    }

    #[test]
    fn agent_estimate_tokens_handles_multipart_array() {
        // Post-parse multipart content (array form).
        let msg = json!({
            "role": "user",
            "content": [
                {"type": "text", "text": "describe"},
                {"type": "image_url", "image_url": {"url": "data:image/png;base64,AAAA"}},
            ],
        });
        assert!(estimate_tokens(&[msg]) >= 1000);
    }

    // ── prompt-override loader (CONTRIBUTOR-FUNNEL §4.1-b) ──────────────

    #[test]
    fn load_prompt_override_returns_none_when_no_override_file() {
        // No file at ~/.phantom-mesh/extensions/prompts/<test-only-name>.md
        // because tests run in real $HOME. Use a unique sentinel name that
        // a real user is extremely unlikely to have customised.
        let sentinel = "agent_runtime_test_zzzz_no_override";
        assert!(load_prompt_override(sentinel).is_none());
    }

    /// Test the override-merge logic directly (no file I/O — exercises the
    /// behaviour described in run_inner's prompt-assembly comments without
    /// the production path's $HOME-dependent file read).
    #[test]
    fn override_prepend_keeps_built_in_below() {
        // Simulated version of the merge logic in run_inner:
        const REPLACE_MARKER: &str = "<!-- replace -->";
        let built_in = "You are master.\nUse tools.";
        let override_text = "Vim style. No emojis.";
        let merged = if override_text.trim_start().starts_with(REPLACE_MARKER) {
            override_text
                .trim_start()
                .trim_start_matches(REPLACE_MARKER)
                .trim_start_matches('\n')
                .to_string()
        } else {
            format!(
                "## User customisation\n{}\n\n## Agent instructions\n{}",
                override_text.trim(),
                built_in
            )
        };
        assert!(merged.starts_with("## User customisation"));
        assert!(merged.contains("Vim style. No emojis."));
        assert!(merged.contains("## Agent instructions"));
        assert!(merged.contains("You are master."));
        assert!(merged.contains("Use tools."));
    }

    #[test]
    fn override_replace_marker_drops_built_in() {
        const REPLACE_MARKER: &str = "<!-- replace -->";
        let built_in = "You are master.\nUse tools.";
        let override_text = "<!-- replace -->\nI am a haiku poet.\nThree lines only.";
        let merged = if override_text.trim_start().starts_with(REPLACE_MARKER) {
            override_text
                .trim_start()
                .trim_start_matches(REPLACE_MARKER)
                .trim_start_matches('\n')
                .to_string()
        } else {
            format!(
                "## User customisation\n{}\n\n## Agent instructions\n{}",
                override_text.trim(),
                built_in
            )
        };
        assert!(
            !merged.contains("You are master"),
            "REPLACE marker must drop built-in"
        );
        assert!(merged.starts_with("I am a haiku poet"));
        assert!(merged.contains("Three lines only."));
    }

    // ── A4/T94: strip_memory_block (compaction-time first-to-drop) ──────────

    #[test]
    fn strip_memory_block_removes_memory_section_only() {
        let mut msgs = vec![json!({
            "role": "system",
            "content": "Header.\n\nCRITICAL RULES:\n- rule 1\n- rule 2\n\n[memory]\n- past 1\n- past 2\n\nUser context here."
        })];
        strip_memory_block(&mut msgs);
        let s = msgs[0]["content"].as_str().unwrap();
        assert!(!s.contains("[memory]"), "memory header survived: {s}");
        assert!(!s.contains("past 1"), "memory body survived: {s}");
        assert!(
            s.contains("CRITICAL RULES"),
            "rules accidentally dropped: {s}"
        );
        assert!(
            s.contains("User context here."),
            "trailing content dropped: {s}"
        );
    }

    #[test]
    fn strip_memory_block_no_op_when_no_memory_present() {
        let original = "Header only. No bracketed sections.";
        let mut msgs = vec![json!({"role": "system", "content": original})];
        strip_memory_block(&mut msgs);
        assert_eq!(msgs[0]["content"].as_str().unwrap(), original);
    }

    #[test]
    fn strip_memory_block_preserves_non_system_messages() {
        let mut msgs = vec![
            json!({"role": "user", "content": "[memory] not a system block — leave alone"}),
            json!({"role": "assistant", "content": "ok"}),
        ];
        let snapshot = msgs.clone();
        strip_memory_block(&mut msgs);
        assert_eq!(msgs, snapshot, "non-system messages must be untouched");
    }

    /// V1 ship-blocker: when provider A returns HTTP 401 (unauthorized /
    /// bad key), the agent MUST skip that provider and fall back to the
    /// next one in the priority list. This is the primary regression guard
    /// for `streaming_with_retry` returning Err on non-retriable status
    /// codes, which the outer `'providers` loop catches and `continue`s.
    ///
    /// We test two things:
    /// 1. `is_non_retriable_status` classifies 401 (and peers) correctly.
    /// 2. `streaming_with_retry` returns Err for 401 (integration test
    ///    with local TCP mock).
    #[test]
    fn provider_failover_on_401_falls_back_to_next() {
        // ── Part 1: pure classification function ────────────────────────────
        // These status codes MUST be classified as non-retriable (skip provider).
        for code in [400u16, 401, 403, 404, 422] {
            assert!(
                is_non_retriable_status(code),
                "HTTP {} must be classified as non-retriable",
                code
            );
        }

        // These MUST be retriable (retry same provider).
        for code in [429u16, 500, 502, 503, 504] {
            assert!(
                !is_non_retriable_status(code),
                "HTTP {} must NOT be classified as non-retriable",
                code
            );
        }

        // 200/201 are success — not in the non-retriable bucket.
        for code in [200u16, 201, 204] {
            assert!(
                !is_non_retriable_status(code),
                "HTTP {} is success, not non-retriable",
                code
            );
        }
    }

    /// Integration test: `streaming_with_retry` against a local mock server
    /// that returns 401. Confirms the function returns `Err` so the caller
    /// (`'providers` loop) can `continue` to the next provider.
    #[tokio::test]
    async fn streaming_with_retry_returns_err_on_401() {
        use tokio::io::AsyncWriteExt;
        use tokio::net::TcpListener;

        // Spin up a tiny HTTP server that always responds 401.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server_handle = tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                // Read until we see \r\n\r\n (end of HTTP headers).
                let mut buf = vec![0u8; 4096];
                let _ = tokio::io::AsyncReadExt::read(&mut socket, &mut buf).await;

                let response = "HTTP/1.1 401 Unauthorized\r\n\
                    Content-Length: 15\r\n\
                    Content-Type: text/plain\r\n\r\n\
                    {\"error\":\"bad\"}";
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            }
        });

        // Build a minimal AgentRuntime just to call streaming_with_retry.
        let config = Arc::new(AgentsConfig::default());
        let runtime = AgentRuntime {
            config,
            http_client: Arc::new(reqwest::Client::new()),
            interrupt: None,
            #[cfg(all(
                feature = "experimental-hermes-curator",
                feature = "experimental-hermes-memory",
                feature = "experimental-hermes-tools",
            ))]
            hermes: None,
            resolver_override: None,
        };

        let url = format!("http://127.0.0.1:{}/v1/chat/completions", addr.port());
        let body = json!({"model": "test", "messages": []});
        let result = runtime
            .streaming_with_retry(&url, "fake-key", &body, "test-provider", &[])
            .await;

        // streaming_with_retry MUST return Err on 401 so the outer loop
        // can skip to the next provider.
        assert!(
            result.is_err(),
            "streaming_with_retry must return Err on 401, got Ok"
        );
        let err_msg = result.unwrap_err();
        assert!(
            err_msg.contains("401"),
            "error message must mention 401, got: {}",
            err_msg
        );

        server_handle.abort();
    }
}
