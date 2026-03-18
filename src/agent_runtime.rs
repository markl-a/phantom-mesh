use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::pin::Pin;
use std::time::Instant;
use tracing::{debug, info, warn};

use crate::providers::{ChatMessage, StreamChunk};
use crate::cluster_hub::ClusterHub;
use crate::context::ContextOptimizer;
use crate::cost_tracker::{CostTracker, CostRecord, BudgetBreaker, estimate_cost};
use crate::injection_guard::InjectionGuard;
use crate::service_tier::ServiceTierManager;
use crate::dispatcher::{self, DispatchMode};
use crate::llm_router::LlmRouter;
use crate::loop_detection::{AdvancedLoopDetector, LoopDetectorConfig, LoopAction, LoopKind};
use crate::memory::MemoryStore;
use crate::response_cache::ResponseCache;
use crate::agent_events::{AgentEventBus, AgentEvent};
use crate::security::{AutonomyLevel, PrivacyGuard};
use crate::tools::{ToolRegistry, ToolSpec};
use crate::capability_broadcast::build_capability_prompt;
use crate::trajectory::{TrajectoryLogger, TrajectoryEntry};
use std::sync::Arc;

const MAX_TOOL_ROUNDS: usize = 10;
const AGENT_TIMEOUT_SECS: u64 = 600;
/// How often (in seconds) to send progress reports during long-running agent tasks
const PROGRESS_INTERVAL_SECS: u64 = 60;

/// Agent configuration from agents.toml
#[derive(Debug, Clone, Deserialize)]
pub struct AgentConfig {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub tools: Option<Vec<String>>,
    pub instructions: Option<String>,
    pub subagents: Option<Vec<String>>,
    /// Daily budget limit in USD. 0 or absent = no limit.
    #[serde(default)]
    pub daily_budget_usd: f64,
    /// Autonomy level: readonly, supervised, full (default: full)
    #[serde(default)]
    pub autonomy: AutonomyLevel,
}

/// Parsed agents section from agents.toml
#[derive(Debug, Deserialize)]
struct AgentsToml {
    #[serde(default)]
    agent: HashMap<String, AgentConfig>,
}

/// Result of a single agent run
#[derive(Debug, Serialize)]
pub struct AgentResult {
    pub agent_name: String,
    pub output: String,
    pub tool_calls_made: usize,
    pub elapsed_secs: f64,
    pub total_tokens: u32,
}

/// Built-in agent specs
fn default_agents() -> HashMap<String, AgentConfig> {
    let mut map = HashMap::new();
    map.insert(
        "master".to_string(),
        AgentConfig {
            provider: Some("ollama".to_string()),
            model: Some("qwen3:8b".to_string()),
            tools: Some(vec!["shell".to_string(), "file_read".to_string(), "file_write".to_string()]),
            instructions: Some(
                "You are Clawtex, a local AI assistant running on the user's machine. \
                 You have access to shell commands, file reading, and file writing tools. \
                 Use tools when the user asks you to perform actions on the system. \
                 Be concise and helpful. Always respond in the same language as the user."
                    .to_string(),
            ),
            subagents: None,
            daily_budget_usd: 0.0,
            autonomy: AutonomyLevel::Full,
        },
    );
    map.insert(
        "coder".to_string(),
        AgentConfig {
            provider: Some("ollama".to_string()),
            model: Some("qwen3:8b".to_string()),
            tools: Some(vec!["shell".to_string(), "file_read".to_string(), "file_write".to_string()]),
            instructions: Some(
                "You are an expert programmer. Write clean, efficient code. \
                 Use file_write to create files and shell to run commands."
                    .to_string(),
            ),
            subagents: None,
            daily_budget_usd: 0.0,
            autonomy: AutonomyLevel::Full,
        },
    );
    map
}

/// Agent Runtime — executes named agents with tool-call loop
pub struct AgentRuntime {
    agents: HashMap<String, AgentConfig>,
    cost_tracker: Option<Arc<CostTracker>>,
    memory_store: Option<Arc<MemoryStore>>,
    cluster_hub: Option<Arc<ClusterHub>>,
    privacy_guard: Option<PrivacyGuard>,
    response_cache: Option<Arc<ResponseCache>>,
    event_bus: Option<Arc<AgentEventBus>>,
    trajectory_logger: Option<Arc<TrajectoryLogger>>,
    budget_breaker: Option<Arc<BudgetBreaker>>,
    injection_guard: Option<Arc<InjectionGuard>>,
    service_tier: Option<Arc<ServiceTierManager>>,
}

impl AgentRuntime {
    pub fn new(config_path: &str) -> Result<Self> {
        let agents = if std::path::Path::new(config_path).exists() {
            let content = std::fs::read_to_string(config_path)?;
            let parsed: AgentsToml = toml::from_str(&content)?;
            if parsed.agent.is_empty() {
                default_agents()
            } else {
                parsed.agent
            }
        } else {
            default_agents()
        };

        Ok(Self { agents, cost_tracker: None, memory_store: None, cluster_hub: None, privacy_guard: None, response_cache: None, event_bus: None, trajectory_logger: None, budget_breaker: None, injection_guard: None, service_tier: None })
    }

    /// Attach a cost tracker to automatically record costs for every agent run
    pub fn set_cost_tracker(&mut self, tracker: Arc<CostTracker>) {
        self.cost_tracker = Some(tracker);
    }

    /// Attach a memory store for automatic context injection before each agent run
    pub fn set_memory_store(&mut self, memory: Arc<MemoryStore>) {
        self.memory_store = Some(memory);
    }

    /// Attach a cluster hub for distributed tool dispatch
    pub fn set_cluster_hub(&mut self, hub: Arc<ClusterHub>) {
        self.cluster_hub = Some(hub);
    }

    /// Get a reference to the cluster hub (if attached)
    pub fn cluster_hub(&self) -> Option<&Arc<ClusterHub>> {
        self.cluster_hub.as_ref()
    }

    /// Attach a privacy guard for sensitivity-based provider routing
    pub fn set_privacy_guard(&mut self, guard: PrivacyGuard) {
        self.privacy_guard = Some(guard);
    }

    /// Attach a response cache for deduplicating LLM calls
    pub fn set_response_cache(&mut self, cache: Arc<ResponseCache>) {
        self.response_cache = Some(cache);
    }

    /// Attach an event bus for real-time observability
    pub fn set_event_bus(&mut self, bus: Arc<AgentEventBus>) {
        self.event_bus = Some(bus);
    }

    /// Get event bus reference (for external subscribers)
    pub fn event_bus(&self) -> Option<&Arc<AgentEventBus>> {
        self.event_bus.as_ref()
    }

    /// Attach a trajectory logger for recording every agent run
    pub fn set_trajectory_logger(&mut self, logger: Arc<TrajectoryLogger>) {
        self.trajectory_logger = Some(logger);
    }

