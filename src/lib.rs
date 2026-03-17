// clawtex-core — LLM Cluster Core Library
pub mod providers;
pub mod context;
pub mod hooks;
pub mod cli;
pub mod llm_router;
pub mod task_queue;
pub mod agent_runtime;
pub mod cluster;
pub mod cluster_hub;
pub mod cluster_worker;
pub mod channel;
pub mod telegram;
pub mod tools;
pub mod sandbox;
pub mod conversation;
pub mod dashboard;
pub mod cron;
pub mod memory;
pub mod skills;
pub mod evaluate;
pub mod mcp;
pub mod security;
pub mod estop;
pub mod gateway;
pub mod approval;
pub mod hands;
pub mod cost_tracker;
pub mod revenue_tracker;
pub mod dispatcher;
pub mod plugins;
pub mod a2a;
pub mod metrics;
pub mod git_ops;
pub mod skeleton;
pub mod revenue_engine;
pub mod loop_detection;
pub mod response_cache;
pub mod context_compactor;
pub mod agent_events;
pub mod think_filter;
pub mod guardrail;
pub mod error_codes;
pub mod circuit_breaker;
pub mod watchdog;
pub mod trajectory;
pub mod prompt_optimizer;
pub mod injection_guard;
pub mod knowledge_capture;
pub mod auto_diagnosis;
pub mod load_test;
pub mod consistency_test;
pub mod audit_log;
pub mod service_tier;
pub mod onboarding;
pub mod multi_tenant;
pub mod order_workflow;
pub mod task_preemption;
pub mod node_scoring;
pub mod customer_health;
pub mod observational_memory;
pub mod ops_report;

