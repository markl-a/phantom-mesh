//! Enhanced Observability Engine — comprehensive metrics collection, alert rules,
//! Prometheus export, and dashboard summaries for the Phantom Mesh cluster.
//!
//! Builds on top of the lightweight `metrics.rs` module with domain-specific
//! tracking for dispatches, LLM calls, hand executions, and worker utilization.

use std::collections::HashMap;
use std::sync::{Mutex, RwLock};
use std::time::Instant;

use serde::Serialize;

// ---------------------------------------------------------------------------
// Core metric types
// ---------------------------------------------------------------------------

/// A single dispatch record for a tool execution on a worker.
#[derive(Debug, Clone)]
struct DispatchRecord {
    worker: String,
    tool: String,
    latency_ms: u64,
    success: bool,
    #[allow(dead_code)]
    timestamp: Instant,
}

/// A single LLM call record.
#[derive(Debug, Clone)]
struct LlmRecord {
    provider: String,
    #[allow(dead_code)]
    model: String,
    tokens: u64,
    latency_ms: u64,
    timestamp: Instant,
}

/// A single hand (workflow) execution record.
#[derive(Debug, Clone)]
struct HandRecord {
    #[allow(dead_code)]
    hand: String,
    success: bool,
    #[allow(dead_code)]
    duration_secs: f64,
    #[allow(dead_code)]
    timestamp: Instant,
}

// ---------------------------------------------------------------------------
// ObservabilityEngine
// ---------------------------------------------------------------------------

/// Collects dispatch_latency_ms, tool_success_rate, worker_utilization,
/// llm_tokens_per_minute, hand_completion_rate, and error_rate_by_type.
pub struct ObservabilityEngine {
    dispatches: RwLock<Vec<DispatchRecord>>,
    llm_calls: RwLock<Vec<LlmRecord>>,
    hand_results: RwLock<Vec<HandRecord>>,
    /// Tracks which workers are known (registered).
    known_workers: RwLock<Vec<String>>,
}

impl ObservabilityEngine {
    /// Create a new empty engine.
    pub fn new() -> Self {
        Self {
            dispatches: RwLock::new(Vec::new()),
            llm_calls: RwLock::new(Vec::new()),
            hand_results: RwLock::new(Vec::new()),
            known_workers: RwLock::new(Vec::new()),
        }
    }

    /// Register a known worker name (for utilization tracking).
    pub fn register_worker(&self, worker: &str) {
        let mut workers = self.known_workers.write().unwrap();
        if !workers.iter().any(|w| w == worker) {
            workers.push(worker.to_string());
        }
    }

    /// Record a tool dispatch to a worker.
    pub fn record_dispatch(&self, worker: &str, tool: &str, latency_ms: u64, success: bool) {
        let record = DispatchRecord {
            worker: worker.to_string(),
            tool: tool.to_string(),
            latency_ms,
            success,
            timestamp: Instant::now(),
        };
        self.dispatches.write().unwrap().push(record);
    }

    /// Record an LLM provider call.
    pub fn record_llm_call(&self, provider: &str, model: &str, tokens: u64, latency_ms: u64) {
        let record = LlmRecord {
            provider: provider.to_string(),
            model: model.to_string(),
            tokens,
            latency_ms,
            timestamp: Instant::now(),
        };
        self.llm_calls.write().unwrap().push(record);
    }

    /// Record a hand (workflow) execution result.
    pub fn record_hand_result(&self, hand: &str, success: bool, duration_secs: f64) {
        let record = HandRecord {
            hand: hand.to_string(),
            success,
            duration_secs,
            timestamp: Instant::now(),
        };
        self.hand_results.write().unwrap().push(record);
    }

    // -----------------------------------------------------------------------
    // Aggregated queries
    // -----------------------------------------------------------------------

    /// Average dispatch latency across all recorded dispatches (ms).
    pub fn avg_dispatch_latency_ms(&self) -> f64 {
        let dispatches = self.dispatches.read().unwrap();
        if dispatches.is_empty() {
            return 0.0;
        }
        let total: u64 = dispatches.iter().map(|d| d.latency_ms).sum();
        total as f64 / dispatches.len() as f64
    }

    /// Tool success rate (0.0 .. 1.0).  Returns 1.0 when there are no records.
    pub fn tool_success_rate(&self) -> f64 {
        let dispatches = self.dispatches.read().unwrap();
        if dispatches.is_empty() {
            return 1.0;
        }
        let successes = dispatches.iter().filter(|d| d.success).count();
        successes as f64 / dispatches.len() as f64
    }

    /// Per-worker dispatch count (a proxy for utilization).
    pub fn worker_utilization(&self) -> HashMap<String, u64> {
        let dispatches = self.dispatches.read().unwrap();
        let mut map: HashMap<String, u64> = HashMap::new();
        for d in dispatches.iter() {
            *map.entry(d.worker.clone()).or_insert(0) += 1;
        }
        map
    }