    /// Get trajectory logger reference
    pub fn trajectory_logger(&self) -> Option<&Arc<TrajectoryLogger>> {
        self.trajectory_logger.as_ref()
    }

    /// Attach a budget breaker for fast-path budget checking
    pub fn set_budget_breaker(&mut self, breaker: Arc<BudgetBreaker>) {
        self.budget_breaker = Some(breaker);
    }

    /// Get budget breaker reference
    pub fn budget_breaker(&self) -> Option<&Arc<BudgetBreaker>> {
        self.budget_breaker.as_ref()
    }

    /// Attach an injection guard for prompt safety checking
    pub fn set_injection_guard(&mut self, guard: Arc<InjectionGuard>) {
        self.injection_guard = Some(guard);
    }

    /// Attach a service tier manager for tool/rate access enforcement
    pub fn set_service_tier(&mut self, tier_mgr: Arc<ServiceTierManager>) {
        self.service_tier = Some(tier_mgr);
    }

    /// Get service tier manager reference
    pub fn service_tier(&self) -> Option<&Arc<ServiceTierManager>> {
        self.service_tier.as_ref()
    }

    /// Run a named agent with tool-call loop
    /// `history` contains previous conversation turns (user+assistant pairs)
    /// `extra_context` is optional text injected into the system prompt (memories, skills, etc.)
    pub async fn run(
        &self,
        agent_name: &str,
        prompt: &str,
        history: &[ChatMessage],
        router: &LlmRouter,
        tool_registry: &ToolRegistry,
        extra_context: Option<&str>,
    ) -> Result<AgentResult> {
        let config = self
            .agents
            .get(agent_name)
            .ok_or_else(|| anyhow!("Unknown agent: {}", agent_name))?;

        self.run_with_config(agent_name, config, prompt, history, router, tool_registry, extra_context, None, None).await
    }

    /// Run a named agent with progress reporting via channel.
    /// Progress messages are sent every 60s during long-running tasks.
    pub async fn run_with_progress(
        &self,
        agent_name: &str,
        prompt: &str,
        history: &[ChatMessage],
        router: &LlmRouter,
        tool_registry: &ToolRegistry,
        extra_context: Option<&str>,
        progress_tx: tokio::sync::mpsc::Sender<String>,
    ) -> Result<AgentResult> {
        let config = self
            .agents
            .get(agent_name)
            .ok_or_else(|| anyhow!("Unknown agent: {}", agent_name))?;

        self.run_with_config(agent_name, config, prompt, history, router, tool_registry, extra_context, None, Some(progress_tx)).await
    }

    /// Run with an explicit AgentConfig — enables ephemeral/dynamic agent configurations.
    /// Used by delegate_to_provider to construct configs at runtime without pre-registration.
    /// `max_tool_rounds` overrides the default MAX_TOOL_ROUNDS if provided.
    /// `progress_tx` sends periodic status updates (every 60s) for long-running tasks.
    pub async fn run_with_config(
        &self,
        agent_name: &str,
        config: &AgentConfig,
        prompt: &str,
        history: &[ChatMessage],
        router: &LlmRouter,
        tool_registry: &ToolRegistry,
        extra_context: Option<&str>,
        max_tool_rounds: Option<usize>,
        progress_tx: Option<tokio::sync::mpsc::Sender<String>>,
    ) -> Result<AgentResult> {
        self.run_with_config_targeted(
            agent_name, config, prompt, history, router, tool_registry,
            extra_context, max_tool_rounds, progress_tx, None,
        ).await
    }

