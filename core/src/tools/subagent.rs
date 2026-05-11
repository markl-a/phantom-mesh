//! `task` / `subagent` tool — let an agent spawn another configured agent.
//!
//! Claude Code's `Task` model: the parent agent (e.g. `master`) decides
//! based on the user's intent that another agent should handle the work,
//! calls this tool with `{agent: "reviewer", prompt: "..."}`, the
//! subagent runs to completion in an ISOLATED context (no shared
//! history), and its full output is returned as the tool result for the
//! parent to fold into its reply.
//!
//! Args:
//!   - `agent`      (string, required): name of an agent in agents.toml
//!     (`master`, `coder`, `reviewer`, `researcher`, or any custom block)
//!   - `prompt`     (string, required): the task description for the subagent
//!   - `max_rounds` (int, optional):    cap the subagent's tool-call rounds
//!     (default = the agent's own configured limit, usually 30)
//!
//! Returns: the subagent's final output as plain text.
//!
//! Lifecycle: the parent's REPL/TUI initialises a global AgentRuntime +
//! CostTracker via `subagent::init_global()` at startup. The tool reads
//! both from the OnceLock; if either is missing the tool returns a
//! diagnostic message rather than panicking.
//!
//! Streaming: v1 runs the subagent blocking and returns the full output
//! at the end. The parent's REPL/TUI just sees a normal `● task(...)
//! → ✓ <output>` line. v1.5 will plumb the subagent's AgentEvent stream
//! into the parent's event channel so progress is visible nested.

use serde_json::Value;
use std::sync::OnceLock;

use crate::agent::AgentRuntime;
use crate::cost::CostTracker;
use crate::providers::traits::ChatMessage;

// ── Fork modes ────────────────────────────────────────────────────────────────
//
// Models Codex's `SpawnAgentForkMode` (see
// `references/codex/codex-rs/core/src/agent/control.rs:45-55`).
// Lets a parent agent — or any orchestrator — start a subagent that
// inherits some, all, or none of the parent's conversation history.
//
// Empty           : v1 default. Subagent starts with no inherited
//                   context. Fastest, no leakage; the parent passes
//                   its task description in `prompt`. This matches
//                   the existing `subagent::spawn` semantics.
// FullHistory     : child sees every prior turn. Use when the child
//                   needs the full back-and-forth (e.g. a reviewer
//                   subagent that must see what was tried earlier).
//                   Cost: full token replay on every spawn.
// LastNTurns      : child sees only the last `n` turns. Compromise
//                   between context depth and token cost; common
//                   pattern for "continue from here" forks. A turn
//                   is one `(user, assistant)` pair plus any tool
//                   messages emitted between them.

#[derive(Debug, Clone)]
pub enum SpawnAgentForkMode {
    Empty,
    FullHistory(Vec<ChatMessage>),
    LastNTurns { history: Vec<ChatMessage>, n: usize },
}

impl SpawnAgentForkMode {
    /// Resolve the inherited slice that should be passed to
    /// `AgentRuntime::run_tracked`. `Empty` returns `&[]`.
    pub fn resolved_history(&self) -> Vec<ChatMessage> {
        match self {
            SpawnAgentForkMode::Empty => Vec::new(),
            SpawnAgentForkMode::FullHistory(h) => h.clone(),
            SpawnAgentForkMode::LastNTurns { history, n } => {
                truncate_history(history, *n)
            }
        }
    }
}

/// Keep the most recent `n` turns. A "turn" is the contiguous run of
/// messages bracketed by user prompts: every user message starts a
/// new turn, and assistant + tool replies belonging to it follow
/// until the next user message. The system prompt (if any, role
/// `"system"`) is preserved at the head — without it the model
/// loses its identity / instructions across the fork.
///
/// If `history` has fewer than `n` user-rooted turns, the whole
/// thing is returned unchanged.
pub fn truncate_history(history: &[ChatMessage], n: usize) -> Vec<ChatMessage> {
    if n == 0 || history.is_empty() {
        return Vec::new();
    }
    let user_idxs: Vec<usize> = history
        .iter()
        .enumerate()
        .filter_map(|(i, m)| if m.role == "user" { Some(i) } else { None })
        .collect();
    if user_idxs.len() <= n {
        return history.to_vec();
    }
    // Cut at the n-th-from-last user message.
    let cut = user_idxs[user_idxs.len() - n];
    let mut out: Vec<ChatMessage> = Vec::with_capacity(history.len() - cut + 1);
    // Preserve the leading system message (if any).
    if let Some(first) = history.first() {
        if first.role == "system" {
            out.push(first.clone());
        }
    }
    out.extend(history[cut..].iter().cloned());
    out
}

