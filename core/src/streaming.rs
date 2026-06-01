//! SSE streaming for real-time token output.
//!
//! Exposes [`stream_agent`] (thin wrapper) and [`stream_agent_full`] which
//! mirrors the full multi-turn agent loop of [`agent.rs`] but with real HTTP
//! streaming so that each text chunk is delivered via [`StreamEvent`] as it
//! arrives.
//!
//! ## Format detection
//!
//! The module inspects each SSE frame at runtime: if a `content_block_delta`
//! event type is found, the stream is treated as Anthropic Messages API
//! format; otherwise the OpenAI-compatible delta format is used.  The initial
//! `is_anthropic` flag (derived from `provider_type`) acts as the default
//! when the stream hasn't yet identified itself.
//!
//! ## Reconnect on disconnect
//!
//! If the byte stream drops before a `[DONE]` sentinel or `message_stop`
//! event, [`stream_one_round`] retries up to `MAX_RECONNECT_ATTEMPTS` times
//! with a 500 ms delay.  The `Last-Event-ID` header is set when the server
//! previously supplied `id:` fields in the SSE stream.
//!
//! ## Metrics
//!
//! [`StreamResult`] now includes `first_token_ms` (wall-clock milliseconds
//! from request to first token) and `tokens_received` (count of token events
//! emitted during the run).
//!
//! ## Backpressure
//!
//! [`StreamSender`] wraps a bounded `tokio::sync::mpsc` channel (capacity
//! [`CHANNEL_CAPACITY`]).  When the receiver is slow and the buffer is full,
//! events are dropped with a `tracing::warn!` rather than blocking the
//! producer.  Use [`StreamSender::subscribe`] to get the [`StreamReceiver`]
//! before starting the agent.

use anyhow::Context;
use futures::StreamExt;
use serde_json::Value;
use std::sync::Arc;
use std::time::Instant;

use crate::config::{AgentsConfig, ProviderEntry};
use crate::providers::traits::ChatMessage;
use crate::providers::{DefaultProviderResolver, LlmProvider};

// ── Public types ──────────────────────────────────────────────────────────

/// Events emitted during a streaming agent run.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// A streamed text chunk from the LLM.
    Token { content: String },
    /// A tool is about to be executed.
    ToolStart {
        id: String,
        name: String,
        args_json: String,
    },
    /// A tool has finished executing.
    ToolDone {
        id: String,
        name: String,
        result_preview: String,
        elapsed_ms: u64,
    },
    /// Extended thinking content (for models that support it).
    Thinking { content: String },
    /// A non-fatal error occurred during streaming.
    Error { message: String },
    /// The run has completed with usage statistics.
    Done { total_tokens: u32, cost_usd: f64 },
}

/// Format a [`StreamEvent`] as a Server-Sent Events frame.
///
/// Each frame is terminated with a trailing `\n\n` so it can be written
/// directly to an SSE response body.
///
/// # Example
///
/// ```ignore
/// let sse = event_to_sse(&StreamEvent::Token { content: "Hello".into() });
/// // "event: token\ndata: {\"content\":\"Hello\"}\n\n"
/// ```
pub fn event_to_sse(event: &StreamEvent) -> String {
    match event {
        StreamEvent::Token { content } => {
            let data = serde_json::json!({"content": content});
            format!("event: token\ndata: {}\n\n", data)
        }
        StreamEvent::ToolStart {
            id,
            name,
            args_json,
        } => {
            let data = serde_json::json!({"id": id, "name": name, "args": args_json});
            format!("event: tool_start\ndata: {}\n\n", data)
        }
        StreamEvent::ToolDone {
            id,
            name,
            result_preview,
            elapsed_ms,
        } => {
            let data = serde_json::json!({
                "id": id,
                "name": name,
                "result_preview": result_preview,
                "elapsed_ms": elapsed_ms,
            });
            format!("event: tool_done\ndata: {}\n\n", data)
        }
        StreamEvent::Thinking { content } => {
            let data = serde_json::json!({"content": content});
            format!("event: thinking\ndata: {}\n\n", data)
        }
        StreamEvent::Error { message } => {
            let data = serde_json::json!({"message": message});
            format!("event: error\ndata: {}\n\n", data)
        }
        StreamEvent::Done {
            total_tokens,
            cost_usd,
        } => {
            let data = serde_json::json!({"total_tokens": total_tokens, "cost_usd": cost_usd});
            format!("event: done\ndata: {}\n\n", data)
        }
    }
}

// ── Backpressure channel ──────────────────────────────────────────────────

/// Bounded channel capacity for streaming events.
///
/// If the consumer falls behind by more than this many events, new events are
/// dropped with a `tracing::warn!` log rather than blocking the producer.
pub const CHANNEL_CAPACITY: usize = 100;

/// Sender half of a bounded streaming event channel.
///
/// Created by [`StreamSender::new`].  Pass the paired [`StreamReceiver`] to
/// the consumer before starting [`stream_agent_full`] with the sender's
/// closure (obtained via [`StreamSender::as_fn`]).
pub struct StreamSender {
    tx: tokio::sync::mpsc::Sender<StreamEvent>,
}

/// Receiver half of a bounded streaming event channel.
pub struct StreamReceiver {
    rx: tokio::sync::mpsc::Receiver<StreamEvent>,
}

impl StreamSender {
    /// Create a new bounded channel pair.
    pub fn new() -> (Self, StreamReceiver) {
        let (tx, rx) = tokio::sync::mpsc::channel(CHANNEL_CAPACITY);
        (Self { tx }, StreamReceiver { rx })
    }

    /// Send an event, dropping it (with a warning) if the buffer is full.
    pub fn send(&self, event: StreamEvent) {
        match self.tx.try_send(event) {
            Ok(()) => {}
            Err(tokio::sync::mpsc::error::TrySendError::Full(dropped)) => {
                tracing::warn!(
                    event_type = %event_type_name(&dropped),
                    "StreamSender: channel full (capacity {}); dropping event",
                    CHANNEL_CAPACITY,
                );
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                tracing::debug!("StreamSender: receiver dropped; discarding event");
            }
        }
    }

    /// Convenience: return a closure that can be passed to [`stream_agent_full`].
    ///
    /// The closure captures a clone of the sender so you can keep using the
    /// original `StreamSender` concurrently.
    pub fn as_fn(&self) -> impl Fn(StreamEvent) + Send + Sync + '_ {
        move |event| self.send(event)
    }
}

impl StreamReceiver {
    /// Receive the next event, returning `None` when all senders are dropped.
    pub async fn recv(&mut self) -> Option<StreamEvent> {
        self.rx.recv().await
    }
}

/// Return a short static name for a [`StreamEvent`] variant (used in logs).
fn event_type_name(event: &StreamEvent) -> &'static str {
    match event {
        StreamEvent::Token { .. } => "Token",
        StreamEvent::ToolStart { .. } => "ToolStart",
        StreamEvent::ToolDone { .. } => "ToolDone",
        StreamEvent::Thinking { .. } => "Thinking",
        StreamEvent::Error { .. } => "Error",
        StreamEvent::Done { .. } => "Done",
    }
}

// ── Progress formatting ───────────────────────────────────────────────────

/// Format a tool execution progress indicator.
///
/// Returns a human-readable string like `⟳ file_read (1.2s)` suitable for
/// display in a terminal or chat UI while the tool is running.
///
/// # Arguments
///
/// * `name`       – tool name, e.g. `"file_read"`
/// * `elapsed_ms` – elapsed time in milliseconds
///
/// # Example
///
/// ```ignore
/// let s = format_tool_progress("file_read", 1234);
/// assert_eq!(s, "⟳ file_read (1.2s)");
/// ```
pub fn format_tool_progress(name: &str, elapsed_ms: u64) -> String {
    let secs = elapsed_ms as f64 / 1000.0;
    format!("⟳ {} ({:.1}s)", name, secs)
}

// ── Token accumulator ─────────────────────────────────────────────────────

/// Collects all [`StreamEvent::Token`] chunks into a single final string.
///
/// Useful when you want to drive a streaming call but still need the complete
/// response text at the end without re-running the request.
///
/// # Example
///
/// ```ignore
/// let mut acc = StreamAccumulator::new();
/// stream_agent_full(&cfg, "master", prompt, &[], None, None, |e| acc.handle(e)).await?;
/// println!("Full response: {}", acc.finish());
/// ```
#[derive(Debug, Default)]
pub struct StreamAccumulator {
    buf: String,
    total_tokens: u32,
    cost_usd: f64,
}

impl StreamAccumulator {
    /// Create a new, empty accumulator.
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a single [`StreamEvent`] to the accumulator.
    ///
    /// Only [`StreamEvent::Token`] events contribute to the text buffer.
    /// [`StreamEvent::Done`] captures usage statistics.
    pub fn handle(&mut self, event: StreamEvent) {
        match event {
            StreamEvent::Token { content } => self.buf.push_str(&content),
            StreamEvent::Done {
                total_tokens,
                cost_usd,
            } => {
                self.total_tokens = total_tokens;
                self.cost_usd = cost_usd;
            }
            _ => {}
        }
    }

    /// Consume the accumulator and return the full concatenated text.
    pub fn finish(self) -> String {
        self.buf
    }

    /// Return a reference to the accumulated text without consuming.
    pub fn as_str(&self) -> &str {
        &self.buf
    }

    /// Return the total token count reported by the `Done` event (0 if not yet received).
    pub fn total_tokens(&self) -> u32 {
        self.total_tokens
    }

    /// Return the cost in USD reported by the `Done` event (0.0 if not yet received).
    pub fn cost_usd(&self) -> f64 {
        self.cost_usd
    }
}

// ── StreamResult ──────────────────────────────────────────────────────────

/// Accumulated result after the full agent run completes.
///
/// ## Streaming metrics
///
/// Per-run metrics (`first_token_ms`, `tokens_received`) are tracked
/// internally and emitted as a `tracing::info!` span at the end of each
/// `stream_agent_full` call.  They are not stored in this struct to preserve
/// the public API surface (callers who construct `StreamResult` directly would
/// break if new fields were added).
#[derive(Debug, Default)]
pub struct StreamResult {
    pub output: String,
    pub tool_calls_made: Vec<serde_json::Value>,
    pub elapsed_secs: f64,
    /// [F1] Sum of `cache_read_input_tokens` across all rounds (Anthropic only;
    /// 0 for non-Anthropic providers). Use this to verify prompt-cache hits in
    /// integration tests: a hit produces `cache_read_input_tokens > 0` on the
    /// second call when the prefix matches.
    pub cache_read_input_tokens: u64,
    /// [F1] Sum of `cache_creation_input_tokens` across all rounds (Anthropic only).
    pub cache_creation_input_tokens: u64,
}

// ── Constants ─────────────────────────────────────────────────────────────

const MAX_ROUNDS: usize = 20;
const MAX_RECONNECT_ATTEMPTS: usize = 2;
const RECONNECT_DELAY_MS: u64 = 500;

// ── Pre-stream retry (T19, 2026-05-15) ────────────────────────────────────
//
// Separate from the mid-stream reconnect loop above. Pre-stream retry fires
// ONLY when we have not yet emitted any StreamEvent to the caller (no partial
// output is at risk of duplication). Post-stream-started errors fall through
// to the existing reconnect loop / propagation logic untouched.
//
// Default sleeps: 1s, 2s, 4s with ±20% jitter (capped at 30s).

/// Tunable parameters for pre-stream-establishment retry.
///
/// Constructed only via [`PreStreamRetryConfig::default`] in production —
/// tests can build a custom one with tiny delays to keep wallclock fast.
#[derive(Debug, Clone)]
pub(crate) struct PreStreamRetryConfig {
    pub max_retries: u32,
    pub base_delay: std::time::Duration,
    pub max_delay: std::time::Duration,
    pub jitter_ratio: f64,
    pub body_excerpt_bytes: usize,
}

impl Default for PreStreamRetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay: std::time::Duration::from_secs(1),
            max_delay: std::time::Duration::from_secs(30),
            jitter_ratio: 0.20,
            body_excerpt_bytes: 200,
        }
    }
}

/// Rich error returned when pre-stream retries are exhausted or a
/// non-retryable status / error is encountered.
///
/// Distinct from the mid-stream reconnect failure path which uses raw
/// `anyhow::Error`. Wrapped into an `anyhow::Error` at the call site via
/// `Into::into` so existing callers see the same `anyhow::Result` signature.
#[derive(Debug)]
pub(crate) struct PreStreamRetryError {
    pub provider: String,
    /// Total attempts made (1-based; minimum 1).
    pub attempts: u32,
    /// HTTP status of the final response, if any.
    pub last_status: Option<u16>,
    /// Parsed numeric `Retry-After` from the final response, if any.
    pub last_retry_after_secs: Option<u64>,
    /// First `body_excerpt_bytes` bytes of the final response body, UTF-8-safe.
    pub last_body_excerpt: Option<String>,
    /// Underlying `reqwest::Error` when the failure was a transport error.
    pub last_source: Option<String>,
}

impl std::fmt::Display for PreStreamRetryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{}] pre-stream retry exhausted after {} attempt(s); \
             last_status={:?} retry_after_secs={:?} body={:?} source={:?}",
            self.provider,
            self.attempts,
            self.last_status,
            self.last_retry_after_secs,
            self.last_body_excerpt,
            self.last_source,
        )
    }
}

impl std::error::Error for PreStreamRetryError {}

/// Return `true` iff a given HTTP status code is one we should retry
/// **before** any SSE event has been dispatched to the caller.
///
/// Per the T19 brief: retry on 429 (Too Many Requests) and 503 (Service
/// Unavailable). Explicitly do NOT retry on 400 / 401 / 403 / 404 — caller
/// bugs, auth problems, or missing model, none of which retrying fixes.
/// Other 5xx (500/502/504) are *also* not retried here — see the unit test
/// for the rationale.
pub(crate) fn is_pre_stream_retryable_status(status: u16) -> bool {
    matches!(status, 429 | 503)
}