    /// Same as run_with_config but with optional targeted dispatch.
    /// When `target_worker` is Some, all dispatchable tools go to that specific worker.
    pub async fn run_with_config_targeted(
        &self,
        agent_name: &str,
        config: &AgentConfig,
        prompt: &str,
        history: &[ChatMessage],
        router: &LlmRouter,
        tool_registry: &ToolRegistry,
        extra_context: Option<&str>,
        max_tool_rounds: Option<usize>,
        progress_tx: Option<tokio::sync::mpsc::Sender<String>>,
        target_worker: Option<String>,
    ) -> Result<AgentResult> {
        info!("Agent '{}' starting: {}...", agent_name, truncate_str(prompt, 60));
        let t0 = Instant::now();

        // ── Injection Guard: check user prompt for injection patterns ──
        if let Some(ref guard) = self.injection_guard {
            let result = guard.check(prompt);
            if let crate::injection_guard::InjectionResult::Suspicious { ref patterns, ref severity } = result {
                match severity {
                    crate::injection_guard::Severity::High => {
                        warn!("InjectionGuard BLOCKED agent '{}': {:?}", agent_name, patterns);
                        return Ok(AgentResult {
                            agent_name: agent_name.to_string(),
                            output: format!("Request blocked by safety filter (detected: {})", patterns.join(", ")),
                            tool_calls_made: 0,
                            elapsed_secs: t0.elapsed().as_secs_f64(),
                            total_tokens: 0,
                        });
                    }
                    crate::injection_guard::Severity::Medium => {
                        warn!("InjectionGuard WARNING for agent '{}': {:?} (allowing with sanitization)", agent_name, patterns);
                    }
                    crate::injection_guard::Severity::Low => {
                        debug!("InjectionGuard LOW for agent '{}': {:?}", agent_name, patterns);
                    }
                }
            }
        }

        // ── Service Tier: check daily rate limit before processing ──
        if let Some(ref tier_mgr) = self.service_tier {
            if let Err(denied) = tier_mgr.check_rate_limit(agent_name) {
                warn!("ServiceTier rate limit denied for agent '{}': {}", agent_name, denied);
                return Ok(AgentResult {
                    agent_name: agent_name.to_string(),
                    output: format!("Service tier rate limit: {}", denied.reason),
                    tool_calls_made: 0,
                    elapsed_secs: t0.elapsed().as_secs_f64(),
                    total_tokens: 0,
                });
            }
            // Record this task against the daily quota
            if let Err(e) = tier_mgr.record_task(agent_name) {
                warn!("Failed to record tier task for '{}': {}", agent_name, e);
            }
        }

        let instructions = config
            .instructions
            .as_deref()
            .unwrap_or("You are a helpful assistant.");

        let provider = config.provider.as_deref().unwrap_or("auto");
        let model = config.model.as_deref().unwrap_or("qwen3:8b");

        // Build tool specs for tools this agent is allowed to use
        let agent_tools = config.tools.as_deref().unwrap_or(&[]);
        let tool_specs = self.build_tool_specs(agent_tools, tool_registry);

        // Determine dispatch mode based on provider
        let dispatch_mode = dispatcher::dispatch_mode_for_provider(provider);

        // Build Ollama-format tool definitions (only sent in Native/Auto mode)
        let tool_defs: Vec<Value> = tool_specs
            .iter()
            .map(|spec| {
                json!({
                    "type": "function",
                    "function": {
                        "name": spec.name,
                        "description": spec.description,
                        "parameters": spec.parameters,
                    }
                })
            })
            .collect();

        // For XML/Auto mode, generate text-based tool instructions
        let xml_instructions = if dispatch_mode != DispatchMode::Native {
            dispatcher::xml_tool_instructions(&tool_specs)
        } else {
            String::new()
        };

        // Inject current timestamp + extra context into system prompt
        let now = chrono::Local::now();
        let extra = extra_context.unwrap_or("");

        // Auto-inject relevant memories (inspired by ZeroClaw's MemoryLoader)
        let memory_context = if let Some(ref memory) = self.memory_store {
            match memory.recall(prompt, 5, None).await {
                Ok(entries) if !entries.is_empty() => {
                    let mem_lines: Vec<String> = entries.iter().map(|e| {
                        format!("- [{}] {}", e.key, e.content)
                    }).collect();
                    format!("\n\n[Relevant memories]\n{}", mem_lines.join("\n"))
                }
                Ok(_) => String::new(),
                Err(e) => {
                    debug!("Memory recall failed (continuing without): {}", e);
                    String::new()
                }
            }
        } else {
            String::new()
        };

        // Build runtime info so the agent knows its own provider/model
        let all_providers = router.provider_names();
        let runtime_info = format!(
            "\nYou are running on provider='{}', model='{}'. Available providers: [{}].",
            provider, model, all_providers.join(", ")
        );

        // Build capability broadcast: compact list of available tools and hands.
        // Injected at session start so the LLM knows what it can do.
        let capability_section = {
            let hands_list: Vec<String> = Vec::new();
            let cap = build_capability_prompt(&tool_specs, &hands_list);
            format!("\n\n[Session capabilities]\n{}", cap)
        };

        let system_prompt = format!(
            "{}{}\n\nCurrent time: {} ({}){}\n\
             You have conversation memory — you CAN and SHOULD recall what the user said in previous messages.\n\
             When searching the web, base your search query on the user's LATEST message, not on old topics.{}{}{}",
            instructions,
            capability_section,
            now.format("%Y-%m-%d %H:%M:%S"),
            now.format("%A"),
            runtime_info,
            extra,
            memory_context,
            xml_instructions,
        );

        // Initialize conversation: system + history + current user message
        let mut messages = Vec::with_capacity(2 + history.len());
        messages.push(ChatMessage {
            role: "system".to_string(),
            content: system_prompt,
            tool_calls: None,
            tool_call_id: None,
        });
        // Add conversation history (previous user+assistant turns)
        messages.extend_from_slice(history);
        // Add current user message
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: prompt.to_string(),
            tool_calls: None,
            tool_call_id: None,
        });

        // Privacy-aware provider routing: when provider is "privacy",
        // classify messages by sensitivity and resolve to the appropriate provider.
        let effective_provider = if provider == "privacy" {
            if let Some(ref guard) = self.privacy_guard {
                let (resolved, tier) = guard.classify_and_route(&messages);
                info!("PrivacyGuard: {} -> provider '{}'", tier, resolved);
                resolved
            } else {
                "auto".to_string()
            }
        } else {
            provider.to_string()
        };

        // Context window management: trim if over budget
        ContextOptimizer::trim_messages(&mut messages, model);

        let mut total_tool_calls = 0;
        let mut total_tokens: u32 = 0;
        let mut loop_detector = AdvancedLoopDetector::new(LoopDetectorConfig::default());

        let effective_max_rounds = max_tool_rounds.unwrap_or(MAX_TOOL_ROUNDS);

        // Emit RunStarted event
        if let Some(ref bus) = self.event_bus {
            bus.emit(AgentEvent::RunStarted {
                agent_name: agent_name.to_string(),
                provider: effective_provider.clone(),
                model: model.to_string(),
                max_rounds: effective_max_rounds,
            });
        }

        let deadline = Instant::now() + std::time::Duration::from_secs(AGENT_TIMEOUT_SECS);
        let mut last_progress_report = Instant::now();
        let mut last_tool_names: Vec<String> = Vec::new();
        for round in 0..effective_max_rounds {
            // ── Budget Breaker fast path: skip DB query if breaker already tripped ──
            if let Some(ref breaker) = self.budget_breaker {
                if breaker.is_tripped(agent_name) {
                    let elapsed = t0.elapsed().as_secs_f64();
                    warn!("Agent '{}' budget breaker is tripped — skipping (fast path)", agent_name);
                    return Ok(AgentResult {
                        agent_name: agent_name.to_string(),
                        output: "Budget breaker tripped — agent temporarily suspended. Wait for cooldown or manual reset.".to_string(),
                        tool_calls_made: total_tool_calls,
                        elapsed_secs: elapsed,
                        total_tokens,
                    });
                }
            }

            // Check budget before each LLM call (DB query)
            if let Some(ref ct) = self.cost_tracker {
                if config.daily_budget_usd > 0.0 {
                    if let Err(e) = ct.check_budget(agent_name, config.daily_budget_usd) {
                        // Trip the budget breaker so future calls skip the DB
                        if let Some(ref breaker) = self.budget_breaker {
                            breaker.trip(agent_name);
                        }
                        let elapsed = t0.elapsed().as_secs_f64();
                        warn!("Agent '{}' budget exceeded: {}", agent_name, e);
                        // Log trajectory (budget exceeded)
                        if let Some(ref logger) = self.trajectory_logger {
                            let tokens_in = (total_tokens as f64 * 0.6) as u32;
                            let tokens_out = total_tokens - tokens_in;
                            let entry = TrajectoryEntry {
                                id: uuid::Uuid::new_v4().to_string(),
                                session_id: None,
                                agent_name: agent_name.to_string(),
                                hand_name: None,
                                phase_name: None,
                                provider: effective_provider.clone(),
                                model: model.to_string(),
                                prompt: prompt.to_string(),
                                output: format!("Budget exceeded: {}", e),
                                tool_calls: total_tool_calls,
                                tool_names: last_tool_names.clone(),
                                total_tokens,
                                duration_secs: elapsed,
                                estimated_cost_usd: estimate_cost(&effective_provider, model, tokens_in, tokens_out),
                                quality_score: None,
                                guardrail_issues: vec![],
                                success: false,
                                error_message: Some(format!("Budget exceeded: {}", e)),
                                worker_name: None,
                                worker_latency_ms: None,
                                created_at: chrono::Utc::now().to_rfc3339(),
                                date_key: chrono::Utc::now().format("%Y-%m-%d").to_string(),
                            };
                            if let Err(te) = logger.log_run(&entry) {
                                warn!("Failed to log trajectory: {}", te);
                            }
                        }
                        return Ok(AgentResult {
                            agent_name: agent_name.to_string(),
                            output: format!("Budget exceeded: {}", e),
                            tool_calls_made: total_tool_calls,
                            elapsed_secs: elapsed,
                            total_tokens,
                        });
                    }
                }
            }

            // Check overall agent timeout
            if Instant::now() > deadline {
                let elapsed = t0.elapsed().as_secs_f64();
                warn!("Agent '{}' timed out after {:.0}s", agent_name, elapsed);
                self.record_run_cost(agent_name, &effective_provider, model, total_tokens, elapsed, Some("timeout"));
                // Log trajectory (timeout)
                if let Some(ref logger) = self.trajectory_logger {
                    let tokens_in = (total_tokens as f64 * 0.6) as u32;
                    let tokens_out = total_tokens - tokens_in;
                    let entry = TrajectoryEntry {
                        id: uuid::Uuid::new_v4().to_string(),
                        session_id: None,
                        agent_name: agent_name.to_string(),
                        hand_name: None,
                        phase_name: None,
                        provider: effective_provider.clone(),
                        model: model.to_string(),
                        prompt: prompt.to_string(),
                        output: format!("Agent timed out after {}s.", AGENT_TIMEOUT_SECS),
                        tool_calls: total_tool_calls,
                        tool_names: last_tool_names.clone(),
                        total_tokens,
                        duration_secs: elapsed,
                        estimated_cost_usd: estimate_cost(&effective_provider, model, tokens_in, tokens_out),
                        quality_score: None,
                        guardrail_issues: vec![],
                        success: false,
                        error_message: Some(format!("Timed out after {}s", AGENT_TIMEOUT_SECS)),
                        worker_name: None,
                        worker_latency_ms: None,
                        created_at: chrono::Utc::now().to_rfc3339(),
                        date_key: chrono::Utc::now().format("%Y-%m-%d").to_string(),
                    };
                    if let Err(te) = logger.log_run(&entry) {
                        warn!("Failed to log trajectory: {}", te);
                    }
                }
                return Ok(AgentResult {
                    agent_name: agent_name.to_string(),
                    output: format!("Agent timed out after {}s.", AGENT_TIMEOUT_SECS),
                    tool_calls_made: total_tool_calls,
                    elapsed_secs: elapsed,
                    total_tokens,
                });
            }

            debug!("Agent '{}' round {}", agent_name, round);

            // Context compaction before each LLM call (multi-strategy)
            if round > 0 {
                use crate::context_compactor::ContextCompactor;

                // Repair orphan tool results/calls before compaction
                ContextCompactor::repair_tool_pairing(&mut messages);

                if let Some(plan) = ContextCompactor::plan(&messages, model) {
                    let to_summarize: Vec<ChatMessage> = messages[plan.summarize_range.clone()].to_vec();
                    let summary_prompt = ContextCompactor::build_summary_prompt(
                        &to_summarize, &plan.tool_pairs, &plan.strategy,
                    );
                    // Use the LLM to summarize (one-shot, no tools)
                    match router.chat_with_tools(
                        &[ChatMessage {
                            role: "user".to_string(),
                            content: summary_prompt,
                            tool_calls: None,
                            tool_call_id: None,
                        }],
                        &[], // no tools
                        &effective_provider,
                    ).await {
                        Ok(resp) => {
                            let summary = resp.message.content;
                            if !summary.trim().is_empty() {
                                debug!(
                                    "Context compacted: strategy={:?}, {} messages → summary ({} chars)",
                                    plan.strategy, to_summarize.len(), summary.len()
                                );
                                ContextCompactor::apply(&mut messages, &plan, &summary);
                            } else {
                                // Fallback to hard trim
                                ContextOptimizer::trim_messages(&mut messages, model);
                            }
                        }
                        Err(e) => {
                            debug!("Context compaction LLM call failed, falling back to hard trim: {}", e);
                            ContextOptimizer::trim_messages(&mut messages, model);
                        }
                    }
                }
            }

            // On the final round, send WITHOUT tools to force a text response
            // instead of letting the model call yet another tool
            let is_final_round = round == effective_max_rounds - 1;
            let round_tools = if is_final_round && effective_max_rounds > 1 {
                debug!("Agent '{}' final round — sending without tools to force text response", agent_name);
                // Inject a nudge so the model knows to summarize
                messages.push(ChatMessage {
                    role: "user".to_string(),
                    content: "Please provide your final response now, summarizing all findings. Do not call any more tools.".to_string(),
                    tool_calls: None,
                    tool_call_id: None,
                });
                &Vec::new()
            } else {
                &tool_defs
            };

            // Response cache: check before LLM call (only on non-tool rounds, i.e., round 0)
            let tool_names: Vec<String> = tool_specs.iter().map(|s| s.name.clone()).collect();
            if round == 0 {
                if let Some(ref cache) = self.response_cache {
                    let cache_key = ResponseCache::cache_key(&messages, &tool_names);
                    if let Some(cached_response) = cache.get(cache_key) {
                        let (cached_text, cached_tcs) = dispatcher::parse_tool_calls(&cached_response, dispatch_mode);
                        if cached_tcs.is_empty() {
                            // Cache hit with no tool calls — return immediately
                            if let Some(ref bus) = self.event_bus {
                                bus.emit(AgentEvent::CacheHit { round });
                            }
                            let elapsed = t0.elapsed().as_secs_f64();
                            info!("Agent '{}' cache hit ({})", agent_name, elapsed);
                            return Ok(AgentResult {
                                agent_name: agent_name.to_string(),
                                output: cached_text,
                                tool_calls_made: 0,
                                elapsed_secs: elapsed,
                                total_tokens: 0,
                            });
                        }
                    }
                }
            }

            // Emit LlmCallStarted
            let llm_call_start = Instant::now();
            if let Some(ref bus) = self.event_bus {
                bus.emit(AgentEvent::LlmCallStarted {
                    round,
                    message_count: messages.len(),
                    estimated_tokens: ContextOptimizer::estimate_messages_tokens(&messages),
                });
            }

            let response = router
                .chat_with_tools(&messages, round_tools, &effective_provider)
                .await?;

            // Emit LlmCallCompleted
            if let Some(ref bus) = self.event_bus {
                let tokens = response.usage.as_ref().map(|u| u.total_tokens).unwrap_or(0);
                let has_tools = response.message.tool_calls.as_ref()
                    .map(|tc| !tc.is_empty()).unwrap_or(false);
                bus.emit(AgentEvent::LlmCallCompleted {
                    round,
                    tokens_used: tokens,
                    has_tool_calls: has_tools,
                    duration_ms: llm_call_start.elapsed().as_millis() as u64,
                });
            }

            // Track token usage
            if let Some(ref usage) = response.usage {
                total_tokens += usage.total_tokens;
                debug!("  Tokens this round: {} (total: {})", usage.total_tokens, total_tokens);
            }

            // Use dispatcher to parse tool calls (supports native + XML formats)
            let (response_text, mut parsed_tool_calls) = dispatcher::parse_tool_calls(&response, dispatch_mode);

            // Filter tool calls by autonomy level
            if !parsed_tool_calls.is_empty() {
                let blocked: Vec<String> = parsed_tool_calls.iter()
                    .filter(|tc| !config.autonomy.allows_tool(&tc.function.name))
                    .map(|tc| tc.function.name.clone())
                    .collect();
                if !blocked.is_empty() {
                    warn!("Agent '{}' autonomy ({}) blocked tools: {:?}", agent_name, config.autonomy, blocked);
                    parsed_tool_calls.retain(|tc| config.autonomy.allows_tool(&tc.function.name));
                }
            }

            // Check if LLM wants to call tools
            if !parsed_tool_calls.is_empty() {
                    info!(
                        "Agent '{}' round {} — {} tool call(s) (mode: {:?})",
                        agent_name, round, parsed_tool_calls.len(), dispatch_mode
                    );

                    // Add assistant message to conversation
                    // For XML-parsed calls, reconstruct the message with tool_calls attached
                    let assistant_msg = ChatMessage {
                        role: "assistant".to_string(),
                        content: response_text,
                        tool_calls: Some(parsed_tool_calls.clone()),
                        tool_call_id: None,
                    };
                    messages.push(assistant_msg);

                    // Execute tool calls in parallel (inspired by ZeroClaw's join_all pattern)
                    let tool_calls = &parsed_tool_calls;
                    total_tool_calls += tool_calls.len();

                    for tc in tool_calls {
                        info!("  Tool: {}({})", tc.function.name,
                            serde_json::to_string(&tc.function.arguments).unwrap_or_default().chars().take(80).collect::<String>());
                    }

                    let cluster_hub_ref = &self.cluster_hub;
                    let target_worker_ref = &target_worker;
                    let service_tier_ref = &self.service_tier;
                    let agent_name_for_tier = agent_name.to_string();
                    let tool_futures: Vec<_> = tool_calls.iter().map(|tc| {
                        let tool_name = tc.function.name.clone();
                        let tool_args = tc.function.arguments.clone();
                        let tool_id = tc.id.clone();
                        let hub_opt = cluster_hub_ref.clone();
                        let tw = target_worker_ref.clone();
                        let tier_opt = service_tier_ref.clone();
                        let agent_for_tier = agent_name_for_tier.clone();
                        async move {
                            // ── Service Tier: check tool access before execution ──
                            if let Some(ref tier_mgr) = tier_opt {
                                if let Err(denied) = tier_mgr.check_access(&agent_for_tier, &tool_name) {
                                    warn!("ServiceTier denied tool '{}' for agent '{}': {}", tool_name, agent_for_tier, denied);
                                    return ChatMessage {
                                        role: "tool".to_string(),
                                        content: format!("Access denied: {}", denied.reason),
                                        tool_calls: None,
                                        tool_call_id: tool_id,
                                    };
                                }
                            }

                            // Try cluster dispatch first if hub is available
                            let result = if let Some(ref hub) = hub_opt {
                                // Targeted dispatch: if target_worker is set, force dispatch there
                                if let Some(ref worker_name) = tw {
                                    if hub.should_dispatch(&tool_name) {
                                        match hub.dispatch_to_worker(worker_name, &tool_name, tool_args.clone()).await {
                                            Ok(val) => {
                                                let output = val.get("output").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                                debug!("  Tool [{}] targeted → worker '{}'", tool_name, worker_name);
                                                output
                                            }
                                            Err(e) => {
                                                warn!("  Targeted dispatch to '{}' failed for [{}], falling back to local: {}", worker_name, tool_name, e);
                                                match tool_registry.execute_tool(&tool_name, tool_args).await {
                                                    Ok(r) => r.output,
                                                    Err(e2) => format!("Tool execution error: {}", e2),
                                                }
                                            }
                                        }
                                    } else {
                                        // Local-only tool — execute locally regardless of target
                                        match tool_registry.execute_tool(&tool_name, tool_args).await {
                                            Ok(r) => r.output,
                                            Err(e) => format!("Tool execution error: {}", e),
                                        }
                                    }
                                } else if hub.should_dispatch(&tool_name) {
                                    match hub.dispatch_tool(&tool_name, tool_args.clone()).await {
                                        Ok(val) => {
                                            let output = val.get("output")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("")
                                                .to_string();
                                            let worker = val.get("worker")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("unknown");
                                            debug!("  Tool [{}] dispatched to worker '{}', len={}", tool_name, worker, output.len());
                                            output
                                        }
                                        Err(e) => {
                                            warn!("  Cluster dispatch failed for [{}], falling back to local: {}", tool_name, e);
                                            // Fallback to local execution
                                            match tool_registry.execute_tool(&tool_name, tool_args).await {
                                                Ok(r) => r.output,
                                                Err(e2) => format!("Tool execution error: {}", e2),
                                            }
                                        }
                                    }
                                } else {
                                    // Local-only tool — execute locally
                                    match tool_registry.execute_tool(&tool_name, tool_args).await {
                                        Ok(r) => {
                                            debug!("  Tool result [{}]: success={}, len={}", tool_name, r.success, r.output.len());
                                            if !r.success && r.output.contains("Rate limit exceeded") {
                                                warn!("  Rate limited [{}]: {}", tool_name, r.output);
                                            }
                                            r.output
                                        }
                                        Err(e) => {
                                            warn!("  Tool error [{}]: {}", tool_name, e);
                                            format!("Tool execution error: {}", e)
                                        }
                                    }
                                }
                            } else {
                                // No cluster hub — local execution
                                match tool_registry.execute_tool(&tool_name, tool_args).await {
                                    Ok(r) => {
                                        debug!("  Tool result [{}]: success={}, len={}", tool_name, r.success, r.output.len());
                                        if !r.success && r.output.contains("Rate limit exceeded") {
                                            warn!("  Rate limited [{}]: {}", tool_name, r.output);
                                        }
                                        r.output
                                    }
                                    Err(e) => {
                                        warn!("  Tool error [{}]: {}", tool_name, e);
                                        format!("Tool execution error: {}", e)
                                    }
                                }
                            };
                            ChatMessage {
                                role: "tool".to_string(),
                                content: result,
                                tool_calls: None,
                                tool_call_id: tool_id,
                            }
                        }
                    }).collect();

                    let tool_results = futures_util::future::join_all(tool_futures).await;
                    messages.extend(tool_results);

                    // Track tool names for progress reporting
                    last_tool_names = parsed_tool_calls.iter()
                        .map(|tc| tc.function.name.clone())
                        .collect();

                    // Periodic progress report (every PROGRESS_INTERVAL_SECS)
                    if let Some(ref tx) = progress_tx {
                        let since_last = last_progress_report.elapsed().as_secs();
                        if since_last >= PROGRESS_INTERVAL_SECS {
                            let elapsed = t0.elapsed().as_secs();
                            let tools_str = last_tool_names.join(", ");
                            let msg = format!(
                                "\u{23f3} 進度: round {}/{}, {} tool calls, {}s elapsed\n\u{1f527} 正在用: {}",
                                round + 1, effective_max_rounds, total_tool_calls, elapsed, tools_str
                            );
                            let _ = tx.send(msg).await;
                            last_progress_report = Instant::now();
                        }
                    }

                    // Advanced loop detection: record round and check for patterns
                    let call_pairs: Vec<(String, String)> = tool_calls.iter().map(|tc| {
                        (tc.function.name.clone(), tc.function.arguments.to_string())
                    }).collect();
                    match loop_detector.record_round(&call_pairs) {
                        LoopAction::Stop(kind) => {
                            let elapsed = t0.elapsed().as_secs_f64();
                            let reason = match &kind {
                                LoopKind::GenericRepeat { count } => format!("same calls repeated {} times", count),
                                LoopKind::PingPong { tool_a, tool_b } => format!("ping-pong between {} and {}", tool_a, tool_b),
                                LoopKind::StaleResult { tool, .. } => format!("stale results from {}", tool),
                            };
                            if let Some(ref bus) = self.event_bus {
                                bus.emit(AgentEvent::LoopDetected {
                                    round,
                                    kind: format!("{:?}", kind),
                                    action: "stop".into(),
                                });
                            }
                            warn!("Agent '{}' loop detected: {}", agent_name, reason);
                            self.record_run_cost(agent_name, &effective_provider, model, total_tokens, elapsed, Some("loop_detected"));
                            // Log trajectory (loop detected)
                            if let Some(ref logger) = self.trajectory_logger {
                                let tokens_in = (total_tokens as f64 * 0.6) as u32;
                                let tokens_out = total_tokens - tokens_in;
                                let entry = TrajectoryEntry {
                                    id: uuid::Uuid::new_v4().to_string(),
                                    session_id: None,
                                    agent_name: agent_name.to_string(),
                                    hand_name: None,
                                    phase_name: None,
                                    provider: effective_provider.clone(),
                                    model: model.to_string(),
                                    prompt: prompt.to_string(),
                                    output: format!("Agent stopped: loop detected ({})", reason),
                                    tool_calls: total_tool_calls,
                                    tool_names: last_tool_names.clone(),
                                    total_tokens,
                                    duration_secs: elapsed,
                                    estimated_cost_usd: estimate_cost(&effective_provider, model, tokens_in, tokens_out),
                                    quality_score: None,
                                    guardrail_issues: vec![],
                                    success: false,
                                    error_message: Some(format!("Loop detected: {}", reason)),
                                    worker_name: None,
                                    worker_latency_ms: None,
                                    created_at: chrono::Utc::now().to_rfc3339(),
                                    date_key: chrono::Utc::now().format("%Y-%m-%d").to_string(),
                                };
                                if let Err(te) = logger.log_run(&entry) {
                                    warn!("Failed to log trajectory: {}", te);
                                }
                            }
                            return Ok(AgentResult {
                                agent_name: agent_name.to_string(),
                                output: format!("Agent stopped: loop detected ({})", reason),
                                tool_calls_made: total_tool_calls,
                                elapsed_secs: elapsed,
                                total_tokens,
                            });
                        }
                        LoopAction::Warn(nudge_msg) => {
                            if let Some(ref bus) = self.event_bus {
                                bus.emit(AgentEvent::LoopDetected {
                                    round,
                                    kind: "warn".into(),
                                    action: "nudge".into(),
                                });
                            }
                            debug!("Agent '{}' loop warning, injecting nudge", agent_name);
                            messages.push(ChatMessage {
                                role: "user".to_string(),
                                content: nudge_msg,
                                tool_calls: None,
                                tool_call_id: None,
                            });
                        }
                        LoopAction::Continue => {}
                    }

                    // Record tool results for stale detection
                    for msg in messages.iter().rev().take(tool_calls.len()) {
                        if msg.role == "tool" {
                            if let Some(ref tool_id) = msg.tool_call_id {
                                if let Some(tc) = tool_calls.iter().find(|tc| tc.id.as_deref() == Some(tool_id)) {
                                    if let LoopAction::Stop(kind) = loop_detector.record_result(&tc.function.name, &msg.content) {
                                        let elapsed = t0.elapsed().as_secs_f64();
                                        let reason = match &kind {
                                            LoopKind::StaleResult { tool, .. } => format!("stale results from {}", tool),
                                            _ => "stale results".to_string(),
                                        };
                                        warn!("Agent '{}' stale result loop: {}", agent_name, reason);
                                        self.record_run_cost(agent_name, &effective_provider, model, total_tokens, elapsed, Some("stale_loop"));
                                        // Log trajectory (stale loop)
                                        if let Some(ref logger) = self.trajectory_logger {
                                            let tokens_in = (total_tokens as f64 * 0.6) as u32;
                                            let tokens_out = total_tokens - tokens_in;
                                            let entry = TrajectoryEntry {
                                                id: uuid::Uuid::new_v4().to_string(),
                                                session_id: None,
                                                agent_name: agent_name.to_string(),
                                                hand_name: None,
                                                phase_name: None,
                                                provider: effective_provider.clone(),
                                                model: model.to_string(),
                                                prompt: prompt.to_string(),
                                                output: format!("Agent stopped: loop detected ({})", reason),
                                                tool_calls: total_tool_calls,
                                                tool_names: last_tool_names.clone(),
                                                total_tokens,
                                                duration_secs: elapsed,
                                                estimated_cost_usd: estimate_cost(&effective_provider, model, tokens_in, tokens_out),
                                                quality_score: None,
                                                guardrail_issues: vec![],
                                                success: false,
                                                error_message: Some(format!("Stale loop: {}", reason)),
                                                worker_name: None,
                                                worker_latency_ms: None,
                                                created_at: chrono::Utc::now().to_rfc3339(),
                                                date_key: chrono::Utc::now().format("%Y-%m-%d").to_string(),
                                            };
                                            if let Err(te) = logger.log_run(&entry) {
                                                warn!("Failed to log trajectory: {}", te);
                                            }
                                        }
                                        return Ok(AgentResult {
                                            agent_name: agent_name.to_string(),
                                            output: format!("Agent stopped: loop detected ({})", reason),
                                            tool_calls_made: total_tool_calls,
                                            elapsed_secs: elapsed,
                                            total_tokens,
                                        });
                                    }
                                }
                            }
                        }
                    }

                    // Continue loop — LLM will process tool results
                    continue;
            }

            // No tool calls — LLM is done, return the final text
            // Cache the response for future dedup (only cache non-tool-call responses)
            if round == 0 {
                if let Some(ref cache) = self.response_cache {
                    let cache_key = ResponseCache::cache_key(&messages, &tool_names);
                    cache.put(cache_key, response.clone());
                }
            }

            let elapsed = t0.elapsed().as_secs_f64();
            info!(
                "Agent '{}' completed in {:.1}s ({} tool calls, {} tokens)",
                agent_name, elapsed, total_tool_calls, total_tokens
            );

            // Emit RunCompleted event
            if let Some(ref bus) = self.event_bus {
                bus.emit(AgentEvent::RunCompleted {
                    agent_name: agent_name.to_string(),
                    output_len: response_text.len(),
                    tool_calls_made: total_tool_calls,
                    total_tokens,
                    elapsed_secs: elapsed,
                });
            }

            let result = AgentResult {
                agent_name: agent_name.to_string(),
                output: response_text,
                tool_calls_made: total_tool_calls,
                elapsed_secs: elapsed,
                total_tokens,
            };
            self.record_run_cost(agent_name, &effective_provider, model, total_tokens, elapsed, None);

            // Log trajectory (successful completion)
            if let Some(ref logger) = self.trajectory_logger {
                let tokens_in = (total_tokens as f64 * 0.6) as u32;
                let tokens_out = total_tokens - tokens_in;
                let entry = TrajectoryEntry {
                    id: uuid::Uuid::new_v4().to_string(),
                    session_id: None,
                    agent_name: agent_name.to_string(),
                    hand_name: None,
                    phase_name: None,
                    provider: effective_provider.clone(),
                    model: model.to_string(),
                    prompt: prompt.to_string(),
                    output: result.output.clone(),
                    tool_calls: total_tool_calls,
                    tool_names: last_tool_names.clone(),
                    total_tokens,
                    duration_secs: elapsed,
                    estimated_cost_usd: estimate_cost(&effective_provider, model, tokens_in, tokens_out),
                    quality_score: None,
                    guardrail_issues: vec![],
                    success: true,
                    error_message: None,
                    worker_name: None,
                    worker_latency_ms: None,
                    created_at: chrono::Utc::now().to_rfc3339(),
                    date_key: chrono::Utc::now().format("%Y-%m-%d").to_string(),
                };
                if let Err(e) = logger.log_run(&entry) {
                    warn!("Failed to log trajectory: {}", e);
                }
            }

            return Ok(result);
        }

        // Hit max rounds
        let elapsed = t0.elapsed().as_secs_f64();
        warn!(
            "Agent '{}' hit max tool rounds ({}) after {:.1}s",
            agent_name, effective_max_rounds, elapsed
        );

        // Return last assistant content with non-empty text.
        // When hitting max rounds, the last assistant message often has empty content
        // (because it only contained tool_calls). Fall back to the last non-empty assistant
        // content, or collect tool results as a summary.
        let last_content = messages
            .iter()
            .rev()
            .find(|m| m.role == "assistant" && !m.content.trim().is_empty())
            .map(|m| m.content.clone())
            .unwrap_or_else(|| {
                // No non-empty assistant message — collect recent tool results as output
                let tool_results: Vec<&str> = messages
                    .iter()
                    .rev()
                    .take(20)
                    .filter(|m| m.role == "tool" && !m.content.trim().is_empty())
                    .map(|m| m.content.as_str())
                    .collect();
                if tool_results.is_empty() {
                    "Agent reached maximum tool rounds without final response.".to_string()
                } else {
                    format!(
                        "[Agent hit max tool rounds. Last tool results:]\n\n{}",
                        tool_results.into_iter().rev().take(3).collect::<Vec<_>>().join("\n\n---\n\n")
                    )
                }
            });

        self.record_run_cost(agent_name, &effective_provider, model, total_tokens, elapsed, None);

        // Log trajectory (hit max rounds)
        if let Some(ref logger) = self.trajectory_logger {
            let tokens_in = (total_tokens as f64 * 0.6) as u32;
            let tokens_out = total_tokens - tokens_in;
            let entry = TrajectoryEntry {
                id: uuid::Uuid::new_v4().to_string(),
                session_id: None,
                agent_name: agent_name.to_string(),
                hand_name: None,
                phase_name: None,
                provider: effective_provider.clone(),
                model: model.to_string(),
                prompt: prompt.to_string(),
                output: last_content.clone(),
                tool_calls: total_tool_calls,
                tool_names: last_tool_names.clone(),
                total_tokens,
                duration_secs: elapsed,
                estimated_cost_usd: estimate_cost(&effective_provider, model, tokens_in, tokens_out),
                quality_score: None,
                guardrail_issues: vec![],
                success: true,
                error_message: None,
                worker_name: None,
                worker_latency_ms: None,
                created_at: chrono::Utc::now().to_rfc3339(),
                date_key: chrono::Utc::now().format("%Y-%m-%d").to_string(),
            };
            if let Err(e) = logger.log_run(&entry) {
                warn!("Failed to log trajectory: {}", e);
            }
        }

        // Post-task knowledge capture: store successful tool patterns in memory
        if total_tool_calls > 0 {
            self.capture_task_knowledge(agent_name, &last_content, total_tool_calls, elapsed);
        }

        Ok(AgentResult {
            agent_name: agent_name.to_string(),
            output: last_content,
            tool_calls_made: total_tool_calls,
            elapsed_secs: elapsed,
            total_tokens,
        })
    }

    /// Record cost for an agent run (if cost tracker is attached)
    fn record_run_cost(&self, agent: &str, provider: &str, model: &str, total_tokens: u32, duration_secs: f64, context: Option<&str>) {
        if let Some(ref ct) = self.cost_tracker {
            // Estimate input/output split (rough: 60% in, 40% out)
            let tokens_in = (total_tokens as f64 * 0.6) as u32;
            let tokens_out = total_tokens - tokens_in;
            let cost = estimate_cost(provider, model, tokens_in, tokens_out);
            let record = CostRecord {
                id: uuid::Uuid::new_v4().to_string(),
                timestamp: chrono::Utc::now(),
                agent: agent.to_string(),
                provider: provider.to_string(),
                model: model.to_string(),
                tokens_in,
                tokens_out,
                total_tokens,
                estimated_cost_usd: cost,
                duration_secs,
                context: context.map(|s| s.to_string()),
            };
            if let Err(e) = ct.record(&record) {
                warn!("Failed to record cost: {}", e);
            }
        }
    }

    /// Post-task knowledge capture: store run statistics in memory for self-improvement.
    /// Only captures if memory_store is attached and output seems successful.
    fn capture_task_knowledge(&self, agent: &str, output: &str, tool_calls: usize, elapsed: f64) {
        if let Some(ref ms) = self.memory_store {
            // Skip failed outputs
            if output.starts_with("Budget exceeded") || output.starts_with("Phase failed") {
                return;
            }
            // Store a lightweight knowledge entry (key = agent_run_stats_<agent>)
            let key = format!("agent_run_stats_{}", agent);
            let content = format!(
                "Last run: {} tool calls, {:.1}s. Output length: {} chars.",
                tool_calls, elapsed, output.len()
            );
            let ms = ms.clone();
            tokio::spawn(async move {
                if let Err(e) = ms.store(&key, &content, crate::memory::MemoryCategory::Custom("system".to_string()), None).await {
                    debug!("Knowledge capture failed (non-critical): {}", e);
                }
            });
        }
    }

    /// Run a named agent, returning a stream for the final response.
    /// Tool-call rounds use non-streaming chat; the final response is wrapped as a stream.
    pub async fn run_streaming(
        &self,
        agent_name: &str,
        prompt: &str,
        history: &[ChatMessage],
        router: &LlmRouter,
        tool_registry: &ToolRegistry,
        extra_context: Option<&str>,
    ) -> Result<Pin<Box<dyn futures_util::Stream<Item = Result<StreamChunk>> + Send>>> {
        // Run the full tool loop (non-streaming)
        let result = self.run(agent_name, prompt, history, router, tool_registry, extra_context).await?;

        // Wrap the final text as a stream of ContentDelta chunks
        let text = result.output;
        let chunks = chunk_text_for_stream(&text, 50); // ~50 chars per chunk

        Ok(Box::pin(futures_util::stream::iter(
            chunks.into_iter()
                .map(|c| Ok(StreamChunk::ContentDelta(c)))
                .chain(std::iter::once(Ok(StreamChunk::Done { usage: None })))
        )))
    }

    /// Build ToolSpec list for the agent's allowed tools
    fn build_tool_specs(&self, agent_tools: &[String], registry: &ToolRegistry) -> Vec<ToolSpec> {
        let all_specs = registry.specs();
        if agent_tools.is_empty() {
            return all_specs; // Empty list = all tools
        }
        all_specs
            .into_iter()
            .filter(|s| agent_tools.iter().any(|t| t == &s.name))
            .collect()
    }

    pub fn list_agents(&self) -> Vec<String> {
        let mut names: Vec<_> = self.agents.keys().cloned().collect();
        names.sort();
        names
    }

    pub fn get_config(&self, agent_name: &str) -> Option<&AgentConfig> {
        self.agents.get(agent_name)
    }
}

