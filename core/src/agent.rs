use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use futures::future::join_all;
use futures::StreamExt;
use serde_json::Value;

use crate::config::{AgentsConfig, AgentEntry, ProviderEntry};
use crate::cost::CostTracker;
use crate::providers::traits::ChatMessage;
use crate::tasks::SessionWriter;

const STALL_THRESHOLD: usize = 2;

/// Maximum per-provider retry attempts for transient errors (network / 429 / 503).
const MAX_RETRIES: u32 = 3;

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
    Token { content: String },
    /// Reasoning / chain-of-thought trace from models that expose it
    /// (Anthropic extended thinking, OpenAI o1, opencode reasoning models).
    /// Captured separately from Token so the UI can render it dimmed/collapsed
    /// above the actual answer.
    Thinking { content: String },
    ToolStart { name: String, args_preview: String },
    ToolDone { name: String, output_preview: String },
    Done { output: String, cost_usd: f64, elapsed_secs: f64 },
    /// Non-fatal heads-up surfaced inline (e.g., the provider truncated the
    /// reply because we hit the max_tokens cap). The stream continues
    /// normally afterward — `Done` still fires. The UI renders this as a
    /// red warning so the user knows the answer is incomplete instead of
    /// thinking phantom hung.
    Notice { message: String },
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
        }
    }
}

pub struct AgentResult {
    pub output: String,
    pub tool_calls_made: Vec<Value>,
    pub turns: u32,
    pub cost_delta_usd: f64,
    pub elapsed_secs: f64,
}

impl AgentRuntime {
    pub fn new(config: AgentsConfig) -> Self {
        Self {
            config: Arc::new(config),
            http_client: Arc::new(build_llm_http_client()),
            interrupt: None,
        }
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
        self.interrupt.as_ref().map(|h| h.is_cancelled()).unwrap_or(false)
    }

    pub async fn run(
        &self,
        agent_name: &str,
        prompt: &str,
        history: &[ChatMessage],
        extra_context: Option<&str>,
    ) -> anyhow::Result<AgentResult> {
        self.run_inner(agent_name, prompt, history, extra_context, None, None, None, None).await
    }