static RUNTIME: OnceLock<AgentRuntime> = OnceLock::new();
static COST:    OnceLock<CostTracker> = OnceLock::new();

/// One past-or-running subagent invocation. Captured by run_one() so the
/// REPL's /tasks slash command can list them.
#[derive(Debug, Clone)]
pub struct TaskRecord {
    pub n: usize,
    pub agent: String,
    pub prompt: String,
    pub status: String,        // "running" | "ok" | "error" | "timeout"
    pub started_ms: i64,
    pub elapsed_secs: f64,
    pub cost_usd: f64,
    pub rounds: u32,
    pub output_preview: String,
}

static TASK_LOG: OnceLock<std::sync::Mutex<Vec<TaskRecord>>> = OnceLock::new();

fn log() -> &'static std::sync::Mutex<Vec<TaskRecord>> {
    TASK_LOG.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

/// Snapshot the task log for `/tasks` to render. Caps at 100 most recent.
pub fn task_log_snapshot() -> Vec<TaskRecord> {
    log().lock().map(|v| v.clone()).unwrap_or_default()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Wire a runtime + cost tracker into the global slot. Call once at startup
/// (in bin/phantom.rs after the AppState is built). Idempotent — second
/// call is a no-op (OnceLock semantics).
pub fn init_global(runtime: AgentRuntime, cost: CostTracker) {
    let _ = RUNTIME.set(runtime);
    let _ = COST.set(cost);
}

/// Output formatting modes for [`spawn`] and [`parallel`]. Lets the caller
/// choose between phantom-native human-readable wrap, parity-with-Claude-Code
/// raw text, or a structured JSON envelope for programmatic consumption.
///
/// See `_planning-audit/07-SUBAGENT-PARITY-PLAN.md` for the rationale —
/// without `Raw`, callers parsing the tool result must strip phantom's
/// `[subagent: name · ... rounds]` prefix, which Claude Code's Agent tool
/// does NOT emit. `Raw` mode makes phantom subagent a drop-in alternative.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    /// Default. Human-readable header line then the agent output:
    /// `[subagent: <agent> · <rounds> rounds · $<cost> · <secs>s]\n\n<output>`
    Wrapped,
    /// Just the agent's output, no header. Byte-for-byte parity with what
    /// Claude Code's `Agent` tool returns. Pick this when chaining results
    /// into another LLM prompt or when the caller has its own envelope.
    Raw,
    /// Structured JSON: `{"agent": "...", "rounds": N, "cost_usd": F, "elapsed_secs": F, "output": "..."}`.
    /// For programmatic consumers (CI, dashboards, scripts) that want
    /// fields without regex-parsing the wrap.
    Json,
}

impl OutputFormat {
    fn parse(arg: Option<&str>) -> Self {
        match arg.unwrap_or("wrapped") {
            "raw"  => OutputFormat::Raw,
            "json" => OutputFormat::Json,
            _      => OutputFormat::Wrapped,
        }
    }
}