    /// Tokens per minute across all LLM calls in the window.
    /// Uses the time span from the first to the last recorded call.
    pub fn llm_tokens_per_minute(&self) -> f64 {
        let calls = self.llm_calls.read().unwrap();
        if calls.is_empty() {
            return 0.0;
        }
        let total_tokens: u64 = calls.iter().map(|c| c.tokens).sum();
        if calls.len() == 1 {
            return total_tokens as f64; // single-point: report raw tokens
        }
        let first = calls.iter().map(|c| c.timestamp).min().unwrap();
        let last = calls.iter().map(|c| c.timestamp).max().unwrap();
        let span_secs = last.duration_since(first).as_secs_f64();
        if span_secs < 0.001 {
            return total_tokens as f64;
        }
        total_tokens as f64 / (span_secs / 60.0)
    }

    /// Hand completion rate (success / total).  Returns 1.0 when empty.
    pub fn hand_completion_rate(&self) -> f64 {
        let results = self.hand_results.read().unwrap();
        if results.is_empty() {
            return 1.0;
        }
        let successes = results.iter().filter(|r| r.success).count();
        successes as f64 / results.len() as f64
    }

    /// Error rate grouped by tool name.
    pub fn error_rate_by_tool(&self) -> HashMap<String, f64> {
        let dispatches = self.dispatches.read().unwrap();
        let mut totals: HashMap<String, (u64, u64)> = HashMap::new(); // (failures, total)
        for d in dispatches.iter() {
            let entry = totals.entry(d.tool.clone()).or_insert((0, 0));
            if !d.success {
                entry.0 += 1;
            }
            entry.1 += 1;
        }
        totals
            .into_iter()
            .map(|(tool, (failures, total))| (tool, failures as f64 / total as f64))
            .collect()
    }

    /// Total dispatch count.
    pub fn dispatch_count(&self) -> usize {
        self.dispatches.read().unwrap().len()
    }

    /// Total LLM call count.
    pub fn llm_call_count(&self) -> usize {
        self.llm_calls.read().unwrap().len()
    }

    /// Total hand execution count.
    pub fn hand_count(&self) -> usize {
        self.hand_results.read().unwrap().len()
    }

    /// Snapshot of all current metrics for alert evaluation.
    pub fn snapshot(&self) -> Metrics {
        Metrics {
            avg_dispatch_latency_ms: self.avg_dispatch_latency_ms(),
            tool_success_rate: self.tool_success_rate(),
            worker_utilization: self.worker_utilization(),
            llm_tokens_per_minute: self.llm_tokens_per_minute(),
            hand_completion_rate: self.hand_completion_rate(),
            error_rate_by_tool: self.error_rate_by_tool(),
            dispatch_count: self.dispatch_count() as u64,
            llm_call_count: self.llm_call_count() as u64,
            hand_count: self.hand_count() as u64,
        }
    }

    /// Render all collected metrics in Prometheus text exposition format.
    pub fn to_prometheus_text(&self) -> String {
        let snap = self.snapshot();
        let mut out = String::new();

        // -- Counters --
        out.push_str("# TYPE phantom_mesh_obs_dispatch_total counter\n");
        out.push_str(&format!("phantom_mesh_obs_dispatch_total {}\n", snap.dispatch_count));

        out.push_str("# TYPE phantom_mesh_obs_llm_call_total counter\n");
        out.push_str(&format!("phantom_mesh_obs_llm_call_total {}\n", snap.llm_call_count));

        out.push_str("# TYPE phantom_mesh_obs_hand_total counter\n");
        out.push_str(&format!("phantom_mesh_obs_hand_total {}\n", snap.hand_count));

        // -- Gauges --
        out.push_str("# TYPE phantom_mesh_obs_avg_dispatch_latency_ms gauge\n");
        out.push_str(&format!(
            "phantom_mesh_obs_avg_dispatch_latency_ms {:.2}\n",
            snap.avg_dispatch_latency_ms
        ));

        out.push_str("# TYPE phantom_mesh_obs_tool_success_rate gauge\n");
        out.push_str(&format!(
            "phantom_mesh_obs_tool_success_rate {:.4}\n",
            snap.tool_success_rate
        ));

        out.push_str("# TYPE phantom_mesh_obs_hand_completion_rate gauge\n");
        out.push_str(&format!(
            "phantom_mesh_obs_hand_completion_rate {:.4}\n",
            snap.hand_completion_rate
        ));

        out.push_str("# TYPE phantom_mesh_obs_llm_tokens_per_minute gauge\n");
        out.push_str(&format!(
            "phantom_mesh_obs_llm_tokens_per_minute {:.2}\n",
            snap.llm_tokens_per_minute
        ));

        // -- Per-worker utilization --
        out.push_str("# TYPE phantom_mesh_obs_worker_dispatches gauge\n");
        let mut workers: Vec<_> = snap.worker_utilization.iter().collect();
        workers.sort_by_key(|(k, _)| (*k).clone());
        for (worker, count) in &workers {
            out.push_str(&format!(
                "phantom_mesh_obs_worker_dispatches{{worker=\"{}\"}} {}\n",
                worker, count
            ));
        }

        // -- Per-tool error rate --
        out.push_str("# TYPE phantom_mesh_obs_tool_error_rate gauge\n");
        let mut tools: Vec<_> = snap.error_rate_by_tool.iter().collect();
        tools.sort_by_key(|(k, _)| (*k).clone());
        for (tool, rate) in &tools {
            out.push_str(&format!(
                "phantom_mesh_obs_tool_error_rate{{tool=\"{}\"}} {:.4}\n",
                tool, rate
            ));
        }

        // -- Dispatch latency histogram --
        let dispatches = self.dispatches.read().unwrap();
        let buckets = [10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0, 2500.0, 5000.0, 10000.0];
        out.push_str("# TYPE phantom_mesh_obs_dispatch_latency_ms histogram\n");
        for bucket in &buckets {
            let count = dispatches.iter().filter(|d| (d.latency_ms as f64) <= *bucket).count();
            out.push_str(&format!(
                "phantom_mesh_obs_dispatch_latency_ms_bucket{{le=\"{}\"}} {}\n",
                bucket, count
            ));
        }
        out.push_str(&format!(
            "phantom_mesh_obs_dispatch_latency_ms_bucket{{le=\"+Inf\"}} {}\n",
            dispatches.len()
        ));
        let sum_ms: u64 = dispatches.iter().map(|d| d.latency_ms).sum();
        out.push_str(&format!("phantom_mesh_obs_dispatch_latency_ms_sum {}\n", sum_ms));
        out.push_str(&format!(
            "phantom_mesh_obs_dispatch_latency_ms_count {}\n",
            dispatches.len()
        ));

        // -- LLM latency histogram --
        let llm_calls = self.llm_calls.read().unwrap();
        out.push_str("# TYPE phantom_mesh_obs_llm_latency_ms histogram\n");
        for bucket in &buckets {
            let count = llm_calls.iter().filter(|c| (c.latency_ms as f64) <= *bucket).count();
            out.push_str(&format!(
                "phantom_mesh_obs_llm_latency_ms_bucket{{le=\"{}\"}} {}\n",
                bucket, count
            ));
        }
        out.push_str(&format!(
            "phantom_mesh_obs_llm_latency_ms_bucket{{le=\"+Inf\"}} {}\n",
            llm_calls.len()
        ));
        let llm_sum: u64 = llm_calls.iter().map(|c| c.latency_ms).sum();
        out.push_str(&format!("phantom_mesh_obs_llm_latency_ms_sum {}\n", llm_sum));
        out.push_str(&format!(
            "phantom_mesh_obs_llm_latency_ms_count {}\n",
            llm_calls.len()
        ));

        out
    }

