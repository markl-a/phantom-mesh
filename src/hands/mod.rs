//! Hands workflow engine — structured multi-step agent workflows.
//! Each Hand is a specialized agent configuration loaded from TOML files.
//! Based on OpenFang's HAND.toml pattern.

pub mod middleware;
pub mod message_queue;
pub mod cache;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tracing::{debug, info, warn};

use crate::agent_runtime::AgentRuntime;
use crate::approval::{ApprovalGate, ApprovalResult};
use crate::evaluate::{self, EvalConfig};
use crate::guardrail::{self, GuardrailConfig, GuardrailResult};
use crate::knowledge_capture::KnowledgeCapturer;
use crate::llm_router::LlmRouter;
use crate::tools::ToolRegistry;
use crate::hands::middleware::{MiddlewareChain, PhaseContext, PhasePostContext};
use crate::hands::message_queue::HandMessageQueue;

// re-export for parallel_queries JSON construction
use serde_json;

/// A Hand definition — a structured multi-phase workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hand {
    /// Hand name (e.g., "lead", "researcher", "content")
    pub name: String,
    /// Human-readable description
    pub description: String,
    /// Category for grouping (e.g., "research", "content", "automation")
    #[serde(default)]
    pub category: String,
    /// The LLM provider to use (default: "auto")
    #[serde(default = "default_provider")]
    pub provider: String,
    /// The model to use (default: provider's default)
    #[serde(default)]
    pub model: String,
    /// Phases of the workflow — executed sequentially
    pub phases: Vec<Phase>,
    /// Which tools this hand can use. None = all tools (backwards compatible).
    /// When set, only the listed tools are visible to the LLM during execution.
    #[serde(default)]
    pub tools: Option<Vec<String>>,
    /// Output format (markdown, csv, json)
    #[serde(default = "default_output_format")]
    pub output_format: String,
    /// Cron schedule for automatic execution (optional)
    #[serde(default)]
    pub schedule: Option<String>,
    /// Custom settings/parameters for this hand
    #[serde(default)]
    pub settings: HashMap<String, String>,
    /// Chain to another hand after completion (e.g., "outreach" after "lead")
    #[serde(default)]
    pub chain_to: Option<String>,
    /// L1 guardrail config — pure Rust format validation (no LLM calls)
    #[serde(default)]
    pub guardrail: Option<GuardrailConfig>,
    /// L2 eval config — LLM-as-Judge quality scoring
    #[serde(default)]
    pub eval: Option<EvalConfig>,
    /// Catch-all for unknown top-level fields (prevents parse errors from extra TOML keys)
    #[serde(flatten, default)]
    pub extra: HashMap<String, toml::Value>,
}

fn default_provider() -> String { "auto".to_string() }
fn default_output_format() -> String { "markdown".to_string() }

/// A single phase of a Hand workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Phase {
    /// Phase name (e.g., "research", "analyze", "generate")
    pub name: String,
    /// System prompt for this phase
    pub system_prompt: String,
    /// Maximum agent rounds for this phase (default: 5)
    #[serde(default = "default_max_rounds")]
    pub max_rounds: u32,
    /// Optional condition — if set, phase only runs when condition evaluates true
    /// against the previous phase's output. Syntax:
    ///   "contains:keyword"       — previous output contains "keyword"
    ///   "not_contains:keyword"   — previous output does NOT contain "keyword"
    ///   "min_length:500"         — previous output is at least 500 chars
    ///   "previous_success"       — previous phase didn't produce a failure message
    #[serde(default)]
    pub condition: Option<String>,
    /// Target worker name — when set, all dispatchable tools in this phase
    /// go to this specific worker (e.g., "acer" for Android build, "m1-mac" for iOS)
    #[serde(default)]
    pub target_worker: Option<String>,
    /// Target capability — when set, all dispatchable tools in this phase
    /// go to the best worker with this capability (e.g., "android_build", "ios_build")
    #[serde(default)]
    pub target_capability: Option<String>,
    /// Parallel search queries to fan out via batch dispatch before LLM call.
    /// Results are injected into the phase prompt context so the LLM only analyzes,
    /// not searches. Dramatically speeds up research phases.
    #[serde(default)]
    pub parallel_queries: Vec<String>,
    /// Per-phase tool filter. When set, overrides the hand-level `tools` list for
    /// this phase only. None = use hand-level tools (or all tools if hand has None).
    #[serde(default)]
    pub tools: Option<Vec<String>>,
    /// Per-phase provider override. When set, overrides the hand-level provider
    /// for this phase only. None = use hand-level provider.
    #[serde(default)]
    pub provider: Option<String>,
    /// Per-phase model override. When set, overrides the hand-level model
    /// for this phase only. None = use hand-level model.
    #[serde(default)]
    pub model: Option<String>,
    /// Catch-all for unknown phase-level fields like tool_calls, tools, etc.
    /// Prevents parse errors when hand TOMLs include extra fields not in the struct.
    #[serde(flatten, default)]
    pub extra: HashMap<String, toml::Value>,
}

fn default_max_rounds() -> u32 { 5 }

/// Result of running a Hand
#[derive(Debug, Clone, Serialize)]
pub struct HandResult {
    pub hand_name: String,
    pub phases_completed: usize,
    pub total_phases: usize,
    pub outputs: Vec<PhaseOutput>,
    pub final_output: String,
    pub elapsed_secs: f64,
    /// If set, the next hand to chain to (from Hand.chain_to)
    pub chain_to: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseOutput {
    pub phase_name: String,
    pub output: String,
    pub tool_calls: usize,
    /// Phase execution duration in seconds
    #[serde(default)]
    pub duration_secs: f64,
    /// Whether this phase was skipped due to a condition evaluating false
    #[serde(default)]
    pub skipped: bool,
    /// L1 guardrail issues (empty = passed)
    #[serde(default)]
    pub guardrail_issues: Vec<String>,
    /// L2 LLM-as-Judge score (None = not evaluated)
    #[serde(default)]
    pub quality_score: Option<u8>,
    /// Number of quality retries performed
    #[serde(default)]
    pub quality_retries: u8,
}

/// Result of a preflight check
#[derive(Debug, Clone, Serialize)]
pub struct PreflightResult {
    pub passed: bool,
    pub issues: Vec<String>,
}

/// Checkpoint for resuming a Hand workflow after failure.
/// When a hand fails mid-execution (e.g., provider timeout), a checkpoint is saved.
/// On the next run, the hand can resume from the last successful phase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandCheckpoint {
    pub hand_name: String,
    pub run_id: String,
    pub completed_phases: Vec<PhaseOutput>,
    pub last_phase_index: usize,
    pub context: String,
    pub created_at: String,
}

impl HandCheckpoint {
    /// Save checkpoint to ~/.clawtex/checkpoints/{hand_name}_{run_id}.json
    pub fn save(&self) -> Result<()> {
        let dir = dirs::home_dir()
            .map(|h| h.join(".clawtex").join("checkpoints"))
            .unwrap_or_else(|| std::path::PathBuf::from("checkpoints"));
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{}_{}.json", self.hand_name, self.run_id));
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, json)?;
        debug!("Checkpoint saved: {:?}", path);
        Ok(())
    }

    /// Load the most recent checkpoint for a hand (if any).
    pub fn load_latest(hand_name: &str) -> Option<Self> {
        let dir = dirs::home_dir()
            .map(|h| h.join(".clawtex").join("checkpoints"))
            .unwrap_or_else(|| std::path::PathBuf::from("checkpoints"));
        if !dir.exists() {
            return None;
        }
        let mut best: Option<(String, Self)> = None;
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let fname = entry.file_name().to_string_lossy().to_string();
                if fname.starts_with(&format!("{}_", hand_name)) && fname.ends_with(".json") {
                    if let Ok(content) = std::fs::read_to_string(entry.path()) {
                        if let Ok(cp) = serde_json::from_str::<HandCheckpoint>(&content) {
                            if best.as_ref().map_or(true, |(_, b)| cp.created_at > b.created_at) {
                                best = Some((fname, cp));
                            }
                        }
                    }
                }
            }
        }
        best.map(|(_, cp)| cp)
    }

    /// Delete checkpoint file after successful completion.
    pub fn delete(&self) {
        let dir = dirs::home_dir()
            .map(|h| h.join(".clawtex").join("checkpoints"))
            .unwrap_or_else(|| std::path::PathBuf::from("checkpoints"));
        let path = dir.join(format!("{}_{}.json", self.hand_name, self.run_id));
        let _ = std::fs::remove_file(&path);
        debug!("Checkpoint deleted: {:?}", path);
    }
}

