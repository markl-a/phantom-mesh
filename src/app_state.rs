//! Application state — extracted from main.rs for test harness access.

use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock as TokioRwLock;

use crate::agent_runtime::AgentRuntime;
use crate::approval::ApprovalGate;
use crate::audit_log::AuditLogger;
use crate::auto_diagnosis::AutoDiagnoser;
use crate::cluster::ClusterRegistry;
use crate::cluster_hub::ClusterHub;
use crate::conversation::ConversationStore;
use crate::cost_tracker::CostTracker;
use crate::cron::Scheduler;
use crate::customer_health::{CustomerHealthManager, ChurnDetector};
use crate::estop::EStop;
use crate::evaluate::EvalConfig;
use crate::event_triggers::EventTriggerManager;
use crate::feedback_loop::FeedbackLoopConfig;
use crate::financial_monitor::FinancialMonitor;
use crate::goals::GoalsStore;
use crate::governor::Governor;
use crate::hands::HandRegistry;
use crate::llm_router::LlmRouter;
use crate::load_test::LoadTester;
use crate::memory::MemoryStore;
use crate::metrics::MetricsRegistry;
use crate::multi_tenant::TenantManager;
use crate::networking::RouteManager;
use crate::node_scoring::NodeScorer;
use crate::observational_memory::ObservationalMemory;
use crate::onboarding::WorkerOnboarder;
use crate::optimizer_store::OptimizerStore;
use crate::order_workflow::OrderWorkflow;
use crate::pipeline::PipelineOrchestrator;
use crate::power_economics::PowerEconomics;
use crate::provider_pricing::ProviderPricingStore;
use crate::revenue_tracker::RevenueTracker;
use crate::roi_gate::RoiGate;
use crate::roi_scheduler::RoiScheduler;
use crate::service_tier::ServiceTierManager;
use crate::skills::SkillRegistry;
use crate::task_preemption::PreemptionManager;
use crate::task_queue::TaskQueue;
use crate::telegram_i18n::TelegramI18n;
use crate::tools::ToolRegistry;
use crate::unit_economics::UnitEconomics;
use crate::user_profile::UserProfile;

/// Central application state shared across all Axum handlers.
#[derive(Clone)]
pub struct AppState {
    pub llm_router: Arc<LlmRouter>,
    pub task_queue: Arc<TaskQueue>,
    pub agent_runtime: Arc<AgentRuntime>,
    pub cluster: Arc<ClusterRegistry>,
    pub tool_registry: Arc<ToolRegistry>,
    pub conversations: Arc<ConversationStore>,
    pub memory_store: Option<Arc<MemoryStore>>,
    pub skill_registry: Arc<SkillRegistry>,
    pub eval_config: EvalConfig,
    pub estop: Arc<EStop>,
    pub hands: Arc<HandRegistry>,
    pub approval_gate: Arc<ApprovalGate>,
    pub scheduler: Option<Arc<Scheduler>>,
    pub cost_tracker: Option<Arc<CostTracker>>,
    pub revenue_tracker: Option<Arc<RevenueTracker>>,
    pub cluster_hub: Option<Arc<ClusterHub>>,
    pub hub_api_key: Option<String>,
    pub dashboard_token: String,
    pub public_url: Option<String>,
    pub metrics_registry: Arc<MetricsRegistry>,
    pub audit_logger: Option<Arc<AuditLogger>>,
    pub load_tester: Option<Arc<LoadTester>>,
    pub worker_onboarder: Option<Arc<WorkerOnboarder>>,
    pub service_tier: Option<Arc<ServiceTierManager>>,
    pub optimizer_store: Option<Arc<OptimizerStore>>,
    pub auto_diagnoser: Option<Arc<AutoDiagnoser>>,
    pub tenant_manager: Option<Arc<TenantManager>>,
    pub order_workflow: Option<Arc<OrderWorkflow>>,
    pub customer_health: Option<Arc<CustomerHealthManager>>,
    pub churn_detector: Option<Arc<ChurnDetector>>,
    pub observational_memory: Option<Arc<ObservationalMemory>>,
    pub preemption_manager: Option<Arc<PreemptionManager>>,
    pub node_scorer: Option<Arc<NodeScorer>>,
    pub power_economics: Option<Arc<PowerEconomics>>,
    pub provider_pricing: Option<Arc<ProviderPricingStore>>,
    pub financial_monitor: Option<Arc<FinancialMonitor>>,
    pub unit_economics: Option<Arc<UnitEconomics>>,
    pub telegram_i18n: Arc<TokioRwLock<TelegramI18n>>,
    /// Shared secret for inter-node cluster authentication.
    /// When set, cluster endpoints (register, heartbeat, poll, result) require
    /// `Authorization: Bearer <secret>`. When `None`, auth is disabled (open cluster).
    pub cluster_secret: Option<String>,
    pub started_at: Instant,
    // Efficiency engine subsystems
    pub roi_gate: Option<Arc<RoiGate>>,
    pub governor: Option<Arc<Governor>>,
    pub pipeline_orchestrator: Option<Arc<TokioRwLock<PipelineOrchestrator>>>,
    pub feedback_loop_config: Option<FeedbackLoopConfig>,
    pub roi_scheduler: Option<Arc<RoiScheduler>>,
    pub route_manager: Option<Arc<RouteManager>>,
    pub goals_store: Option<Arc<GoalsStore>>,
    pub user_profile: Arc<std::sync::RwLock<UserProfile>>,
    /// Event trigger manager — shared with cron tick loop and Telegram /alerts handler.
    pub trigger_manager: Option<Arc<std::sync::Mutex<EventTriggerManager>>>,
    /// Background networking task handles (for shutdown cleanup).
    #[allow(dead_code)]
    pub networking_tasks: Arc<tokio::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>>,
}