    /// Produce a JSON-friendly dashboard summary.
    pub fn dashboard_summary(&self) -> DashboardSummary {
        let snap = self.snapshot();

        // Top 5 busiest workers
        let mut workers: Vec<_> = snap.worker_utilization.iter().collect();
        workers.sort_by(|a, b| b.1.cmp(a.1));
        let top_workers: Vec<WorkerSummary> = workers
            .iter()
            .take(5)
            .map(|(name, count)| WorkerSummary {
                name: name.to_string(),
                dispatch_count: **count,
            })
            .collect();

        // Top 5 error-prone tools
        let mut err_tools: Vec<_> = snap.error_rate_by_tool.iter().collect();
        err_tools.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap_or(std::cmp::Ordering::Equal));
        let top_error_tools: Vec<ToolErrorSummary> = err_tools
            .iter()
            .take(5)
            .map(|(tool, rate)| ToolErrorSummary {
                tool: tool.to_string(),
                error_rate: **rate,
            })
            .collect();

        // Per-provider LLM stats
        let llm_calls = self.llm_calls.read().unwrap();
        let mut provider_stats: HashMap<String, (u64, u64, u64)> = HashMap::new(); // tokens, latency_sum, count
        for c in llm_calls.iter() {
            let entry = provider_stats.entry(c.provider.clone()).or_insert((0, 0, 0));
            entry.0 += c.tokens;
            entry.1 += c.latency_ms;
            entry.2 += 1;
        }
        let mut llm_providers: Vec<LlmProviderSummary> = provider_stats
            .into_iter()
            .map(|(provider, (tokens, latency_sum, count))| LlmProviderSummary {
                provider,
                total_tokens: tokens,
                avg_latency_ms: if count > 0 { latency_sum as f64 / count as f64 } else { 0.0 },
                call_count: count,
            })
            .collect();
        llm_providers.sort_by(|a, b| b.call_count.cmp(&a.call_count));

        DashboardSummary {
            dispatch_count: snap.dispatch_count,
            llm_call_count: snap.llm_call_count,
            hand_count: snap.hand_count,
            avg_dispatch_latency_ms: snap.avg_dispatch_latency_ms,
            tool_success_rate: snap.tool_success_rate,
            hand_completion_rate: snap.hand_completion_rate,
            llm_tokens_per_minute: snap.llm_tokens_per_minute,
            top_workers,
            top_error_tools,
            llm_providers,
        }
    }
}