// Re-export types from their canonical locations
pub use providers::{
    Provider, ProviderRouter, ProviderConfig, ProviderCapabilities, StreamChunk,
    ChatMessage, ChatResponse, ToolCall, TokenUsage, MockProvider,
    ProviderRotation, RotationConfig, ProviderRotationStatus,
    CodexTokenManager, CodexAwareProvider, CodexCredential, CodexUsageSnapshot, ModelInfo,
};
// Backward-compat re-export from llm_router
pub use llm_router::LlmRouter;
pub use task_queue::{TaskQueue, TaskPriority};
pub use agent_runtime::AgentRuntime;
pub use cluster::{ClusterRegistry, ClusterNode};
pub use cluster_hub::{ClusterHub, ClusterMetrics, WorkerStats, ToolRouting, PollTaskResponse, TaskResultPayload, AgentTask};
pub use cluster_worker::{ClusterWorker, ClusterConfig, WorkerConfig};
pub use channel::{Channel, ChannelMessage, ChannelType, ChannelRegistry};
pub use telegram::{TelegramChannel, TelegramConfig};
pub use tools::{ToolRegistry, SecurityConfig, RateLimitConfig, ActionTracker, scrub_credentials};
pub use tools::web_search::SearchConfig;
pub use tools::ai_code::AiCodeConfig;
pub use tools::computer_use::ComputerUseConfig;
pub use tools::email::EmailConfig;
pub use tools::email_receive::ImapConfig;
pub use tools::twitter::TwitterConfig;
pub use tools::blog_publish::BlogConfig;
pub use tools::slack::SlackConfig;
pub use tools::discord::DiscordConfig;
pub use tools::line_notify::LineConfig;
pub use tools::whatsapp::WhatsAppConfig;
pub use conversation::ConversationStore;
pub use cron::{Scheduler, CronStore, CronJob, Schedule, JobAction, JobStatus};
pub use memory::{MemoryStore, MemoryConfig, MemoryCategory, MemoryEntry};
pub use skills::{SkillRegistry, SkillDef, LoadedSkill, TrustLevel};
pub use evaluate::{EvalConfig, EvalResult};
pub use context::ContextOptimizer;
pub use hooks::{HookRunner, HookContext, HookResult};
pub use mcp::{McpBridge, McpServerConfig, McpToolProxy};
pub use security::{SecretManager, AutonomyLevel, Role, RoleRegistry, PrivacyGuard, PrivacyConfig, PrivacyTier};
pub use estop::{EStop, EStopError, Heartbeat};
pub use gateway::{GatewayState, AgentThinkRequest, AgentThinkResponse};
pub use approval::{ApprovalGate, ApprovalConfig, ApprovalResult, ApprovalNotifier, ApprovalTier, ApprovalPolicy, TierPolicyConfig, tier_for_tool, is_emergency_operation};
pub use hands::{Hand, HandRegistry, HandRunner, HandResult, PhaseOutput, PreflightResult};
pub use cost_tracker::{CostTracker, CostRecord, CostSummary, estimate_cost, BudgetBreaker, BudgetStatus};
pub use revenue_tracker::{RevenueTracker, RevenueRecord, RevenueSummary, RevenueStatus};
pub use dispatcher::{DispatchMode, parse_tool_calls, xml_tool_instructions, dispatch_mode_for_provider};
pub use plugins::{PluginRegistry, PluginManifest, PluginCapability, PluginInfo, PluginStatus};
pub use a2a::{AgentCard, A2ATask, TaskStatus, CreateTaskRequest};
pub use metrics::MetricsRegistry;
pub use git_ops::GitBranch;
pub use skeleton::{SkeletonRunner, SkeletonConfig, SkeletonResult};
pub use loop_detection::{AdvancedLoopDetector, LoopDetectorConfig, LoopAction, LoopKind};
pub use response_cache::{ResponseCache, ResponseCacheConfig, CacheStats};
pub use context_compactor::{ContextCompactor, CompactionStrategy, CompactionPlan};
pub use agent_events::{AgentEventBus, AgentEvent};
pub use revenue_engine::{
    RevenueEngine, RevenueEngineConfig, RouteROI, BudgetState, DashboardData,
    OptimizationDecision, RouteAdjustment, AdjustmentAction, ProviderSwitch,
    Alert, AlertLevel, TrendDirection, ScheduleEntry,
    route_hands, default_schedule_entries, default_cron_schedules,
};
pub use error_codes::{ErrorCode, ClawtexError, error_class_to_code};
pub use watchdog::{WorkerWatchdog, RecoveryConfig, WatchdogEvent, WatchdogStatus};
pub use trajectory::{TrajectoryLogger, TrajectoryEntry, QualityStats, WorkerEfficiency};
pub use circuit_breaker::{ProviderCircuitBreaker, BreakerConfig, CircuitStatus};
pub use prompt_optimizer::{PromptOptimizer, OptimizationResult, OptimizationConfig};
pub use injection_guard::{InjectionGuard, InjectionResult, Severity};
pub use knowledge_capture::{KnowledgeCapturer, KnowledgeNode};
pub use auto_diagnosis::{AutoDiagnoser, ErrorContext, DiagnosisReport, ErrorCategory, KnownIssue, SimilarDiagnosis};
pub use load_test::{
    LoadTester, LoadTestStore, LoadTestStatus,
    StressTestConfig, StressTestReport, EnduranceConfig, EnduranceReport,
    HandExecutor, RealHandExecutor, TimelinePoint,
};
pub use consistency_test::{ConsistencyTester, ConsistencyReport, WorkerResult, PairSimilarity, BatchSummary, PREDEFINED_TEST_SUITE};
pub use audit_log::{AuditLogger, AuditEntry, AuditFilter, ActionType, RiskLevel, Outcome, risk_level_for_tool};
pub use service_tier::{ServiceTierManager, ServiceTier, TierConfig, TierLimits, TierUsage, TierDenied, AllowedSet, default_tier_config};
pub use onboarding::{WorkerOnboarder, OnboardConfig, OnboardResult, OnboardStep, StepStatus, HealthStatus, OnboardingStatus, OnboardingState};
pub use multi_tenant::{TenantManager, Tenant, extract_tenant_key};
pub use order_workflow::{OrderWorkflow, Order, OrderStatus, PipelineSummary};
pub use task_preemption::{PreemptionManager, PreemptionPlan, PreemptionDecision, RunningTask, PreemptedRecord};
pub use node_scoring::{NodeScorer, NodeScore, NodeMetrics, NodeGrade};
pub use customer_health::{CustomerHealthManager, ChurnDetector, CustomerHealth, HealthGrade, ChurnAlert, ChurnRiskLevel, ChurnSummary};
pub use observational_memory::{ObservationalMemory, Observation, ConversationMessage};
pub use ops_report::{OpsReporter, OpsReport, ReportType, NodeHealth, NodeStatus, CostSection, TaskSection, PipelineSection};