/// Compute the sleep before the *next* retry attempt.
///
/// * `attempt` is 0-based — attempt 0 is the first retry slot (= base_delay).
/// * `retry_after`, when `Some`, overrides the exponential calculation but is
///   still clamped to `cfg.max_delay` so a buggy or malicious server can't
///   stall us indefinitely.
/// * `jitter_fn(0.0..=1.0)` is the unit-interval jitter source. Production
///   uses `|_| rand::random::<f64>()`; tests pass `|_| 0.5` for determinism.
///
/// The exponential factor is `2^attempt`. The final multiplier applied is
/// `(1 - jitter_ratio) + 2*jitter_ratio*jitter_fn()`, so when `jitter_fn` is
/// uniform on `[0, 1]` the multiplier is uniform on `[1 - r, 1 + r]`.
pub(crate) fn compute_pre_stream_backoff<F: FnOnce(f64) -> f64>(
    cfg: &PreStreamRetryConfig,
    attempt: u32,
    retry_after: Option<std::time::Duration>,
    jitter_fn: F,
) -> std::time::Duration {
    if let Some(d) = retry_after {
        return std::cmp::min(d, cfg.max_delay);
    }
    let base_ms = cfg.base_delay.as_millis() as f64;
    // 2^attempt — saturate at attempt = 30 to avoid f64 overflow.
    let factor = 2f64.powi(attempt.min(30) as i32);
    let raw_ms = base_ms * factor;
    let jitter = jitter_fn(0.0); // argument is a placeholder; closure ignores it
    let multiplier = (1.0 - cfg.jitter_ratio) + 2.0 * cfg.jitter_ratio * jitter;
    let jittered_ms = raw_ms * multiplier;
    let capped_ms = jittered_ms.min(cfg.max_delay.as_millis() as f64);
    std::time::Duration::from_millis(capped_ms.round() as u64)
}

/// Parse a `Retry-After` header value into a number of seconds.
///
/// Supports the numeric-seconds form only (RFC 7231 §7.1.3 first variant).
/// Returns `None` for HTTP-date values, non-numeric strings, zero, or any
/// negative value — callers fall back to the exponential calculation.
pub(crate) fn parse_retry_after_seconds(value: &reqwest::header::HeaderValue) -> Option<u64> {
    let s = value.to_str().ok()?;
    let n: i64 = s.trim().parse().ok()?;
    if n <= 0 {
        return None;
    }
    Some(n as u64)
}

// ── Provider resolution indirection (DEMO-1 gap 1 Phase 3) ───────────────
//
// The streaming path used to make its Anthropic-vs-OpenAI-compat dispatch
// decision by string-comparing `provider.provider_type == "anthropic"`.
// Phase 3 replaces that with a call through the `LlmProvider` trait, so a
// test (or a future embedder) can swap the dispatch path by injecting a
// different resolver.
//
// We keep an internal `ResolveProvider` trait (object-safe, two methods:
// `resolve_by_name` + a debug ident) so production code can use
// `DefaultProviderResolver` and tests can install a `MockResolver` that
// records calls without spinning up a real provider impl.
//
// **Why we don't route the actual HTTP send through `LlmProvider::stream`:**
// the trait builds its own request body (plain OpenAI-compat shape, no
// cache_control, no Anthropic `thinking` adaptive omitted, no
// content_block conversion for multimodal). Streaming.rs needs all three
// of those for prompt-caching ($-saving) and Opus 4.7 adaptive thinking.
// Migrating those into the trait would change Phase 2's API surface, so
// we defer that to a follow-up — see PR body for the gap report. For now,
// the trait gives us identity + URL identification; body shaping stays in
// `build_request_body`.

/// Trait that streaming.rs uses to ask "what `LlmProvider` impl should I
/// dispatch to for this provider name?".
///
/// Default production impl is a thin wrapper around
/// `DefaultProviderResolver`. Tests (or future embedders, Phase 5) can
/// install a custom resolver to record calls and verify the migrated code
/// path actually goes through the trait, or to swap dispatch live.
///
/// Public so the `streaming_trait_migration` integration tests + a future
/// Phase 5 `Agent::with_resolver` can inject. The default production path
/// (`stream_agent_full`) still uses `DefaultProviderResolver` automatically
/// — embedders only need to touch this trait when they want to override.
pub trait ResolveProvider: Send + Sync {
    fn resolve_by_name(&self, name: &str) -> Option<Arc<dyn LlmProvider>>;
}

// ── DEMO-1 gap 1 Phase 5 (2026-05-17) ─────────────────────────────────────
// `DefaultProviderResolver` gets a direct `ResolveProvider` impl so
// `agent.rs::AgentRuntime::with_resolver` can use the same trait object the
// production code path already uses — no extra adapter needed. The Phase 3
// `DefaultResolveAdapter` stays in place for backwards-compat with the
// `stream_agent_full` entry point (its `.inner.resolve(name)` call survives
// the addition because the inherent `resolve` method on
// `DefaultProviderResolver` is unchanged; this `impl` just adds a second
// route into the same lookup).
impl ResolveProvider for DefaultProviderResolver {
    fn resolve_by_name(&self, name: &str) -> Option<Arc<dyn LlmProvider>> {
        self.resolve(name)
    }
}

/// Wraps `DefaultProviderResolver` so it implements `ResolveProvider`.
struct DefaultResolveAdapter {
    inner: DefaultProviderResolver,
}

impl ResolveProvider for DefaultResolveAdapter {
    fn resolve_by_name(&self, name: &str) -> Option<Arc<dyn LlmProvider>> {
        self.inner.resolve(name)
    }
}

// ── Entry points ──────────────────────────────────────────────────────────

/// Full multi-turn streaming agent with tool support.
///
/// Builds messages the same way `agent.rs::run_inner` does, loops up to
/// `MAX_ROUNDS`, streams tokens from the LLM for each round, handles tool
/// calls inline and tracks costs via `cost_tracker` if provided.
///
/// Per-token callbacks can be received in two ways:
///   1. Via `on_event`: match `StreamEvent::Token { content }` in the callback.
///   2. Via the thin wrapper [`stream_agent`] which forwards only token
///      chunks to a simpler `on_token: impl Fn(&str)` callback.
pub async fn stream_agent_full<F>(
    config: &crate::config::AgentsConfig,
    agent_name: &str,
    prompt: &str,
    history: &[crate::providers::traits::ChatMessage],
    extra_context: Option<&str>,
    cost_tracker: Option<&crate::cost::CostTracker>,
    on_event: F,
) -> anyhow::Result<StreamResult>
where
    F: Fn(StreamEvent) + Send + Sync,
{
    // DEMO-1 gap 1 Phase 3: build the `LlmProvider` trait resolver once per
    // call, then delegate to the resolver-aware variant. Public API stays
    // unchanged for callers; tests use the `_with_resolver` variant to
    // inject a `MockResolver`.
    let resolver: Arc<dyn ResolveProvider> = Arc::new(DefaultResolveAdapter {
        inner: DefaultProviderResolver::from_config(config),
    });
    stream_agent_full_with_resolver(
        config,
        agent_name,
        prompt,
        history,
        extra_context,
        cost_tracker,
        resolver,
        on_event,
    )
    .await
}

/// Test-visible variant of [`stream_agent_full`] that lets the caller inject
/// a `ResolveProvider`. Production code path goes through `stream_agent_full`
/// which uses `DefaultProviderResolver`. Public so the
/// `streaming_trait_migration` integration tests can inject a mock; a future
/// Phase 5 `Agent::with_resolver` will use the same hook.
pub async fn stream_agent_full_with_resolver<F>(
    config: &crate::config::AgentsConfig,
    agent_name: &str,
    prompt: &str,
    history: &[crate::providers::traits::ChatMessage],
    extra_context: Option<&str>,
    cost_tracker: Option<&crate::cost::CostTracker>,
    resolver: Arc<dyn ResolveProvider>,
    on_event: F,
) -> anyhow::Result<StreamResult>
where
    F: Fn(StreamEvent) + Send + Sync,
{
    let start = Instant::now();

    let agent_cfg = config
        .agent
        .get(agent_name)
        .or_else(|| config.agent.get("master"))
        .cloned()
        .context("No agent configuration found. Check agents.toml.")?;

    // Build tool definitions list.
    let tool_defs: Vec<Value> = agent_cfg
        .tools
        .iter()
        .filter_map(|t| crate::tools::schema(t))
        .collect();

    // Build system prompt (mirrors agent.rs::run_inner exactly).
    let mut system = agent_cfg.instructions.clone();
    if !tool_defs.is_empty() {
        system.push_str(
            "\n\nCRITICAL RULES:\n\
            - You MUST call the appropriate tool function to perform any action. NEVER describe what you would do — just call the tool.\n\
            - To modify a file: call file_read first, then file_edit with exact old_string and new_string.\n\
            - To run a command: call shell with the exact command string.\n\
            - Never output code blocks as a substitute for calling a tool."
        );
    }
    let ws_ctx = crate::context::WorkspaceContext::capture().to_system_context();
    let combined_extra = match extra_context {
        Some(extra) if !extra.is_empty() => format!("{}\n\n{}", ws_ctx, extra),
        _ => ws_ctx,
    };
    if !combined_extra.is_empty() {
        system.push_str("\n\n");
        system.push_str(&combined_extra);
    }

    // Assemble initial message list.
    let mut messages: Vec<Value> = Vec::new();
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

    // Provider ordering — uses the same resolver as agent.rs's non-streaming
    // path so all four control surfaces work consistently for the streaming
    // chat reply (which is the most common code path):
    //   1. PHANTOM_RUNTIME_OVERRIDE env (set by /model X:Y in TUI)
    //   2. agent.providers list ([agent.X] providers = [...] in agents.toml)
    //   3. agent.provider singular field
    //   4. alphabetical fallback for the rest
    // Without this fix the streaming path was hardcoded to (provider + alphabetical),
    // so /model X:Y and the providers = [...] priority list were both no-ops
    // for chat replies — the auto-failover the user expected never happened.
    let mut provider_names = crate::agent::resolve_provider_order(
        &agent_cfg,
        config.providers.keys().map(|s| s.as_str()),
    );
    // Read both the env var (per-process) AND the file at
    // ~/.phantom-mesh/runtime-override (shared across phantom processes —
    // so /model X:Y in the TUI also affects the local `phantom serve`
    // and any subagent dispatched cluster-wide).
    if let Some(over) = crate::cli_config::read_runtime_override() {
        let trimmed = over.trim();
        if !trimmed.is_empty() {
            provider_names.retain(|n| n != trimmed);
            provider_names.insert(0, trimmed.to_string());
        }
    }

    let client = reqwest::Client::new();

    let mut all_tool_calls: Vec<Value> = Vec::new();
    let mut final_output = String::new();
    // Metrics accumulated across all rounds.
    let mut first_token_ms: u64 = 0;
    let mut tokens_received: usize = 0;
    // [F1] Anthropic prompt-cache token totals across all rounds (0 for non-Anthropic).
    let mut total_cache_read_tokens: u64 = 0;
    let mut total_cache_creation_tokens: u64 = 0;

    'rounds: for _round in 0..MAX_ROUNDS {
        // Try providers in order for this round.
        // `continue_rounds` is set true if we should proceed to the next round (tool calls handled).
        let mut continue_rounds = false;

        for (attempt, entry) in provider_names.iter().enumerate() {
            if attempt > 0 {
                let delay = 1u64 << (attempt - 1);
                tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
            }

            // Each entry can be bare `<provider>` or `<provider>:<model>`.
            // Same parser as agent.rs::call_with_fallback so the two paths
            // honor the user's selection identically.
            let (provider_name, entry_model) = crate::agent::parse_provider_entry(entry);

            let Some(provider) = config.providers.get(provider_name) else {
                continue;
            };

            let api_key = provider.api_key.clone().or_else(|| {
                provider
                    .api_key_env
                    .as_ref()
                    .and_then(|env| std::env::var(env).ok())
            });
            let Some(key) = api_key.filter(|k| !k.is_empty()) else {
                continue;
            };

            // Per-entry model overrides agent default + provider default.
            let model = entry_model
                .map(|m| m.to_string())
                .unwrap_or_else(|| resolve_stream_model(provider, &agent_cfg.model));

            // DEMO-1 gap 1 Phase 3: dispatch through the LlmProvider trait
            // for provider-type identification. Falls back to the legacy
            // string-compare if the resolver returns None (e.g. provider not
            // in the resolver's snapshot of the config, or trait dispatch
            // hasn't been wired in for this provider type yet).
            let llm_provider = resolver.resolve_by_name(provider_name);

            let result = stream_one_round(
                &client,
                provider,
                llm_provider.as_deref(),
                provider_name,
                &model,
                &system,
                &messages,
                &tool_defs,
                &key,
                &start,
                &on_event,
                &mut first_token_ms,
                &mut tokens_received,
            )
            .await;

            match result {
                Err(e) => {
                    tracing::warn!("Streaming provider {} failed: {}", provider_name, e);
                    // try next provider
                }
                Ok(RoundResult::TextOnly {
                    text,
                    prompt_tokens,
                    completion_tokens,
                    cache_read_tokens,
                    cache_creation_tokens,
                }) => {
                    if let Some(ct) = cost_tracker {
                        if prompt_tokens > 0 || completion_tokens > 0 {
                            ct.record(&model, prompt_tokens, completion_tokens).await;
                        }
                    }
                    total_cache_read_tokens =
                        total_cache_read_tokens.saturating_add(cache_read_tokens);
                    total_cache_creation_tokens =
                        total_cache_creation_tokens.saturating_add(cache_creation_tokens);
                    if !text.is_empty() {
                        final_output = text;
                    }
                    // No more rounds needed — the model returned plain text.
                    break 'rounds;
                }
                Ok(RoundResult::ToolCalls {
                    text,
                    tool_calls,
                    assistant_message,
                    prompt_tokens,
                    completion_tokens,
                    cache_read_tokens,
                    cache_creation_tokens,
                }) => {
                    if let Some(ct) = cost_tracker {
                        if prompt_tokens > 0 || completion_tokens > 0 {
                            ct.record(&model, prompt_tokens, completion_tokens).await;
                        }
                    }
                    total_cache_read_tokens =
                        total_cache_read_tokens.saturating_add(cache_read_tokens);
                    total_cache_creation_tokens =
                        total_cache_creation_tokens.saturating_add(cache_creation_tokens);
                    if !text.is_empty() {
                        final_output = text.clone();
                    }

                    // Append the assistant message (with tool_calls) to history.
                    messages.push(assistant_message);

                    // Execute each tool and append results.
                    for tc in &tool_calls {
                        let tc_id = tc["id"]
                            .as_str()
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| format!("call_{}", all_tool_calls.len()));
                        let fn_name = tc["function"]["name"]
                            .as_str()
                            .unwrap_or("unknown")
                            .to_string();
                        let fn_args: Value = tc["function"]["arguments"]
                            .as_str()
                            .and_then(|s| serde_json::from_str(s).ok())
                            .unwrap_or(Value::Object(serde_json::Map::new()));

                        let args_json = fn_args.to_string();
                        on_event(StreamEvent::ToolStart {
                            id: tc_id.clone(),
                            name: fn_name.clone(),
                            args_json,
                        });

                        let tool_start = Instant::now();
                        let output = crate::tools::execute(&fn_name, &fn_args, &config.tools).await;
                        let elapsed_ms = tool_start.elapsed().as_millis() as u64;
                        tracing::debug!("tool {} → {} chars", fn_name, output.len());

                        let result_preview: String = output.chars().take(100).collect();
                        on_event(StreamEvent::ToolDone {
                            id: tc_id.clone(),
                            name: fn_name.clone(),
                            result_preview,
                            elapsed_ms,
                        });

                        all_tool_calls.push(serde_json::json!({
                            "tool": fn_name,
                            "args": fn_args,
                        }));

                        messages.push(serde_json::json!({
                            "role": "tool",
                            "tool_call_id": tc_id,
                            "content": output,
                        }));
                    }

                    continue_rounds = true;
                    break; // done with provider fallback loop; continue to next round
                }
            }
        }

        if !continue_rounds {
            // Either all providers failed or we already broke 'rounds above.
            break;
        }
    }

    let total_tokens = (tokens_received as u32).saturating_add(0);
    on_event(StreamEvent::Done {
        total_tokens,
        cost_usd: 0.0,
    });

    tracing::info!(
        first_token_ms,
        tokens_received,
        elapsed_secs = start.elapsed().as_secs_f64(),
        "stream_agent_full complete"
    );

    Ok(StreamResult {
        output: final_output,
        tool_calls_made: all_tool_calls,
        elapsed_secs: start.elapsed().as_secs_f64(),
        cache_read_input_tokens: total_cache_read_tokens,
        cache_creation_input_tokens: total_cache_creation_tokens,
    })
}