/// Tool entry point. Dispatched from `core/src/tools/mod.rs::execute`.
///
/// Accepts either `{"agent": "...", "prompt": "..."}` (phantom native) or
/// `{"subagent_type": "...", "prompt": "...", "description": "..."}`
/// (Claude Code's Agent tool shape). Description is logged for parity
/// but doesn't affect behavior — it's a label for the parent's UI which
/// phantom doesn't surface yet.
pub async fn spawn(args: &Value) -> String {
    // Accept both `agent` (phantom native) and `subagent_type` (Claude
    // Code Agent tool name). Whichever is present wins; if both, `agent`
    // takes precedence (phantom's namespace is the source of truth here).
    let agent = match args.get("agent").and_then(|v| v.as_str())
        .or_else(|| args.get("subagent_type").and_then(|v| v.as_str()))
    {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return "[task error] missing required field: agent (or subagent_type)".to_string(),
    };
    let prompt = match args.get("prompt").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return "[task error] missing required field: prompt".to_string(),
    };
    // `description` is a Claude-Code-Agent compatibility field — kept for
    // schema parity but not behaviorally significant in phantom v1.
    // Future use: surface as the task log's display label for /tasks UI.
    let _description: Option<&str> = args.get("description").and_then(|v| v.as_str());
    let max_rounds: Option<usize> = args.get("max_rounds").and_then(|v| v.as_u64()).map(|n| n as usize);
    let max_secs:   Option<u64>   = args.get("max_secs").and_then(|v| v.as_u64());
    let max_cost:   Option<f64>   = args.get("max_cost_usd").and_then(|v| v.as_f64());
    let node:       Option<String> = args.get("node").and_then(|v| v.as_str()).map(String::from);
    let auto_snapshot: bool = args
        .get("auto_snapshot")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let format: OutputFormat = OutputFormat::parse(args.get("format").and_then(|v| v.as_str()));

    // Optional macOS APFS safety net: take a `tmutil localsnapshot` before
    // the subagent starts touching anything. Cheap (~1s, no sudo). The
    // snapshot id is prepended to the result so the caller can pass it
    // straight to `phantom snapshot rollback <id>` if the run goes badly.
    // No-op on non-Mac builds; ignore creation failures (best effort).
    let snapshot_prefix: String = {
        #[cfg(target_os = "macos")]
        {
            if auto_snapshot {
                let label = format!("subagent:{}:{}", agent, prompt.chars().take(40).collect::<String>());
                match crate::snapshot::create(Some(&label)).await {
                    Ok(info) => format!("[snapshot pinned: {}]\n", info.id),
                    Err(e) => format!("[snapshot skip: {}]\n", e),
                }
            } else {
                String::new()
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = auto_snapshot;
            String::new()
        }
    };

    let body = if let Some(n) = node {
        run_remote(&agent, &prompt, &n, max_secs, format).await
    } else {
        run_one(&agent, &prompt, max_rounds, max_secs, max_cost, format).await
    };

    if snapshot_prefix.is_empty() {
        body
    } else {
        format!("{}{}", snapshot_prefix, body)
    }
}

/// Cross-mesh task — route the subagent run to a configured peer.
/// `node` may be:
///   - exact peer URL (`http://100.87.70.65:7879`)
///   - host:port substring (`100.87.70.65:7879` or `:7879`)
///   - shorter prefix (e.g. `100.87.70.65` if unique)
async fn run_remote(agent: &str, prompt: &str, node: &str, max_secs: Option<u64>, format: OutputFormat) -> String {
    let Some(runtime) = RUNTIME.get() else {
        return "[task error] runtime not initialised".to_string();
    };
    let cfg = runtime.config();
    let peers: Vec<String> = cfg.cluster.peers.clone();
    drop(cfg);

    let matches: Vec<&String> = peers.iter().filter(|p| p.contains(node)).collect();
    let target = match matches.len() {
        0 => return format!("[task error] no peer matches '{}'.  configured peers: {:?}", node, peers),
        1 => matches[0].clone(),
        _ => return format!("[task error] '{}' is ambiguous — matches {} peers: {:?}", node, matches.len(), matches),
    };

    let log_idx = {
        let mut l = log().lock().unwrap();
        let n = l.len() + 1;
        l.push(TaskRecord {
            n, agent: format!("{}@{}", agent, node),
            prompt: prompt.chars().take(120).collect(),
            status: "running".into(),
            started_ms: now_ms(),
            elapsed_secs: 0.0,
            cost_usd: 0.0,
            rounds: 0,
            output_preview: String::new(),
        });
        if l.len() > 100 { let drop_n = l.len() - 100; l.drain(0..drop_n); }
        l.len() - 1
    };

    let url = format!("{}/rpc/message", target.trim_end_matches('/'));
    let body = serde_json::json!({ "message": prompt, "agent": agent });
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(max_secs.unwrap_or(120)))
        .build()
        .unwrap_or_default();

    let started = std::time::Instant::now();
    let resp = client.post(&url).json(&body).send().await;
    let elapsed = started.elapsed().as_secs_f64();

    let update_log = |status: &str, preview: &str| {
        if let Ok(mut l) = log().lock() {
            if let Some(rec) = l.get_mut(log_idx) {
                rec.status = status.into();
                rec.elapsed_secs = elapsed;
                rec.output_preview = preview.chars().take(240).collect();
            }
        }
    };

    match resp {
        Ok(r) if r.status().is_success() => {
            let v: Value = match r.json().await {
                Ok(v) => v,
                Err(e) => { update_log("error", &e.to_string()); return format!("[remote task error] decode: {}", e); }
            };
            if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
                update_log("error", err);
                return format!("[remote task error: {}@{}] {}", agent, node, err);
            }
            let out = v.get("output").and_then(|o| o.as_str()).unwrap_or("").to_string();
            update_log("ok", &out);
            match format {
                OutputFormat::Raw => out,
                OutputFormat::Json => serde_json::to_string(&serde_json::json!({
                    "agent": agent,
                    "node": node,
                    "remote": true,
                    "elapsed_secs": elapsed,
                    "output": out,
                    "status": "ok",
                })).unwrap_or_else(|_| out.clone()),
                OutputFormat::Wrapped => format!(
                    "[subagent: {}@{} · remote · {:.1}s]\n\n{}",
                    agent, node, elapsed, out,
                ),
            }
        }
        Ok(r) => {
            let code = r.status().as_u16();
            let body = r.text().await.unwrap_or_default();
            update_log("error", &format!("HTTP {}", code));
            match format {
                OutputFormat::Raw => format!("HTTP {}: {}", code, body),
                OutputFormat::Json => serde_json::to_string(&serde_json::json!({
                    "agent": agent, "node": node, "remote": true,
                    "status": "error", "error": format!("HTTP {}: {}", code, body),
                })).unwrap_or_default(),
                OutputFormat::Wrapped => format!("[remote task error: {}@{}] HTTP {}: {}", agent, node, code, body),
            }
        }
        Err(e) => {
            update_log("error", &e.to_string());
            match format {
                OutputFormat::Raw => e.to_string(),
                OutputFormat::Json => serde_json::to_string(&serde_json::json!({
                    "agent": agent, "node": node, "remote": true,
                    "status": "error", "error": e.to_string(),
                })).unwrap_or_default(),
                OutputFormat::Wrapped => format!("[remote task error: {}@{}] {}", agent, node, e),
            }
        }
    }
}

