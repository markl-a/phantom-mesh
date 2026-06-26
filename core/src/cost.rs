//! Per-model cost accounting for LLM API usage.
//!
//! This module owns two things:
//!
//! 1. **The pricing tables** ([`price_per_million`]) — the canonical
//!    per-model USD rates, expressed as `(input_price_per_million,
//!    output_price_per_million)` for one million tokens. A default
//!    fallback estimate is returned for unknown models so spend is never
//!    silently undercounted.
//! 2. **The token/cost accounting struct** ([`CostTracker`]) — accumulates
//!    prompt/completion token counts and derives `cost_usd` from the
//!    pricing tables. It tracks lifetime totals (persisted to
//!    `~/.phantom-mesh/costs.json`), a resettable global session, named
//!    per-session buckets, and optional lifetime/task-scoped budget caps.
//!
//! Costs are computed automatically inside [`CostTracker::record`]: callers
//! supply only the model name and token counts, and the tracker looks up
//! the rates and updates every relevant total under a single lock.

use std::collections::HashMap;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Pricing tables
// ---------------------------------------------------------------------------

/// Returns (input_price_per_million, output_price_per_million) in USD.
/// Used by `record()` to compute cost_usd automatically.
pub fn price_per_million(model: &str) -> (f64, f64) {
    match model {
        m if m.contains("claude-sonnet") => (3.0, 15.0),
        m if m.contains("claude-haiku") => (0.25, 1.25),
        m if m.contains("claude-opus") => (15.0, 75.0),
        m if m.contains("gpt-4o-mini") => (0.15, 0.60),
        m if m.contains("gpt-4o") => (2.5, 10.0),
        m if m.contains("gpt-4.1") => (2.0, 8.0),
        m if m.contains("gemini-2.5-pro") => (1.25, 10.0),
        m if m.contains("gemini-2.0-flash") => (0.075, 0.30),
        m if m.contains("groq") || m.contains("llama") => (0.05, 0.08),
        _ => (1.0, 5.0), // default estimate
    }
}

// ---------------------------------------------------------------------------
// Internal data structures
// ---------------------------------------------------------------------------

#[derive(Clone, Default, serde::Serialize)]
struct ModelCost {
    input_tokens: u64,
    output_tokens: u64,
    cost_usd: f64,
}

#[derive(Default)]
struct SessionData {
    usd: f64,
    prompt_tokens: u64,
    completion_tokens: u64,
    by_model: HashMap<String, ModelCost>,
}

struct CostTrackerInner {
    // Lifetime totals (persisted to disk)
    total_usd: f64,
    total_requests: u64,
    total_prompt_tokens: u64,
    total_completion_tokens: u64,

    // Default/global session totals (reset on reset_session())
    session_usd: f64,
    session_prompt_tokens: u64,
    session_completion_tokens: u64,

    // Per-model breakdown (global session-scoped)
    by_model: HashMap<String, ModelCost>,

    // Most-recent request cost
    last_request_cost: f64,

    // Budget limit (0.0 means no limit)
    budget_limit_usd: f64,
    // True once total_usd >= budget_limit_usd (and limit > 0)
    over_budget: bool,

    // Task-scoped budget: cap spend since `task_baseline_usd` was set.
    // Used by subagent::call to enforce max_cost_usd in-flight, so the
    // agent loop can break out at the next round once it overshoots.
    task_budget_usd: f64,
    task_baseline_usd: f64,

    // Named per-session tracking
    sessions: HashMap<String, SessionData>,
}