impl Default for ObservabilityEngine {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Metrics snapshot (used for alert evaluation)
// ---------------------------------------------------------------------------

/// Point-in-time snapshot of all observability metrics.
#[derive(Debug, Clone, Serialize)]
pub struct Metrics {
    pub avg_dispatch_latency_ms: f64,
    pub tool_success_rate: f64,
    pub worker_utilization: HashMap<String, u64>,
    pub llm_tokens_per_minute: f64,
    pub hand_completion_rate: f64,
    pub error_rate_by_tool: HashMap<String, f64>,
    pub dispatch_count: u64,
    pub llm_call_count: u64,
    pub hand_count: u64,
}

// ---------------------------------------------------------------------------
// Dashboard summary
// ---------------------------------------------------------------------------

/// JSON-friendly summary for web dashboard consumption.
#[derive(Debug, Clone, Serialize)]
pub struct DashboardSummary {
    pub dispatch_count: u64,
    pub llm_call_count: u64,
    pub hand_count: u64,
    pub avg_dispatch_latency_ms: f64,
    pub tool_success_rate: f64,
    pub hand_completion_rate: f64,
    pub llm_tokens_per_minute: f64,
    pub top_workers: Vec<WorkerSummary>,
    pub top_error_tools: Vec<ToolErrorSummary>,
    pub llm_providers: Vec<LlmProviderSummary>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkerSummary {
    pub name: String,
    pub dispatch_count: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolErrorSummary {
    pub tool: String,
    pub error_rate: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct LlmProviderSummary {
    pub provider: String,
    pub total_tokens: u64,
    pub avg_latency_ms: f64,
    pub call_count: u64,
}

// ---------------------------------------------------------------------------
// Alert system
// ---------------------------------------------------------------------------

/// Severity level for alerts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum AlertSeverity {
    Info,
    Warning,
    Critical,
}

/// A threshold-based condition for an alert.
#[derive(Debug, Clone)]
pub enum AlertCondition {
    /// Fires when the tool error rate exceeds the given fraction (0.0..1.0).
    ErrorRateAbove(f64),
    /// Fires when average dispatch latency exceeds the given ms value.
    LatencyAbove(f64),
    /// Fires when hand completion rate drops below the given fraction.
    HandCompletionBelow(f64),
    /// Fires when tool success rate drops below the given fraction.
    ToolSuccessBelow(f64),
    /// Fires when LLM tokens per minute exceed the given value.
    TokenRateAbove(f64),
    /// Fires when total dispatch count exceeds the threshold (burst detection).
    DispatchCountAbove(u64),
}

impl AlertCondition {
    /// Evaluate this condition against a metrics snapshot.
    pub fn evaluate(&self, metrics: &Metrics) -> bool {
        match self {
            AlertCondition::ErrorRateAbove(threshold) => {
                let overall_error = 1.0 - metrics.tool_success_rate;
                overall_error > *threshold
            }
            AlertCondition::LatencyAbove(threshold) => {
                metrics.avg_dispatch_latency_ms > *threshold
            }
            AlertCondition::HandCompletionBelow(threshold) => {
                metrics.hand_completion_rate < *threshold
            }
            AlertCondition::ToolSuccessBelow(threshold) => {
                metrics.tool_success_rate < *threshold
            }
            AlertCondition::TokenRateAbove(threshold) => {
                metrics.llm_tokens_per_minute > *threshold
            }
            AlertCondition::DispatchCountAbove(threshold) => {
                metrics.dispatch_count > *threshold
            }
        }
    }
}

/// A named alert rule with severity and cooldown.
#[derive(Debug, Clone)]
pub struct AlertRule {
    pub name: String,
    pub condition: AlertCondition,
    pub severity: AlertSeverity,
    pub cooldown_secs: u64,
}

/// A fired alert with metadata.
#[derive(Debug, Clone, Serialize)]
pub struct FiredAlert {
    pub rule_name: String,
    pub severity: AlertSeverity,
    pub message: String,
}

/// Engine that evaluates alert rules against metrics, respecting cooldowns.
pub struct AlertEngine {
    rules: RwLock<Vec<AlertRule>>,
    /// Last fire time per rule name — used for cooldown enforcement.
    last_fired: Mutex<HashMap<String, Instant>>,
}

impl AlertEngine {
    /// Create an empty alert engine.
    pub fn new() -> Self {
        Self {
            rules: RwLock::new(Vec::new()),
            last_fired: Mutex::new(HashMap::new()),
        }
    }

    /// Create an alert engine pre-loaded with sensible defaults:
    /// - error_rate > 50% = Critical (cooldown 5min)
    /// - hand_completion < 50% = Warning (cooldown 10min)
    /// - latency > 5000ms = Warning (cooldown 5min)
    pub fn with_defaults() -> Self {
        let engine = Self::new();
        engine.add_rule(AlertRule {
            name: "high_error_rate".to_string(),
            condition: AlertCondition::ErrorRateAbove(0.5),
            severity: AlertSeverity::Critical,
            cooldown_secs: 300,
        });
        engine.add_rule(AlertRule {
            name: "low_hand_completion".to_string(),
            condition: AlertCondition::HandCompletionBelow(0.5),
            severity: AlertSeverity::Warning,
            cooldown_secs: 600,
        });
        engine.add_rule(AlertRule {
            name: "high_latency".to_string(),
            condition: AlertCondition::LatencyAbove(5000.0),
            severity: AlertSeverity::Warning,
            cooldown_secs: 300,
        });
        engine
    }

    /// Add a rule to the engine.  If a rule with the same name exists it is replaced.
    pub fn add_rule(&self, rule: AlertRule) {
        let mut rules = self.rules.write().unwrap();
        // Replace if same name exists
        if let Some(pos) = rules.iter().position(|r| r.name == rule.name) {
            rules[pos] = rule;
        } else {
            rules.push(rule);
        }
    }

    /// Evaluate all rules against the given metrics snapshot, respecting cooldowns.
    pub fn check_alerts(&self, metrics: &Metrics) -> Vec<FiredAlert> {
        let mut fired = Vec::new();
        let now = Instant::now();
        let rules = self.rules.read().unwrap();
        let mut last_fired = self.last_fired.lock().unwrap();

        for rule in rules.iter() {
            // Check cooldown
            if let Some(last) = last_fired.get(&rule.name) {
                if let Some(elapsed) = now.checked_duration_since(*last) {
                    if elapsed.as_secs() < rule.cooldown_secs {
                        continue;
                    }
                }
            }
            // Evaluate condition
            if rule.condition.evaluate(metrics) {
                last_fired.insert(rule.name.clone(), now);
                fired.push(FiredAlert {
                    rule_name: rule.name.clone(),
                    severity: rule.severity,
                    message: format!(
                        "[{:?}] Alert '{}' fired",
                        rule.severity, rule.name
                    ),
                });
            }
        }

        fired
    }

    /// Number of configured rules.
    pub fn rule_count(&self) -> usize {
        self.rules.read().unwrap().len()
    }

    /// Reset all cooldowns (useful for testing).
    pub fn reset_cooldowns(&self) {
        self.last_fired.lock().unwrap().clear();
    }
}

impl Default for AlertEngine {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- ObservabilityEngine: construction --

    #[test]
    fn test_new_engine_empty() {
        let engine = ObservabilityEngine::new();
        assert_eq!(engine.dispatch_count(), 0);
        assert_eq!(engine.llm_call_count(), 0);
        assert_eq!(engine.hand_count(), 0);
    }

    #[test]
    fn test_default_trait() {
        let engine = ObservabilityEngine::default();
        assert_eq!(engine.dispatch_count(), 0);
    }

    // -- ObservabilityEngine: dispatch recording --

    #[test]
    fn test_record_dispatch() {
        let engine = ObservabilityEngine::new();
        engine.record_dispatch("z13", "shell", 120, true);
        engine.record_dispatch("m1-mac", "web_search", 250, false);
        assert_eq!(engine.dispatch_count(), 2);
    }

    #[test]
    fn test_avg_dispatch_latency() {
        let engine = ObservabilityEngine::new();
        engine.record_dispatch("w1", "t1", 100, true);
        engine.record_dispatch("w1", "t2", 300, true);
        let avg = engine.avg_dispatch_latency_ms();
        assert!((avg - 200.0).abs() < 0.01, "expected ~200, got {}", avg);
    }

    #[test]
    fn test_avg_dispatch_latency_empty() {
        let engine = ObservabilityEngine::new();
        assert_eq!(engine.avg_dispatch_latency_ms(), 0.0);
    }

    #[test]
    fn test_tool_success_rate() {
        let engine = ObservabilityEngine::new();
        engine.record_dispatch("w1", "t1", 10, true);
        engine.record_dispatch("w1", "t2", 20, true);
        engine.record_dispatch("w1", "t3", 30, false);
        let rate = engine.tool_success_rate();
        assert!((rate - 2.0 / 3.0).abs() < 0.01);
    }

    #[test]
    fn test_tool_success_rate_empty() {
        let engine = ObservabilityEngine::new();
        assert_eq!(engine.tool_success_rate(), 1.0);
    }

    #[test]
    fn test_tool_success_rate_all_failures() {
        let engine = ObservabilityEngine::new();
        engine.record_dispatch("w1", "t1", 10, false);
        engine.record_dispatch("w1", "t2", 20, false);
        assert_eq!(engine.tool_success_rate(), 0.0);
    }

    #[test]
    fn test_worker_utilization() {
        let engine = ObservabilityEngine::new();
        engine.record_dispatch("z13", "shell", 10, true);
        engine.record_dispatch("z13", "web_search", 20, true);
        engine.record_dispatch("m1-mac", "ai_code", 30, true);
        let util = engine.worker_utilization();
        assert_eq!(util.get("z13"), Some(&2));
        assert_eq!(util.get("m1-mac"), Some(&1));
    }

    // -- ObservabilityEngine: LLM recording --

    #[test]
    fn test_record_llm_call() {
        let engine = ObservabilityEngine::new();
        engine.record_llm_call("gemini", "gemini-2.5-pro", 1500, 800);
        engine.record_llm_call("ollama", "llama3.2:1b", 200, 150);
        assert_eq!(engine.llm_call_count(), 2);
    }

    #[test]
    fn test_llm_tokens_per_minute_single() {
        let engine = ObservabilityEngine::new();
        engine.record_llm_call("gemini", "model", 500, 100);
        // Single call: returns raw tokens
        assert_eq!(engine.llm_tokens_per_minute(), 500.0);
    }

    #[test]
    fn test_llm_tokens_per_minute_empty() {
        let engine = ObservabilityEngine::new();
        assert_eq!(engine.llm_tokens_per_minute(), 0.0);
    }

    // -- ObservabilityEngine: hand recording --

    #[test]
    fn test_record_hand_result() {
        let engine = ObservabilityEngine::new();
        engine.record_hand_result("content", true, 120.5);
        engine.record_hand_result("seo_content", false, 60.0);
        assert_eq!(engine.hand_count(), 2);
    }

    #[test]
    fn test_hand_completion_rate() {
        let engine = ObservabilityEngine::new();
        engine.record_hand_result("a", true, 10.0);
        engine.record_hand_result("b", true, 20.0);
        engine.record_hand_result("c", false, 5.0);
        engine.record_hand_result("d", true, 15.0);
        let rate = engine.hand_completion_rate();
        assert!((rate - 0.75).abs() < 0.01);
    }

    #[test]
    fn test_hand_completion_rate_empty() {
        let engine = ObservabilityEngine::new();
        assert_eq!(engine.hand_completion_rate(), 1.0);
    }

    // -- ObservabilityEngine: error rates --

    #[test]
    fn test_error_rate_by_tool() {
        let engine = ObservabilityEngine::new();
        engine.record_dispatch("w1", "shell", 10, true);
        engine.record_dispatch("w1", "shell", 20, false);
        engine.record_dispatch("w1", "web_search", 30, true);
        engine.record_dispatch("w1", "web_search", 40, true);
        let rates = engine.error_rate_by_tool();
        assert!((rates["shell"] - 0.5).abs() < 0.01);
        assert!((rates["web_search"] - 0.0).abs() < 0.01);
    }

    // -- ObservabilityEngine: snapshot --

    #[test]
    fn test_snapshot() {
        let engine = ObservabilityEngine::new();
        engine.record_dispatch("w1", "t1", 100, true);
        engine.record_llm_call("p1", "m1", 500, 200);
        engine.record_hand_result("h1", true, 30.0);
        let snap = engine.snapshot();
        assert_eq!(snap.dispatch_count, 1);
        assert_eq!(snap.llm_call_count, 1);
        assert_eq!(snap.hand_count, 1);
        assert_eq!(snap.tool_success_rate, 1.0);
    }

    // -- ObservabilityEngine: worker registration --

    #[test]
    fn test_register_worker() {
        let engine = ObservabilityEngine::new();
        engine.register_worker("z13");
        engine.register_worker("z13"); // duplicate
        engine.register_worker("m1-mac");
        let workers = engine.known_workers.read().unwrap();
        assert_eq!(workers.len(), 2);
    }

    // -- Prometheus export --

    #[test]
    fn test_prometheus_contains_counters() {
        let engine = ObservabilityEngine::new();
        engine.record_dispatch("w1", "shell", 50, true);
        let prom = engine.to_prometheus_text();
        assert!(prom.contains("phantom_mesh_obs_dispatch_total 1"));
    }

    #[test]
    fn test_prometheus_contains_gauges() {
        let engine = ObservabilityEngine::new();
        engine.record_dispatch("w1", "t1", 100, true);
        engine.record_dispatch("w1", "t2", 200, false);
        let prom = engine.to_prometheus_text();
        assert!(prom.contains("phantom_mesh_obs_tool_success_rate"));
        assert!(prom.contains("phantom_mesh_obs_avg_dispatch_latency_ms"));
    }

    #[test]
    fn test_prometheus_histogram_buckets() {
        let engine = ObservabilityEngine::new();
        engine.record_dispatch("w1", "t1", 15, true);
        engine.record_dispatch("w1", "t2", 5000, true);
        let prom = engine.to_prometheus_text();
        assert!(prom.contains("phantom_mesh_obs_dispatch_latency_ms_bucket"));
        assert!(prom.contains("phantom_mesh_obs_dispatch_latency_ms_count 2"));
        assert!(prom.contains("phantom_mesh_obs_dispatch_latency_ms_sum 5015"));
    }

    #[test]
    fn test_prometheus_per_worker_labels() {
        let engine = ObservabilityEngine::new();
        engine.record_dispatch("z13", "shell", 10, true);
        engine.record_dispatch("m1-mac", "ai_code", 20, true);
        let prom = engine.to_prometheus_text();
        assert!(prom.contains("worker=\"z13\""));
        assert!(prom.contains("worker=\"m1-mac\""));
    }

    #[test]
    fn test_prometheus_llm_histogram() {
        let engine = ObservabilityEngine::new();
        engine.record_llm_call("gemini", "2.5pro", 1000, 500);
        let prom = engine.to_prometheus_text();
        assert!(prom.contains("phantom_mesh_obs_llm_latency_ms_bucket"));
        assert!(prom.contains("phantom_mesh_obs_llm_latency_ms_count 1"));
        assert!(prom.contains("phantom_mesh_obs_llm_latency_ms_sum 500"));
    }

    #[test]
    fn test_prometheus_empty() {
        let engine = ObservabilityEngine::new();
        let prom = engine.to_prometheus_text();
        assert!(prom.contains("phantom_mesh_obs_dispatch_total 0"));
        assert!(prom.contains("phantom_mesh_obs_llm_call_total 0"));
    }

    #[test]
    fn test_prometheus_per_tool_error_rate() {
        let engine = ObservabilityEngine::new();
        engine.record_dispatch("w1", "shell", 10, false);
        engine.record_dispatch("w1", "shell", 20, false);
        let prom = engine.to_prometheus_text();
        assert!(prom.contains("phantom_mesh_obs_tool_error_rate{tool=\"shell\"}"));
    }

    // -- Dashboard summary --

    #[test]
    fn test_dashboard_summary_basic() {
        let engine = ObservabilityEngine::new();
        engine.record_dispatch("w1", "shell", 100, true);
        engine.record_llm_call("gemini", "model", 500, 200);
        engine.record_hand_result("content", true, 60.0);
        let summary = engine.dashboard_summary();
        assert_eq!(summary.dispatch_count, 1);
        assert_eq!(summary.llm_call_count, 1);
        assert_eq!(summary.hand_count, 1);
        assert_eq!(summary.tool_success_rate, 1.0);
    }

    #[test]
    fn test_dashboard_top_workers() {
        let engine = ObservabilityEngine::new();
        for _ in 0..10 {
            engine.record_dispatch("z13", "shell", 10, true);
        }
        for _ in 0..3 {
            engine.record_dispatch("m1-mac", "ai_code", 20, true);
        }
        let summary = engine.dashboard_summary();
        assert_eq!(summary.top_workers[0].name, "z13");
        assert_eq!(summary.top_workers[0].dispatch_count, 10);
    }

    #[test]
    fn test_dashboard_llm_providers() {
        let engine = ObservabilityEngine::new();
        engine.record_llm_call("gemini", "pro", 1000, 300);
        engine.record_llm_call("gemini", "flash", 500, 100);
        engine.record_llm_call("ollama", "llama", 200, 50);
        let summary = engine.dashboard_summary();
        // gemini has 2 calls, ollama has 1 — gemini should be first
        assert_eq!(summary.llm_providers[0].provider, "gemini");
        assert_eq!(summary.llm_providers[0].call_count, 2);
        assert_eq!(summary.llm_providers[0].total_tokens, 1500);
    }

    #[test]
    fn test_dashboard_serializable() {
        let engine = ObservabilityEngine::new();
        engine.record_dispatch("w1", "t1", 100, true);
        let summary = engine.dashboard_summary();
        let json = serde_json::to_value(&summary).unwrap();
        assert!(json["dispatch_count"].as_u64().is_some());
        assert!(json["top_workers"].as_array().is_some());
    }

    // -- AlertCondition evaluation --

    fn make_metrics(
        success_rate: f64,
        latency: f64,
        hand_rate: f64,
        tpm: f64,
        dispatch_count: u64,
    ) -> Metrics {
        Metrics {
            avg_dispatch_latency_ms: latency,
            tool_success_rate: success_rate,
            worker_utilization: HashMap::new(),
            llm_tokens_per_minute: tpm,
            hand_completion_rate: hand_rate,
            error_rate_by_tool: HashMap::new(),
            dispatch_count,
            llm_call_count: 0,
            hand_count: 0,
        }
    }

    #[test]
    fn test_alert_condition_error_rate_above() {
        let m = make_metrics(0.4, 100.0, 1.0, 0.0, 10); // error rate = 0.6
        assert!(AlertCondition::ErrorRateAbove(0.5).evaluate(&m));
        assert!(!AlertCondition::ErrorRateAbove(0.7).evaluate(&m));
    }

    #[test]
    fn test_alert_condition_latency_above() {
        let m = make_metrics(1.0, 6000.0, 1.0, 0.0, 1);
        assert!(AlertCondition::LatencyAbove(5000.0).evaluate(&m));
        assert!(!AlertCondition::LatencyAbove(7000.0).evaluate(&m));
    }

    #[test]
    fn test_alert_condition_hand_completion_below() {
        let m = make_metrics(1.0, 0.0, 0.3, 0.0, 0);
        assert!(AlertCondition::HandCompletionBelow(0.5).evaluate(&m));
        assert!(!AlertCondition::HandCompletionBelow(0.2).evaluate(&m));
    }

    #[test]
    fn test_alert_condition_tool_success_below() {
        let m = make_metrics(0.6, 0.0, 1.0, 0.0, 10);
        assert!(AlertCondition::ToolSuccessBelow(0.7).evaluate(&m));
        assert!(!AlertCondition::ToolSuccessBelow(0.5).evaluate(&m));
    }

    #[test]
    fn test_alert_condition_token_rate_above() {
        let m = make_metrics(1.0, 0.0, 1.0, 50000.0, 0);
        assert!(AlertCondition::TokenRateAbove(40000.0).evaluate(&m));
        assert!(!AlertCondition::TokenRateAbove(60000.0).evaluate(&m));
    }

    #[test]
    fn test_alert_condition_dispatch_count_above() {
        let m = make_metrics(1.0, 0.0, 1.0, 0.0, 1000);
        assert!(AlertCondition::DispatchCountAbove(500).evaluate(&m));
        assert!(!AlertCondition::DispatchCountAbove(2000).evaluate(&m));
    }

    // -- AlertEngine --

    #[test]
    fn test_alert_engine_fires_critical() {
        let engine = AlertEngine::new();
        engine.add_rule(AlertRule {
            name: "high_error".to_string(),
            condition: AlertCondition::ErrorRateAbove(0.5),
            severity: AlertSeverity::Critical,
            cooldown_secs: 0,
        });

        let m = make_metrics(0.3, 0.0, 1.0, 0.0, 10); // 70% error
        let alerts = engine.check_alerts(&m);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].severity, AlertSeverity::Critical);
        assert_eq!(alerts[0].rule_name, "high_error");
    }