/// Backward-compatible single-turn streaming entry point.
///
/// This is a thin wrapper around [`stream_agent_full`] that maps the old
/// `on_token` callback to [`StreamEvent::Token`].
pub async fn stream_agent(
    config: &AgentsConfig,
    agent_name: &str,
    prompt: &str,
    history: &[ChatMessage],
    extra_context: Option<&str>,
    on_token: impl Fn(&str) + Send + Sync,
) -> anyhow::Result<StreamResult> {
    stream_agent_full(
        config,
        agent_name,
        prompt,
        history,
        extra_context,
        None,
        move |event| {
            if let StreamEvent::Token { content } = event {
                on_token(&content);
            }
        },
    )
    .await
}

// ── Per-round streaming ───────────────────────────────────────────────────

/// Outcome of one streaming round.
enum RoundResult {
    TextOnly {
        text: String,
        prompt_tokens: u64,
        completion_tokens: u64,
        /// [F1] Anthropic prompt-cache token counts (0 for non-Anthropic providers).
        cache_read_tokens: u64,
        cache_creation_tokens: u64,
    },
    ToolCalls {
        text: String,
        tool_calls: Vec<Value>,
        /// The full assistant message JSON (including tool_calls array).
        assistant_message: Value,
        prompt_tokens: u64,
        completion_tokens: u64,
        /// [F1] Anthropic prompt-cache token counts (0 for non-Anthropic providers).
        cache_read_tokens: u64,
        cache_creation_tokens: u64,
    },
}

/// Stream one LLM round and collect either streamed text or tool calls.
///
/// Implements reconnect-on-disconnect: if the SSE byte stream drops before a
/// terminal sentinel, the request is re-issued up to [`MAX_RECONNECT_ATTEMPTS`]
/// times with a 500 ms pause between attempts.  When the server has supplied
/// `id:` fields in the stream, the last seen event-id is sent as
/// `Last-Event-ID` on reconnect so the server can resume from the correct
/// position.
///
/// In-stream error objects (`{"error": {...}}`) are detected and converted to
/// descriptive [`anyhow::Error`] values rather than being silently dropped.
#[allow(clippy::too_many_arguments)]
async fn stream_one_round<F>(
    client: &reqwest::Client,
    provider: &ProviderEntry,
    llm_provider: Option<&dyn LlmProvider>,
    provider_name: &str,
    model: &str,
    system: &str,
    messages: &[Value],
    tool_defs: &[Value],
    key: &str,
    run_start: &Instant,
    on_event: &F,
    first_token_ms: &mut u64,
    tokens_received: &mut usize,
) -> anyhow::Result<RoundResult>
where
    F: Fn(StreamEvent) + Send + Sync,
{
    // DEMO-1 gap 1 Phase 3: prefer the LlmProvider trait's `provider_type()`
    // for the wire-format decision. Phase 3 preserves the legacy semantics
    // exactly — only "anthropic" maps to the Anthropic wire format; other
    // types (including `claude_cli`, which speaks Anthropic protocol but
    // the legacy code routed via OpenAI-compat) stay where they were.
    // Fixing the `claude_cli` classification is left to Phase 4 along with
    // the other 2 call-path migrations.
    let is_anthropic = match llm_provider {
        Some(p) => p.provider_type() == "anthropic",
        None => provider.provider_type == "anthropic",
    };

    let (url, body) =
        build_request_body(provider, is_anthropic, model, system, messages, tool_defs);

    let mut last_event_id: Option<String> = None;
    let mut error_result: Option<anyhow::Error> = None;

    for reconnect_attempt in 0..=MAX_RECONNECT_ATTEMPTS {
        if reconnect_attempt > 0 {
            tracing::warn!(
                "[{}] SSE stream disconnected; reconnect attempt {}/{}",
                provider_name,
                reconnect_attempt,
                MAX_RECONNECT_ATTEMPTS
            );
            tokio::time::sleep(std::time::Duration::from_millis(RECONNECT_DELAY_MS)).await;
        }

        // ── Pre-stream retry loop (T19) ───────────────────────────────────
        //
        // Retry the request-establishment phase up to `cfg.max_retries`
        // times when we get 429 / 503 / network error and NOTHING has been
        // dispatched to the caller yet. Once we successfully receive a
        // response with `is_success()`, fall through to the existing SSE
        // consume loop below — its mid-stream reconnect semantics are
        // unchanged.
        //
        // Tests may override delays to keep wallclock fast. Production unset.
        let pre_stream_cfg = if std::env::var_os("PHANTOM_TEST_PRE_STREAM_FAST").is_some() {
            PreStreamRetryConfig {
                base_delay: std::time::Duration::from_millis(5),
                max_delay: std::time::Duration::from_millis(50),
                ..PreStreamRetryConfig::default()
            }
        } else {
            PreStreamRetryConfig::default()
        };
        // 1 initial + max_retries total attempts.
        let total_attempts = pre_stream_cfg.max_retries.saturating_add(1);

        let mut pre_last_status: Option<u16> = None;
        let mut pre_last_retry_after_secs: Option<u64> = None;
        let mut pre_last_body_excerpt: Option<String> = None;
        let mut pre_last_source: Option<String> = None;

        let mut resp_opt: Option<reqwest::Response> = None;
        for pre_attempt_idx in 0..total_attempts {
            let attempt_number = pre_attempt_idx + 1; // 1-based for humans
            let mut req = build_http_request(client, provider, &url, key, is_anthropic);

            // Pass Last-Event-ID on reconnect if we have one.
            if let Some(ref eid) = last_event_id {
                req = req.header("Last-Event-ID", eid.clone());
            }

            match req.json(&body).send().await {
                Ok(r) => {
                    let status = r.status();
                    if status.is_success() {
                        resp_opt = Some(r);
                        break;
                    }
                    let status_u16 = status.as_u16();
                    pre_last_status = Some(status_u16);
                    let retry_after_secs = r
                        .headers()
                        .get(reqwest::header::RETRY_AFTER)
                        .and_then(parse_retry_after_seconds);
                    pre_last_retry_after_secs = retry_after_secs;

                    // Capture body excerpt for diagnostics. This consumes the
                    // response, but we already know we're not going to stream it.
                    let text = r.text().await.unwrap_or_default();
                    pre_last_body_excerpt = Some(
                        crate::tools::floor_char_boundary(&text, pre_stream_cfg.body_excerpt_bytes)
                            .to_string(),
                    );

                    if !is_pre_stream_retryable_status(status_u16) {
                        // Terminal status — break out, error_result is set below.
                        break;
                    }
                    if pre_attempt_idx + 1 >= total_attempts {
                        // Exhausted — break out, error_result is set below.
                        break;
                    }
                    let sleep = compute_pre_stream_backoff(
                        &pre_stream_cfg,
                        pre_attempt_idx,
                        retry_after_secs.map(std::time::Duration::from_secs),
                        |_| rand::random::<f64>(),
                    );
                    tracing::info!(
                        provider = %provider_name,
                        attempt = attempt_number,
                        status_code = status_u16,
                        sleep_ms = sleep.as_millis() as u64,
                        "stream pre-stream retry: backing off on retryable status"
                    );
                    tokio::time::sleep(sleep).await;
                    continue;
                }
                Err(e) => {
                    pre_last_source = Some(format!("{}", e));
                    // Treat timeout / connect / generic request errors as
                    // retryable when nothing has streamed yet. Anything else
                    // (e.g. body-encoding error) is non-transient.
                    let transient = e.is_timeout() || e.is_connect() || e.is_request();
                    if !transient || pre_attempt_idx + 1 >= total_attempts {
                        break;
                    }
                    let sleep =
                        compute_pre_stream_backoff(&pre_stream_cfg, pre_attempt_idx, None, |_| {
                            rand::random::<f64>()
                        });
                    tracing::info!(
                        provider = %provider_name,
                        attempt = attempt_number,
                        sleep_ms = sleep.as_millis() as u64,
                        "stream pre-stream retry: backing off on transient network error"
                    );
                    tokio::time::sleep(sleep).await;
                    continue;
                }
            }
        }

        let resp = match resp_opt {
            Some(r) => r,
            None => {
                // All pre-stream attempts failed. Build a rich error and feed
                // it into the outer reconnect-attempt loop's `error_result`,
                // which mirrors the existing semantics: the round fails and
                // `stream_agent_full` then tries the next provider.
                let pse = PreStreamRetryError {
                    provider: provider_name.to_string(),
                    attempts: total_attempts,
                    last_status: pre_last_status,
                    last_retry_after_secs: pre_last_retry_after_secs,
                    last_body_excerpt: pre_last_body_excerpt,
                    last_source: pre_last_source,
                };
                error_result = Some(anyhow::anyhow!("{}", pse));
                continue;
            }
        };

        // --- Consume the SSE response body as a real byte stream ---
        // Each network chunk is appended to a line buffer.  Complete SSE frames
        // (delimited by "\n\n") are extracted and dispatched to `process_frame`
        // immediately, so `on_event(StreamEvent::Token {...})` fires as each chunk
        // arrives rather than in a burst after the full response completes.
        let mut byte_stream = resp.bytes_stream();

        let mut accumulated_text = String::new();
        let mut tool_calls_map: std::collections::BTreeMap<usize, ToolCallDelta> =
            std::collections::BTreeMap::new();
        let mut prompt_tokens: u64 = 0;
        let mut completion_tokens: u64 = 0;
        // [F1] Anthropic prompt-cache token counts (Anthropic GA 2026-02-19).
        // Updated by `process_anthropic_event` from message_start/message_delta usage blocks.
        let mut cache_read_tokens: u64 = 0;
        let mut cache_creation_tokens: u64 = 0;

        // `line_buf` accumulates raw bytes until we have complete SSE frames.
        let mut line_buf = String::new();
        // Track whether we cleanly finished (saw [DONE] or message_stop).
        let mut stream_finished = false;
        // Track whether the current provider is Anthropic — may be overridden by
        // runtime format detection once we see `content_block_delta` in the stream.
        let mut detected_anthropic = is_anthropic;
        // Track in-stream errors.
        let mut stream_error: Option<String> = None;

        'stream: while let Some(chunk_result) = byte_stream.next().await {
            let chunk = match chunk_result {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("[{}] stream read error: {}", provider_name, e);
                    break 'stream; // triggers reconnect
                }
            };
            line_buf.push_str(&String::from_utf8_lossy(&chunk));

            // Extract and process every complete SSE frame ("\n\n" separated).
            loop {
                if let Some(pos) = line_buf.find("\n\n") {
                    let frame: String = line_buf[..pos].to_string();
                    line_buf = line_buf[pos + 2..].to_string();

                    // Extract id: fields for reconnect support.
                    if let Some(eid) = extract_event_id(&frame) {
                        last_event_id = Some(eid);
                    }

                    // Runtime format detection: if we see content_block_delta,
                    // this is Anthropic format regardless of provider_type.
                    if !detected_anthropic && frame_is_anthropic(&frame) {
                        detected_anthropic = true;
                        tracing::debug!("[{}] auto-detected Anthropic SSE format", provider_name);
                    }

                    // Detect in-stream error objects before normal dispatch.
                    if let Some(err_msg) = extract_stream_error(&frame) {
                        stream_error = Some(err_msg);
                        stream_finished = true;
                        break 'stream;
                    }

                    let done = process_frame(
                        &frame,
                        detected_anthropic,
                        on_event,
                        &mut accumulated_text,
                        &mut tool_calls_map,
                        &mut prompt_tokens,
                        &mut completion_tokens,
                        &mut cache_read_tokens,
                        &mut cache_creation_tokens,
                        run_start,
                        first_token_ms,
                        tokens_received,
                    );
                    if done {
                        stream_finished = true;
                        break 'stream;
                    }
                } else {
                    break;
                }
            }
        }

        // Drain any trailing content (stream ended without a final "\n\n").
        let remainder = line_buf.trim().to_string();
        if !remainder.is_empty() {
            // Detect in-stream error in trailing content.
            if let Some(err_msg) = extract_stream_error(&remainder) {
                return Err(anyhow::anyhow!(
                    "[{}] in-stream error: {}",
                    provider_name,
                    err_msg
                ));
            }
            process_frame(
                &remainder,
                detected_anthropic,
                on_event,
                &mut accumulated_text,
                &mut tool_calls_map,
                &mut prompt_tokens,
                &mut completion_tokens,
                &mut cache_read_tokens,
                &mut cache_creation_tokens,
                run_start,
                first_token_ms,
                tokens_received,
            );
            stream_finished = true; // trailing content counts as a clean end
        }

        // Propagate in-stream errors.
        if let Some(err_msg) = stream_error {
            return Err(anyhow::anyhow!(
                "[{}] in-stream error: {}",
                provider_name,
                err_msg
            ));
        }

        // If stream finished cleanly, return results.
        if stream_finished || reconnect_attempt == MAX_RECONNECT_ATTEMPTS {
            // If no tool calls, return text.
            if tool_calls_map.is_empty() {
                return Ok(RoundResult::TextOnly {
                    text: accumulated_text,
                    prompt_tokens,
                    completion_tokens,
                    cache_read_tokens,
                    cache_creation_tokens,
                });
            }

            // Reconstruct tool_calls array from deltas.
            let mut tool_calls: Vec<Value> = Vec::new();
            for (_, delta) in tool_calls_map {
                tool_calls.push(serde_json::json!({
                    "id": delta.id,
                    "type": "function",
                    "function": {
                        "name": delta.name,
                        "arguments": delta.arguments,
                    }
                }));
            }

            // Build the assistant message that will be appended to history.
            let mut assistant_message = serde_json::json!({
                "role": "assistant",
                "tool_calls": tool_calls,
            });
            if !accumulated_text.is_empty() {
                assistant_message["content"] = Value::String(accumulated_text.clone());
            }

            return Ok(RoundResult::ToolCalls {
                text: accumulated_text,
                tool_calls: assistant_message["tool_calls"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default(),
                assistant_message,
                prompt_tokens,
                completion_tokens,
                cache_read_tokens,
                cache_creation_tokens,
            });
        }
        // Stream ended prematurely without clean finish — loop to reconnect.
    }

    // All reconnect attempts exhausted.
    Err(error_result.unwrap_or_else(|| {
        anyhow::anyhow!(
            "[{}] stream failed after {} reconnect attempts",
            provider_name,
            MAX_RECONNECT_ATTEMPTS
        )
    }))
}