impl Default for CostTrackerInner {
    fn default() -> Self {
        Self {
            total_usd: 0.0,
            total_requests: 0,
            total_prompt_tokens: 0,
            total_completion_tokens: 0,
            session_usd: 0.0,
            session_prompt_tokens: 0,
            session_completion_tokens: 0,
            by_model: HashMap::new(),
            last_request_cost: 0.0,
            budget_limit_usd: 0.0,
            over_budget: false,
            task_budget_usd: 0.0,
            task_baseline_usd: 0.0,
            sessions: HashMap::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Thread-safe accumulator for LLM token usage and derived USD cost.
///
/// Cloning is cheap: clones share the same underlying state via an
/// [`Arc`]-wrapped mutex, so all handles observe the same running totals.
/// Lifetime totals are persisted to `~/.phantom-mesh/costs.json`.
#[derive(Clone)]
pub struct CostTracker {
    inner: Arc<tokio::sync::Mutex<CostTrackerInner>>,
    path: std::path::PathBuf,
}

impl CostTracker {
    /// Create a tracker, loading lifetime totals from
    /// `~/.phantom-mesh/costs.json` if the file exists. Session-scoped and
    /// per-model state always starts empty; a missing or unreadable ledger
    /// falls back to zeroed totals.
    pub fn new() -> Self {
        let path = crate::cli_config::phantom_data_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from(".").join(".phantom-mesh"))
            .join("costs.json");
        let inner = if path.exists() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                    CostTrackerInner {
                        total_usd: v["total_usd"].as_f64().unwrap_or(0.0),
                        total_requests: v["requests"].as_u64().unwrap_or(0),
                        total_prompt_tokens: v["prompt_tokens"].as_u64().unwrap_or(0),
                        total_completion_tokens: v["completion_tokens"].as_u64().unwrap_or(0),
                        ..Default::default()
                    }
                } else {
                    CostTrackerInner::default()
                }
            } else {
                CostTrackerInner::default()
            }
        } else {
            CostTrackerInner::default()
        };
        Self {
            inner: Arc::new(tokio::sync::Mutex::new(inner)),
            path,
        }
    }

    // -----------------------------------------------------------------------
    // Budget
    // -----------------------------------------------------------------------

    /// Set a hard spending limit in USD. Once `total_usd` reaches this value,
    /// `is_over_budget()` returns `true`. Pass `0.0` to disable.
    pub async fn set_budget_limit(&self, usd: f64) {
        let mut inner = self.inner.lock().await;
        inner.budget_limit_usd = usd;
        inner.over_budget = usd > 0.0 && inner.total_usd >= usd;
    }

    /// Returns `true` when total lifetime spend has reached the configured budget limit.
    pub async fn is_over_budget(&self) -> bool {
        self.inner.lock().await.over_budget
    }

    /// Set a task-scoped budget. The cap is measured against spend accrued
    /// *after* this call returns. Pass `0.0` to disable. The agent loop
    /// polls `is_over_budget()` after each round and breaks out when this
    /// (or the lifetime limit) is exceeded.
    pub async fn set_task_budget(&self, usd: f64) {
        let mut inner = self.inner.lock().await;
        inner.task_budget_usd = usd;
        inner.task_baseline_usd = inner.total_usd;
        // Reset task-induced over-budget; lifetime check still applies via record().
        if inner.budget_limit_usd == 0.0 || inner.total_usd < inner.budget_limit_usd {
            inner.over_budget = false;
        }
    }

    // -----------------------------------------------------------------------
    // Recording
    // -----------------------------------------------------------------------

    /// Record a single API call. Cost is computed automatically from the model
    /// pricing table. Also updates the `over_budget` flag if a limit is set.
    pub async fn record(&self, model: &str, prompt_tokens: u64, completion_tokens: u64) {
        let (input_price, output_price) = price_per_million(model);
        let cost = (prompt_tokens as f64 / 1_000_000.0) * input_price
            + (completion_tokens as f64 / 1_000_000.0) * output_price;

        // Snapshot persistable state under the lock, then drop the guard before
        // touching the filesystem. Holding the mutex across a blocking
        // `std::fs::write` was deadlocking the tokio runtime on Android.
        let snapshot = {
            let mut inner = self.inner.lock().await;

            inner.total_usd += cost;
            inner.total_requests += 1;
            inner.total_prompt_tokens += prompt_tokens;
            inner.total_completion_tokens += completion_tokens;

            inner.session_usd += cost;
            inner.session_prompt_tokens += prompt_tokens;
            inner.session_completion_tokens += completion_tokens;

            let entry = inner.by_model.entry(model.to_string()).or_default();
            entry.input_tokens += prompt_tokens;
            entry.output_tokens += completion_tokens;
            entry.cost_usd += cost;

            inner.last_request_cost = cost;

            if inner.budget_limit_usd > 0.0 && inner.total_usd >= inner.budget_limit_usd {
                inner.over_budget = true;
            }
            if inner.task_budget_usd > 0.0
                && (inner.total_usd - inner.task_baseline_usd) >= inner.task_budget_usd
            {
                inner.over_budget = true;
            }

            (
                inner.total_usd,
                inner.total_requests,
                inner.total_prompt_tokens,
                inner.total_completion_tokens,
            )
        };

        let path = self.path.clone();
        let json = serde_json::json!({
            "total_usd": snapshot.0,
            "requests": snapshot.1,
            "prompt_tokens": snapshot.2,
            "completion_tokens": snapshot.3,
        })
        .to_string();
        // Write off the runtime so a slow disk doesn't stall request handling.
        tokio::task::spawn_blocking(move || {
            if let Some(parent) = path.parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    tracing::warn!(path = %parent.display(), "cost ledger mkdir failed: {}", e);
                }
            }
            if let Err(e) = std::fs::write(&path, json) {
                tracing::warn!(path = %path.display(), "cost ledger write failed (in-memory total may drift from disk): {}", e);
            }
        });
    }

    /// Like `record()` but also tracks costs under a named `session_id` bucket.
    pub async fn record_for_session(
        &self,
        session_id: &str,
        model: &str,
        prompt_tokens: u64,
        completion_tokens: u64,
    ) {
        // First, do the global record
        self.record(model, prompt_tokens, completion_tokens).await;

        // Then add to the named session
        let (input_price, output_price) = price_per_million(model);
        let cost = (prompt_tokens as f64 / 1_000_000.0) * input_price
            + (completion_tokens as f64 / 1_000_000.0) * output_price;

        let mut inner = self.inner.lock().await;
        let sess = inner.sessions.entry(session_id.to_string()).or_default();
        sess.usd += cost;
        sess.prompt_tokens += prompt_tokens;
        sess.completion_tokens += completion_tokens;

        let entry = sess.by_model.entry(model.to_string()).or_default();
        entry.input_tokens += prompt_tokens;
        entry.output_tokens += completion_tokens;
        entry.cost_usd += cost;
    }

    // -----------------------------------------------------------------------
    // Summaries
    // -----------------------------------------------------------------------

    /// Return a JSON snapshot of lifetime and global-session totals,
    /// including the per-model breakdown and current budget state. USD
    /// figures are rounded to four decimal places for display.
    pub async fn summary(&self) -> serde_json::Value {
        let inner = self.inner.lock().await;
        serde_json::json!({
            "total_usd": (inner.total_usd * 10000.0).round() / 10000.0,
            "session_usd": (inner.session_usd * 10000.0).round() / 10000.0,
            "requests": inner.total_requests,
            "prompt_tokens": inner.total_prompt_tokens,
            "completion_tokens": inner.total_completion_tokens,
            "by_model": inner.by_model,
            "budget_limit_usd": inner.budget_limit_usd,
            "over_budget": inner.over_budget,
        })
    }

    /// Returns a JSON summary for a specific named session (created via
    /// `record_for_session`). Returns `null` fields if the session is unknown.
    pub async fn session_summary(&self, session_id: &str) -> serde_json::Value {
        let inner = self.inner.lock().await;
        match inner.sessions.get(session_id) {
            Some(sess) => serde_json::json!({
                "session_id": session_id,
                "usd": (sess.usd * 10000.0).round() / 10000.0,
                "prompt_tokens": sess.prompt_tokens,
                "completion_tokens": sess.completion_tokens,
                "by_model": sess.by_model,
            }),
            None => serde_json::json!({
                "session_id": session_id,
                "usd": null,
                "prompt_tokens": null,
                "completion_tokens": null,
                "by_model": null,
            }),
        }
    }

    // -----------------------------------------------------------------------
    // Accessors
    // -----------------------------------------------------------------------

    /// Returns the global session cost in USD.
    pub async fn session_cost(&self) -> f64 {
        self.inner.lock().await.session_usd
    }

    /// Returns the cost of the most recent `record()` / `record_for_session()` call.
    pub async fn last_request_cost(&self) -> f64 {
        self.inner.lock().await.last_request_cost
    }

    /// Synchronous shorthand for displaying after each CLI turn.
    /// Equivalent to `.last_request_cost().await` but named per spec.
    pub async fn last_turn_cost(&self) -> f64 {
        self.inner.lock().await.last_request_cost
    }

    // -----------------------------------------------------------------------
    // Reset
    // -----------------------------------------------------------------------

    /// Reset global session counters without affecting lifetime totals or named sessions.
    pub async fn reset_session(&self) {
        let mut inner = self.inner.lock().await;
        inner.session_usd = 0.0;
        inner.session_prompt_tokens = 0;
        inner.session_completion_tokens = 0;
        inner.by_model.clear();
        inner.last_request_cost = 0.0;
    }

    // -----------------------------------------------------------------------
    // Static helpers
    // -----------------------------------------------------------------------

    /// Format a USD amount for human display:
    /// - < $1.0   → e.g. "$0.0123"   (4 decimal places — needed for sub-cent
    ///             precision on cheap streaming models)
    /// - >= $1.0  → e.g. "$1.23"     (2 decimal places — full-dollar amounts
    ///             don't benefit from the extra digits)
    pub fn format_cost(usd: f64) -> String {
        if usd < 1.0 {
            format!("${:.4}", usd)
        } else {
            format!("${:.2}", usd)
        }
    }

    // -----------------------------------------------------------------------
    // Internal
    // -----------------------------------------------------------------------

    #[allow(dead_code)]
    fn persist(&self, inner: &CostTrackerInner) {
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let json = serde_json::json!({
            "total_usd": inner.total_usd,
            "requests": inner.total_requests,
            "prompt_tokens": inner.total_prompt_tokens,
            "completion_tokens": inner.total_completion_tokens,
        });
        if let Err(e) = std::fs::write(&self.path, json.to_string()) {
            tracing::warn!(path = %self.path.display(), "cost ledger write failed (in-memory total may drift from disk): {}", e);
        }
    }
}