/// Public API: spawn an agent with an explicit fork mode. Used by
/// orchestrators (TUI fork command, evolve, mesh handoff) that want
/// to seed the subagent with parent context. The LLM-facing `task`
/// tool stays on the empty-history path until there's a UX need —
/// adding it there requires plumbing the parent's `messages` vec
/// through the tool dispatch boundary, which is a bigger change.
///
/// Returns the raw `AgentResult` (not the formatted string the `task`
/// tool returns) so callers can inspect cost / turns / output.
pub async fn run_with_fork(
    agent: &str,
    prompt: &str,
    fork: SpawnAgentForkMode,
    max_rounds: Option<usize>,
    max_secs:   Option<u64>,
    max_cost:   Option<f64>,
) -> anyhow::Result<crate::agent::AgentResult> {
    let runtime = RUNTIME
        .get()
        .ok_or_else(|| anyhow::anyhow!("subagent runtime not initialised"))?;
    let cfg = runtime.config();
    if !cfg.agent.contains_key(agent) {
        let names: Vec<String> = cfg.agent.keys().cloned().collect();
        return Err(anyhow::anyhow!(
            "unknown agent '{}'. configured: {}", agent, names.join(", ")
        ));
    }
    drop(cfg);

    let cost = CostTracker::new();
    if let Some(b) = max_cost {
        cost.set_task_budget(b).await;
    }

    let history = fork.resolved_history();

    // Same env-var-based max_rounds escape hatch as `run_one`.
    let mut prev_env: Option<String> = None;
    if let Some(mr) = max_rounds {
        prev_env = std::env::var("PHANTOM_MAX_ROUNDS").ok();
        std::env::set_var("PHANTOM_MAX_ROUNDS", mr.to_string());
    }

    let run_fut = runtime.run_tracked(agent, prompt, &history, None, &cost);
    let outcome = match max_secs {
        Some(s) => tokio::time::timeout(std::time::Duration::from_secs(s), run_fut)
            .await
            .map_err(|_| anyhow::anyhow!("subagent exceeded max_secs={}", s))?,
        None => run_fut.await,
    };

    if max_rounds.is_some() {
        match prev_env {
            Some(v) => std::env::set_var("PHANTOM_MAX_ROUNDS", v),
            None    => std::env::remove_var("PHANTOM_MAX_ROUNDS"),
        }
    }

    outcome
}