// ── Request building helpers ──────────────────────────────────────────────

fn build_request_body(
    provider: &ProviderEntry,
    is_anthropic: bool,
    model: &str,
    system: &str,
    messages: &[Value],
    tool_defs: &[Value],
) -> (String, Value) {
    if is_anthropic {
        let url = streaming_url_anthropic(provider);

        // For native Anthropic Messages API, multimodal `image_url` parts must
        // be rewritten into Anthropic's `image` / `source` shape. Plain string
        // content and assistant tool_calls pass through unchanged.
        let user_messages: Vec<Value> = messages
            .iter()
            .filter(|m| m["role"].as_str() != Some("system"))
            .map(|m| crate::multimodal::convert_message_for_anthropic(m))
            .collect();

        let mut body = serde_json::json!({
            "model": model,
            "max_tokens": crate::config::default_max_tokens(),
            "stream": true,
            "messages": user_messages,
        });

        // [F1] Auto prompt caching (GA 2026-02-19): render system as a single
        // text block with cache_control so the entire system prompt is treated
        // as a cacheable prefix. Older Claude models that pre-date prompt
        // caching ignore the cache_control field gracefully (no 400).
        if !system.is_empty() {
            body["system"] = serde_json::json!([{
                "type": "text",
                "text": system,
                "cache_control": {"type": "ephemeral"},
            }]);
        }

        // Convert from OpenAI function schema to Anthropic tool schema, and
        // place a cache_control breakpoint on the LAST tool so tool defs +
        // system are cached together (Anthropic render order: tools → system).
        if !tool_defs.is_empty() {
            let last_idx = tool_defs.len() - 1;
            let anthropic_tools: Vec<Value> = tool_defs
                .iter()
                .enumerate()
                .map(|(i, td)| {
                    let mut tool = serde_json::json!({
                        "name": td["function"]["name"],
                        "description": td["function"]["description"],
                        "input_schema": td["function"]["parameters"],
                    });
                    if i == last_idx {
                        tool["cache_control"] = serde_json::json!({"type": "ephemeral"});
                    }
                    tool
                })
                .collect();
            body["tools"] = Value::Array(anthropic_tools);
        }

        // [F1] thinking.display "omitted" (Anthropic 2026-03-16): for Opus 4.7+,
        // request adaptive thinking with display=omitted so the model still
        // produces a signature we can persist but does not stream back the
        // (often large) thinking text. Older models 400 on `display`, so gate
        // strictly on the model name prefix.
        if model_supports_thinking_display_omitted(model) {
            body["thinking"] = serde_json::json!({
                "type": "adaptive",
                "display": "omitted",
            });
        }

        (url, body)
    } else {
        let url = streaming_url_openai(provider);

        let mut body = serde_json::json!({
            "model": model,
            "max_tokens": crate::config::default_max_tokens(),
            "stream": true,
            "messages": messages,
        });
        if !tool_defs.is_empty() {
            body["tools"] = Value::Array(tool_defs.to_vec());
            body["tool_choice"] = Value::String("auto".into());
            // Request usage data in stream final chunk.
            body["stream_options"] = serde_json::json!({"include_usage": true});
        }

        (url, body)
    }
}

fn build_http_request(
    client: &reqwest::Client,
    _provider: &ProviderEntry,
    url: &str,
    key: &str,
    is_anthropic: bool,
) -> reqwest::RequestBuilder {
    if is_anthropic {
        client
            .post(url)
            .header("x-api-key", key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
    } else {
        client
            .post(url)
            .header("Authorization", format!("Bearer {}", key))
            .header("content-type", "application/json")
    }
}

// ── SSE frame helpers ─────────────────────────────────────────────────────

/// Extract the `id:` field value from an SSE frame, if present.
fn extract_event_id(frame: &str) -> Option<String> {
    for line in frame.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("id:") {
            let id = rest.trim().to_string();
            if !id.is_empty() {
                return Some(id);
            }
        }
    }
    None
}

/// Detect whether an SSE frame looks like Anthropic Messages API format
/// by checking for `content_block_delta` event type or data type.
fn frame_is_anthropic(frame: &str) -> bool {
    for line in frame.lines() {
        let line = line.trim();
        if line == "event: content_block_delta"
            || line == "event: content_block_start"
            || line == "event: message_start"
            || line == "event: message_stop"
            || line == "event: message_delta"
        {
            return true;
        }
        // Also detect from the data payload's `type` field.
        if let Some(data) = line.strip_prefix("data:") {
            let payload = data.trim();
            if payload.contains("\"content_block_delta\"")
                || payload.contains("\"message_start\"")
                || payload.contains("\"content_block_start\"")
            {
                return true;
            }
        }
    }
    false
}

/// Detect an in-stream error object and return the error message string.
///
/// Handles both `{"error": {"message": "..."}}` (OpenAI) and
/// `{"type": "error", "error": {"message": "..."}}` (Anthropic).
fn extract_stream_error(frame: &str) -> Option<String> {
    for line in frame.lines() {
        let line = line.trim();
        let payload = if let Some(rest) = line.strip_prefix("data:") {
            rest.trim()
        } else if line.starts_with('{') {
            // Bare JSON line (e.g. trailing content without data: prefix).
            line
        } else {
            continue;
        };

        if payload.is_empty() || payload == "[DONE]" {
            continue;
        }

        let Ok(json) = serde_json::from_str::<Value>(payload) else {
            continue;
        };

        // OpenAI style: top-level "error" object.
        if let Some(err_obj) = json.get("error") {
            if err_obj.is_object() || err_obj.is_string() {
                let msg = err_obj["message"]
                    .as_str()
                    .map(str::to_string)
                    .or_else(|| err_obj.as_str().map(str::to_string))
                    .unwrap_or_else(|| err_obj.to_string());
                return Some(msg);
            }
        }

        // Anthropic style: {"type": "error", "error": {...}}.
        if json["type"].as_str() == Some("error") {
            if let Some(err_obj) = json.get("error") {
                let msg = err_obj["message"]
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| err_obj.to_string());
                return Some(msg);
            }
        }
    }
    None
}

// ── Streaming delta accumulation ──────────────────────────────────────────

/// Partial tool-call state accumulated across stream deltas.
#[derive(Default)]
struct ToolCallDelta {
    id: String,
    name: String,
    arguments: String,
}

/// Process one SSE frame (the content between two `\n\n` separators).
///
/// Robustness guarantees:
/// - `data: [DONE]` ends the stream without error.
/// - Empty `data:` lines (heartbeats / keep-alives) are silently skipped.
/// - `event:` lines carry the Anthropic event type; the companion `data:` line
///   is the JSON payload. The function respects both and never panics on
///   malformed input — parse errors are logged at DEBUG level and skipped.
/// - Multi-line JSON assembled inside a single frame (lines without a `data:`
///   prefix following a `data:` opener) is concatenated before parsing.
/// - Returns `true` if a `[DONE]` sentinel was encountered so the caller can
///   stop consuming the stream early.
#[allow(clippy::too_many_arguments)]
fn process_frame<F>(
    frame: &str,
    is_anthropic: bool,
    on_event: &F,
    accumulated_text: &mut String,
    tool_calls_map: &mut std::collections::BTreeMap<usize, ToolCallDelta>,
    prompt_tokens: &mut u64,
    completion_tokens: &mut u64,
    cache_read_tokens: &mut u64,
    cache_creation_tokens: &mut u64,
    run_start: &Instant,
    first_token_ms: &mut u64,
    tokens_received: &mut usize,
) -> bool
// returns true on [DONE]
where
    F: Fn(StreamEvent) + Send + Sync,
{
    // Within a single SSE frame we may see:
    //   event: content_block_delta        ← Anthropic type hint (optional)
    //   data: {"type": ...}               ← JSON payload (may span multiple continuation lines)
    //
    // We collect the current `data:` payload across continuation lines so that
    // a hypothetical multi-line JSON object is assembled before being parsed.
    let mut current_event_type: Option<String> = None;
    let mut pending_data: Option<String> = None;

    // Helper: dispatch a fully-assembled `data:` payload.
    macro_rules! dispatch {
        ($payload:expr) => {{
            let payload: &str = $payload;
            // Empty lines are heartbeats — skip silently.
            if payload.is_empty() {
                // nothing
            } else if payload == "[DONE]" {
                return true;
            } else {
                match serde_json::from_str::<Value>(payload) {
                    Ok(json) => {
                        let _ = current_event_type.take(); // consumed
                        if is_anthropic {
                            process_anthropic_event(
                                &json,
                                on_event,
                                accumulated_text,
                                tool_calls_map,
                                prompt_tokens,
                                completion_tokens,
                                cache_read_tokens,
                                cache_creation_tokens,
                                run_start,
                                first_token_ms,
                                tokens_received,
                            );
                        } else {
                            process_openai_event(
                                &json,
                                on_event,
                                accumulated_text,
                                tool_calls_map,
                                prompt_tokens,
                                completion_tokens,
                                run_start,
                                first_token_ms,
                                tokens_received,
                            );
                        }
                    }
                    Err(e) => {
                        tracing::debug!(
                            "SSE JSON parse error (skipping): {} — payload: {:.120}",
                            e,
                            payload
                        );
                    }
                }
            }
        }};
    }

    for raw_line in frame.lines() {
        let line = raw_line.trim();

        if line.is_empty() {
            // Blank line inside a frame: flush any pending data.
            if let Some(data) = pending_data.take() {
                dispatch!(&data);
            }
            continue;
        }

        if let Some(rest) = line.strip_prefix("event:") {
            // Flush any prior pending data before switching event context.
            if let Some(data) = pending_data.take() {
                dispatch!(&data);
            }
            current_event_type = Some(rest.trim().to_string());
            continue;
        }

        if let Some(rest) = line.strip_prefix("data:") {
            // Flush previous data line if present (shouldn't normally happen
            // inside a single frame, but be safe).
            if let Some(data) = pending_data.take() {
                dispatch!(&data);
            }
            pending_data = Some(rest.trim().to_string());
            continue;
        }

        // Continuation line: append to the current data payload.
        if let Some(ref mut data) = pending_data {
            data.push_str(line);
        }
        // Lines that are neither `event:`, `data:`, nor continuations
        // (e.g. `id:`, `retry:`, comments starting with `:`) are ignored.
    }

    // Flush anything left at end of frame.
    if let Some(data) = pending_data.take() {
        dispatch!(&data);
    }

    false // no [DONE] encountered
}