    pub async fn run_tracked(
        &self,
        agent_name: &str,
        prompt: &str,
        history: &[ChatMessage],
        extra_context: Option<&str>,
        cost_tracker: &CostTracker,
    ) -> anyhow::Result<AgentResult> {
        self.run_inner(agent_name, prompt, history, extra_context, Some(cost_tracker), None, None, None).await
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
            if let Ok(mut f) = on_event_cell.lock() { f(ev); }
        };
        let result = self.run_inner(
            agent_name, prompt, history, extra_context, Some(cost_tracker), Some(&on_event_fn), None, None,
        ).await?;
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
            if let Ok(mut f) = on_event_cell.lock() { f(ev); }
        };
        let gate_box: Box<ToolGate> = Box::new(gate);
        let result = self.run_inner(
            agent_name, prompt, history, extra_context,
            Some(cost_tracker), Some(&on_event_fn), None, Some(&*gate_box),
        ).await?;
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
        let now_ms = || std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        let agent_cfg = self.config.agent.get(agent_name)
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
            let deny: Vec<&str>  = cfg.deny.iter().map(String::as_str).collect();
            let ask: Vec<&str>   = cfg.ask.iter().map(String::as_str).collect();
            let allow: Vec<&str> = cfg.allow.iter().map(String::as_str).collect();
            crate::permission::Engine::from_lists(&deny, &ask, &allow)
                .map(|e| e.statically_denied_tools())
                .unwrap_or_default()
        };

        let mut tool_defs: Vec<Value> = agent_cfg.tools.iter()
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
        if let Some(extra) = extra_context {
            if !extra.is_empty() {
                system.push_str("\n\n");
                system.push_str(extra);
            }
        }
        if !system.is_empty() {
            messages.push(serde_json::json!({"role": "system", "content": system}));
        }
        for msg in history {
            messages.push(serde_json::json!({"role": msg.role, "content": msg.content}));
        }
        messages.push(serde_json::json!({
            "role": "user",
            "content": crate::multimodal::prompt_to_content_value(prompt),
        }));
        if let Some(s) = session {
            let _ = s.append(pm_types::SessionEntry::User {
                content: prompt.to_string(),
                timestamp: now_ms(),
            }).await;
        }

        let mut all_tool_calls: Vec<Value> = Vec::new();
        let mut final_output = String::new();
        let mut stall_rounds: usize = 0;
        let mut last_output = String::new();
        let mut provider_error: Option<anyhow::Error> = None;
        let mut rounds_used: u32 = 0;
        let mut cost_delta_usd: f64 = 0.0;

        let max_rounds = self.config.max_rounds;
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
                match self.call_with_streaming(&agent_cfg, &messages, &tool_defs, f).await {
                    Ok(pair) => pair,
                    Err(e) => { provider_error = Some(e); break; }
                }
            } else {
                match self.call_with_fallback(&agent_cfg, &messages, &tool_defs).await {
                    Ok(pair) => pair,
                    Err(e) => { provider_error = Some(e); break; }
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
                    ct.record(&model_used, prompt_tokens, completion_tokens).await;
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
                        let _ = s.append(pm_types::SessionEntry::Assistant {
                            content: text.to_string(),
                            timestamp: now_ms(),
                        }).await;
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
                    let tc_id = tc["id"].as_str()
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
                            fn_name, preview
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
                    let _ = s.append(pm_types::SessionEntry::ToolCall {
                        call_id: tc_id.clone(),
                        name: fn_name.clone(),
                        args: fn_args.clone(),
                        timestamp: now_ms(),
                    }).await;
                }
            }

            // Execute all tools concurrently — but consult the optional `gate`
            // first. Denied tools never run; their reason string is returned
            // to the model as if it were the tool's output.
            let tools_config = &self.config.tools;
            let results: Vec<String> = join_all(work.iter().map(|(_, fn_name, fn_args, _)| {
                let decision = match gate {
                    Some(g) => g(fn_name, fn_args),
                    None    => ToolGateDecision::Allow,
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
                    let _ = s.append(pm_types::SessionEntry::ToolResult {
                        call_id: tc_id.clone(),
                        output: result.clone(),
                        synthetic: false,
                        timestamp: now_ms(),
                    }).await;
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
        provider_type: &str,
    ) -> Result<reqwest::Response, String> {
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

            let mut req = self.http_client
                .post(url)
                .header("Authorization", format!("Bearer {}", key))
                .header("Content-Type", "application/json");
            if provider_type == "anthropic" {
                req = req.header("anthropic-version", "2023-06-01");
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
            if matches!(status_u16, 400 | 401 | 403 | 404 | 422) {
                let text = r.text().await.unwrap_or_default();
                last_err = format!(
                    "[{}] HTTP {} from {}: {}",
                    provider_name, status_u16, url, crate::tools::floor_char_boundary(&text, 200)
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
            } else { 0 };
            let text = r.text().await.unwrap_or_default();
            last_err = format!(
                "[{}] HTTP {} from {}: {}",
                provider_name, status_u16, url, crate::tools::floor_char_boundary(&text, 200)
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
        let mut provider_names = resolve_provider_order(
            agent_cfg,
            self.config.providers.keys().map(|s| s.as_str()),
        );
        let runtime_over = std::env::var("PHANTOM_RUNTIME_OVERRIDE").ok()
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
                let msg = format!("[{}] not in [providers.*] (no such block in agents.toml)", provider_name);
                errors.push(msg.clone());
                crate::diag::record("provider_skip", format!("(streaming) {}", msg));
                continue;
            };
            let api_key = provider.api_key.clone()
                .or_else(|| provider.api_key_env.as_ref().and_then(|env| std::env::var(env).ok()));
            let Some(key) = api_key.filter(|k| !k.is_empty()) else {
                let env_name = provider.api_key_env.as_deref().unwrap_or("(no api_key_env)");
                let msg = format!("[{}] no key — env var {} unset (vault sync? `phantom config pull`)",
                    provider_name, env_name);
                errors.push(msg.clone());
                crate::diag::record("provider_skip", format!("(streaming) {}", msg));
                continue;
            };
            tried_any = true;
            crate::diag::record("provider_attempt",
                format!("[{}] (streaming) trying with model={}",
                    provider_name, entry_model.unwrap_or("(default)")));

            // Per-entry model > agent.model > provider.default_model.
            let model = entry_model
                .map(|m| m.to_string())
                .filter(|m| !m.is_empty())
                .or_else(|| (!agent_cfg.model.is_empty()).then(|| agent_cfg.model.clone()))
                .or_else(|| provider.default_model.clone())
                .unwrap_or_default();
            if model.is_empty() {
                eprintln!("  [provider {}] skipped: no model configured", provider_name);
                let msg = format!("[{}] no model — entry isn't `provider:model` and provider has no default_model", provider_name);
                errors.push(msg.clone());
                last_err = msg;
                continue 'providers;
            }

            let url = provider_url(provider_name, provider);
            let mut body = serde_json::json!({
                "model": model,
                "messages": messages,
                "max_tokens": crate::config::default_max_tokens(),
                "stream": true,
            });
            if !tool_defs.is_empty() {
                body["tools"] = Value::Array(tool_defs.to_vec());
                body["tool_choice"] = Value::String("auto".into());
            }

            // --- per-provider retry loop for streaming ---
            // Returns Ok(resp) on success, Err(true) to skip provider, Err(false) exhausted.
            let resp_result = self.streaming_with_retry(
                &url, &key, &body, provider_name, &provider.provider_type
            ).await;
            let resp = match resp_result {
                Ok(r) => r,
                Err(err_msg) => {
                    eprintln!("  [provider {}] failed: {}", provider_name, err_msg);
                    crate::diag::record("provider_fail",
                        format!("[{}] (streaming) {}",
                            provider_name,
                            err_msg.chars().take(200).collect::<String>()));
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
                let next_chunk = tokio::time::timeout(
                    std::time::Duration::from_secs(30),
                    stream.next(),
                );
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
                    Err(_) => continue,
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
                            Err(_) => continue,
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
                                        on_token(AgentEvent::Thinking { content: t.to_string() });
                                    }
                                }
                                continue;
                            }
                            if let Some(token) = json["delta"]["text"].as_str() {
                                if !token.is_empty() {
                                    full_content.push_str(token);
                                    on_token(AgentEvent::Token { content: token.to_string() });
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
                                        on_token(AgentEvent::Thinking { content: t.to_string() });
                                    }
                                }
                            }
                            // Text token
                            if let Some(token) = delta.get("content").and_then(|v| v.as_str()) {
                                if !token.is_empty() {
                                    full_content.push_str(token);
                                    on_token(AgentEvent::Token { content: token.to_string() });
                                }
                            }
                            // Tool call deltas
                            if let Some(tc_array) = delta.get("tool_calls").and_then(|v| v.as_array()) {
                                for tc in tc_array {
                                    let index = tc["index"].as_u64().unwrap_or(0) as usize;
                                    let entry = tool_calls_map.entry(index).or_insert_with(|| {
                                        (String::new(), String::new(), String::new())
                                    });
                                    if let Some(id) = tc["id"].as_str() {
                                        if entry.0.is_empty() { entry.0 = id.to_string(); }
                                    }
                                    if let Some(name) = tc["function"]["name"].as_str() {
                                        if entry.1.is_empty() { entry.1 = name.to_string(); }
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
            let tool_calls_json: Vec<Value> = sorted_indices.iter().map(|idx| {
                let (id, name, args) = &tool_calls_map[idx];
                serde_json::json!({
                    "id": id,
                    "type": "function",
                    "function": {
                        "name": name,
                        "arguments": args,
                    }
                })
            }).collect();

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
        Err(anyhow::anyhow!("All providers failed (streaming). Tried {} provider(s):{}{}",
            errors.len(), breakdown, hint))
    }

    async fn call_with_fallback(
        &self,
        agent_cfg: &AgentEntry,
        messages: &[Value],
        tool_defs: &[Value],
    ) -> anyhow::Result<(Value, String)> {
        let mut provider_names = resolve_provider_order(
            agent_cfg,
            self.config.providers.keys().map(|s| s.as_str()),
        );
        // Per-session runtime override. Two sources, env first then file:
        //   1. PHANTOM_RUNTIME_OVERRIDE env (this process)
        //   2. ~/.phantom-mesh/runtime-override (shared across all phantom
        //      processes — so /model X:Y in the TUI also affects the
        //      local `phantom serve` daemon and cluster RPC dispatch.)
        // First non-empty wins. Prepended to provider chain, de-duped.
        let runtime_over = std::env::var("PHANTOM_RUNTIME_OVERRIDE").ok()
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
            // (real bug hit on acer: groq's llama-3.3-70b-versatile was
            // sent to opencode endpoint → ModelError 401).
            let (provider_name, entry_model) = parse_provider_entry(entry);
            let Some(provider) = self.config.providers.get(provider_name) else {
                let msg = format!("[{}] not in [providers.*] (no such block in agents.toml)", provider_name);
                errors.push(msg.clone());
                crate::diag::record("provider_skip", msg);
                continue;
            };
            let api_key = provider.api_key.clone()
                .or_else(|| provider.api_key_env.as_ref().and_then(|env| std::env::var(env).ok()));
            let Some(key) = api_key.filter(|k| !k.is_empty()) else {
                let env_name = provider.api_key_env.as_deref().unwrap_or("(no api_key_env)");
                let msg = format!("[{}] no key — env var {} unset (vault sync? `phantom config pull`)",
                    provider_name, env_name);
                errors.push(msg.clone());
                crate::diag::record("provider_skip", msg);
                continue;
            };
            tried_any = true;
            crate::diag::record("provider_attempt",
                format!("[{}] trying with model={}",
                    provider_name, entry_model.unwrap_or("(default)")));

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
                eprintln!("  [provider {}] skipped: no model configured", provider_name);
                let msg = format!("[{}] no model — entry isn't `provider:model` and provider has no default_model", provider_name);
                errors.push(msg.clone());
                last_err = msg;
                continue 'providers;
            }

            let url = provider_url(provider_name, provider);
            // Lower default keeps Groq daily TPD quota usage reasonable (was 4096 → tripped quota
            // limits on small chat requests because Groq pre-allocates max_tokens against the cap).
            // Agents that need long output should request explicitly via tool/system context.
            let mut body = serde_json::json!({
                "model": model,
                "messages": messages,
                "max_tokens": crate::config::default_max_tokens(),
            });
            if !tool_defs.is_empty() {
                body["tools"] = Value::Array(tool_defs.to_vec());
                body["tool_choice"] = Value::String("auto".into());
            }

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

                let mut req = self.http_client
                    .post(&url)
                    .header("Authorization", format!("Bearer {}", key))
                    .header("Content-Type", "application/json");
                if provider.provider_type == "anthropic" {
                    req = req.header("anthropic-version", "2023-06-01");
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
                if matches!(status_u16, 400 | 401 | 403 | 404 | 422) {
                    let text = resp.text().await.unwrap_or_default();
                    let msg = format!(
                        "[{}] HTTP {} {} ({})",
                        provider_name, status_u16,
                        match status_u16 { 401 => "unauthorized — bad/expired key",
                                           403 => "forbidden — key lacks model access",
                                           404 => "not found — model id wrong?",
                                           400 => "bad request — schema mismatch?",
                                           _ => "client error" },
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
                    let wait = resp.headers()
                        .get("retry-after")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or(0)
                        .min(30);
                    let text = resp.text().await.unwrap_or_default();
                    last_err = format!(
                        "[{}] HTTP {} from {}: {}",
                        provider_name, status_u16, url, crate::tools::floor_char_boundary(&text, 200)
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
                    provider_name, status_u16,
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
            format!("(no providers tried — check [agent.X].providers + [providers.*]; last_err: {})", last_err)
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
        Err(anyhow::anyhow!("All providers failed. Tried {} provider(s):{}{}",
            errors.len(), breakdown, hint))
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
    let push = |list: &mut Vec<String>, seen: &mut std::collections::HashSet<String>, name: &str| {
        if !name.is_empty() && seen.insert(name.to_string()) {
            list.push(name.to_string());
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

fn provider_url(provider_name: &str, provider: &ProviderEntry) -> String {
    if let Some(explicit) = &provider.url {
        let base = explicit.trim_end_matches('/');
        if base.ends_with("/chat/completions") { return base.to_string(); }
        // Don't double-add /v1 if base_url already contains a version segment
        let already_versioned = base.ends_with("/v1")
            || base.contains("/v1/")
            || base.contains("/v1beta");
        if already_versioned {
            return format!("{}/chat/completions", base);
        }
        return format!("{}/v1/chat/completions", base);
    }
    let key = if provider.provider_type.is_empty() {
        provider_name
    } else {
        provider.provider_type.as_str()
    };
    match key {
        "openai" | "openai_compat" => "https://api.openai.com/v1/chat/completions".into(),
        "groq" => "https://api.groq.com/openai/v1/chat/completions".into(),
        "gemini" => "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions".into(),
        "anthropic" => "https://api.anthropic.com/v1/chat/completions".into(),
        // OpenCode Zen gateway. /api/v1 was the legacy path and now returns
        // 404 (Cloudflare HTML); /zen/v1 is the live OpenAI-compatible endpoint
        // — same path streaming.rs::provider_chat_url already uses.
        "opencode" => "https://opencode.ai/zen/v1/chat/completions".into(),
        "openrouter" => "https://openrouter.ai/api/v1/chat/completions".into(),
        "cerebras" => "https://api.cerebras.ai/v1/chat/completions".into(),
        "deepseek" => "https://api.deepseek.com/v1/chat/completions".into(),
        // Unknown provider — preserve the historical openrouter fallback so
        // users with a custom `[providers.foo]` block + OPENROUTER_API_KEY
        // continue to work. New configs should set `type` or `url` explicitly.
        _ => "https://openrouter.ai/api/v1/chat/completions".into(),
    }
}

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
    if estimate_tokens(messages) <= budget { return; }

    let system_msgs: Vec<Value> = messages.iter()
        .filter(|m| m["role"].as_str() == Some("system"))
        .cloned().collect();
    let conv_msgs: Vec<Value> = messages.iter()
        .filter(|m| m["role"].as_str() != Some("system"))
        .cloned().collect();

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

    tracing::info!("Context compacted: dropped {} messages, {} remaining", dropped, messages.len());

    // Emergency pass
    if estimate_tokens(messages) > budget {
        let conv2: Vec<Value> = messages.iter()
            .filter(|m| m["role"].as_str() != Some("system"))
            .cloned().collect();
        let mut kept2 = conv2.clone();
        while estimate_tokens(&kept2) > conv_budget && kept2.len() > 2 {
            kept2.remove(0);
        }
        if kept2.len() > 4 { kept2 = kept2[kept2.len() - 4..].to_vec(); }
        kept2 = strip_orphans(strip_leading_tool(kept2));
        let emergency = serde_json::json!({
            "role": "system",
            "content": "[Emergency compaction: context was still too large; older messages dropped.]"
        });
        *messages = system_msgs;
        messages.push(emergency);
        messages.extend(kept2);
        tracing::warn!("Emergency compaction applied, {} messages remaining", messages.len());
    }
}

fn strip_leading_tool(mut msgs: Vec<Value>) -> Vec<Value> {
    while msgs.first().map(|m| m["role"].as_str() == Some("tool")).unwrap_or(false) {
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
                && msgs[i]["tool_calls"].as_array().map(|a| !a.is_empty()).unwrap_or(false);
            if is_tool_call {
                let has_result = msgs.get(i + 1)
                    .map(|m| m["role"].as_str() == Some("tool"))
                    .unwrap_or(false);
                if !has_result { msgs.remove(i); changed = true; continue; }
            }
            i += 1;
        }
        if !changed { break; }
    }
    msgs
}

fn output_unchanged(prev: &str, current: &str) -> bool {
    if prev.is_empty() { return false; }
    let max_len = prev.len().max(current.len());
    if max_len == 0 { return true; }
    let diff = prev.chars().zip(current.chars()).filter(|(a, b)| a != b).count()
        + prev.len().abs_diff(current.len());
    diff * 10 < max_len
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── detect_truncation_notice ──────────────────────────────────────────

    // ── provider_url ──────────────────────────────────────────────────────

    fn p(provider_type: &str, url: Option<&str>) -> ProviderEntry {
        ProviderEntry {
            provider_type: provider_type.to_string(),
            url: url.map(|s| s.to_string()),
            api_key: None,
            api_key_env: None,
            default_model: None,
            tier: None,
        }
    }

    fn agent(provider: &str, providers: Option<Vec<&str>>) -> AgentEntry {
        AgentEntry {
            provider: provider.to_string(),
            providers: providers.map(|v| v.into_iter().map(String::from).collect()),
            model: String::new(),
            tools: Vec::new(),
            instructions: String::new(),
        }
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
        let order = resolve_provider_order(&cfg, ["opencode", "groq", "cerebras", "anthropic"].into_iter());
        // groq, cerebras (from list); opencode (from provider field);
        // anthropic (alphabetical of unlisted).
        assert_eq!(order, vec!["groq", "cerebras", "opencode", "anthropic"]);
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
        assert_eq!(parse_provider_entry("opencode:claude-sonnet-4-6"),
                   ("opencode", Some("claude-sonnet-4-6")));
        // Empty after colon falls back to None (resolver uses default).
        assert_eq!(parse_provider_entry("groq:"), ("groq", None));
        // Multi-colon: only first split point is used; the rest is the model id.
        // (Some model ids contain colons in some catalogs, e.g. "qwen3:8b".)
        assert_eq!(parse_provider_entry("local-ollama:qwen3:8b"),
                   ("local-ollama", Some("qwen3:8b")));
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

    #[test]
    fn provider_url_routes_opencode_to_opencode_ai_not_openrouter() {
        // Regression test for the silent-misroute bug: with no `type =`
        // and no `url =`, a `[providers.opencode]` block was falling
        // through to openrouter.ai because provider_type defaults to "".
        // Now provider_url falls back to the section name (provider_name)
        // when provider_type is empty.
        let url = provider_url("opencode", &p("", None));
        assert!(
            url.starts_with("https://opencode.ai/"),
            "opencode should route to opencode.ai, got: {url}",
        );
        // Confirms it's NOT going to openrouter
        assert!(!url.contains("openrouter"), "got: {url}");
        // The /api/v1 path returns Cloudflare 404 since 2026-04;
        // /zen/v1 is the live OpenAI-compatible endpoint. Lock in
        // the working path so a future regression to /api/v1 fails CI.
        assert!(
            url.contains("/zen/v1/"),
            "opencode default URL must use /zen/v1, got: {url}",
        );
    }

    #[test]
    fn provider_url_routes_explicit_known_providers_correctly() {
        assert!(provider_url("groq", &p("", None)).contains("api.groq.com"));
        assert!(provider_url("gemini", &p("", None)).contains("generativelanguage"));
        assert!(provider_url("anthropic", &p("", None)).contains("api.anthropic.com"));
        assert!(provider_url("openai", &p("", None)).contains("api.openai.com"));
        assert!(provider_url("openrouter", &p("", None)).contains("openrouter.ai"));
        assert!(provider_url("cerebras", &p("", None)).contains("api.cerebras.ai"));
        assert!(provider_url("deepseek", &p("", None)).contains("api.deepseek.com"));
    }

    #[test]
    fn provider_url_explicit_url_wins_over_section_name() {
        // If a user sets `url = "https://my-proxy/..."` it must NOT be
        // overridden by the section-name match. This is how we support
        // self-hosted proxies and on-prem deployments.
        let url = provider_url("opencode", &p("", Some("https://proxy.local/v1")));
        assert!(url.starts_with("https://proxy.local/"), "got: {url}");
        assert!(!url.contains("opencode.ai"), "got: {url}");
    }

    #[test]
    fn provider_url_explicit_type_wins_over_section_name() {
        // If user sets `type = "groq"` in `[providers.weirdname]`, route
        // by type not section name.
        let url = provider_url("weirdname", &p("groq", None));
        assert!(url.contains("api.groq.com"), "got: {url}");
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
        assert!(tokens >= 1000, "expected >=1000 tokens for image msg, got {}", tokens);

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
            override_text.trim_start()
                .trim_start_matches(REPLACE_MARKER)
                .trim_start_matches('\n')
                .to_string()
        } else {
            format!(
                "## User customisation\n{}\n\n## Agent instructions\n{}",
                override_text.trim(), built_in
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
            override_text.trim_start()
                .trim_start_matches(REPLACE_MARKER)
                .trim_start_matches('\n')
                .to_string()
        } else {
            format!(
                "## User customisation\n{}\n\n## Agent instructions\n{}",
                override_text.trim(), built_in
            )
        };
        assert!(!merged.contains("You are master"), "REPLACE marker must drop built-in");
        assert!(merged.starts_with("I am a haiku poet"));
        assert!(merged.contains("Three lines only."));
    }
}