/// Internal helper used by both `spawn` and `parallel`.
async fn run_one(
    agent: &str,
    prompt: &str,
    max_rounds: Option<usize>,
    max_secs:   Option<u64>,
    max_cost:   Option<f64>,
    format:     OutputFormat,
) -> String {
    let Some(runtime) = RUNTIME.get() else {
        return "[task error] runtime not initialised — subagent::init_global() was never called".to_string();
    };

    let cfg = runtime.config();
    if !cfg.agent.contains_key(agent) {
        let names: Vec<String> = cfg.agent.keys().cloned().collect();
        return format!("[task error] unknown agent '{}'. configured: {}", agent, names.join(", "));
    }
    drop(cfg);

    // Per-subagent CostTracker so we can enforce max_cost_usd in isolation.
    let cost = CostTracker::new();
    if let Some(b) = max_cost {
        cost.set_task_budget(b).await;
    }

    // Push a "running" entry into the task log so /tasks can show it live.
    let log_idx = {
        let mut l = log().lock().unwrap();
        let n = l.len() + 1;
        l.push(TaskRecord {
            n, agent: agent.to_string(),
            prompt: prompt.chars().take(120).collect(),
            status: "running".into(),
            started_ms: now_ms(),
            elapsed_secs: 0.0,
            cost_usd: 0.0,
            rounds: 0,
            output_preview: String::new(),
        });
        // Keep last 100
        if l.len() > 100 { let drop = l.len() - 100; l.drain(0..drop); }
        l.len() - 1  // 0-based index of just-pushed entry
    };

    // Apply per-call max_rounds via env var (cleared after).
    let mut prev_env: Option<String> = None;
    if let Some(mr) = max_rounds {
        prev_env = std::env::var("PHANTOM_MAX_ROUNDS").ok();
        std::env::set_var("PHANTOM_MAX_ROUNDS", mr.to_string());
    }

    // Run with optional wall-clock budget.
    let started = std::time::Instant::now();
    let run_fut = runtime.run_tracked(agent, prompt, &[], None, &cost);
    let outcome = match max_secs {
        Some(s) => {
            match tokio::time::timeout(std::time::Duration::from_secs(s), run_fut).await {
                Ok(r) => Ok(r),
                Err(_) => Err(()),
            }
        }
        None => Ok(run_fut.await),
    };

    if max_rounds.is_some() {
        match prev_env {
            Some(v) => std::env::set_var("PHANTOM_MAX_ROUNDS", v),
            None    => std::env::remove_var("PHANTOM_MAX_ROUNDS"),
        }
    }

    let elapsed = started.elapsed().as_secs_f64();

    let update_log = |status: &str, rounds: u32, cost_usd: f64, preview: &str| {
        if let Ok(mut l) = log().lock() {
            if let Some(rec) = l.get_mut(log_idx) {
                rec.status = status.into();
                rec.elapsed_secs = elapsed;
                rec.cost_usd = cost_usd;
                rec.rounds = rounds;
                rec.output_preview = preview.chars().take(240).collect();
            }
        }
    };

    let result = match outcome {
        Ok(r) => r,
        Err(()) => {
            update_log("timeout", 0, 0.0, "");
            let msg = format!("exceeded max_secs={} (elapsed {:.1}s)", max_secs.unwrap_or(0), elapsed);
            return match format {
                OutputFormat::Raw => msg,
                OutputFormat::Json => serde_json::to_string(&serde_json::json!({
                    "agent": agent, "status": "timeout", "elapsed_secs": elapsed, "error": msg,
                })).unwrap_or_default(),
                OutputFormat::Wrapped => format!("[task aborted: {}] {}", agent, msg),
            };
        }
    };

    match result {
        Ok(r) => {
            update_log("ok", r.turns, r.cost_delta_usd, &r.output);
            // Budget-exceeded path: wrap the warning around the output in
            // wrapped mode, surface as a status field in JSON, prepend a
            // single inline note in raw mode.
            if let Some(max) = max_cost {
                if r.cost_delta_usd > max {
                    return match format {
                        OutputFormat::Raw => format!("(budget exceeded: ${:.4} > ${:.4})\n{}", r.cost_delta_usd, max, r.output),
                        OutputFormat::Json => serde_json::to_string(&serde_json::json!({
                            "agent": agent, "status": "budget_exceeded",
                            "rounds": r.turns, "cost_usd": r.cost_delta_usd, "elapsed_secs": elapsed,
                            "max_cost_usd": max, "output": r.output,
                        })).unwrap_or_else(|_| r.output.clone()),
                        OutputFormat::Wrapped => format!(
                            "[task budget exceeded: {} · cost ${:.4} > max ${:.4}]\n\n{}",
                            agent, r.cost_delta_usd, max, r.output,
                        ),
                    };
                }
            }
            match format {
                OutputFormat::Raw => r.output,
                OutputFormat::Json => serde_json::to_string(&serde_json::json!({
                    "agent": agent, "status": "ok",
                    "rounds": r.turns, "cost_usd": r.cost_delta_usd, "elapsed_secs": elapsed,
                    "output": r.output,
                })).unwrap_or_else(|_| r.output.clone()),
                OutputFormat::Wrapped => format!(
                    "[subagent: {} · {} rounds · ${:.4} · {:.1}s]\n\n{}",
                    agent, r.turns, r.cost_delta_usd, elapsed, r.output,
                ),
            }
        }
        Err(e) => {
            update_log("error", 0, 0.0, &e.to_string());
            match format {
                OutputFormat::Raw => e.to_string(),
                OutputFormat::Json => serde_json::to_string(&serde_json::json!({
                    "agent": agent, "status": "error", "error": e.to_string(),
                })).unwrap_or_default(),
                OutputFormat::Wrapped => format!("[task error: {}] {}", agent, e),
            }
        }
    }
}