#[allow(clippy::too_many_arguments)]
fn process_openai_event<F>(
    json: &Value,
    on_event: &F,
    accumulated_text: &mut String,
    tool_calls_map: &mut std::collections::BTreeMap<usize, ToolCallDelta>,
    prompt_tokens: &mut u64,
    completion_tokens: &mut u64,
    run_start: &Instant,
    first_token_ms: &mut u64,
    tokens_received: &mut usize,
) where
    F: Fn(StreamEvent) + Send + Sync,
{
    // Usage data arrives in a final chunk (when stream_options.include_usage is set).
    if let Some(usage) = json.get("usage") {
        if let Some(pt) = usage["prompt_tokens"].as_u64() {
            *prompt_tokens = pt;
        }
        if let Some(ct) = usage["completion_tokens"].as_u64() {
            *completion_tokens = ct;
        }
    }

    // Guard: choices array may be absent or empty on usage-only chunks.
    let choice = match json["choices"].as_array().and_then(|a| a.first()) {
        Some(c) => c,
        None => return,
    };

    // Emit a trace when the model signals completion.
    if choice["finish_reason"].as_str() == Some("stop") {
        tracing::debug!("OpenAI stream: finish_reason=stop");
        // No token to emit; the stream will send [DONE] shortly after.
    }

    let delta = &choice["delta"];
    if delta.is_null() || !delta.is_object() {
        return;
    }

    // Reasoning trace: opencode/groq/openrouter expose chain-of-thought via
    // either `delta.reasoning` (string) or `delta.reasoning_content` (string).
    // Some providers also send a `reasoning` object {content: "..."} — handle both.
    for k in ["reasoning", "reasoning_content"] {
        match &delta[k] {
            Value::String(s) if !s.is_empty() => {
                on_event(StreamEvent::Thinking { content: s.clone() });
            }
            Value::Object(o) => {
                if let Some(s) = o.get("content").and_then(|v| v.as_str()) {
                    if !s.is_empty() {
                        on_event(StreamEvent::Thinking {
                            content: s.to_string(),
                        });
                    }
                }
            }
            _ => {}
        }
    }

    // Text content chunk.  `delta.content` may be absent (tool-call chunks)
    // or explicitly null (role-only chunks) — only emit when it's a non-empty string.
    if let Some(content) = delta["content"].as_str() {
        if !content.is_empty() {
            // Record first-token latency on the very first token.
            if *tokens_received == 0 && *first_token_ms == 0 {
                *first_token_ms = run_start.elapsed().as_millis() as u64;
            }
            *tokens_received += 1;
            on_event(StreamEvent::Token {
                content: content.to_string(),
            });
            accumulated_text.push_str(content);
        }
    }

    // Tool call deltas.
    if let Some(tc_deltas) = delta["tool_calls"].as_array() {
        for tc_delta in tc_deltas {
            let idx = tc_delta["index"].as_u64().unwrap_or(0) as usize;
            let entry = tool_calls_map.entry(idx).or_default();

            if let Some(id) = tc_delta["id"].as_str() {
                if !id.is_empty() {
                    entry.id = id.to_string();
                }
            }
            if let Some(name) = tc_delta["function"]["name"].as_str() {
                if !name.is_empty() {
                    entry.name = name.to_string();
                }
            }
            if let Some(args) = tc_delta["function"]["arguments"].as_str() {
                entry.arguments.push_str(args);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn process_anthropic_event<F>(
    json: &Value,
    on_event: &F,
    accumulated_text: &mut String,
    tool_calls_map: &mut std::collections::BTreeMap<usize, ToolCallDelta>,
    prompt_tokens: &mut u64,
    completion_tokens: &mut u64,
    cache_read_tokens: &mut u64,
    cache_creation_tokens: &mut u64,
    run_start: &Instant,
    first_token_ms: &mut u64,
    tokens_received: &mut usize,
) where
    F: Fn(StreamEvent) + Send + Sync,
{
    match json["type"].as_str() {
        Some("message_start") => {
            // Input tokens come here.
            let usage = &json["message"]["usage"];
            *prompt_tokens = usage["input_tokens"].as_u64().unwrap_or(0);
            // [F1] Capture prompt-cache token counts (Anthropic GA 2026-02-19).
            // Both fields are absent on older models that don't support
            // prompt caching, in which case `as_u64()` returns None and we
            // leave the existing accumulator unchanged.
            *cache_read_tokens = usage["cache_read_input_tokens"]
                .as_u64()
                .unwrap_or(*cache_read_tokens);
            *cache_creation_tokens = usage["cache_creation_input_tokens"]
                .as_u64()
                .unwrap_or(*cache_creation_tokens);
        }
        Some("message_delta") => {
            // Output tokens come here.
            *completion_tokens = json["usage"]["output_tokens"].as_u64().unwrap_or(0);
            // Some Anthropic models also emit cache token updates on message_delta.
            if let Some(v) = json["usage"]["cache_read_input_tokens"].as_u64() {
                *cache_read_tokens = v;
            }
            if let Some(v) = json["usage"]["cache_creation_input_tokens"].as_u64() {
                *cache_creation_tokens = v;
            }
        }
        Some("content_block_start") => {
            // Tool use block starts here for Anthropic.
            let block = &json["content_block"];
            if block["type"].as_str() == Some("tool_use") {
                let idx = json["index"].as_u64().unwrap_or(0) as usize;
                let entry = tool_calls_map.entry(idx).or_default();
                if let Some(id) = block["id"].as_str() {
                    entry.id = id.to_string();
                }
                if let Some(name) = block["name"].as_str() {
                    entry.name = name.to_string();
                }
            }
        }
        Some("content_block_delta") => {
            let delta = &json["delta"];
            match delta["type"].as_str() {
                Some("text_delta") => {
                    if let Some(text) = delta["text"].as_str() {
                        if !text.is_empty() {
                            // Record first-token latency on the very first token.
                            if *tokens_received == 0 && *first_token_ms == 0 {
                                *first_token_ms = run_start.elapsed().as_millis() as u64;
                            }
                            *tokens_received += 1;
                            on_event(StreamEvent::Token {
                                content: text.to_string(),
                            });
                            accumulated_text.push_str(text);
                        }
                    }
                }
                Some("thinking_delta") => {
                    // Extended thinking content from models that support it.
                    if let Some(thinking) = delta["thinking"].as_str() {
                        if !thinking.is_empty() {
                            on_event(StreamEvent::Thinking {
                                content: thinking.to_string(),
                            });
                        }
                    }
                }
                Some("input_json_delta") => {
                    // Streamed tool input JSON.
                    let idx = json["index"].as_u64().unwrap_or(0) as usize;
                    let entry = tool_calls_map.entry(idx).or_default();
                    if let Some(partial) = delta["partial_json"].as_str() {
                        entry.arguments.push_str(partial);
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }
}

// ── args_preview helper ───────────────────────────────────────────────────

#[allow(dead_code)]
fn make_args_preview(fn_name: &str, fn_args: &Value) -> String {
    match fn_name {
        "shell" => {
            let cmd = fn_args["command"].as_str().unwrap_or("");
            truncate_chars(cmd, 80)
        }
        "file_read" | "file_write" | "file_edit" | "list_dir" | "list_files" | "delete_file"
        | "create_dir" => fn_args["path"].as_str().unwrap_or("").to_string(),
        _ => {
            let s = fn_args.to_string();
            truncate_chars(&s, 80)
        }
    }
}

/// Truncate a string to at most `max_chars` Unicode scalar values, appending
/// `…` if truncation occurred.
#[allow(dead_code)]
fn truncate_chars(s: &str, max_chars: usize) -> String {
    let mut chars = s.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        // There were more characters — append ellipsis.
        format!("{}…", truncated)
    } else {
        truncated
    }
}

// ── Backward-compat SSE frame parsing (used by tests) ────────────────────

#[cfg(test)]
fn parse_sse_frame(frame: &str, is_anthropic: bool) -> Option<String> {
    for line in frame.lines() {
        let line = line.trim();
        if !line.starts_with("data:") {
            continue;
        }
        let payload = line["data:".len()..].trim();
        if payload == "[DONE]" {
            return None;
        }
        let Ok(json) = serde_json::from_str::<Value>(payload) else {
            continue;
        };
        if is_anthropic {
            return parse_anthropic_delta(&json);
        } else {
            return parse_openai_delta(&json);
        }
    }
    None
}

#[cfg(test)]
fn parse_anthropic_delta(json: &Value) -> Option<String> {
    if json["type"].as_str() == Some("content_block_delta")
        && json["delta"]["type"].as_str() == Some("text_delta")
    {
        json["delta"]["text"].as_str().map(str::to_string)
    } else {
        None
    }
}

#[cfg(test)]
fn parse_openai_delta(json: &Value) -> Option<String> {
    json["choices"][0]["delta"]["content"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

// ── URL helpers ───────────────────────────────────────────────────────────

/// Resolve the model name to use, applying provider-specific defaults.
/// Falls back to `"minimax-m2.5-free"` for opencode.ai when no model is
/// explicitly configured — that's the cheapest tier-0 free model still
/// accepted as of 2026-04. The earlier hard-default of `claude-sonnet-4-5`
/// is in opencode's PAID tier and produced "Model not supported" errors
/// for users without a payment method on file (B1 root cause).
///
/// Other free-tier opencode models (queryable at GET https://opencode.ai/zen/v1/models):
///   minimax-m2.5-free, nemotron-3-super-free, ling-2.6-flash-free, hy3-preview-free
/// Paid tier (require payment method): claude-sonnet-4-5/6, claude-opus-4-5/6/7, gpt-*, gemini-*
fn resolve_stream_model(provider: &ProviderEntry, fallback: &str) -> String {
    if let Some(m) = &provider.default_model {
        if !m.is_empty() {
            return m.clone();
        }
    }
    if provider.provider_type == "opencode"
        || provider
            .url
            .as_deref()
            .unwrap_or("")
            .contains("opencode.ai")
    {
        return "minimax-m2.5-free".into();
    }
    fallback.to_string()
}

/// [F1] Returns true for Claude Opus 4.7 and later models that support the
/// `thinking.display` field (Anthropic 2026-03-16). Older Claude models 400
/// on `display`, so this gate keeps the request body backwards-compatible
/// for Sonnet 4.6, Opus 4.6, Haiku 4.5, and earlier.
///
/// Conservative match: only models whose normalized ID starts with
/// `claude-opus-4-7`, `claude-opus-4-8`, `claude-opus-4-9`, or `claude-opus-5`.
/// When future versions add support, extend this list.
fn model_supports_thinking_display_omitted(model: &str) -> bool {
    let m = model.to_lowercase();
    m.starts_with("claude-opus-4-7")
        || m.starts_with("claude-opus-4-8")
        || m.starts_with("claude-opus-4-9")
        || m.starts_with("claude-opus-5")
}

fn streaming_url_anthropic(provider: &ProviderEntry) -> String {
    if let Some(explicit) = &provider.url {
        let base = explicit.trim_end_matches('/');
        if base.ends_with("/v1/messages") {
            return base.to_string();
        }
        let base = base
            .trim_end_matches("/v1/chat/completions")
            .trim_end_matches('/');
        return format!("{}/v1/messages", base);
    }
    "https://api.anthropic.com/v1/messages".into()
}

fn streaming_url_openai(provider: &ProviderEntry) -> String {
    if let Some(explicit) = &provider.url {
        let base = explicit.trim_end_matches('/');
        if base.ends_with("/chat/completions") {
            return base.to_string();
        }
        // If the URL already ends with /v1, only append /chat/completions to
        // avoid a double-/v1 path (e.g. "https://opencode.ai/zen/v1").
        if base.ends_with("/v1") {
            return format!("{}/chat/completions", base);
        }
        return format!("{}/v1/chat/completions", base);
    }
    match provider.provider_type.as_str() {
        "openai" => "https://api.openai.com/v1/chat/completions".into(),
        "groq" => "https://api.groq.com/openai/v1/chat/completions".into(),
        "gemini" => {
            "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions".into()
        }
        "opencode" => "https://opencode.ai/zen/v1/chat/completions".into(),
        // V1 (2026-05-21): canonical native endpoint per provider so the
        // openrouter fallback below doesn't silently catch a known type.
        // Surfaced during V1 verification: an agent configured with
        // `provider = "mistral"` (and a valid MISTRAL_API_KEY) was hitting
        // openrouter.ai/api/v1/chat/completions and 401-ing because the
        // Bearer token isn't valid there.
        //
        // URLs mirror the `DEFAULT_BASE_URL` + path suffix defined in each
        // `core/src/providers/<name>.rs::streaming_url()`. We hardcode here
        // instead of delegating to those functions because the
        // `core/src/providers/<name>` modules are gated behind the
        // `experimental-hermes-providers` cargo feature — they aren't
        // compiled into the default `phantom` binary, which is the build
        // path the V1 round-trip exercise was using when it tripped this
        // bug. Keep the strings in sync with each module's
        // `DEFAULT_BASE_URL` + `streaming_url()` path-suffix logic; the
        // pin-tests below catch drift.
        "mistral" => "https://api.mistral.ai/v1/chat/completions".into(),
        "together" => "https://api.together.xyz/v1/chat/completions".into(),
        "nvidia" => "https://integrate.api.nvidia.com/v1/chat/completions".into(),
        "fireworks" => "https://api.fireworks.ai/inference/v1/chat/completions".into(),
        "xai" => "https://api.x.ai/v1/chat/completions".into(),
        "ai21" => "https://api.ai21.com/studio/v1/chat/completions".into(),
        // Perplexity does NOT use a /v1/ path segment — endpoint lives at
        // bare /chat/completions. Matches providers/perplexity.rs.
        "perplexity" => "https://api.perplexity.ai/chat/completions".into(),
        // Cohere uses its native /v1/chat (NOT /chat/completions) — see
        // the comment block in providers/cohere.rs::streaming_url.
        "cohere" => "https://api.cohere.com/v1/chat".into(),
        _ => "https://openrouter.ai/api/v1/chat/completions".into(),
    }
}

// ── Cooperative cancellation helper ───────────────────────────────────────
//
// `agent.rs` already races each SSE chunk against `InterruptHandle::cancelled()`
// inside the streaming loop (see agent.rs:1180-1196). This helper extracts the
// pattern into a small, unit-testable utility so we can pin the behaviour: when
// the user hits ESC in the TUI, the streaming dispatcher must unwind within a
// bounded latency (well under the next-token gap) instead of waiting for the
// model to emit another chunk.
//
// V1-track P0 (12-provider production default) requires this to be deterministic.

/// Outcome of [`next_or_interrupt`].
#[derive(Debug)]
pub enum InterruptibleNext<T> {
    /// The stream produced the next item before the interrupt fired.
    Item(T),
    /// The interrupt handle was cancelled before the next item arrived.
    Cancelled,
    /// The stream ended (no more items).
    End,
}

/// Await the next item from `stream`, or return [`InterruptibleNext::Cancelled`]
/// as soon as `interrupt` is fired.
///
/// The race is `biased` toward the interrupt: if both are ready in the same
/// poll, cancellation wins. This guarantees that pressing ESC during a slow
/// stream (e.g. one token per 100 ms) returns immediately instead of waiting
/// for the next token.
///
/// Used by `agent.rs` indirectly via the same `tokio::select!` shape; this
/// helper exists so the cancellation latency can be measured in isolation.
pub async fn next_or_interrupt<S>(
    stream: &mut S,
    interrupt: &crate::interrupt::InterruptHandle,
) -> InterruptibleNext<S::Item>
where
    S: futures::Stream + Unpin,
{
    tokio::select! {
        biased;
        _ = interrupt.cancelled() => InterruptibleNext::Cancelled,
        next = stream.next() => match next {
            Some(item) => InterruptibleNext::Item(item),
            None => InterruptibleNext::End,
        },
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interrupt::InterruptHandle;

    /// V1 P0 cancellation contract: when the user hits ESC in the TUI,
    /// `next_or_interrupt` must return `Cancelled` within a bounded latency,
    /// without waiting for the next token. The cancellation latency must be
    /// much smaller than the inter-token gap.
    ///
    /// Setup: a fake SSE stream emits one token every 100 ms. We fire the
    /// interrupt (mirrors what `tui.rs` does on KeyCode::Esc → `pending_interrupt`
    /// → `InterruptHandle::interrupt(None)`) at t≈10 ms while `next_or_interrupt`
    /// is awaiting the second token, and assert the helper resolves within 50 ms
    /// of the interrupt — i.e. comfortably under the 100 ms next-token gap.
    ///
    /// Uses real timers (no `tokio::time::pause()`) so we exercise the actual
    /// `tokio::select!` wakeup path; 50 ms is generous on node-a. If this ever
    /// regresses we'd see ESC feel laggy in the TUI even on local providers.
    #[tokio::test]
    async fn cancellation_via_esc_returns_immediately() {
        use futures::stream;
        use std::time::Duration;
        use tokio::time::Instant as TokioInstant;

        // Fake SSE stream: one token per 100 ms.
        let token_gap = Duration::from_millis(100);
        let mut slow_stream = Box::pin(stream::unfold(0u32, move |i| async move {
            if i >= 10 {
                None
            } else {
                tokio::time::sleep(token_gap).await;
                Some((format!("tok{}", i), i + 1))
            }
        }));

        let interrupt = InterruptHandle::new();
        let interrupt_fire = interrupt.clone();

        // Drain one token to confirm the stream actually streams, then race the
        // next read against the ESC interrupt fired ~10 ms later.
        match next_or_interrupt(&mut slow_stream, &interrupt).await {
            InterruptibleNext::Item(s) => assert_eq!(s, "tok0"),
            other => panic!("expected first token before any cancel, got {:?}", other),
        }

        // Fire the interrupt from a background task while the main task is
        // blocked inside next_or_interrupt awaiting tok1.
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            interrupt_fire.interrupt(None);
        });

        let started = TokioInstant::now();
        let outcome = next_or_interrupt(&mut slow_stream, &interrupt).await;
        let elapsed = started.elapsed();

        match outcome {
            InterruptibleNext::Cancelled => {}
            other => panic!("expected Cancelled, got {:?}", other),
        }

        // Latency budget: 10 ms scheduling delay + ESC dispatch must complete
        // well under the 100 ms token gap. 50 ms is the V1 ship threshold on
        // node-a (`docs/tdd/INDEX.md` line 39).
        assert!(
            elapsed < Duration::from_millis(50),
            "cancellation took {} ms (must be < 50 ms; token gap is 100 ms)",
            elapsed.as_millis()
        );
        // And — crucially — the helper must return BEFORE the next token would
        // have been emitted. Otherwise we'd be passing this test merely because
        // 50 ms < some larger budget, and ESC would still feel laggy.
        assert!(
            elapsed < token_gap,
            "cancellation latency ({} ms) reached the next-token gap ({} ms) — \
             the helper is waiting for the next token instead of unwinding immediately",
            elapsed.as_millis(),
            token_gap.as_millis(),
        );

        assert!(interrupt.is_cancelled());
    }

    // Thin helper that wraps the new process_frame signature for the legacy
    // unit tests that don't care about metrics.
    fn run_frame(frame: &str, is_anthropic: bool) -> (String, bool, Vec<String>) {
        use std::sync::{Arc, Mutex};
        let mut text = String::new();
        let mut tc_map = std::collections::BTreeMap::new();
        let mut pt = 0u64;
        let mut ct = 0u64;
        let mut crt = 0u64;
        let mut cct = 0u64;
        let mut ftms = 0u64;
        let mut tr = 0usize;
        let start = Instant::now();
        let tokens: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let tokens_clone = tokens.clone();

        let done = process_frame(
            frame,
            is_anthropic,
            &move |event: StreamEvent| {
                if let StreamEvent::Token { content } = event {
                    tokens_clone.lock().unwrap().push(content);
                }
            },
            &mut text,
            &mut tc_map,
            &mut pt,
            &mut ct,
            &mut crt,
            &mut cct,
            &start,
            &mut ftms,
            &mut tr,
        );

        let collected = tokens.lock().unwrap().clone();
        (text, done, collected)
    }

    #[test]
    fn parse_anthropic_text_delta() {
        let frame =
            r#"data: {"type":"content_block_delta","delta":{"type":"text_delta","text":"hello"}}"#;
        let result = parse_sse_frame(frame, true);
        assert_eq!(result, Some("hello".into()));
    }

    #[test]
    fn parse_anthropic_non_delta_event() {
        let frame = r#"data: {"type":"message_start","message":{"id":"x"}}"#;
        let result = parse_sse_frame(frame, true);
        assert!(result.is_none());
    }

    #[test]
    fn parse_openai_delta() {
        let frame = r#"data: {"choices":[{"delta":{"content":"world"}}]}"#;
        let result = parse_sse_frame(frame, false);
        assert_eq!(result, Some("world".into()));
    }

    #[test]
    fn parse_openai_done_sentinel() {
        let frame = "data: [DONE]";
        let result = parse_sse_frame(frame, false);
        assert!(result.is_none());
    }

    #[test]
    fn parse_openai_empty_content() {
        let frame = r#"data: {"choices":[{"delta":{"role":"assistant","content":""}}]}"#;
        let result = parse_sse_frame(frame, false);
        assert!(result.is_none());
    }

    #[test]
    fn streaming_url_anthropic_default() {
        let p = ProviderEntry {
            provider_type: "anthropic".into(),
            url: None,
            ..Default::default()
        };
        assert_eq!(
            streaming_url_anthropic(&p),
            "https://api.anthropic.com/v1/messages"
        );
    }

    #[test]
    fn streaming_url_anthropic_strips_chat_completions() {
        let p = ProviderEntry {
            provider_type: "anthropic".into(),
            url: Some("https://custom.example.com/v1/chat/completions".into()),
            ..Default::default()
        };
        assert_eq!(
            streaming_url_anthropic(&p),
            "https://custom.example.com/v1/messages"
        );
    }

    #[test]
    fn streaming_url_openai_groq() {
        let p = ProviderEntry {
            provider_type: "groq".into(),
            url: None,
            ..Default::default()
        };
        assert_eq!(
            streaming_url_openai(&p),
            "https://api.groq.com/openai/v1/chat/completions"
        );
    }

    #[test]
    fn args_preview_shell_truncates() {
        let long_cmd = "a".repeat(100);
        let args = serde_json::json!({"command": long_cmd});
        let preview = make_args_preview("shell", &args);
        // 80 'a' chars + 1 '…' char = 81 Unicode scalar values
        assert_eq!(preview.chars().count(), 81);
        assert!(preview.ends_with('…'));
    }

    #[test]
    fn args_preview_file_shows_path() {
        let args = serde_json::json!({"path": "/tmp/foo.rs"});
        let preview = make_args_preview("file_read", &args);
        assert_eq!(preview, "/tmp/foo.rs");
    }

    #[test]
    fn args_preview_unknown_shows_json() {
        let args = serde_json::json!({"query": "test"});
        let preview = make_args_preview("web_search", &args);
        assert!(!preview.is_empty());
    }

    /// Verify that `process_frame` correctly accumulates text from an OpenAI-
    /// format SSE delta frame.
    #[test]
    fn process_frame_accumulates_openai_text() {
        let (text, done, tokens) = run_frame(
            r#"data: {"choices":[{"delta":{"content":"Hello"}}]}"#,
            false,
        );
        assert_eq!(text, "Hello");
        assert!(!done);
        assert_eq!(tokens, vec!["Hello".to_string()]);
    }

    /// `data: [DONE]` must return done=true and emit nothing.
    #[test]
    fn process_frame_done_sentinel() {
        let (text, done, tokens) = run_frame("data: [DONE]", false);
        assert!(done);
        assert!(text.is_empty());
        assert!(tokens.is_empty());
    }

    /// Empty `data:` line (heartbeat) is silently skipped.
    #[test]
    fn process_frame_heartbeat_skipped() {
        let (text, done, tokens) = run_frame("data:", false);
        assert!(!done);
        assert!(text.is_empty());
        assert!(tokens.is_empty());
    }

    /// `delta.content` = null must not emit a token.
    #[test]
    fn process_frame_null_content_skipped() {
        let (text, done, tokens) =
            run_frame(r#"data: {"choices":[{"delta":{"content":null}}]}"#, false);
        assert!(!done);
        assert!(text.is_empty());
        assert!(tokens.is_empty());
    }

    /// `finish_reason = "stop"` must not panic and must not emit a token.
    #[test]
    fn process_frame_finish_reason_stop() {
        let (text, done, tokens) = run_frame(
            r#"data: {"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
            false,
        );
        assert!(!done);
        assert!(text.is_empty());
        assert!(tokens.is_empty());
    }

    /// Malformed JSON is skipped without panicking.
    #[test]
    fn process_frame_malformed_json_no_panic() {
        let (text, done, tokens) = run_frame("data: {not valid json!!}", false);
        assert!(!done);
        assert!(text.is_empty());
        assert!(tokens.is_empty());
    }

    /// Anthropic `event:` + `data:` pair is parsed correctly.
    #[test]
    fn process_frame_anthropic_event_data_pair() {
        let frame = "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"world\"}}";
        let (text, done, tokens) = run_frame(frame, true);
        assert_eq!(text, "world");
        assert!(!done);
        assert_eq!(tokens, vec!["world".to_string()]);
    }

    /// Choices array absent (usage-only chunk) must not panic.
    #[test]
    fn process_frame_no_choices_no_panic() {
        let (text, done, tokens) = run_frame(
            r#"data: {"usage":{"prompt_tokens":10,"completion_tokens":5}}"#,
            false,
        );
        assert!(!done);
        assert!(text.is_empty());
        assert!(tokens.is_empty());
    }

    // ── New feature tests ─────────────────────────────────────────────────

    /// Anthropic format detection from SSE event line.
    #[test]
    fn frame_is_anthropic_detects_event_line() {
        assert!(frame_is_anthropic("event: content_block_delta\ndata: {}"));
        assert!(frame_is_anthropic("event: message_start\ndata: {}"));
        assert!(!frame_is_anthropic("data: {\"choices\":[]}"));
    }

    /// Anthropic format detection from data payload type field.
    #[test]
    fn frame_is_anthropic_detects_data_type() {
        let frame =
            r#"data: {"type":"content_block_delta","delta":{"type":"text_delta","text":"hi"}}"#;
        assert!(frame_is_anthropic(frame));
    }

    /// In-stream OpenAI error object is extracted correctly.
    #[test]
    fn extract_stream_error_openai_format() {
        let frame =
            r#"data: {"error":{"message":"Rate limit exceeded","type":"rate_limit_error"}}"#;
        let err = extract_stream_error(frame);
        assert_eq!(err, Some("Rate limit exceeded".into()));
    }

    /// In-stream Anthropic error object is extracted correctly.
    #[test]
    fn extract_stream_error_anthropic_format() {
        let frame =
            r#"data: {"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#;
        let err = extract_stream_error(frame);
        assert_eq!(err, Some("Overloaded".into()));
    }

    /// Non-error frames must not trigger error extraction.
    #[test]
    fn extract_stream_error_none_for_normal_frames() {
        let frame = r#"data: {"choices":[{"delta":{"content":"hello"}}]}"#;
        assert!(extract_stream_error(frame).is_none());
        assert!(extract_stream_error("data: [DONE]").is_none());
    }

    /// id: field is extracted correctly for Last-Event-ID reconnect support.
    #[test]
    fn extract_event_id_present() {
        let frame = "id: msg_123\nevent: content_block_delta\ndata: {}";
        assert_eq!(extract_event_id(frame), Some("msg_123".into()));
    }

    #[test]
    fn extract_event_id_absent() {
        let frame = "event: content_block_delta\ndata: {}";
        assert_eq!(extract_event_id(frame), None);
    }

    /// Metrics: first_token_ms and tokens_received are updated by process_frame.
    #[test]
    fn process_frame_updates_metrics() {
        let start = Instant::now();
        let mut text = String::new();
        let mut tc_map = std::collections::BTreeMap::new();
        let mut pt = 0u64;
        let mut ct = 0u64;
        let mut crt = 0u64;
        let mut cct = 0u64;
        let mut ftms = 0u64;
        let mut tr = 0usize;

        process_frame(
            r#"data: {"choices":[{"delta":{"content":"hi"}}]}"#,
            false,
            &|_: StreamEvent| {},
            &mut text,
            &mut tc_map,
            &mut pt,
            &mut ct,
            &mut crt,
            &mut cct,
            &start,
            &mut ftms,
            &mut tr,
        );

        assert_eq!(tr, 1, "tokens_received should be 1");
        // first_token_ms should be a small non-negative number
        assert!(
            ftms < 5_000,
            "first_token_ms should be less than 5 seconds, got {}",
            ftms
        );
    }

    /// A second token must NOT reset first_token_ms.
    #[test]
    fn process_frame_first_token_ms_not_overwritten() {
        let start = Instant::now();
        let mut text = String::new();
        let mut tc_map = std::collections::BTreeMap::new();
        let mut pt = 0u64;
        let mut ct = 0u64;
        let mut crt = 0u64;
        let mut cct = 0u64;
        let mut ftms = 0u64;
        let mut tr = 0usize;
        let noop = |_: StreamEvent| {};

        process_frame(
            r#"data: {"choices":[{"delta":{"content":"a"}}]}"#,
            false,
            &noop,
            &mut text,
            &mut tc_map,
            &mut pt,
            &mut ct,
            &mut crt,
            &mut cct,
            &start,
            &mut ftms,
            &mut tr,
        );
        let first = ftms;

        process_frame(
            r#"data: {"choices":[{"delta":{"content":"b"}}]}"#,
            false,
            &noop,
            &mut text,
            &mut tc_map,
            &mut pt,
            &mut ct,
            &mut crt,
            &mut cct,
            &start,
            &mut ftms,
            &mut tr,
        );

        assert_eq!(
            ftms, first,
            "first_token_ms must not be overwritten on second token"
        );
        assert_eq!(tr, 2, "tokens_received should be 2");
    }

    // ── Tests for new features ────────────────────────────────────────────

    /// event_to_sse formats Token events correctly.
    #[test]
    fn event_to_sse_token() {
        let event = StreamEvent::Token {
            content: "Hello, world!".into(),
        };
        let sse = event_to_sse(&event);
        assert!(sse.starts_with("event: token\n"));
        assert!(sse.contains("\"content\":\"Hello, world!\""));
        assert!(sse.ends_with("\n\n"));
    }

    /// event_to_sse formats ToolStart events correctly.
    #[test]
    fn event_to_sse_tool_start() {
        let event = StreamEvent::ToolStart {
            id: "call_abc".into(),
            name: "file_read".into(),
            args_json: "{\"path\":\"/tmp/foo\"}".into(),
        };
        let sse = event_to_sse(&event);
        assert!(sse.starts_with("event: tool_start\n"));
        assert!(sse.contains("\"id\":\"call_abc\""));
        assert!(sse.contains("\"name\":\"file_read\""));
        assert!(sse.ends_with("\n\n"));
    }

    /// event_to_sse formats Done events correctly.
    #[test]
    fn event_to_sse_done() {
        let event = StreamEvent::Done {
            total_tokens: 42,
            cost_usd: 0.001,
        };
        let sse = event_to_sse(&event);
        assert!(sse.starts_with("event: done\n"));
        assert!(sse.contains("\"total_tokens\":42"));
        assert!(sse.ends_with("\n\n"));
    }

    /// format_tool_progress formats elapsed time correctly.
    #[test]
    fn tool_progress_formatting() {
        assert_eq!(
            format_tool_progress("file_read", 1234),
            "⟳ file_read (1.2s)"
        );
        assert_eq!(format_tool_progress("shell", 500), "⟳ shell (0.5s)");
        assert_eq!(format_tool_progress("web_search", 0), "⟳ web_search (0.0s)");
    }

    /// StreamAccumulator collects Token events into a string.
    #[test]
    fn stream_accumulator_basic() {
        let mut acc = StreamAccumulator::new();
        acc.handle(StreamEvent::Token {
            content: "Hello".into(),
        });
        acc.handle(StreamEvent::Token {
            content: ", ".into(),
        });
        acc.handle(StreamEvent::Token {
            content: "world!".into(),
        });
        acc.handle(StreamEvent::Done {
            total_tokens: 10,
            cost_usd: 0.0005,
        });
        assert_eq!(acc.as_str(), "Hello, world!");
        assert_eq!(acc.total_tokens(), 10);
        assert!((acc.cost_usd() - 0.0005).abs() < 1e-9);
        assert_eq!(acc.finish(), "Hello, world!");
    }

    /// StreamAccumulator ignores non-Token events (except Done).
    #[test]
    fn stream_accumulator_ignores_tool_events() {
        let mut acc = StreamAccumulator::new();
        acc.handle(StreamEvent::ToolStart {
            id: "x".into(),
            name: "shell".into(),
            args_json: "{}".into(),
        });
        acc.handle(StreamEvent::Token {
            content: "abc".into(),
        });
        acc.handle(StreamEvent::Error {
            message: "oops".into(),
        });
        assert_eq!(acc.finish(), "abc");
    }

    /// OpenAI-compat `delta.reasoning` chunks are surfaced as Thinking events.
    #[test]
    fn process_frame_emits_thinking_from_openai_reasoning() {
        use std::sync::{Arc, Mutex};
        let mut text = String::new();
        let mut tc_map = std::collections::BTreeMap::new();
        let (mut pt, mut ct, mut ftms, mut tr) = (0u64, 0u64, 0u64, 0usize);
        let mut crt = 0u64;
        let mut cct = 0u64;
        let start = Instant::now();
        let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let cc = captured.clone();
        process_frame(
            r#"data: {"choices":[{"delta":{"reasoning":"Let me think step by step..."}}]}"#,
            false,
            &move |ev: StreamEvent| {
                if let StreamEvent::Thinking { content } = ev {
                    cc.lock().unwrap().push(content);
                }
            },
            &mut text,
            &mut tc_map,
            &mut pt,
            &mut ct,
            &mut crt,
            &mut cct,
            &start,
            &mut ftms,
            &mut tr,
        );
        let v = captured.lock().unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0], "Let me think step by step...");
        // Reasoning must NOT pollute the assistant text buffer.
        assert!(text.is_empty());
    }

    /// `delta.reasoning_content` (sibling field name used by some providers)
    /// is also surfaced as a Thinking event.
    #[test]
    fn process_frame_emits_thinking_from_openai_reasoning_content() {
        use std::sync::{Arc, Mutex};
        let mut text = String::new();
        let mut tc_map = std::collections::BTreeMap::new();
        let (mut pt, mut ct, mut ftms, mut tr) = (0u64, 0u64, 0u64, 0usize);
        let mut crt = 0u64;
        let mut cct = 0u64;
        let start = Instant::now();
        let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let cc = captured.clone();
        process_frame(
            r#"data: {"choices":[{"delta":{"reasoning_content":"hmm"}}]}"#,
            false,
            &move |ev: StreamEvent| {
                if let StreamEvent::Thinking { content } = ev {
                    cc.lock().unwrap().push(content);
                }
            },
            &mut text,
            &mut tc_map,
            &mut pt,
            &mut ct,
            &mut crt,
            &mut cct,
            &start,
            &mut ftms,
            &mut tr,
        );
        assert_eq!(captured.lock().unwrap().clone(), vec!["hmm".to_string()]);
    }

    /// Anthropic `thinking_delta` event emits a Thinking event.
    #[test]
    fn process_frame_emits_thinking_from_anthropic_thinking_delta() {
        use std::sync::{Arc, Mutex};
        let mut text = String::new();
        let mut tc_map = std::collections::BTreeMap::new();
        let (mut pt, mut ct, mut ftms, mut tr) = (0u64, 0u64, 0u64, 0usize);
        let mut crt = 0u64;
        let mut cct = 0u64;
        let start = Instant::now();
        let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let cc = captured.clone();
        let frame = "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"because borrows\"}}";
        process_frame(
            frame,
            true,
            &move |ev: StreamEvent| {
                if let StreamEvent::Thinking { content } = ev {
                    cc.lock().unwrap().push(content);
                }
            },
            &mut text,
            &mut tc_map,
            &mut pt,
            &mut ct,
            &mut crt,
            &mut cct,
            &start,
            &mut ftms,
            &mut tr,
        );
        assert_eq!(
            captured.lock().unwrap().clone(),
            vec!["because borrows".to_string()]
        );
        assert!(text.is_empty());
    }

    /// event_to_sse formats Thinking events with `event: thinking` framing.
    #[test]
    fn event_to_sse_thinking() {
        let event = StreamEvent::Thinking {
            content: "step 1".into(),
        };
        let sse = event_to_sse(&event);
        assert!(sse.starts_with("event: thinking\n"));
        assert!(sse.contains("\"content\":\"step 1\""));
    }

    /// StreamSender drops events when the buffer is full (no panic, no block).
    #[test]
    fn stream_sender_drops_when_full() {
        let (sender, _rx) = StreamSender::new();
        // Fill the channel beyond capacity — should not panic or block.
        for i in 0..=(CHANNEL_CAPACITY + 10) {
            sender.send(StreamEvent::Token {
                content: format!("tok{}", i),
            });
        }
        // If we reach here without panic the test passes.
    }

    // ── [F1] Anthropic SDK freebies tests ──────────────────────────────────

    /// [F1] After processing a `message_start` SSE frame whose `message.usage`
    /// contains `cache_read_input_tokens`, the value must be exposed via the
    /// `cache_read_tokens` out-param so callers (and integration tests) can
    /// prove the cache hit on the second call.
    #[test]
    fn process_anthropic_event_records_cache_read_tokens() {
        let json: Value = serde_json::from_str(
            r#"{
            "type": "message_start",
            "message": {
                "id": "msg_abc",
                "usage": {
                    "input_tokens": 12,
                    "cache_read_input_tokens": 1024,
                    "cache_creation_input_tokens": 0
                }
            }
        }"#,
        )
        .unwrap();

        let mut accumulated = String::new();
        let mut tool_calls = std::collections::BTreeMap::new();
        let mut prompt_tokens = 0u64;
        let mut completion_tokens = 0u64;
        let mut cache_read = 0u64;
        let mut cache_creation = 0u64;
        let mut first_token_ms = 0u64;
        let mut tokens_received = 0usize;
        let run_start = Instant::now();

        process_anthropic_event(
            &json,
            &|_: StreamEvent| {},
            &mut accumulated,
            &mut tool_calls,
            &mut prompt_tokens,
            &mut completion_tokens,
            &mut cache_read,
            &mut cache_creation,
            &run_start,
            &mut first_token_ms,
            &mut tokens_received,
        );

        assert_eq!(prompt_tokens, 12);
        assert_eq!(cache_read, 1024, "cache_read_input_tokens must be captured");
        assert_eq!(cache_creation, 0);
    }

    /// [F1] Auto prompt caching: when is_anthropic, the system block must be
    /// rendered as an array containing a `cache_control: {type: "ephemeral"}`
    /// breakpoint, so the API treats the system prompt as cacheable. The last
    /// tool definition must also receive a cache_control breakpoint so that
    /// tools + system are cached together (per Anthropic 2026-02-19 GA docs).
    #[test]
    fn build_request_body_anthropic_injects_cache_control_on_system() {
        let provider = ProviderEntry {
            provider_type: "anthropic".into(),
            url: None,
            ..Default::default()
        };
        let messages = vec![serde_json::json!({"role": "user", "content": "hello"})];
        let tool_defs = vec![serde_json::json!({
            "type": "function",
            "function": {"name": "shell", "description": "run", "parameters": {"type": "object"}},
        })];
        let (_url, body) = build_request_body(
            &provider,
            true,
            "claude-sonnet-4-6",
            "you are helpful",
            &messages,
            &tool_defs,
        );

        // System must be rendered as an array of text blocks (not a bare string),
        // with cache_control on the last block.
        let system = body.get("system").expect("system field present");
        let system_arr = system
            .as_array()
            .expect("system is an array of text blocks");
        assert_eq!(system_arr.len(), 1);
        assert_eq!(system_arr[0]["type"], "text");
        assert_eq!(system_arr[0]["text"], "you are helpful");
        assert_eq!(system_arr[0]["cache_control"]["type"], "ephemeral");

        // The last tool def must also carry cache_control so tools+system cache together.
        let tools = body.get("tools").expect("tools array").as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["cache_control"]["type"], "ephemeral");
    }

    /// [F1] thinking.display "omitted" (Anthropic 2026-03-16): for Opus 4.7+
    /// the request must include adaptive thinking with display=omitted, so
    /// thinking text is not streamed back (saving tokens) but the signature
    /// is still preserved for downstream verification.
    #[test]
    fn build_request_body_anthropic_injects_thinking_display_omitted_for_opus_4_7() {
        let provider = ProviderEntry {
            provider_type: "anthropic".into(),
            url: None,
            ..Default::default()
        };
        let messages = vec![serde_json::json!({"role": "user", "content": "x"})];
        let (_url, body) =
            build_request_body(&provider, true, "claude-opus-4-7", "sys", &messages, &[]);
        assert_eq!(body["thinking"]["type"], "adaptive");
        assert_eq!(body["thinking"]["display"], "omitted");
    }

    /// [F1] Backwards compat: older Claude models (pre-4.7) MUST NOT receive
    /// the thinking.display field — they don't support it and would 400.
    /// Sonnet 4.6 / Opus 4.6 / Haiku 4.5 are daily-driver models in this codebase.
    #[test]
    fn build_request_body_anthropic_omits_thinking_for_pre_opus_4_7() {
        let provider = ProviderEntry {
            provider_type: "anthropic".into(),
            url: None,
            ..Default::default()
        };
        let messages = vec![serde_json::json!({"role": "user", "content": "x"})];

        for model in ["claude-sonnet-4-6", "claude-opus-4-6", "claude-haiku-4-5"] {
            let (_url, body) = build_request_body(&provider, true, model, "sys", &messages, &[]);
            assert!(
                body.get("thinking").is_none(),
                "model {} must not receive thinking field, got: {:?}",
                model,
                body.get("thinking")
            );
        }
    }

    /// [F1] Helper gating function — must match Opus 4.7 and forward variants
    /// without false-positive matching older Claude models.
    #[test]
    fn model_supports_thinking_display_omitted_gating() {
        assert!(model_supports_thinking_display_omitted("claude-opus-4-7"));
        assert!(model_supports_thinking_display_omitted(
            "claude-opus-4-7-20260315"
        ));
        assert!(model_supports_thinking_display_omitted("CLAUDE-OPUS-4-7"));
        assert!(!model_supports_thinking_display_omitted("claude-opus-4-6"));
        assert!(!model_supports_thinking_display_omitted("claude-opus-4-5"));
        assert!(!model_supports_thinking_display_omitted(
            "claude-sonnet-4-6"
        ));
        assert!(!model_supports_thinking_display_omitted("claude-haiku-4-5"));
        assert!(!model_supports_thinking_display_omitted("gpt-4o"));
    }

    /// [F1] Regression: with thinking.display=omitted, Anthropic streams
    /// thinking_delta with an empty `thinking` string but a real `signature`.
    /// We must not emit a Thinking event for empty text, and must not panic.
    #[test]
    fn process_anthropic_event_handles_empty_thinking_with_signature() {
        let json: Value = serde_json::from_str(
            r#"{
            "type": "content_block_delta",
            "index": 0,
            "delta": {
                "type": "thinking_delta",
                "thinking": "",
                "signature": "EuYBCkQYAiJAxxx=="
            }
        }"#,
        )
        .unwrap();

        let mut accumulated = String::new();
        let mut tool_calls = std::collections::BTreeMap::new();
        let mut prompt_tokens = 0u64;
        let mut completion_tokens = 0u64;
        let mut cache_read = 0u64;
        let mut cache_creation = 0u64;
        let mut first_token_ms = 0u64;
        let mut tokens_received = 0usize;
        let run_start = Instant::now();

        let emitted = std::sync::Mutex::new(Vec::<StreamEvent>::new());
        let on_event = |e: StreamEvent| {
            emitted.lock().unwrap().push(e);
        };

        process_anthropic_event(
            &json,
            &on_event,
            &mut accumulated,
            &mut tool_calls,
            &mut prompt_tokens,
            &mut completion_tokens,
            &mut cache_read,
            &mut cache_creation,
            &run_start,
            &mut first_token_ms,
            &mut tokens_received,
        );

        let events = emitted.lock().unwrap();
        assert!(
            events
                .iter()
                .all(|e| !matches!(e, StreamEvent::Thinking { .. })),
            "must not emit Thinking event for empty thinking text, got: {:?}",
            *events
        );
    }
}

#[cfg(test)]
mod pre_stream_retry_unit_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn default_config_matches_brief() {
        let c = PreStreamRetryConfig::default();
        assert_eq!(c.max_retries, 3);
        assert_eq!(c.base_delay, Duration::from_secs(1));
        assert_eq!(c.max_delay, Duration::from_secs(30));
        assert!((c.jitter_ratio - 0.20).abs() < 1e-9);
    }

    #[test]
    fn error_display_contains_provider_and_attempts() {
        let err = PreStreamRetryError {
            provider: "anthropic".into(),
            attempts: 4,
            last_status: Some(429),
            last_retry_after_secs: Some(5),
            last_body_excerpt: Some("Too Many Requests".into()),
            last_source: None,
        };
        let msg = format!("{}", err);
        assert!(
            msg.contains("[anthropic]"),
            "missing provider tag in: {}",
            msg
        );
        assert!(msg.contains("4"), "missing attempts in: {}", msg);
        assert!(msg.contains("429"), "missing last_status in: {}", msg);
        assert!(msg.contains("5"), "missing retry_after in: {}", msg);
        assert!(
            msg.contains("Too Many Requests"),
            "missing body excerpt in: {}",
            msg
        );
    }

    #[test]
    fn classify_retryable_429() {
        assert!(is_pre_stream_retryable_status(429));
    }

    #[test]
    fn classify_retryable_503() {
        assert!(is_pre_stream_retryable_status(503));
    }

    #[test]
    fn classify_non_retryable_400_401_403_404() {
        assert!(!is_pre_stream_retryable_status(400));
        assert!(!is_pre_stream_retryable_status(401));
        assert!(!is_pre_stream_retryable_status(403));
        assert!(!is_pre_stream_retryable_status(404));
    }

    #[test]
    fn classify_other_5xx_not_retryable_by_default() {
        // The brief explicitly lists only 503. 500/502/504 are intentionally
        // left out — those usually indicate a different failure class
        // (overload vs. config). Widen later only with evidence.
        assert!(!is_pre_stream_retryable_status(500));
        assert!(!is_pre_stream_retryable_status(502));
        assert!(!is_pre_stream_retryable_status(504));
    }

    #[test]
    fn classify_success_not_retryable() {
        assert!(!is_pre_stream_retryable_status(200));
        assert!(!is_pre_stream_retryable_status(201));
    }

    #[test]
    fn backoff_attempt_0_centred_is_base_delay() {
        let cfg = PreStreamRetryConfig::default();
        let d = compute_pre_stream_backoff(&cfg, 0, None, |_| 0.5);
        assert_eq!(d, Duration::from_secs(1));
    }

    #[test]
    fn backoff_exponential_no_jitter() {
        let cfg = PreStreamRetryConfig::default();
        assert_eq!(
            compute_pre_stream_backoff(&cfg, 0, None, |_| 0.5),
            Duration::from_secs(1)
        );
        assert_eq!(
            compute_pre_stream_backoff(&cfg, 1, None, |_| 0.5),
            Duration::from_secs(2)
        );
        assert_eq!(
            compute_pre_stream_backoff(&cfg, 2, None, |_| 0.5),
            Duration::from_secs(4)
        );
    }

    #[test]
    fn backoff_jitter_low_end_is_minus_20pct() {
        // jitter_fn = 0.0 → multiplier = (1 - 0.20) = 0.80
        let cfg = PreStreamRetryConfig::default();
        let d = compute_pre_stream_backoff(&cfg, 0, None, |_| 0.0);
        assert_eq!(d, Duration::from_millis(800));
    }

    #[test]
    fn backoff_jitter_high_end_is_plus_20pct() {
        // jitter_fn = 1.0 → multiplier = (1 + 0.20) = 1.20
        let cfg = PreStreamRetryConfig::default();
        let d = compute_pre_stream_backoff(&cfg, 0, None, |_| 1.0);
        assert_eq!(d, Duration::from_millis(1200));
    }

    #[test]
    fn backoff_capped_at_max_delay() {
        let cfg = PreStreamRetryConfig {
            max_delay: Duration::from_millis(1500),
            ..PreStreamRetryConfig::default()
        };
        // attempt 2 with no jitter would be 4000ms; cap forces 1500.
        let d = compute_pre_stream_backoff(&cfg, 2, None, |_| 0.5);
        assert_eq!(d, Duration::from_millis(1500));
    }

    #[test]
    fn backoff_retry_after_seconds_overrides_calculation() {
        let cfg = PreStreamRetryConfig::default();
        let d = compute_pre_stream_backoff(&cfg, 0, Some(Duration::from_secs(7)), |_| 0.5);
        assert_eq!(d, Duration::from_secs(7));
    }

    #[test]
    fn backoff_retry_after_capped_at_max_delay() {
        let cfg = PreStreamRetryConfig::default();
        // Server says wait 5 minutes; we cap at 30s so callers don't hang.
        let d = compute_pre_stream_backoff(&cfg, 0, Some(Duration::from_secs(300)), |_| 0.5);
        assert_eq!(d, Duration::from_secs(30));
    }

    #[test]
    fn retry_after_parses_seconds() {
        let v = reqwest::header::HeaderValue::from_static("5");
        assert_eq!(parse_retry_after_seconds(&v), Some(5));
    }

    #[test]
    fn retry_after_parses_seconds_with_whitespace() {
        let v = reqwest::header::HeaderValue::from_static("  12  ");
        assert_eq!(parse_retry_after_seconds(&v), Some(12));
    }

    #[test]
    fn retry_after_ignores_zero_and_negative() {
        let zero = reqwest::header::HeaderValue::from_static("0");
        assert_eq!(parse_retry_after_seconds(&zero), None);
        let neg = reqwest::header::HeaderValue::from_static("-3");
        assert_eq!(parse_retry_after_seconds(&neg), None);
    }

    #[test]
    fn retry_after_ignores_http_date() {
        // Brief says: parse seconds form; ignore HTTP-date.
        let date = reqwest::header::HeaderValue::from_static("Wed, 21 Oct 2026 07:28:00 GMT");
        assert_eq!(parse_retry_after_seconds(&date), None);
    }

    #[test]
    fn retry_after_ignores_garbage() {
        let g = reqwest::header::HeaderValue::from_static("soonish");
        assert_eq!(parse_retry_after_seconds(&g), None);
    }

    // ── V1 (2026-05-21): pin streaming_url_openai routes for each
    //    OpenAI-compat provider so the openrouter fallback never silently
    //    catches a known type again. Before this fix, e.g. mistral fell
    //    through to "https://openrouter.ai/api/v1/chat/completions" and
    //    401-ed because the Bearer key wasn't valid there. See the
    //    "V1 (2026-05-21)" comment block on streaming_url_openai.
    fn p(t: &str) -> ProviderEntry {
        ProviderEntry {
            provider_type: t.into(),
            url: None,
            ..Default::default()
        }
    }

    #[test]
    fn streaming_url_openai_routes_mistral_to_native() {
        assert_eq!(
            streaming_url_openai(&p("mistral")),
            "https://api.mistral.ai/v1/chat/completions"
        );
    }

    #[test]
    fn streaming_url_openai_routes_together_to_native() {
        assert_eq!(
            streaming_url_openai(&p("together")),
            "https://api.together.xyz/v1/chat/completions"
        );
    }

    #[test]
    fn streaming_url_openai_routes_nvidia_to_native() {
        assert_eq!(
            streaming_url_openai(&p("nvidia")),
            "https://integrate.api.nvidia.com/v1/chat/completions"
        );
    }

    #[test]
    fn streaming_url_openai_routes_fireworks_to_native() {
        let url = streaming_url_openai(&p("fireworks"));
        assert!(url.starts_with("https://api.fireworks.ai/"), "got {}", url);
        assert!(url.ends_with("/chat/completions"), "got {}", url);
    }

    #[test]
    fn streaming_url_openai_routes_xai_to_native() {
        let url = streaming_url_openai(&p("xai"));
        assert!(url.starts_with("https://api.x.ai/"), "got {}", url);
    }

    #[test]
    fn streaming_url_openai_routes_ai21_to_native() {
        let url = streaming_url_openai(&p("ai21"));
        assert!(url.contains("ai21.com"), "got {}", url);
    }

    #[test]
    fn streaming_url_openai_routes_perplexity_to_native() {
        let url = streaming_url_openai(&p("perplexity"));
        assert!(url.contains("perplexity.ai"), "got {}", url);
    }

    #[test]
    fn streaming_url_openai_routes_cohere_to_native() {
        let url = streaming_url_openai(&p("cohere"));
        assert!(url.contains("cohere"), "got {}", url);
    }

    #[test]
    fn streaming_url_openai_explicit_url_still_wins_for_typed_providers() {
        // If the operator sets an explicit proxy URL, it must override the
        // built-in match arms (e.g. a self-hosted vLLM at proxy.local).
        let provider = ProviderEntry {
            provider_type: "mistral".into(),
            url: Some("https://proxy.example.com/v1".into()),
            ..Default::default()
        };
        assert_eq!(
            streaming_url_openai(&provider),
            "https://proxy.example.com/v1/chat/completions"
        );
    }

    #[test]
    fn streaming_url_openai_unknown_type_still_falls_back_to_openrouter() {
        // The openrouter default is *intentional* for genuinely-unknown
        // provider types — many small OpenAI-compat APIs are reachable via
        // openrouter.ai. We only added explicit arms for the V1 12 set;
        // anything outside that should still hit the fallback.
        assert_eq!(
            streaming_url_openai(&p("some-new-provider")),
            "https://openrouter.ai/api/v1/chat/completions"
        );
    }
}