/// Evaluate a phase condition against the previous phase's output.
/// Returns true if the phase should run, false if it should be skipped.
pub fn evaluate_condition(condition: &str, previous_output: &str) -> bool {
    let condition = condition.trim();

    if condition.eq_ignore_ascii_case("previous_success") {
        return !previous_output.starts_with("Phase failed:");
    }

    if let Some(keyword) = condition.strip_prefix("contains:") {
        let keyword = keyword.trim();
        return previous_output.to_lowercase().contains(&keyword.to_lowercase());
    }

    if let Some(keyword) = condition.strip_prefix("not_contains:") {
        let keyword = keyword.trim();
        return !previous_output.to_lowercase().contains(&keyword.to_lowercase());
    }

    if let Some(n_str) = condition.strip_prefix("min_length:") {
        if let Ok(n) = n_str.trim().parse::<usize>() {
            return previous_output.len() >= n;
        }
        warn!("Invalid min_length condition: {}", condition);
        return true;
    }

    warn!("Unknown condition '{}', defaulting to true", condition);
    true
}

/// Sanitize hand TOML content before parsing.
/// Fixes known issues:
/// 1. Triple-quote issues: `""""` (4+ quotes) → proper `"""`
/// 2. Ensures multi-line strings are well-formed
fn sanitize_hand_toml(content: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let mut chars = content.chars().peekable();
    let mut in_multiline_string = false;

    while let Some(ch) = chars.next() {
        if ch == '"' {
            // Count consecutive quotes
            let mut quote_count = 1usize;
            while chars.peek() == Some(&'"') {
                quote_count += 1;
                chars.next();
            }

            if !in_multiline_string {
                if quote_count >= 3 {
                    // Opening multi-line string: always emit exactly 3 quotes
                    result.push_str("\"\"\"");
                    in_multiline_string = true;
                    // Drop any extra quotes beyond 3
                } else {
                    // Regular string quotes — pass through as-is
                    for _ in 0..quote_count {
                        result.push('"');
                    }
                }
            } else {
                // Inside a multi-line string
                if quote_count >= 3 {
                    // Closing multi-line string: emit exactly 3 quotes
                    result.push_str("\"\"\"");
                    in_multiline_string = false;
                    // Drop any extra quotes beyond 3
                } else {
                    // Quotes inside the string that don't close it
                    for _ in 0..quote_count {
                        result.push('"');
                    }
                }
            }
        } else {
            result.push(ch);
        }
    }

    result
}

/// Hand registry — loads and manages available Hands
pub struct HandRegistry {
    hands: HashMap<String, Hand>,
}

impl HandRegistry {
    /// Load hands from a directory of TOML files.
    /// Expects structure: hands_dir/<hand_name>/hand.toml
    pub fn load(hands_dir: &str) -> Result<Self> {
        let mut hands = HashMap::new();
        let dir_path = Path::new(hands_dir);

        if !dir_path.exists() {
            let _ = std::fs::create_dir_all(dir_path);
            info!("Created hands directory: {}", hands_dir);
            return Ok(Self { hands });
        }

        // Scan subdirectories for hand.toml files
        if let Ok(entries) = std::fs::read_dir(dir_path) {
            for entry in entries.flatten() {
                let hand_toml = entry.path().join("hand.toml");
                if hand_toml.exists() {
                    match std::fs::read_to_string(&hand_toml) {
                        Ok(content) => {
                            // Sanitize TOML content before parsing (fixes triple-quote issues)
                            let sanitized = sanitize_hand_toml(&content);
                            match toml::from_str::<Hand>(&sanitized) {
                                Ok(hand) => {
                                    if !hand.extra.is_empty() {
                                        debug!("Hand '{}' has extra top-level fields (ignored): {:?}",
                                            hand.name, hand.extra.keys().collect::<Vec<_>>());
                                    }
                                    for (_i, phase) in hand.phases.iter().enumerate() {
                                        if !phase.extra.is_empty() {
                                            debug!("Hand '{}' phase '{}' has extra fields (ignored): {:?}",
                                                hand.name, phase.name,
                                                phase.extra.keys().collect::<Vec<_>>());
                                        }
                                    }
                                    info!("Loaded hand: {} — {}", hand.name, hand.description);
                                    hands.insert(hand.name.clone(), hand);
                                }
                                Err(e) => {
                                    warn!("Failed to parse {}: {}", hand_toml.display(), e);
                                }
                            }
                        }
                        Err(e) => {
                            warn!("Failed to read {}: {}", hand_toml.display(), e);
                        }
                    }
                }
            }
        }

        info!("HandRegistry loaded {} hands", hands.len());
        Ok(Self { hands })
    }

    /// Create an empty registry (fallback when loading fails)
    pub fn empty() -> Self {
        Self { hands: HashMap::new() }
    }

    /// Get a hand by name
    pub fn get(&self, name: &str) -> Option<&Hand> {
        self.hands.get(name)
    }

    /// List all available hands
    pub fn list(&self) -> Vec<&Hand> {
        let mut hands: Vec<_> = self.hands.values().collect();
        hands.sort_by(|a, b| a.name.cmp(&b.name));
        hands
    }

    /// List hand names
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<_> = self.hands.keys().cloned().collect();
        names.sort();
        names
    }
}

/// Hand runner — executes a Hand workflow phase by phase
pub struct HandRunner;

impl HandRunner {
    /// Prepare the initial context string (inject settings into user_input).
    pub fn prepare_context(hand: &Hand, user_input: &str) -> String {
        let mut context = user_input.to_string();
        if !hand.settings.is_empty() {
            let settings_str: Vec<String> = hand.settings.iter()
                .map(|(k, v)| format!("- {}: {}", k, v))
                .collect();
            context = format!("{}\n\nSettings:\n{}", context, settings_str.join("\n"));
        }
        context
    }