/// `parallel_tasks` tool — spawn multiple subagents concurrently.
///
/// Output shape obeys `format`:
/// - `wrapped` (default): a single string with `[parallel_tasks · N subagents]`
///   header and `── [#i] agent ──\n<output>` blocks per subagent. Each block's
///   inner output is itself wrapped (own header line per subagent).
/// - `raw`: a single string joining each subagent's RAW output with double
///   newlines. Loses per-task labels — use `json` if you need them.
/// - `json`: a JSON array of `{label, agent, status, rounds, cost_usd,
///   elapsed_secs, output, ...}` objects, one per subagent. Lets the caller
///   iterate exactly the way Claude Code returns N separate Agent tool
///   results.
pub async fn parallel(args: &Value) -> String {
    let tasks = match args.get("tasks").and_then(|v| v.as_array()) {
        Some(arr) if !arr.is_empty() => arr.clone(),
        _ => return "[parallel_tasks error] 'tasks' must be a non-empty array of {agent, prompt}".to_string(),
    };

    let max_rounds: Option<usize> = args.get("max_rounds").and_then(|v| v.as_u64()).map(|n| n as usize);
    let max_secs:   Option<u64>   = args.get("max_secs").and_then(|v| v.as_u64());
    let max_cost:   Option<f64>   = args.get("max_cost_usd").and_then(|v| v.as_f64());
    let format: OutputFormat = OutputFormat::parse(args.get("format").and_then(|v| v.as_str()));

    // Internal per-subagent format: when caller asked for `Json` at the
    // outer level we want each subagent's result also as JSON so we can
    // splice the structured envelope into our array. For `Raw`/`Wrapped`
    // we let each subagent inherit the same outer choice.
    let inner_format = match format {
        OutputFormat::Json => OutputFormat::Json,
        OutputFormat::Raw  => OutputFormat::Raw,
        OutputFormat::Wrapped => OutputFormat::Wrapped,
    };

    let futures = tasks.iter().enumerate().map(|(i, t)| {
        // Accept both `agent` and `subagent_type` per the parity plan.
        let agent  = t.get("agent").and_then(|v| v.as_str())
            .or_else(|| t.get("subagent_type").and_then(|v| v.as_str()))
            .unwrap_or("master").to_string();
        let prompt = t.get("prompt").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let node = t.get("node").and_then(|v| v.as_str()).map(String::from);
        let label = match &node {
            Some(n) => format!("[#{}] {}@{}", i + 1, agent, n),
            None    => format!("[#{}] {}", i + 1, agent),
        };
        async move {
            let out = match node {
                Some(n) => run_remote(&agent, &prompt, &n, max_secs, inner_format).await,
                None    => run_one(&agent, &prompt, max_rounds, max_secs, max_cost, inner_format).await,
            };
            (label, agent, out)
        }
    });

    let results: Vec<(String, String, String)> = futures::future::join_all(futures).await;

    match format {
        OutputFormat::Wrapped => {
            let mut joined = String::new();
            joined.push_str(&format!("[parallel_tasks · {} subagents]\n", results.len()));
            for (label, _agent, out) in results.iter() {
                joined.push_str(&format!("\n── {} ──\n{}\n", label, out));
            }
            joined
        }
        OutputFormat::Raw => {
            // Drop labels; just concatenate outputs with separator.
            results.iter()
                .map(|(_, _, out)| out.as_str())
                .collect::<Vec<&str>>()
                .join("\n\n")
        }
        OutputFormat::Json => {
            // Each subagent's `out` is already a JSON object string; merge
            // into an array. If parsing fails (e.g. legacy mismatch), fall
            // through to a stringified form so the caller still gets data.
            let array: Vec<Value> = results.into_iter().map(|(label, agent, out)| {
                match serde_json::from_str::<Value>(&out) {
                    Ok(mut v) => {
                        if let Some(obj) = v.as_object_mut() {
                            obj.insert("label".to_string(), Value::String(label));
                            // Ensure `agent` is set even if remote path didn't.
                            obj.entry("agent".to_string()).or_insert(Value::String(agent));
                        }
                        v
                    }
                    Err(_) => serde_json::json!({
                        "label": label,
                        "agent": agent,
                        "output": out,
                    }),
                }
            }).collect();
            serde_json::to_string(&array).unwrap_or_else(|_| String::from("[]"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(role: &str, content: &str) -> ChatMessage {
        ChatMessage {
            role: role.into(),
            content: content.into(),
            tool_calls: None,
        }
    }

    #[test]
    fn truncate_history_empty_returns_empty() {
        assert!(truncate_history(&[], 5).is_empty());
        assert!(truncate_history(&[msg("user", "hi")], 0).is_empty());
    }

    #[test]
    fn truncate_history_n_larger_than_turns_returns_all() {
        let h = vec![
            msg("system", "you are helpful"),
            msg("user", "first"),
            msg("assistant", "first reply"),
            msg("user", "second"),
            msg("assistant", "second reply"),
        ];
        let out = truncate_history(&h, 5);
        assert_eq!(out.len(), 5);
        assert_eq!(out[0].role, "system");
    }

    #[test]
    fn truncate_history_keeps_system_and_last_n_turns() {
        let h = vec![
            msg("system", "instructions"),
            msg("user", "T1"),
            msg("assistant", "R1"),
            msg("user", "T2"),
            msg("assistant", "R2"),
            msg("user", "T3"),
            msg("assistant", "R3"),
        ];
        let out = truncate_history(&h, 1);
        // System preserved + only the last user-rooted turn (T3 + R3).
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].role, "system");
        assert_eq!(out[1].content, "T3");
        assert_eq!(out[2].content, "R3");
    }

    #[test]
    fn truncate_history_with_tool_messages_in_turn() {
        // A turn can include tool-role messages between user + assistant.
        // truncate_history slices on user-message boundaries, so the
        // tool messages belonging to the kept turn travel with it.
        let h = vec![
            msg("user", "T1"),
            msg("assistant", "R1"),
            msg("user", "T2"),
            msg("tool", "tool out 1"),
            msg("tool", "tool out 2"),
            msg("assistant", "R2"),
        ];
        let out = truncate_history(&h, 1);
        assert_eq!(out.iter().map(|m| m.role.as_str()).collect::<Vec<_>>(),
                   vec!["user", "tool", "tool", "assistant"]);
        assert_eq!(out[0].content, "T2");
    }

    #[test]
    fn fork_mode_resolved_history() {
        let h = vec![
            msg("user", "T1"),
            msg("assistant", "R1"),
            msg("user", "T2"),
            msg("assistant", "R2"),
        ];
        assert!(SpawnAgentForkMode::Empty.resolved_history().is_empty());
        assert_eq!(
            SpawnAgentForkMode::FullHistory(h.clone()).resolved_history().len(),
            4
        );
        let last1 = SpawnAgentForkMode::LastNTurns { history: h.clone(), n: 1 }
            .resolved_history();
        assert_eq!(last1.len(), 2);
        assert_eq!(last1[0].content, "T2");
    }
}