    #[test]
    fn test_alert_engine_respects_cooldown() {
        let engine = AlertEngine::new();
        engine.add_rule(AlertRule {
            name: "test_rule".to_string(),
            condition: AlertCondition::LatencyAbove(100.0),
            severity: AlertSeverity::Warning,
            cooldown_secs: 3600, // 1 hour
        });

        let m = make_metrics(1.0, 200.0, 1.0, 0.0, 1);

        // First check should fire
        let a1 = engine.check_alerts(&m);
        assert_eq!(a1.len(), 1);

        // Second check within cooldown should NOT fire
        let a2 = engine.check_alerts(&m);
        assert_eq!(a2.len(), 0);
    }

    #[test]
    fn test_alert_engine_no_fire_when_ok() {
        let engine = AlertEngine::new();
        engine.add_rule(AlertRule {
            name: "high_error".to_string(),
            condition: AlertCondition::ErrorRateAbove(0.5),
            severity: AlertSeverity::Critical,
            cooldown_secs: 0,
        });

        let m = make_metrics(0.95, 50.0, 1.0, 0.0, 20); // only 5% error
        let alerts = engine.check_alerts(&m);
        assert!(alerts.is_empty());
    }

    #[test]
    fn test_alert_engine_multiple_rules() {
        let engine = AlertEngine::new();
        engine.add_rule(AlertRule {
            name: "err".to_string(),
            condition: AlertCondition::ErrorRateAbove(0.5),
            severity: AlertSeverity::Critical,
            cooldown_secs: 0,
        });
        engine.add_rule(AlertRule {
            name: "lat".to_string(),
            condition: AlertCondition::LatencyAbove(1000.0),
            severity: AlertSeverity::Warning,
            cooldown_secs: 0,
        });

        let m = make_metrics(0.3, 2000.0, 1.0, 0.0, 10);
        let alerts = engine.check_alerts(&m);
        assert_eq!(alerts.len(), 2);
    }