/// Safely truncate a string at a character boundary (not byte boundary)
fn truncate_str(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

/// Split text into chunks for streaming delivery.
/// Tries to split at word boundaries for cleaner output.
fn chunk_text_for_stream(text: &str, target_size: usize) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }
    if text.len() <= target_size {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if remaining.len() <= target_size {
            chunks.push(remaining.to_string());
            break;
        }
        // Try to split at a space near the target size
        let split_at = remaining[..target_size]
            .rfind(' ')
            .map(|i| i + 1) // include the space in the current chunk
            .unwrap_or(target_size);
        chunks.push(remaining[..split_at].to_string());
        remaining = &remaining[split_at..];
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_agents_loaded() {
        let runtime = AgentRuntime::new("/nonexistent/path.toml").unwrap();
        let agents = runtime.list_agents();
        assert!(agents.contains(&"master".to_string()));
        assert!(agents.contains(&"coder".to_string()));
    }

    #[test]
    fn test_master_has_tools() {
        let runtime = AgentRuntime::new("/nonexistent/path.toml").unwrap();
        let master = runtime.get_config("master").unwrap();
        let tools = master.tools.as_ref().unwrap();
        assert!(tools.contains(&"shell".to_string()));
        assert!(tools.contains(&"file_read".to_string()));
        assert!(tools.contains(&"file_write".to_string()));
    }

    #[test]
    fn test_unknown_agent_returns_none() {
        let runtime = AgentRuntime::new("/nonexistent/path.toml").unwrap();
        assert!(runtime.get_config("nonexistent").is_none());
    }

    #[test]
    fn test_chunk_text_small() {
        let chunks = chunk_text_for_stream("hello", 50);
        assert_eq!(chunks, vec!["hello"]);
    }

    #[test]
    fn test_chunk_text_empty() {
        let chunks = chunk_text_for_stream("", 50);
        assert_eq!(chunks, vec![""]);
    }

    #[test]
    fn test_chunk_text_splits() {
        let text = "hello world this is a test of the chunking function for streaming";
        let chunks = chunk_text_for_stream(text, 20);
        assert!(chunks.len() > 1);
        // Reassembled text should match original
        let reassembled: String = chunks.join("");
        assert_eq!(reassembled, text);
    }

    #[test]
    fn test_chunk_text_no_spaces() {
        let text = "a".repeat(100);
        let chunks = chunk_text_for_stream(&text, 30);
        assert!(chunks.len() > 1);
        let reassembled: String = chunks.join("");
        assert_eq!(reassembled, text);
    }

    #[test]
    fn test_truncate_str() {
        assert_eq!(truncate_str("hello world", 5), "hello");
        assert_eq!(truncate_str("hi", 10), "hi");
        assert_eq!(truncate_str("你好世界", 2), "你好");
    }

    // ── Advanced Loop Detector integration tests ─────────────────────────────

    #[test]
    fn test_advanced_loop_detector_no_loop() {
        let mut detector = AdvancedLoopDetector::new(LoopDetectorConfig::default());
        let c1 = vec![("shell".into(), r#"{"command":"ls"}"#.into())];
        let c2 = vec![("shell".into(), r#"{"command":"pwd"}"#.into())];
        assert_eq!(detector.record_round(&c1), LoopAction::Continue);
        assert_eq!(detector.record_round(&c2), LoopAction::Continue);
    }

    #[test]
    fn test_advanced_loop_detector_warns_at_threshold() {
        let mut detector = AdvancedLoopDetector::new(LoopDetectorConfig::default());
        let c = vec![("shell".into(), r#"{"command":"ls"}"#.into())];
        detector.record_round(&c); // 1
        detector.record_round(&c); // 2
        match detector.record_round(&c) { // 3 = warn
            LoopAction::Warn(_) => {}
            other => panic!("Expected Warn, got {:?}", other),
        }
    }

    #[test]
    fn test_advanced_loop_detector_stops_at_threshold() {
        let mut detector = AdvancedLoopDetector::new(LoopDetectorConfig::default());
        let c = vec![("shell".into(), r#"{"command":"ls"}"#.into())];
        for _ in 0..7 { detector.record_round(&c); }
        match detector.record_round(&c) { // 8 = stop
            LoopAction::Stop(LoopKind::GenericRepeat { .. }) => {}
            other => panic!("Expected Stop(GenericRepeat), got {:?}", other),
        }
    }

    #[test]
    fn test_agent_has_daily_budget() {
        let runtime = AgentRuntime::new("/nonexistent/path.toml").unwrap();
        let master = runtime.get_config("master").unwrap();
        // Default budget is 0 (no limit)
        assert_eq!(master.daily_budget_usd, 0.0);
    }
}
