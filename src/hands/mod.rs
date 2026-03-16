//! Hands workflow engine — structured multi-step agent workflows.
//! Each Hand is a specialized agent configuration loaded from TOML files.
//! Based on OpenFang's HAND.toml pattern.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tracing::{debug, info, warn};

use crate::agent_runtime::AgentRuntime;
use crate::approval::{ApprovalGate, ApprovalResult};
use crate::evaluate::{self, EvalConfig, EvalResult};
use crate::guardrail::{self, GuardrailConfig, GuardrailResult};
use crate::llm_router::LlmRouter;
use crate::tools::ToolRegistry;

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
    /// Which tools this hand can use
    #[serde(default)]
    pub tools: Vec<String>,
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
                            match toml::from_str::<Hand>(&content) {
                                Ok(hand) => {
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

        // Use the phase's max_rounds setting to limit the agent's tool-call loop
        let phase_max_rounds = Some(phase.max_rounds as usize);

        let mut agent_config = runtime.get_config("master")
            .ok_or_else(|| anyhow::anyhow!("Agent 'master' not found"))?
            .clone();

        // Override agent config with hand-level tools and provider
        if !hand.tools.is_empty() {
            agent_config.tools = Some(hand.tools.clone());
        }
        if hand.provider != "auto" && !hand.provider.is_empty() {
            agent_config.provider = Some(hand.provider.clone());
        }
        if !hand.model.is_empty() {
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

        // Check required tools are registered
        let available_tools = tool_registry.names();
        for tool_name in &hand.tools {
            if !available_tools.contains(tool_name) {
                issues.push(format!("Tool '{}' not found in registry", tool_name));
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
    pub async fn run(
        hand: &Hand,
        user_input: &str,
        runtime: &AgentRuntime,
        router: &LlmRouter,
        tool_registry: &ToolRegistry,
        approval_gate: Option<&Arc<ApprovalGate>>,
    ) -> Result<HandResult> {
        let start = std::time::Instant::now();

        // Check require_approval setting
        if hand.settings.get("require_approval").map(|v| v == "true").unwrap_or(false) {
            if let Some(gate) = approval_gate {
                let description = format!(
                    "Hand '{}' requires approval to proceed.\nPhases: {}\nTools: {:?}",
                    hand.name, hand.phases.len(), hand.tools
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
            if checkpoint.last_phase_index < hand.phases.len() {
                info!(
                    "Hand '{}' resuming from checkpoint (run_id={}, completed {}/{} phases)",
                    hand.name, checkpoint.run_id, checkpoint.completed_phases.len(), hand.phases.len()
                );
                outputs = checkpoint.completed_phases;
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
        }

        let final_output = outputs.last()
            .map(|o| o.output.clone())
            .unwrap_or_else(|| "No output".to_string());

        // ── Delete checkpoint on full success (no failures) ──
        if !had_failure {
            let cleanup = HandCheckpoint {
                hand_name: hand.name.clone(),
                run_id: run_id.clone(),
                completed_phases: vec![],
                last_phase_index: 0,
                context: String::new(),
                created_at: String::new(),
            };
            cleanup.delete();
            debug!("Hand '{}' completed all phases — checkpoint cleaned up", hand.name);
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
                },
                Phase {
                    name: "analyze".to_string(),
                    system_prompt: "Analyze the findings".to_string(),
                    max_rounds: 3,
                    condition: None,
                    target_worker: None,
                    target_capability: None,
                    parallel_queries: Vec::new(),
                },
            ],
            tools: vec!["web_search".to_string(), "file_write".to_string()],
            output_format: "markdown".to_string(),
            schedule: None,
            settings: HashMap::new(),
            chain_to: None,
            guardrail: None,
            eval: None,
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
}