    #[test]
    fn test_alert_engine_reset_cooldowns() {
        let engine = AlertEngine::new();
        engine.add_rule(AlertRule {
            name: "test".to_string(),
            condition: AlertCondition::LatencyAbove(50.0),
            severity: AlertSeverity::Info,
            cooldown_secs: 3600,
        });

        let m = make_metrics(1.0, 100.0, 1.0, 0.0, 1);

        let a1 = engine.check_alerts(&m);
        assert_eq!(a1.len(), 1);

        // Suppressed by cooldown
        let a2 = engine.check_alerts(&m);
        assert_eq!(a2.len(), 0);

        // Reset and fire again
        engine.reset_cooldowns();
        let a3 = engine.check_alerts(&m);
        assert_eq!(a3.len(), 1);
    }

    #[test]
    fn test_alert_engine_default() {
        let engine = AlertEngine::default();
        assert_eq!(engine.rule_count(), 0);
    }

    #[test]
    fn test_alert_with_defaults_has_rules() {
        let engine = AlertEngine::with_defaults();
        assert_eq!(engine.rule_count(), 3);
    }

    #[test]
    fn test_alert_add_rule_replace() {
        let engine = AlertEngine::new();
        engine.add_rule(AlertRule {
            name: "test".to_string(),
            condition: AlertCondition::LatencyAbove(100.0),
            severity: AlertSeverity::Info,
            cooldown_secs: 60,
        });
        assert_eq!(engine.rule_count(), 1);

        // Replace with same name
        engine.add_rule(AlertRule {
            name: "test".to_string(),
            condition: AlertCondition::LatencyAbove(200.0),
            severity: AlertSeverity::Critical,
            cooldown_secs: 120,
        });
        assert_eq!(engine.rule_count(), 1); // still 1 — replaced, not added
    }