impl AppState {
    /// Create an AppState with minimal required fields for testing.
    /// All `Option<Arc<T>>` fields default to `None`.
    pub async fn test_default(
        llm_router: Arc<LlmRouter>,
        agent_runtime: Arc<AgentRuntime>,
        tool_registry: Arc<ToolRegistry>,
        temp_dir: &std::path::Path,
    ) -> anyhow::Result<Self> {
        let db_path = temp_dir.join("core.db");
        let task_queue = Arc::new(TaskQueue::new(db_path.to_str().unwrap()).await?);
        let cluster = Arc::new(ClusterRegistry::new(":memory:").await?);
        let conversations = Arc::new(ConversationStore::new(
            temp_dir.join("conversations.db").to_str().unwrap(),
        ).await?);
        let skill_registry = Arc::new(SkillRegistry::new());
        let hands = Arc::new(HandRegistry::default());
        let metrics = Arc::new(MetricsRegistry::new());
        let estop = Arc::new(EStop::new());
        let approval = Arc::new(ApprovalGate::new(Default::default()));
        let i18n = Arc::new(TokioRwLock::new(TelegramI18n::new()));
        let profile = Arc::new(std::sync::RwLock::new(UserProfile::default()));

        Ok(Self {
            llm_router,
            task_queue,
            agent_runtime,
            cluster,
            tool_registry,
            conversations,
            memory_store: None,
            skill_registry,
            eval_config: EvalConfig::default(),
            estop,
            hands,
            approval_gate: approval,
            scheduler: None,
            cost_tracker: None,
            revenue_tracker: None,
            cluster_hub: None,
            hub_api_key: None,
            dashboard_token: "test-token".to_string(),
            public_url: None,
            metrics_registry: metrics,
            audit_logger: None,
            load_tester: None,
            worker_onboarder: None,
            service_tier: None,
            optimizer_store: None,
            auto_diagnoser: None,
            tenant_manager: None,
            order_workflow: None,
            customer_health: None,
            churn_detector: None,
            observational_memory: None,
            preemption_manager: None,
            node_scorer: None,
            power_economics: None,
            provider_pricing: None,
            financial_monitor: None,
            unit_economics: None,
            telegram_i18n: i18n,
            cluster_secret: None,
            started_at: Instant::now(),
            roi_gate: None,
            governor: None,
            pipeline_orchestrator: None,
            feedback_loop_config: None,
            roi_scheduler: None,
            route_manager: None,
            goals_store: None,
            user_profile: profile,
            trigger_manager: None,
            networking_tasks: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        })
    }
}