impl Default for CostTracker {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Serde for SessionData (needed for session_summary by_model serialisation)
// ---------------------------------------------------------------------------

impl serde::Serialize for SessionData {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(4))?;
        map.serialize_entry("usd", &self.usd)?;
        map.serialize_entry("prompt_tokens", &self.prompt_tokens)?;
        map.serialize_entry("completion_tokens", &self.completion_tokens)?;
        map.serialize_entry("by_model", &self.by_model)?;
        map.end()
    }
}

#[cfg(test)]
mod tests {
    use super::price_per_million;

    /// V1 P0 — pins the public pricing table so silent edits (e.g. a
    /// swapped pair of input/output rates) trip CI before a paying
    /// user gets billed wrong. Covers one representative model per
    /// provider the dispatcher knows about; default-fallback path is
    /// also asserted so unknown models don't return 0.
    #[test]
    fn per_provider_cost_calc_uses_correct_rates() {
        // (model substring, expected (input_per_million, output_per_million))
        let canonical: &[(&str, (f64, f64))] = &[
            ("claude-sonnet-4-6", (3.0, 15.0)),
            ("claude-haiku-4-5", (0.25, 1.25)),
            ("claude-opus-4-7", (15.0, 75.0)),
            ("gpt-4o-mini", (0.15, 0.60)),
            ("gpt-4o", (2.5, 10.0)),
            ("gpt-4.1", (2.0, 8.0)),
            ("gemini-2.5-pro", (1.25, 10.0)),
            ("gemini-2.0-flash", (0.075, 0.30)),
            ("groq-llama-3.3-70b", (0.05, 0.08)),
            ("llama-3.1-8b-instruct", (0.05, 0.08)),
        ];

        for (model, (want_in, want_out)) in canonical {
            let (got_in, got_out) = price_per_million(model);
            assert!(
                (got_in - want_in).abs() < 1e-9,
                "input rate mismatch for {model}: got {got_in}, want {want_in}",
            );
            assert!(
                (got_out - want_out).abs() < 1e-9,
                "output rate mismatch for {model}: got {got_out}, want {want_out}",
            );
            // Universal invariant: output ≥ input. Catches future
            // accidental swaps (output should always cost ≥ input).
            assert!(
                got_out >= got_in,
                "output rate < input rate for {model} ({got_out} < {got_in}) — \
                 likely an input/output swap in the pricing table",
            );
        }

        // Unknown model → default fallback (must NOT be (0.0, 0.0),
        // otherwise budget tracking silently undercounts spend).
        let (fb_in, fb_out) = price_per_million("totally-fictional-model-7");
        assert!(fb_in > 0.0 && fb_out > 0.0, "fallback rates must be > 0");
        assert!(fb_out >= fb_in, "fallback output ≥ input");
    }
}