    // -- Serialization --

    #[test]
    fn test_fired_alert_serializable() {
        let alert = FiredAlert {
            rule_name: "test".to_string(),
            severity: AlertSeverity::Warning,
            message: "test alert".to_string(),
        };
        let json = serde_json::to_value(&alert).unwrap();
        assert_eq!(json["rule_name"], "test");
        assert_eq!(json["severity"], "Warning");
    }

    #[test]
    fn test_metrics_serializable() {
        let metrics = make_metrics(0.95, 123.45, 0.9, 5000.0, 100);
        let json = serde_json::to_value(&metrics).unwrap();
        assert_eq!(json["dispatch_count"], 100);
        assert!((json["tool_success_rate"].as_f64().unwrap() - 0.95).abs() < 0.001);
    }

    // -- Integration: engine + alerts --

    #[test]
    fn test_engine_snapshot_feeds_alerts() {
        let obs = ObservabilityEngine::new();
        // Record many failures
        for _ in 0..8 {
            obs.record_dispatch("w1", "shell", 100, false);
        }
        for _ in 0..2 {
            obs.record_dispatch("w1", "shell", 50, true);
        }

        let alert_engine = AlertEngine::with_defaults();
        let snap = obs.snapshot();
        let alerts = alert_engine.check_alerts(&snap);
        // error rate = 80% > 50% threshold => should fire high_error_rate
        assert!(alerts.iter().any(|a| a.rule_name == "high_error_rate"));
    }
}