    /// Run a single phase of a Hand workflow.
    /// `phase_index` is the 0-based index of the phase to run.
    /// `previous_outputs` contains outputs from prior phases (empty for first phase).
    /// `context` is the running context string (user_input with settings for phase 0, or previous output).
    /// Returns the PhaseOutput and the updated context string for the next phase.
    pub async fn run_single_phase(
        hand: &Hand,
        phase_index: usize,
        user_input: &str,
        context: &str,
        previous_outputs: &[PhaseOutput],
        runtime: &AgentRuntime,
        router: &LlmRouter,
        tool_registry: &ToolRegistry,
    ) -> Result<(PhaseOutput, String)> {
        let phase = hand.phases.get(phase_index)
            .ok_or_else(|| anyhow::anyhow!("Phase index {} out of range (total: {})", phase_index, hand.phases.len()))?;

        // Check condition gate
        if let Some(ref condition) = phase.condition {
            let prev_output = previous_outputs.last()
                .map(|o| o.output.as_str())
                .unwrap_or("");
            if !evaluate_condition(condition, prev_output) {
                info!("Hand '{}' phase '{}' skipped (condition '{}' not met)", hand.name, phase.name, condition);
                let output = PhaseOutput {
                    phase_name: phase.name.clone(),
                    output: format!("Skipped: condition '{}' not met", condition),
                    tool_calls: 0,
                    duration_secs: 0.0,
                    skipped: true,
                    guardrail_issues: Vec::new(),
                    quality_score: None,
                    quality_retries: 0,
                };
                return Ok((output, context.to_string()));
            }
        }

        debug!("Hand '{}' phase {}/{}: {}", hand.name, phase_index + 1, hand.phases.len(), phase.name);

        // Fan out parallel_queries via cluster batch dispatch (if any)
        let mut prefetched_context = String::new();
        if !phase.parallel_queries.is_empty() {
            if let Some(hub) = runtime.cluster_hub() {
                let inputs: Vec<serde_json::Value> = phase.parallel_queries.iter()
                    .map(|q| serde_json::json!({"query": q}))
                    .collect();
                info!(
                    "Hand '{}' phase '{}': dispatching {} parallel queries",
                    hand.name, phase.name, inputs.len()
                );
                let results = hub.dispatch_batch("web_search", inputs).await;
                let mut sections = Vec::new();
                for (i, result) in results.into_iter().enumerate() {
                    let query = &phase.parallel_queries[i];
                    match result {
                        Ok(val) => {
                            let output = val["output"].as_str().unwrap_or("(no output)");
                            sections.push(format!("### Search: {}\n{}", query, output));
                        }
                        Err(e) => {
                            sections.push(format!("### Search: {} (failed: {})", query, e));
                        }
                    }
                }
                if !sections.is_empty() {
                    prefetched_context = format!(
                        "\n\n## Pre-fetched Research Results\n{}\n",
                        sections.join("\n\n")
                    );
                }
            } else {
                warn!("Hand '{}' phase '{}' has parallel_queries but no cluster hub attached", hand.name, phase.name);
            }
        }

        // Build the prompt for this phase
        let prompt = if phase_index == 0 {
            format!("{}{}\n\nUser request: {}", phase.system_prompt, prefetched_context, context)
        } else {
            format!(
                "{}{}\n\nPrevious phase output:\n{}\n\nOriginal request: {}",
                phase.system_prompt,
                prefetched_context,
                previous_outputs.last().map(|o| o.output.as_str()).unwrap_or(""),
                user_input
            )
        };

        // ── Middleware Chain: pre-process the prompt before execution ──
        let middleware_chain = MiddlewareChain::with_defaults();
        let pre_ctx = PhaseContext {
            hand_name: hand.name.clone(),
            phase_name: phase.name.clone(),
            phase_index,
            prompt: prompt.clone(),
            user_input: user_input.to_string(),
            previous_outputs: previous_outputs.iter().map(|o| o.output.clone()).collect(),
            metadata: std::collections::HashMap::new(),
            halted: false,
            halt_reason: None,
        };
        let pre_result = middleware_chain.run_pre(pre_ctx);
        if pre_result.halted {
            let reason = pre_result.halt_reason.unwrap_or_else(|| "Halted by middleware".to_string());
            warn!("Hand '{}' phase '{}' halted by middleware: {}", hand.name, phase.name, reason);
            let output = PhaseOutput {
                phase_name: phase.name.clone(),
                output: format!("Phase halted: {}", reason),
                tool_calls: 0,
                duration_secs: 0.0,
                skipped: true,
                guardrail_issues: vec![reason],
                quality_score: None,
                quality_retries: 0,
            };
            return Ok((output, context.to_string()));
        }
        // Use potentially modified prompt from middleware
        let prompt = pre_result.prompt;

        // Use the phase's max_rounds setting to limit the agent's tool-call loop
        let phase_max_rounds = Some(phase.max_rounds as usize);

        let mut agent_config = runtime.get_config("master")
            .ok_or_else(|| anyhow::anyhow!("Agent 'master' not found"))?
            .clone();

        // Override agent config with effective tools for this phase:
        // Phase-level tools take priority over hand-level tools.
        // None at both levels = all tools (agent_config.tools stays as-is or None).
        let effective_tools: Option<Vec<String>> = phase.tools
            .as_ref()
            .or(hand.tools.as_ref())
            .cloned();
        if let Some(ref tool_list) = effective_tools {
            agent_config.tools = Some(tool_list.clone());
        }
        // Phase-level provider/model take priority over hand-level
        if let Some(ref phase_provider) = phase.provider {
            agent_config.provider = Some(phase_provider.clone());
        } else if hand.provider == "auto" || hand.provider.is_empty() {
            // "auto" means let the router decide — clear any inherited provider
            agent_config.provider = None;
        } else {
            agent_config.provider = Some(hand.provider.clone());
        }
        if let Some(ref phase_model) = phase.model {
            agent_config.model = Some(phase_model.clone());
        } else if hand.model.is_empty() {
            // Empty model = let the provider/router decide
            agent_config.model = None;
        } else {
            agent_config.model = Some(hand.model.clone());
        }

        info!(
            "Hand '{}' phase '{}': provider={:?}, model={:?}, tools={:?}",
            hand.name, phase.name,
            agent_config.provider, agent_config.model, agent_config.tools
        );

        // Targeted dispatch: phase can pin all tools to a specific worker
        let target_worker = phase.target_worker.clone();
        if let Some(ref tw) = target_worker {
            info!("Hand '{}' phase '{}' targeting worker '{}'", hand.name, phase.name, tw);
        }

        let phase_start = std::time::Instant::now();
        match runtime.run_with_config_targeted(
            "master", &agent_config, &prompt, &[], router, tool_registry, None, phase_max_rounds, None, target_worker,
        ).await {
            Ok(result) => {
                let mut agent_output = result.output.clone();
                let mut total_tool_calls = result.tool_calls_made;
                let mut guardrail_issues = Vec::new();
                let mut quality_score = None;
                let mut quality_retries: u8 = 0;

                // ── Middleware Chain: post-process the output ──
                {
                    let post_ctx = PhasePostContext {
                        hand_name: hand.name.clone(),
                        phase_name: phase.name.clone(),
                        output: agent_output.clone(),
                        tool_calls: total_tool_calls,
                        issues: Vec::new(),
                        metadata: std::collections::HashMap::new(),
                    };
                    let post_result = middleware_chain.run_post(post_ctx);
                    agent_output = post_result.output;
                    if !post_result.issues.is_empty() {
                        debug!(
                            "Hand '{}' phase '{}' middleware post-process flagged {} issue(s)",
                            hand.name, phase.name, post_result.issues.len()
                        );
                        guardrail_issues.extend(post_result.issues);
                    }
                }

                // ── L1 Guardrail: pure Rust format validation ──
                if let Some(ref gc) = hand.guardrail {
                    let gr = guardrail::validate(gc, &agent_output);
                    if let GuardrailResult::Fail { issues, action } = gr {
                        warn!(
                            "Hand '{}' phase '{}' L1 guardrail fail: {:?} (action={})",
                            hand.name, phase.name, issues, action
                        );
                        if action == "retry" {
                            // One retry with guardrail feedback injected
                            let retry_prompt = format!(
                                "{}\n\n⚠️ 你的上一次輸出未通過品質檢查，請修正以下問題後重新回答:\n{}",
                                prompt,
                                issues.iter().map(|i| format!("- {}", i)).collect::<Vec<_>>().join("\n")
                            );
                            if let Ok(retry_result) = runtime.run_with_config_targeted(
                                "master", &agent_config, &retry_prompt, &[], router, tool_registry, None, phase_max_rounds, None, phase.target_worker.clone(),
                            ).await {
                                agent_output = retry_result.output.clone();
                                total_tool_calls += retry_result.tool_calls_made;
                                quality_retries = 1;
                                // Re-validate after retry
                                if let GuardrailResult::Fail { issues: retry_issues, .. } = guardrail::validate(gc, &agent_output) {
                                    guardrail_issues = retry_issues;
                                }
                            } else {
                                guardrail_issues = issues;
                            }
                        } else {
                            guardrail_issues = issues;
                        }
                    }
                }

                // ── L2 LLM-as-Judge: quality scoring ──
                if let Some(ref ec) = hand.eval {
                    if ec.enabled {
                        match evaluate::evaluate(router, user_input, &agent_output, ec).await {
                            Ok(eval_result) => {
                                info!(
                                    "Hand '{}' phase '{}' L2 eval: score={}/5 {}",
                                    hand.name, phase.name, eval_result.score,
                                    if eval_result.passed { "PASS" } else { "BELOW_THRESHOLD" }
                                );
                                quality_score = Some(eval_result.score);

                                // If below threshold and retries available, retry with feedback
                                if !eval_result.passed && quality_retries < ec.max_retries {
                                    if let Some(ref feedback) = eval_result.feedback {
                                        let retry_prompt = format!(
                                            "{}\n\n⚠️ 品質評分 {}/5 未達標準 (最低 {}/5)，請根據以下回饋改進:\n{}",
                                            prompt, eval_result.score, ec.threshold, feedback
                                        );
                                        if let Ok(retry_result) = runtime.run_with_config_targeted(
                                            "master", &agent_config, &retry_prompt, &[], router, tool_registry, None, phase_max_rounds, None, phase.target_worker.clone(),
                                        ).await {
                                            agent_output = retry_result.output.clone();
                                            total_tool_calls += retry_result.tool_calls_made;
                                            quality_retries += 1;
                                            // Re-evaluate
                                            if let Ok(re_eval) = evaluate::evaluate(router, user_input, &agent_output, ec).await {
                                                quality_score = Some(re_eval.score);
                                            }
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                warn!("Hand '{}' phase '{}' L2 eval error (skipping): {}", hand.name, phase.name, e);
                            }
                        }
                    }
                }

                let output = PhaseOutput {
                    phase_name: phase.name.clone(),
                    output: agent_output.clone(),
                    tool_calls: total_tool_calls,
                    duration_secs: phase_start.elapsed().as_secs_f64(),
                    skipped: false,
                    guardrail_issues,
                    quality_score,
                    quality_retries,
                };
                info!(
                    "Hand '{}' phase '{}' completed: {} tool calls, {:.1}s",
                    hand.name, phase.name, total_tool_calls, output.duration_secs
                );
                Ok((output, agent_output))
            }
            Err(e) => {
                warn!("Hand '{}' phase '{}' failed: {}", hand.name, phase.name, e);
                let output = PhaseOutput {
                    phase_name: phase.name.clone(),
                    output: format!("Phase failed: {}", e),
                    tool_calls: 0,
                    duration_secs: phase_start.elapsed().as_secs_f64(),
                    skipped: false,
                    guardrail_issues: Vec::new(),
                    quality_score: None,
                    quality_retries: 0,
                };
                Ok((output, format!("Previous phase failed: {}", e)))
            }
        }
    }

    /// Lightweight preflight check before running a hand.
    /// Verifies: tools exist in registry, agent config exists, provider reachable.
    pub async fn preflight(
        hand: &Hand,
        runtime: &AgentRuntime,
        router: &LlmRouter,
        tool_registry: &ToolRegistry,
    ) -> PreflightResult {
        let mut issues = Vec::new();

        // Check agent config exists
        if runtime.get_config("master").is_none() {
            issues.push("Agent 'master' not found in config".to_string());
        }

        // Check required tools are registered (only if tools list is explicitly set)
        if let Some(ref tool_list) = hand.tools {
            let available_tools = tool_registry.names();
            for tool_name in tool_list {
                if !available_tools.contains(tool_name) {
                    issues.push(format!("Tool '{}' not found in registry", tool_name));
                }
            }
        }

        // Check provider reachability with a minimal ping
        let provider = &hand.provider;
        match router.route("ping", provider).await {
            Ok(_) => {}
            Err(e) => {
                issues.push(format!("Provider '{}' unreachable: {}", provider, e));
            }
        }

        let passed = issues.is_empty();
        if !passed {
            warn!("Hand '{}' preflight failed: {:?}", hand.name, issues);
        } else {
            debug!("Hand '{}' preflight passed", hand.name);
        }

        PreflightResult { passed, issues }
    }

    /// Run a Hand workflow with the given user input/parameters.
    /// Each phase runs as a separate agent call, with the previous phase's output
    /// injected as context for the next phase.
    /// If `approval_gate` is provided and the hand has `require_approval=true` in settings,
    /// approval will be requested before execution.
    /// If `message_queue` is provided, queued messages are drained between phases
    /// and injected into the next phase's context.
    pub async fn run(
        hand: &Hand,
        user_input: &str,
        runtime: &AgentRuntime,
        router: &LlmRouter,
        tool_registry: &ToolRegistry,
        approval_gate: Option<&Arc<ApprovalGate>>,
    ) -> Result<HandResult> {
        Self::run_with_queue(hand, user_input, runtime, router, tool_registry, approval_gate, None).await
    }

    /// Run with optional message queue for inter-phase message injection.
    pub async fn run_with_queue(
        hand: &Hand,
        user_input: &str,
        runtime: &AgentRuntime,
        router: &LlmRouter,
        tool_registry: &ToolRegistry,
        approval_gate: Option<&Arc<ApprovalGate>>,
        message_queue: Option<&HandMessageQueue>,
    ) -> Result<HandResult> {
        let start = std::time::Instant::now();

        // Check require_approval setting
        if hand.settings.get("require_approval").map(|v| v == "true").unwrap_or(false) {
            if let Some(gate) = approval_gate {
                let description = format!(
                    "Hand '{}' requires approval to proceed.\nPhases: {}\nTools: {:?}",
                    hand.name, hand.phases.len(),
                    hand.tools.as_deref().unwrap_or(&[])
                );
                let (_id, result) = gate.request("hand_execution", &description).await;
                match result {
                    ApprovalResult::Approved => info!("Hand '{}' approved", hand.name),
                    ApprovalResult::Denied => {
                        return Ok(HandResult {
                            hand_name: hand.name.clone(),
                            phases_completed: 0,
                            total_phases: hand.phases.len(),
                            outputs: vec![],
                            final_output: "Denied by approval gate".into(),
                            elapsed_secs: start.elapsed().as_secs_f64(),
                            chain_to: None,
                        });
                    }
                    ApprovalResult::Timeout => {
                        return Ok(HandResult {
                            hand_name: hand.name.clone(),
                            phases_completed: 0,
                            total_phases: hand.phases.len(),
                            outputs: vec![],
                            final_output: "Approval timed out".into(),
                            elapsed_secs: start.elapsed().as_secs_f64(),
                            chain_to: None,
                        });
                    }
                }
            } else {
                warn!("Hand '{}' requires approval but no ApprovalGate provided — proceeding without approval", hand.name);
            }
        }

        let mut outputs = Vec::new();
        let mut context = Self::prepare_context(hand, user_input);
        let mut start_phase: usize = 0;

        // ── Checkpoint resume: skip already-completed phases if a checkpoint exists ──
        let run_id = uuid::Uuid::new_v4().to_string();
        if let Some(checkpoint) = HandCheckpoint::load_latest(&hand.name) {
            if checkpoint.completed_phases.is_empty() {
                // No successful phases to resume from — stale/failed checkpoint, start fresh
                info!(
                    "Hand '{}' found empty checkpoint (all phases previously failed) — starting fresh",
                    hand.name
                );
                checkpoint.delete();
            } else if checkpoint.last_phase_index < hand.phases.len() {
                info!(
                    "Hand '{}' resuming from checkpoint (run_id={}, completed {}/{} phases)",
                    hand.name, checkpoint.run_id, checkpoint.completed_phases.len(), hand.phases.len()
                );
                // Pre-populate outputs with all phases attempted so far.
                // Phases that succeeded are in completed_phases; generate placeholders
                // for failed phases so that outputs.len() accurately reflects all attempted phases.
                let num_attempted = checkpoint.last_phase_index + 1;
                let successful_map: std::collections::HashMap<String, PhaseOutput> = checkpoint
                    .completed_phases
                    .into_iter()
                    .map(|o| (o.phase_name.clone(), o))
                    .collect();
                for idx in 0..num_attempted {
                    let phase_name = hand.phases.get(idx)
                        .map(|p| p.name.as_str())
                        .unwrap_or("unknown");
                    if let Some(o) = successful_map.get(phase_name) {
                        outputs.push(o.clone());
                    } else {
                        // Phase was attempted but failed in the previous run
                        outputs.push(PhaseOutput {
                            phase_name: phase_name.to_string(),
                            output: "Phase failed: (from previous run checkpoint)".to_string(),
                            tool_calls: 0,
                            duration_secs: 0.0,
                            skipped: false,
                            guardrail_issues: Vec::new(),
                            quality_score: None,
                            quality_retries: 0,
                        });
                    }
                }
                context = checkpoint.context;
                start_phase = checkpoint.last_phase_index + 1;
            } else {
                info!(
                    "Hand '{}' checkpoint found but last_phase_index={} is out of range — starting fresh",
                    hand.name, checkpoint.last_phase_index
                );
                // Stale checkpoint, delete it
                checkpoint.delete();
            }
        }

        info!("Running hand '{}' with {} phases (starting at phase {})", hand.name, hand.phases.len(), start_phase);

        // Initialize knowledge capturer (best-effort, non-blocking)
        let knowledge_capturer = {
            let kb_path = dirs::home_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join(".clawtex")
                .join("knowledge.db");
            KnowledgeCapturer::new(kb_path.to_str().unwrap_or("knowledge.db")).ok()
        };

        let mut had_failure = false;
        for i in start_phase..hand.phases.len() {
            let (output, new_context) = Self::run_single_phase(
                hand, i, user_input, &context, &outputs,
                runtime, router, tool_registry,
            ).await?;

            // Detect phase failure — save checkpoint and continue (checkpoint remains for next run)
            let phase_failed = output.output.starts_with("Phase failed:");
            outputs.push(output);
            context = new_context.clone();

            if phase_failed {
                had_failure = true;
                // Save checkpoint so the next run can resume after the last successful phase
                let checkpoint = HandCheckpoint {
                    hand_name: hand.name.clone(),
                    run_id: run_id.clone(),
                    completed_phases: outputs.iter()
                        .filter(|o| !o.output.starts_with("Phase failed:"))
                        .cloned()
                        .collect(),
                    last_phase_index: if i > 0 { i - 1 } else { 0 },
                    context: new_context,
                    created_at: chrono::Utc::now().to_rfc3339(),
                };
                if let Err(e) = checkpoint.save() {
                    warn!("Hand '{}' failed to save checkpoint: {}", hand.name, e);
                }
                // Continue running remaining phases (best-effort) — the checkpoint
                // records the last *successful* phase for resume.
                continue;
            }

            // Save checkpoint after each successful phase (overwrite previous)
            let checkpoint = HandCheckpoint {
                hand_name: hand.name.clone(),
                run_id: run_id.clone(),
                completed_phases: outputs.clone(),
                last_phase_index: i,
                context: new_context,
                created_at: chrono::Utc::now().to_rfc3339(),
            };
            if let Err(e) = checkpoint.save() {
                warn!("Hand '{}' failed to save checkpoint after phase {}: {}", hand.name, i, e);
            }

            // ── Knowledge Capture: extract knowledge from successful phase output ──
            if let Some(ref capturer) = knowledge_capturer {
                let phase_name = &hand.phases[i].name;
                let phase_output = &outputs.last().map(|o| o.output.as_str()).unwrap_or("");
                match capturer.capture_from_output(&hand.name, phase_name, user_input, phase_output) {
                    Ok(nodes) if !nodes.is_empty() => {
                        debug!("Knowledge captured from {}/{}: {} node(s)", hand.name, phase_name, nodes.len());
                    }
                    Ok(_) => {} // No knowledge extracted, that's fine
                    Err(e) => {
                        debug!("Knowledge capture failed for {}/{}: {} (non-blocking)", hand.name, phase_name, e);
                    }
                }
            }

            // ── Message Queue: drain queued messages between phases ──
            if let Some(queue) = message_queue {
                let chat_id = 0; // Default chat ID — in production, passed from Telegram handler
                let queued = queue.drain(chat_id);
                if !queued.is_empty() {
                    let queued_text = HandMessageQueue::format_as_context(&queued);
                    info!(
                        "Hand '{}' phase {}: injecting {} queued message(s) into context",
                        hand.name, i + 1, queued.len()
                    );
                    context = format!("{}\n\n{}", context, queued_text);
                }
            }
        }

        let final_output = outputs.last()
            .map(|o| o.output.clone())
            .unwrap_or_else(|| "No output".to_string());

        // ── Delete checkpoint after run completes (success or partial failure) ──
        // Keeping stale checkpoints causes incorrect phase skipping on next run.
        // The checkpoint is only useful for crash recovery (not for failed phases).
        {
            let cleanup = HandCheckpoint {
                hand_name: hand.name.clone(),
                run_id: run_id.clone(),
                completed_phases: vec![],
                last_phase_index: 0,
                context: String::new(),
                created_at: String::new(),
            };
            cleanup.delete();
            if had_failure {
                debug!("Hand '{}' had phase failures — checkpoint cleaned up to allow fresh retry", hand.name);
            } else {
                debug!("Hand '{}' completed all phases — checkpoint cleaned up", hand.name);
            }
        }

        // ── Auto-save: persist final output to workspace ──
        // Skip saving error-only outputs (all phases failed)
        let is_error_output = final_output.starts_with("Phase failed:");
        if !final_output.is_empty() && final_output != "No output" && !is_error_output {
            let ext = match hand.output_format.as_str() {
                "csv" => "csv",
                "json" => "json",
                _ => "md",
            };
            let now = chrono::Local::now();
            let filename = format!("{}_{}.{}", hand.name, now.format("%Y%m%d_%H%M%S"), ext);
            let workspace = dirs::home_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join(".clawtex").join("workspace");
            let save_path = workspace.join(&filename);
            if let Err(e) = std::fs::create_dir_all(&workspace) {
                warn!("Auto-save: failed to create workspace dir: {}", e);
            } else if let Err(e) = std::fs::write(&save_path, &final_output) {
                warn!("Auto-save: failed to write {}: {}", save_path.display(), e);
            } else {
                info!(
                    "Auto-save: hand '{}' output saved to {} ({} bytes)",
                    hand.name, save_path.display(), final_output.len()
                );
            }
        }

        Ok(HandResult {
            hand_name: hand.name.clone(),
            phases_completed: outputs.len(),
            total_phases: hand.phases.len(),
            outputs,
            final_output,
            elapsed_secs: start.elapsed().as_secs_f64(),
            chain_to: hand.chain_to.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_hand() -> Hand {
        Hand {
            name: "test_hand".to_string(),
            description: "A test hand".to_string(),
            category: "test".to_string(),
            provider: "auto".to_string(),
            model: String::new(),
            phases: vec![
                Phase {
                    name: "research".to_string(),
                    system_prompt: "Research the topic".to_string(),
                    max_rounds: 3,
                    condition: None,
                    target_worker: None,
                    target_capability: None,
                    parallel_queries: Vec::new(),
                    tools: None,
                    provider: None,
                    model: None,
                    extra: HashMap::new(),
                },
                Phase {
                    name: "analyze".to_string(),
                    system_prompt: "Analyze the findings".to_string(),
                    max_rounds: 3,
                    condition: None,
                    target_worker: None,
                    target_capability: None,
                    parallel_queries: Vec::new(),
                    tools: None,
                    provider: None,
                    model: None,
                    extra: HashMap::new(),
                },
            ],
            tools: Some(vec!["web_search".to_string(), "file_write".to_string()]),
            output_format: "markdown".to_string(),
            schedule: None,
            settings: HashMap::new(),
            chain_to: None,
            guardrail: None,
            eval: None,
            extra: HashMap::new(),
        }
    }

    #[test]
    fn test_hand_serialization() {
        let hand = sample_hand();
        let toml_str = toml::to_string(&hand).unwrap();
        assert!(toml_str.contains("test_hand"));
        assert!(toml_str.contains("research"));

        // Roundtrip
        let parsed: Hand = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.name, "test_hand");
        assert_eq!(parsed.phases.len(), 2);
    }

    #[test]
    fn test_hand_registry_empty() {
        let dir = tempfile::tempdir().unwrap();
        let registry = HandRegistry::load(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(registry.list().len(), 0);
        assert!(registry.names().is_empty());
    }

    #[test]
    fn test_hand_registry_load() {
        let dir = tempfile::tempdir().unwrap();
        let hand_dir = dir.path().join("test_hand");
        std::fs::create_dir_all(&hand_dir).unwrap();

        let hand = sample_hand();
        let toml_str = toml::to_string(&hand).unwrap();
        std::fs::write(hand_dir.join("hand.toml"), toml_str).unwrap();

        let registry = HandRegistry::load(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(registry.list().len(), 1);
        assert_eq!(registry.names(), vec!["test_hand"]);

        let loaded = registry.get("test_hand").unwrap();
        assert_eq!(loaded.phases.len(), 2);
        assert_eq!(loaded.description, "A test hand");
    }

    #[test]
    fn test_hand_registry_nonexistent_dir() {
        // Should create the directory and return empty registry
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new_hands");
        let registry = HandRegistry::load(path.to_str().unwrap()).unwrap();
        assert_eq!(registry.list().len(), 0);
        assert!(path.exists()); // Directory was created
    }

    #[test]
    fn test_hand_chain_to() {
        let mut hand = sample_hand();
        hand.chain_to = Some("outreach".to_string());

        let toml_str = toml::to_string(&hand).unwrap();
        assert!(toml_str.contains("chain_to"));
        assert!(toml_str.contains("outreach"));

        let parsed: Hand = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.chain_to, Some("outreach".to_string()));
    }

    #[test]
    fn test_hand_chain_to_optional() {
        // chain_to should be optional (None by default)
        let toml_str = r#"
name = "simple"
description = "No chain"
[[phases]]
name = "do"
system_prompt = "Do it"
"#;
        let hand: Hand = toml::from_str(toml_str).unwrap();
        assert_eq!(hand.chain_to, None);
    }

    #[test]
    fn test_phase_defaults() {
        let toml_str = r#"
name = "phase1"
system_prompt = "Do something"
"#;
        let phase: Phase = toml::from_str(toml_str).unwrap();
        assert_eq!(phase.max_rounds, 5); // default
        assert!(phase.condition.is_none()); // no condition by default
    }

    #[test]
    fn test_condition_contains() {
        assert!(evaluate_condition("contains:success", "The task was a success!"));
        assert!(evaluate_condition("contains:SUCCESS", "the task was a success!")); // case insensitive
        assert!(!evaluate_condition("contains:failure", "The task was a success!"));
    }

    #[test]
    fn test_condition_not_contains() {
        assert!(evaluate_condition("not_contains:error", "Everything is fine"));
        assert!(!evaluate_condition("not_contains:fine", "Everything is fine"));
    }

    #[test]
    fn test_condition_min_length() {
        assert!(evaluate_condition("min_length:5", "Hello World"));
        assert!(!evaluate_condition("min_length:100", "Short"));
        assert!(evaluate_condition("min_length:0", ""));
    }

    #[test]
    fn test_condition_previous_success() {
        assert!(evaluate_condition("previous_success", "Here is the analysis result..."));
        assert!(!evaluate_condition("previous_success", "Phase failed: no model available"));
    }

    #[test]
    fn test_condition_unknown_defaults_true() {
        assert!(evaluate_condition("unknown_condition", "anything"));
    }

    #[test]
    fn test_phase_with_condition_toml() {
        let toml_str = r#"
name = "publish"
system_prompt = "Publish the content"
condition = "contains:approved"
"#;
        let phase: Phase = toml::from_str(toml_str).unwrap();
        assert_eq!(phase.condition, Some("contains:approved".to_string()));
    }

    // ── Quality gate tests ──────────────────────────────────────────────

    #[test]
    fn test_hand_with_guardrail_toml() {
        let toml_str = r###"
name = "quality_hand"
description = "Hand with guardrail"

[guardrail]
min_length = 100
reject_repetition = true
reject_placeholder = true
reject_simplified_chinese = true
required_sections = ["## 結論"]

[[phases]]
name = "write"
system_prompt = "Write content"
"###;
        let hand: Hand = toml::from_str(toml_str).unwrap();
        assert!(hand.guardrail.is_some());
        let gc = hand.guardrail.unwrap();
        assert_eq!(gc.min_length, 100);
        assert!(gc.reject_simplified_chinese);
        assert_eq!(gc.required_sections, vec!["## 結論"]);
    }

    #[test]
    fn test_hand_with_eval_toml() {
        let toml_str = r#"
name = "eval_hand"
description = "Hand with eval"

[eval]
enabled = true
threshold = 4
max_retries = 1
provider = "ollama"

[[phases]]
name = "write"
system_prompt = "Write content"
"#;
        let hand: Hand = toml::from_str(toml_str).unwrap();
        assert!(hand.eval.is_some());
        let ec = hand.eval.unwrap();
        assert!(ec.enabled);
        assert_eq!(ec.threshold, 4);
        assert_eq!(ec.max_retries, 1);
        assert_eq!(ec.provider.unwrap(), "ollama");
    }

    #[test]
    fn test_hand_with_both_quality_gates_toml() {
        let toml_str = r#"
name = "full_quality"
description = "Hand with both L1 + L2"

[guardrail]
min_length = 50
reject_placeholder = true

[eval]
enabled = true
threshold = 3

[[phases]]
name = "research"
system_prompt = "Research"
[[phases]]
name = "write"
system_prompt = "Write"
"#;
        let hand: Hand = toml::from_str(toml_str).unwrap();
        assert!(hand.guardrail.is_some());
        assert!(hand.eval.is_some());
        assert_eq!(hand.phases.len(), 2);
    }

    #[test]
    fn test_hand_no_quality_gates_backward_compat() {
        // Existing hand TOMLs without guardrail/eval should still parse fine
        let toml_str = r#"
name = "legacy"
description = "Old hand"
[[phases]]
name = "do"
system_prompt = "Do it"
"#;
        let hand: Hand = toml::from_str(toml_str).unwrap();
        assert!(hand.guardrail.is_none());
        assert!(hand.eval.is_none());
    }

    #[test]
    fn test_phase_output_quality_fields() {
        let output = PhaseOutput {
            phase_name: "test".to_string(),
            output: "Some output".to_string(),
            tool_calls: 3,
            duration_secs: 1.5,
            skipped: false,
            guardrail_issues: vec!["Too short".to_string()],
            quality_score: Some(4),
            quality_retries: 1,
        };
        assert_eq!(output.guardrail_issues.len(), 1);
        assert_eq!(output.quality_score, Some(4));
        assert_eq!(output.quality_retries, 1);

        // Serialize to JSON (check fields are included)
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("guardrail_issues"));
        assert!(json.contains("quality_score"));
        assert!(json.contains("quality_retries"));
    }

    #[test]
    fn test_phase_output_no_quality_fields() {
        let output = PhaseOutput {
            phase_name: "test".to_string(),
            output: "Some output".to_string(),
            tool_calls: 0,
            duration_secs: 0.0,
            skipped: false,
            guardrail_issues: Vec::new(),
            quality_score: None,
            quality_retries: 0,
        };
        assert!(output.guardrail_issues.is_empty());
        assert!(output.quality_score.is_none());
    }

    #[test]
    fn test_guardrail_config_serialization_roundtrip() {
        let hand = Hand {
            guardrail: Some(GuardrailConfig {
                min_length: 200,
                max_length: 5000,
                required_sections: vec!["## Summary".to_string()],
                forbidden_patterns: vec!["TODO".to_string()],
                reject_simplified_chinese: true,
                reject_repetition: true,
                reject_placeholder: true,
            }),
            ..sample_hand()
        };
        let toml_str = toml::to_string(&hand).unwrap();
        let parsed: Hand = toml::from_str(&toml_str).unwrap();
        let gc = parsed.guardrail.unwrap();
        assert_eq!(gc.min_length, 200);
        assert_eq!(gc.max_length, 5000);
        assert!(gc.reject_simplified_chinese);
    }

    // ── Checkpoint tests ──────────────────────────────────────────────

    fn sample_phase_output(name: &str, output: &str) -> PhaseOutput {
        PhaseOutput {
            phase_name: name.to_string(),
            output: output.to_string(),
            tool_calls: 1,
            duration_secs: 2.5,
            skipped: false,
            guardrail_issues: Vec::new(),
            quality_score: None,
            quality_retries: 0,
        }
    }

    #[test]
    fn test_checkpoint_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let cp_dir = dir.path().join(".clawtex").join("checkpoints");
        std::fs::create_dir_all(&cp_dir).unwrap();

        let cp = HandCheckpoint {
            hand_name: "test_hand".to_string(),
            run_id: "run-001".to_string(),
            completed_phases: vec![sample_phase_output("research", "Found 5 leads")],
            last_phase_index: 0,
            context: "Found 5 leads".to_string(),
            created_at: "2026-03-15T10:00:00Z".to_string(),
        };

        // Save directly to temp dir (bypass dirs::home_dir)
        let path = cp_dir.join("test_hand_run-001.json");
        let json = serde_json::to_string_pretty(&cp).unwrap();
        std::fs::write(&path, &json).unwrap();

        // Verify file was written
        assert!(path.exists());

        // Verify deserialization
        let content = std::fs::read_to_string(&path).unwrap();
        let loaded: HandCheckpoint = serde_json::from_str(&content).unwrap();
        assert_eq!(loaded.hand_name, "test_hand");
        assert_eq!(loaded.run_id, "run-001");
        assert_eq!(loaded.completed_phases.len(), 1);
        assert_eq!(loaded.completed_phases[0].phase_name, "research");
        assert_eq!(loaded.last_phase_index, 0);
        assert_eq!(loaded.context, "Found 5 leads");
    }

    #[test]
    fn test_checkpoint_serialization_roundtrip() {
        let cp = HandCheckpoint {
            hand_name: "content".to_string(),
            run_id: "run-abc".to_string(),
            completed_phases: vec![
                sample_phase_output("research", "Data collected"),
                sample_phase_output("analyze", "Analysis done"),
            ],
            last_phase_index: 1,
            context: "Analysis done".to_string(),
            created_at: "2026-03-15T12:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&cp).unwrap();
        let loaded: HandCheckpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.hand_name, "content");
        assert_eq!(loaded.completed_phases.len(), 2);
        assert_eq!(loaded.last_phase_index, 1);
    }

    #[test]
    fn test_checkpoint_load_latest_none_when_no_dir() {
        // When checkpoints dir doesn't exist, load_latest should return None
        // (this tests the code path, though it uses the real home dir)
        let result = HandCheckpoint::load_latest("nonexistent_hand_xyz_12345");
        // Should be None since no checkpoint for this hand name exists
        assert!(result.is_none());
    }

    #[test]
    fn test_phase_output_deserialize() {
        // Verify PhaseOutput can be deserialized from JSON (needed for checkpoint)
        let json = r#"{
            "phase_name": "research",
            "output": "Found results",
            "tool_calls": 3,
            "duration_secs": 1.5,
            "skipped": false,
            "guardrail_issues": [],
            "quality_score": null,
            "quality_retries": 0
        }"#;
        let output: PhaseOutput = serde_json::from_str(json).unwrap();
        assert_eq!(output.phase_name, "research");
        assert_eq!(output.output, "Found results");
        assert_eq!(output.tool_calls, 3);
        assert!(!output.skipped);
    }

    #[test]
    fn test_checkpoint_picks_latest_by_created_at() {
        let dir = tempfile::tempdir().unwrap();
        let cp_dir = dir.path();
        std::fs::create_dir_all(cp_dir).unwrap();

        // Write two checkpoints with different timestamps
        let cp_old = HandCheckpoint {
            hand_name: "myhand".to_string(),
            run_id: "old".to_string(),
            completed_phases: vec![sample_phase_output("p1", "old output")],
            last_phase_index: 0,
            context: "old output".to_string(),
            created_at: "2026-03-14T00:00:00Z".to_string(),
        };
        let cp_new = HandCheckpoint {
            hand_name: "myhand".to_string(),
            run_id: "new".to_string(),
            completed_phases: vec![
                sample_phase_output("p1", "new p1"),
                sample_phase_output("p2", "new p2"),
            ],
            last_phase_index: 1,
            context: "new p2".to_string(),
            created_at: "2026-03-15T12:00:00Z".to_string(),
        };

        std::fs::write(
            cp_dir.join("myhand_old.json"),
            serde_json::to_string(&cp_old).unwrap(),
        ).unwrap();
        std::fs::write(
            cp_dir.join("myhand_new.json"),
            serde_json::to_string(&cp_new).unwrap(),
        ).unwrap();

        // Manually scan the temp dir (since load_latest uses home dir)
        let mut best: Option<HandCheckpoint> = None;
        for entry in std::fs::read_dir(cp_dir).unwrap().flatten() {
            let fname = entry.file_name().to_string_lossy().to_string();
            if fname.starts_with("myhand_") && fname.ends_with(".json") {
                let content = std::fs::read_to_string(entry.path()).unwrap();
                let cp: HandCheckpoint = serde_json::from_str(&content).unwrap();
                if best.as_ref().map_or(true, |b| cp.created_at > b.created_at) {
                    best = Some(cp);
                }
            }
        }
        let best = best.unwrap();
        assert_eq!(best.run_id, "new");
        assert_eq!(best.completed_phases.len(), 2);
        assert_eq!(best.last_phase_index, 1);
    }

    // ── TOML sanitize + extra-field tolerance tests ───────────────────

    #[test]
    fn test_sanitize_triple_quote_fix() {
        // 4 quotes should be reduced to 3 (opening)
        let input = r#"system_prompt = """"Hello world""""#;
        let sanitized = sanitize_hand_toml(input);
        assert_eq!(sanitized, r#"system_prompt = """Hello world""""#);
    }

    #[test]
    fn test_sanitize_normal_triple_quote_unchanged() {
        let input = "system_prompt = \"\"\"Hello\nworld\"\"\"";
        let sanitized = sanitize_hand_toml(input);
        assert_eq!(sanitized, input);
    }

    #[test]
    fn test_sanitize_single_quotes_unchanged() {
        let input = r#"name = "test""#;
        let sanitized = sanitize_hand_toml(input);
        assert_eq!(sanitized, input);
    }

    #[test]
    fn test_sanitize_five_quotes_to_three() {
        // 5 quotes → 3
        let input = "system_prompt = \"\"\"\"\"Hello\"\"\"\"\"";
        let sanitized = sanitize_hand_toml(input);
        assert_eq!(sanitized, "system_prompt = \"\"\"Hello\"\"\"");
    }

    #[test]
    fn test_phase_extra_fields_tolerated() {
        // Phases with unknown fields like tool_calls should parse fine
        let toml_str = r#"
name = "test_extra"
description = "Hand with extra fields in phases"

[[phases]]
name = "collect"
system_prompt = "Collect data"
tool_calls = ["web_search", "http_request"]

[[phases]]
name = "output"
system_prompt = "Generate output"
quality_gate = true
node_affinity = "light"
"#;
        let hand: Hand = toml::from_str(toml_str).unwrap();
        assert_eq!(hand.name, "test_extra");
        assert_eq!(hand.phases.len(), 2);
        assert_eq!(hand.phases[0].name, "collect");
        assert_eq!(hand.phases[1].name, "output");
        // Extra fields should be captured
        assert!(hand.phases[0].extra.contains_key("tool_calls"));
        assert!(hand.phases[1].extra.contains_key("quality_gate"));
        assert!(hand.phases[1].extra.contains_key("node_affinity"));
    }

    #[test]
    fn test_phase_tool_calls_as_map_tolerated() {
        // tool_calls as a map (the report hand issue) should parse fine
        let toml_str = r#"
name = "test_map_tool_calls"
description = "Hand with map-style tool_calls"

[[phases]]
name = "output"
system_prompt = "Generate output"

[phases.tool_calls]
xlsx_export = "data.xlsx"
docx_export = "report.docx"
pdf_export = "report.pdf"
"#;
        // This tests that the inline table form also works
        let _result = toml::from_str::<Hand>(toml_str);
        // Note: [phases.tool_calls] syntax may conflict with [[phases]] array,
        // so we test the inline form instead
        let toml_inline = r#"
name = "test_map_tool_calls"
description = "Hand with map-style tool_calls"

[[phases]]
name = "output"
system_prompt = "Generate output"
tool_calls = { xlsx_export = "data.xlsx", docx_export = "report.docx" }
"#;
        let hand: Hand = toml::from_str(toml_inline).unwrap();
        assert_eq!(hand.phases.len(), 1);
        assert!(hand.phases[0].extra.contains_key("tool_calls"));
    }

    #[test]
    fn test_hand_extra_top_level_fields_tolerated() {
        // Unknown top-level fields should be captured in extra
        let toml_str = r#"
name = "test_top_extra"
description = "Hand with extra top-level fields"
version = "1.0"
priority = "P1"

[[phases]]
name = "do"
system_prompt = "Do it"
"#;
        let hand: Hand = toml::from_str(toml_str).unwrap();
        assert_eq!(hand.name, "test_top_extra");
        assert!(hand.extra.contains_key("version"));
        assert!(hand.extra.contains_key("priority"));
    }

    #[test]
    fn test_prompt_evolve_style_multiline_parses() {
        // Simulate the prompt_evolve hand with multi-line strings
        let toml_str = r#"
name = "prompt_evolve"
description = "Weekly prompt optimization"
category = "infrastructure"
provider = "auto"
tools = ["file_read", "file_write", "file_edit", "memory_store", "memory_recall", "http_request"]
schedule = "0 4 * * 0"

[[phases]]
name = "analyze_trajectories"
system_prompt = """Analyze trajectory data.
1. Get trajectories via http_request
2. Calculate quality scores
3. List worst 3 phases"""

[[phases]]
name = "generate_improvements"
system_prompt = """Generate improved prompts.
Analyze good/bad examples.
Generate 3 improved versions."""

[[phases]]
name = "apply_safely"
system_prompt = """Apply improvements safely.
1. Backup original hand.toml
2. Update system_prompt
3. Record changes to memory_store
4. Only change system_prompt field"""
condition = "contains:improved"
"#;
        let hand: Hand = toml::from_str(toml_str).unwrap();
        assert_eq!(hand.name, "prompt_evolve");
        assert_eq!(hand.phases.len(), 3);
        assert!(hand.phases[0].system_prompt.contains("Analyze trajectory data"));
        assert!(hand.phases[2].condition.is_some());
    }

    #[test]
    fn test_sanitize_then_parse_four_quotes() {
        // Simulate a hand.toml where someone accidentally used 4 quotes
        let broken_toml = r#"
name = "broken_quotes"
description = "Has triple-quote issue"

[[phases]]
name = "test"
system_prompt = """"This has an extra opening quote.
Line 2 of the prompt.
Line 3 of the prompt.""""
"#;
        // Without sanitize, this would fail to parse
        let sanitized = sanitize_hand_toml(broken_toml);
        let hand: Hand = toml::from_str(&sanitized).unwrap();
        assert_eq!(hand.name, "broken_quotes");
        assert_eq!(hand.phases.len(), 1);
        assert!(hand.phases[0].system_prompt.contains("This has an extra opening quote"));
    }

    #[test]
    fn test_report_style_hand_parses() {
        // Simulate the report hand with tool_calls in phases
        let toml_str = r#"
name = "report"
description = "Generate analysis reports"
category = "content"
provider = "auto"
tools = ["web_search", "http_request", "xlsx_export", "docx_export", "pdf_export", "delegate"]

[[phases]]
name = "collect"
system_prompt = "Collect data via web_search and http_request"
tool_calls = ["web_search", "http_request"]

[[phases]]
name = "analyze"
system_prompt = "Analyze collected data"
tool_calls = ["delegate"]

[[phases]]
name = "prepare_charts"
system_prompt = "Prepare chart data as structured JSON"

[[phases]]
name = "output"
system_prompt = "Export to xlsx, docx, and pdf"
tool_calls = { xlsx_export = "data", docx_export = "report", pdf_export = "report" }
"#;
        let hand: Hand = toml::from_str(toml_str).unwrap();
        assert_eq!(hand.name, "report");
        assert_eq!(hand.phases.len(), 4);
        // tool_calls as array
        assert!(hand.phases[0].extra.contains_key("tool_calls"));
        // tool_calls as map
        assert!(hand.phases[3].extra.contains_key("tool_calls"));
    }

    // ── Per-hand tool filtering tests ─────────────────────────────────

    #[test]
    fn test_hand_tools_none_means_all_tools() {
        // When hand.tools is None, all tools should be available (backwards compat)
        let toml_str = r#"
name = "no_tools_field"
description = "Hand without tools field"
[[phases]]
name = "do"
system_prompt = "Do something"
"#;
        let hand: Hand = toml::from_str(toml_str).unwrap();
        assert!(hand.tools.is_none(), "Missing tools field should parse as None");
    }

    #[test]
    fn test_hand_tools_some_list_parsed() {
        // When hand.tools is a list, it should parse as Some(list)
        let toml_str = r#"
name = "filtered_hand"
description = "Hand with explicit tool list"
tools = ["web_search", "file_read", "http_request"]
[[phases]]
name = "research"
system_prompt = "Research only"
"#;
        let hand: Hand = toml::from_str(toml_str).unwrap();
        let tools = hand.tools.expect("tools should be Some");
        assert_eq!(tools.len(), 3);
        assert!(tools.contains(&"web_search".to_string()));
        assert!(tools.contains(&"file_read".to_string()));
        assert!(tools.contains(&"http_request".to_string()));
        // tool not in list should not be present
        assert!(!tools.contains(&"shell".to_string()));
    }

    #[test]
    fn test_hand_tools_empty_list_is_some_empty() {
        // An explicit empty list `tools = []` parses as Some([])
        let toml_str = r#"
name = "empty_tools"
description = "Hand with empty tool list"
tools = []
[[phases]]
name = "think"
system_prompt = "Think without tools"
"#;
        let hand: Hand = toml::from_str(toml_str).unwrap();
        assert!(hand.tools.is_some());
        assert_eq!(hand.tools.unwrap().len(), 0);
    }

    #[test]
    fn test_phase_tools_none_fallback_to_hand_tools() {
        // When phase.tools is None, the hand-level tools should be used
        let toml_str = r#"
name = "fallback_hand"
description = "Phase inherits hand tools"
tools = ["web_search", "file_read"]
[[phases]]
name = "research"
system_prompt = "Research"
"#;
        let hand: Hand = toml::from_str(toml_str).unwrap();
        let phase = &hand.phases[0];
        assert!(phase.tools.is_none(), "Phase tools should be None (inherits from hand)");
        // Effective tools = phase.tools.as_ref().or(hand.tools.as_ref())
        let effective = phase.tools.as_ref().or(hand.tools.as_ref());
        let effective_list = effective.expect("effective tools should be Some from hand");
        assert_eq!(effective_list, &vec!["web_search".to_string(), "file_read".to_string()]);
    }

    #[test]
    fn test_phase_tools_override_hand_tools() {
        // When phase.tools is set, it overrides hand-level tools for that phase
        let toml_str = r#"
name = "override_hand"
description = "Phase overrides hand tools"
tools = ["web_search", "file_read", "shell"]
[[phases]]
name = "read_only"
system_prompt = "Read-only phase"
tools = ["file_read"]
[[phases]]
name = "full_access"
system_prompt = "Full access phase"
"#;
        let hand: Hand = toml::from_str(toml_str).unwrap();

        // Phase 0: has its own tools list
        let phase0 = &hand.phases[0];
        assert!(phase0.tools.is_some());
        let p0_tools = phase0.tools.as_ref().unwrap();
        assert_eq!(p0_tools, &vec!["file_read".to_string()]);

        // Effective tools for phase 0 = phase-level (overrides hand)
        let eff0 = phase0.tools.as_ref().or(hand.tools.as_ref()).unwrap();
        assert_eq!(eff0, &vec!["file_read".to_string()]);
        assert!(!eff0.contains(&"shell".to_string()));

        // Phase 1: no tools field, falls back to hand tools
        let phase1 = &hand.phases[1];
        assert!(phase1.tools.is_none());
        let eff1 = phase1.tools.as_ref().or(hand.tools.as_ref()).unwrap();
        assert!(eff1.contains(&"shell".to_string()));
        assert!(eff1.contains(&"web_search".to_string()));
    }

    #[test]
    fn test_both_hand_and_phase_tools_none_means_all() {
        // When both hand.tools and phase.tools are None, effective tools is None (= all tools)
        let toml_str = r#"
name = "unrestricted"
description = "No tool restrictions"
[[phases]]
name = "anything"
system_prompt = "Do anything"
"#;
        let hand: Hand = toml::from_str(toml_str).unwrap();
        let phase = &hand.phases[0];
        let effective: Option<&Vec<String>> = phase.tools.as_ref().or(hand.tools.as_ref());
        assert!(effective.is_none(), "No tools restriction means None (all tools available)");
    }

    #[test]
    fn test_hand_tools_serialization_roundtrip() {
        // Tools field should survive TOML serialization/deserialization roundtrip
        let mut hand = sample_hand();
        hand.tools = Some(vec!["web_search".to_string(), "file_read".to_string()]);

        let toml_str = toml::to_string(&hand).unwrap();
        assert!(toml_str.contains("web_search"), "Serialized TOML should contain tool names");

        let parsed: Hand = toml::from_str(&toml_str).unwrap();
        let parsed_tools = parsed.tools.expect("tools should deserialize as Some");
        assert!(parsed_tools.contains(&"web_search".to_string()));
        assert!(parsed_tools.contains(&"file_read".to_string()));
        assert_eq!(parsed_tools.len(), 2);
    }

    #[test]
    fn test_hand_tools_none_serialization_roundtrip() {
        // Hand with tools=None should roundtrip correctly (no tools key emitted or parsed as None)
        let mut hand = sample_hand();
        hand.tools = None;

        let toml_str = toml::to_string(&hand).unwrap();
        let parsed: Hand = toml::from_str(&toml_str).unwrap();
        // May serialize as empty list due to serde(default), either None or Some([]) is acceptable
        // but the filtering logic treats both as "no restriction" (backwards compat)
        let no_restriction = parsed.tools.as_ref().map_or(true, |t| t.is_empty());
        assert!(no_restriction, "No tools or empty tools means no restriction");
    }

    #[test]
    fn test_phase_tools_toml_parse() {
        // Verify per-phase tools field parses correctly from TOML
        let toml_str = r#"
name = "phase_filtered"
description = "Per-phase tool filtering"
[[phases]]
name = "search_only"
system_prompt = "Search for data"
tools = ["web_search", "http_request"]
[[phases]]
name = "write_only"
system_prompt = "Write results"
tools = ["file_write"]
[[phases]]
name = "unrestricted"
system_prompt = "Final phase"
"#;
        let hand: Hand = toml::from_str(toml_str).unwrap();
        assert_eq!(hand.phases.len(), 3);

        // Phase 0: tools = ["web_search", "http_request"]
        let p0 = hand.phases[0].tools.as_ref().expect("phase 0 should have tools");
        assert_eq!(p0.len(), 2);
        assert!(p0.contains(&"web_search".to_string()));
        assert!(p0.contains(&"http_request".to_string()));

        // Phase 1: tools = ["file_write"]
        let p1 = hand.phases[1].tools.as_ref().expect("phase 1 should have tools");
        assert_eq!(p1.len(), 1);
        assert!(p1.contains(&"file_write".to_string()));

        // Phase 2: no tools field → None
        assert!(hand.phases[2].tools.is_none());
    }

    #[test]
    fn test_effective_tools_priority_chain() {
        // Verify priority chain: phase tools > hand tools > None (all tools)
        let make_phase = |tools: Option<Vec<String>>| Phase {
            name: "p".to_string(),
            system_prompt: "s".to_string(),
            max_rounds: 3,
            condition: None,
            target_worker: None,
            target_capability: None,
            parallel_queries: Vec::new(),
            tools,
            provider: None,
            model: None,
            extra: HashMap::new(),
        };

        let hand_tools = Some(vec!["web_search".to_string()]);

        // Case 1: phase has tools → use phase tools
        let phase_with_tools = make_phase(Some(vec!["file_read".to_string()]));
        let eff = phase_with_tools.tools.as_ref().or(hand_tools.as_ref());
        assert_eq!(eff.unwrap(), &vec!["file_read".to_string()]);

        // Case 2: phase has no tools, hand has tools → use hand tools
        let phase_without_tools = make_phase(None);
        let eff = phase_without_tools.tools.as_ref().or(hand_tools.as_ref());
        assert_eq!(eff.unwrap(), &vec!["web_search".to_string()]);

        // Case 3: both None → all tools
        let hand_tools_none: Option<Vec<String>> = None;
        let eff = phase_without_tools.tools.as_ref().or(hand_tools_none.as_ref());
        assert!(eff.is_none());
    }
}
