//! Plugin Bus — unified module lifecycle manager with phased initialization.
//!
//! Manages `PluginModule` registration, ordered init (by phase), event emission,
//! and shutdown with rollback on failure.

use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use tracing::{error, info};

use crate::app_context::{AppContext, PluginEvent};
use crate::health_check::HealthStatus;

// ---------------------------------------------------------------------------
// PluginModule trait
// ---------------------------------------------------------------------------

/// The universal interface for all phantom-mesh modules.
///
/// NOTE: Spec defines `version() -> semver::Version`, but we use `&str` to
/// avoid adding the `semver` dependency. No code currently parses versions.
/// Upgrade to `semver::Version` when version comparison is needed.
#[async_trait]
pub trait PluginModule: Send + Sync {
    /// Unique identifier (e.g. "health-check", "trajectory-logger").
    fn id(&self) -> &str;
    /// Semantic version string.
    fn version(&self) -> &str;
    /// Capabilities this module provides (for discovery).
    fn capabilities(&self) -> Vec<String>;
    /// Initialize the module. Register services in `ctx` for other modules.
    async fn init(&self, ctx: &AppContext) -> Result<()>;
    /// Gracefully shut down. Release resources, flush buffers.
    async fn shutdown(&self) -> Result<()>;
    /// Current health status.
    fn health(&self) -> HealthStatus;
}

// ---------------------------------------------------------------------------
// PluginBus
// ---------------------------------------------------------------------------

/// Lifecycle manager for all `PluginModule` instances.
///
/// Modules are registered with a phase number (1–7). `init_all()` initializes
/// them in ascending phase order. Within a phase, modules are initialized
/// sequentially (parallel init is a future enhancement).
///
/// On init failure, already-initialized modules are shut down in reverse order
/// (rollback).
pub struct PluginBus {
    phases: BTreeMap<u8, Vec<Arc<dyn PluginModule>>>,
    ctx: AppContext,
    initialized: Vec<String>,
    /// 防止 init_all() 被重複呼叫
    is_initialized: bool,
}

impl PluginBus {
    pub fn new() -> Self {
        Self {
            phases: BTreeMap::new(),
            ctx: AppContext::new(),
            initialized: Vec::new(),
            is_initialized: false,
        }
    }

    /// Register a module in the given phase.
    /// Returns an error if a module with the same ID already exists in any phase.
    pub fn register(&mut self, phase: u8, module: Arc<dyn PluginModule>) -> Result<()> {
        let new_id = module.id();
        for modules in self.phases.values() {
            for existing in modules {
                if existing.id() == new_id {
                    return Err(anyhow::anyhow!(
                        "Duplicate module ID '{}': already registered",
                        new_id
                    ));
                }
            }
        }
        self.phases.entry(phase).or_default().push(module);
        Ok(())
    }

    pub fn context(&self) -> &AppContext {
        &self.ctx
    }

    pub async fn init_all(&mut self) -> Result<()> {
        if self.is_initialized {
            return Err(anyhow::anyhow!(
                "PluginBus is already initialized; call shutdown_all() before re-initializing"
            ));
        }

        // Collect (phase, module) pairs up-front so we don't hold an immutable
        // borrow of `self.phases` across the await points where we may need
        // `&mut self` for rollback.
        let ordered: Vec<(u8, Arc<dyn PluginModule>)> = self
            .phases
            .iter()
            .flat_map(|(phase, modules)| {
                modules.iter().map(move |m| (*phase, m.clone()))
            })
            .collect();

        for (phase, module) in &ordered {
            info!("[PluginBus] Phase {} — initializing '{}'...", phase, module.id());
            match module.init(&self.ctx).await {
                Ok(()) => {
                    self.initialized.push(module.id().to_string());
                    self.ctx.emit(PluginEvent::ModuleInitialized {
                        module_id: module.id().to_string(),
                    });
                    info!("[PluginBus] '{}' initialized OK", module.id());
                }
                Err(e) => {
                    let mod_id = module.id().to_string();
                    let err_str = e.to_string();
                    error!("[PluginBus] '{}' init failed: {} — rolling back", mod_id, err_str);
                    self.ctx.emit(PluginEvent::ModuleFailed {
                        module_id: mod_id.clone(),
                        error: err_str.clone(),
                    });
                    self.rollback_shutdown().await;
                    return Err(anyhow::anyhow!(
                        "Module '{}' failed to initialize: {}",
                        mod_id, err_str
                    ));
                }
            }
        }
        self.is_initialized = true;
        Ok(())
    }

    pub async fn shutdown_all(&mut self) -> Result<()> {
        let mut errors = Vec::new();
        for module_id in self.initialized.iter().rev() {
            if let Some(module) = self.find_module(module_id) {
                info!("[PluginBus] Shutting down '{}'...", module_id);
                if let Err(e) = module.shutdown().await {
                    let err_str = e.to_string();
                    error!("[PluginBus] '{}' shutdown error: {}", module_id, err_str);
                    errors.push(format!("{}: {}", module_id, err_str));
                    self.ctx.emit(PluginEvent::ModuleShutdownFailed {
                        module_id: module_id.clone(),
                        error: err_str,
                    });
                } else {
                    self.ctx.emit(PluginEvent::ModuleShutdown {
                        module_id: module_id.clone(),
                    });
                }
            }
        }
        self.initialized.clear();
        self.is_initialized = false;
        self.ctx.clear();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(anyhow::anyhow!("Shutdown errors: {}", errors.join("; ")))
        }
    }

    pub fn module_ids(&self) -> Vec<String> {
        self.phases
            .values()
            .flat_map(|modules| modules.iter().map(|m| m.id().to_string()))
            .collect()
    }

    pub fn initialized_ids(&self) -> &[String] {
        &self.initialized
    }

    pub fn system_health(&self) -> HealthStatus {
        let mut worst = HealthStatus::Healthy;
        for module_id in &self.initialized {
            if let Some(module) = self.find_module(module_id) {
                worst = worst.worse(module.health());
            }
        }
        worst
    }

    async fn rollback_shutdown(&mut self) {
        info!("[PluginBus] Rolling back {} initialized modules...", self.initialized.len());
        for module_id in self.initialized.iter().rev() {
            if let Some(module) = self.find_module(module_id) {
                if let Err(e) = module.shutdown().await {
                    error!("[PluginBus] Rollback: '{}' shutdown error: {}", module_id, e);
                } else {
                    self.ctx.emit(PluginEvent::ModuleShutdown {
                        module_id: module_id.clone(),
                    });
                }
            }
        }
        self.initialized.clear();
        self.is_initialized = false;
        self.ctx.clear();
    }

    fn find_module(&self, id: &str) -> Option<Arc<dyn PluginModule>> {
        self.phases
            .values()
            .flat_map(|modules| modules.iter())
            .find(|m| m.id() == id)
            .cloned()
    }
}

impl Default for PluginBus {
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
    use std::sync::atomic::{AtomicBool, Ordering};

    // -----------------------------------------------------------------------
    // DummyPlugin — tracks init/shutdown via AtomicBool
    // -----------------------------------------------------------------------

    struct DummyPlugin {
        name: String,
        init_called: Arc<AtomicBool>,
        shutdown_called: Arc<AtomicBool>,
    }

    impl DummyPlugin {
        fn new(name: &str) -> (Arc<Self>, Arc<AtomicBool>, Arc<AtomicBool>) {
            let init_called = Arc::new(AtomicBool::new(false));
            let shutdown_called = Arc::new(AtomicBool::new(false));
            let plugin = Arc::new(Self {
                name: name.to_string(),
                init_called: init_called.clone(),
                shutdown_called: shutdown_called.clone(),
            });
            (plugin, init_called, shutdown_called)
        }
    }

    #[async_trait]
    impl PluginModule for DummyPlugin {
        fn id(&self) -> &str { &self.name }
        fn version(&self) -> &str { "0.1.0" }
        fn capabilities(&self) -> Vec<String> { vec!["dummy".to_string()] }
        async fn init(&self, _ctx: &AppContext) -> Result<()> {
            self.init_called.store(true, Ordering::SeqCst);
            Ok(())
        }
        async fn shutdown(&self) -> Result<()> {
            self.shutdown_called.store(true, Ordering::SeqCst);
            Ok(())
        }
        fn health(&self) -> HealthStatus { HealthStatus::Healthy }
    }

    // -----------------------------------------------------------------------
    // OrderPlugin — records init/shutdown order into a shared Vec
    // -----------------------------------------------------------------------

    struct OrderPlugin {
        name: String,
        order: Arc<std::sync::Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl PluginModule for OrderPlugin {
        fn id(&self) -> &str { &self.name }
        fn version(&self) -> &str { "0.1.0" }
        fn capabilities(&self) -> Vec<String> { vec![] }
        async fn init(&self, _ctx: &AppContext) -> Result<()> {
            self.order.lock().unwrap().push(format!("init:{}", self.name));
            Ok(())
        }
        async fn shutdown(&self) -> Result<()> {
            self.order.lock().unwrap().push(format!("shutdown:{}", self.name));
            Ok(())
        }
        fn health(&self) -> HealthStatus { HealthStatus::Healthy }
    }

    // -----------------------------------------------------------------------
    // FailPlugin — always fails on init
    // -----------------------------------------------------------------------

    struct FailPlugin {
        name: String,
    }

    #[async_trait]
    impl PluginModule for FailPlugin {
        fn id(&self) -> &str { &self.name }
        fn version(&self) -> &str { "0.1.0" }
        fn capabilities(&self) -> Vec<String> { vec![] }
        async fn init(&self, _ctx: &AppContext) -> Result<()> {
            Err(anyhow::anyhow!("boom"))
        }
        async fn shutdown(&self) -> Result<()> { Ok(()) }
        fn health(&self) -> HealthStatus { HealthStatus::Unhealthy }
    }

    // -----------------------------------------------------------------------
    // DegradedPlugin — reports Degraded health
    // -----------------------------------------------------------------------

    struct DegradedPlugin {
        name: String,
        init_called: Arc<AtomicBool>,
    }

    #[async_trait]
    impl PluginModule for DegradedPlugin {
        fn id(&self) -> &str { &self.name }
        fn version(&self) -> &str { "0.1.0" }
        fn capabilities(&self) -> Vec<String> { vec![] }
        async fn init(&self, _ctx: &AppContext) -> Result<()> {
            self.init_called.store(true, Ordering::SeqCst);
            Ok(())
        }
        async fn shutdown(&self) -> Result<()> { Ok(()) }
        fn health(&self) -> HealthStatus { HealthStatus::Degraded }
    }

    // -----------------------------------------------------------------------
    // Tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_init_all_calls_init_on_all_modules() {
        let (p1, init1, _shut1) = DummyPlugin::new("alpha");
        let (p2, init2, _shut2) = DummyPlugin::new("beta");

        let mut bus = PluginBus::new();
        bus.register(1, p1).unwrap();
        bus.register(1, p2).unwrap();

        bus.init_all().await.unwrap();

        assert!(init1.load(Ordering::SeqCst), "alpha should be initialized");
        assert!(init2.load(Ordering::SeqCst), "beta should be initialized");
    }

    #[tokio::test]
    async fn test_shutdown_all_calls_shutdown_in_reverse() {
        let order = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));

        let a: Arc<dyn PluginModule> = Arc::new(OrderPlugin {
            name: "a".to_string(),
            order: order.clone(),
        });
        let b: Arc<dyn PluginModule> = Arc::new(OrderPlugin {
            name: "b".to_string(),
            order: order.clone(),
        });
        let c: Arc<dyn PluginModule> = Arc::new(OrderPlugin {
            name: "c".to_string(),
            order: order.clone(),
        });

        let mut bus = PluginBus::new();
        bus.register(1, a).unwrap();
        bus.register(2, b).unwrap();
        bus.register(3, c).unwrap();

        bus.init_all().await.unwrap();

        // Clear init entries so we only see shutdown order.
        order.lock().unwrap().clear();

        bus.shutdown_all().await.unwrap();

        let entries = order.lock().unwrap().clone();
        assert_eq!(entries, vec!["shutdown:c", "shutdown:b", "shutdown:a"]);
    }

    #[tokio::test]
    async fn test_rollback_on_init_failure() {
        let order = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));

        let ok1: Arc<dyn PluginModule> = Arc::new(OrderPlugin {
            name: "ok1".to_string(),
            order: order.clone(),
        });
        let ok2: Arc<dyn PluginModule> = Arc::new(OrderPlugin {
            name: "ok2".to_string(),
            order: order.clone(),
        });
        let fail: Arc<dyn PluginModule> = Arc::new(FailPlugin {
            name: "fail".to_string(),
        });

        let mut bus = PluginBus::new();
        bus.register(1, ok1).unwrap();
        bus.register(2, ok2).unwrap();
        bus.register(3, fail).unwrap();

        let result = bus.init_all().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("boom"));

        // ok1 and ok2 should have been rolled back (shutdown called).
        let entries = order.lock().unwrap().clone();
        assert!(entries.contains(&"shutdown:ok2".to_string()));
        assert!(entries.contains(&"shutdown:ok1".to_string()));

        // initialized list should be cleared after rollback.
        assert!(bus.initialized_ids().is_empty());
    }

    #[tokio::test]
    async fn test_phase_ordering() {
        let order = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));

        let p3: Arc<dyn PluginModule> = Arc::new(OrderPlugin {
            name: "phase3".to_string(),
            order: order.clone(),
        });
        let p1: Arc<dyn PluginModule> = Arc::new(OrderPlugin {
            name: "phase1".to_string(),
            order: order.clone(),
        });
        let p2: Arc<dyn PluginModule> = Arc::new(OrderPlugin {
            name: "phase2".to_string(),
            order: order.clone(),
        });

        let mut bus = PluginBus::new();
        // Register out of order.
        bus.register(3, p3).unwrap();
        bus.register(1, p1).unwrap();
        bus.register(2, p2).unwrap();

        bus.init_all().await.unwrap();

        let entries = order.lock().unwrap().clone();
        assert_eq!(
            entries,
            vec!["init:phase1", "init:phase2", "init:phase3"]
        );
    }

    #[tokio::test]
    async fn test_system_health_worst_of() {
        let (healthy, _init_h, _shut_h) = DummyPlugin::new("healthy");
        let degraded_init = Arc::new(AtomicBool::new(false));
        let degraded: Arc<dyn PluginModule> = Arc::new(DegradedPlugin {
            name: "degraded".to_string(),
            init_called: degraded_init,
        });

        let mut bus = PluginBus::new();
        bus.register(1, healthy).unwrap();
        bus.register(1, degraded).unwrap();

        bus.init_all().await.unwrap();

        assert_eq!(bus.system_health(), HealthStatus::Degraded);
    }

    #[tokio::test]
    async fn test_module_ids_and_initialized_ids() {
        let (p1, _, _) = DummyPlugin::new("alpha");
        let (p2, _, _) = DummyPlugin::new("beta");

        let mut bus = PluginBus::new();
        bus.register(1, p1).unwrap();
        bus.register(2, p2).unwrap();

        // Before init: module_ids should list all, initialized_ids should be empty.
        let all_ids = bus.module_ids();
        assert_eq!(all_ids, vec!["alpha", "beta"]);
        assert!(bus.initialized_ids().is_empty());

        // After init: initialized_ids should match.
        bus.init_all().await.unwrap();
        assert_eq!(bus.initialized_ids(), &["alpha", "beta"]);
    }

    #[tokio::test]
    async fn test_init_emits_events() {
        let (p1, _, _) = DummyPlugin::new("emitter");

        let mut bus = PluginBus::new();
        bus.register(1, p1).unwrap();

        // Subscribe before init.
        let mut rx = bus.context().subscribe();

        bus.init_all().await.unwrap();

        let event = rx.recv().await.unwrap();
        match event {
            PluginEvent::ModuleInitialized { module_id } => {
                assert_eq!(module_id, "emitter");
            }
            _ => panic!("Expected ModuleInitialized event"),
        }
    }

    // -----------------------------------------------------------------------
    // New tests: duplicate ID rejection (Fix 3)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_register_duplicate_id_same_phase_rejected() {
        let (p1, _, _) = DummyPlugin::new("dup");
        let (p2, _, _) = DummyPlugin::new("dup");

        let mut bus = PluginBus::new();
        bus.register(1, p1).unwrap();
        let err = bus.register(1, p2).unwrap_err();
        assert!(err.to_string().contains("Duplicate module ID 'dup'"));
    }

    #[tokio::test]
    async fn test_register_duplicate_id_different_phase_rejected() {
        let (p1, _, _) = DummyPlugin::new("dup");
        let (p2, _, _) = DummyPlugin::new("dup");

        let mut bus = PluginBus::new();
        bus.register(1, p1).unwrap();
        let err = bus.register(3, p2).unwrap_err();
        assert!(err.to_string().contains("Duplicate module ID 'dup'"));
    }

    #[tokio::test]
    async fn test_register_different_ids_accepted() {
        let (p1, _, _) = DummyPlugin::new("alpha");
        let (p2, _, _) = DummyPlugin::new("beta");

        let mut bus = PluginBus::new();
        bus.register(1, p1).unwrap();
        bus.register(1, p2).unwrap();
        assert_eq!(bus.module_ids().len(), 2);
    }

    // -----------------------------------------------------------------------
    // New tests: idempotency guard (Fix 2)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_double_init_returns_error() {
        let (p1, _, _) = DummyPlugin::new("guard");

        let mut bus = PluginBus::new();
        bus.register(1, p1).unwrap();

        bus.init_all().await.unwrap();

        // 第二次呼叫 init_all() 應回傳錯誤
        let err = bus.init_all().await.unwrap_err();
        assert!(err.to_string().contains("already initialized"));
    }

    #[tokio::test]
    async fn test_reinit_after_shutdown_succeeds() {
        let (p1, _, _) = DummyPlugin::new("reinit");

        let mut bus = PluginBus::new();
        bus.register(1, p1).unwrap();

        bus.init_all().await.unwrap();
        bus.shutdown_all().await.unwrap();

        // shutdown_all 重設 is_initialized，再次 init 應成功
        bus.init_all().await.unwrap();
        assert_eq!(bus.initialized_ids(), &["reinit"]);
    }

    #[tokio::test]
    async fn test_reinit_after_rollback_succeeds() {
        let order = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));

        let ok1: Arc<dyn PluginModule> = Arc::new(OrderPlugin {
            name: "ok_rollback".to_string(),
            order: order.clone(),
        });
        let fail: Arc<dyn PluginModule> = Arc::new(FailPlugin {
            name: "fail_rollback".to_string(),
        });

        let mut bus = PluginBus::new();
        bus.register(1, ok1).unwrap();
        bus.register(2, fail).unwrap();

        // 初始化失敗 → rollback 重設 is_initialized
        assert!(bus.init_all().await.is_err());

        // rollback 之後應可重新 init（雖然仍會失敗，但不會因為 idempotency guard）
        let err = bus.init_all().await.unwrap_err();
        assert!(err.to_string().contains("boom"), "should fail from FailPlugin, not idempotency guard");
    }

    // -----------------------------------------------------------------------
    // New tests: shutdown event semantics (Fix 4)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_shutdown_emits_correct_events_on_success() {
        let (p1, _, _) = DummyPlugin::new("evt_ok");

        let mut bus = PluginBus::new();
        bus.register(1, p1).unwrap();

        bus.init_all().await.unwrap();

        let mut rx = bus.context().subscribe();

        bus.shutdown_all().await.unwrap();

        let event = rx.recv().await.unwrap();
        match event {
            PluginEvent::ModuleShutdown { module_id } => {
                assert_eq!(module_id, "evt_ok");
            }
            _ => panic!("Expected ModuleShutdown event, got {:?}", event),
        }
    }

    #[tokio::test]
    async fn test_shutdown_emits_failed_event_on_error() {
        // ShutdownFailPlugin — always fails on shutdown
        struct ShutdownFailPlugin { name: String }
        #[async_trait]
        impl PluginModule for ShutdownFailPlugin {
            fn id(&self) -> &str { &self.name }
            fn version(&self) -> &str { "0.1.0" }
            fn capabilities(&self) -> Vec<String> { vec![] }
            async fn init(&self, _ctx: &AppContext) -> Result<()> { Ok(()) }
            async fn shutdown(&self) -> Result<()> {
                Err(anyhow::anyhow!("shutdown_boom"))
            }
            fn health(&self) -> HealthStatus { HealthStatus::Healthy }
        }

        let fail_shut: Arc<dyn PluginModule> = Arc::new(ShutdownFailPlugin {
            name: "shut_fail".to_string(),
        });

        let mut bus = PluginBus::new();
        bus.register(1, fail_shut).unwrap();
        bus.init_all().await.unwrap();

        let mut rx = bus.context().subscribe();

        let result = bus.shutdown_all().await;
        assert!(result.is_err());

        let event = rx.recv().await.unwrap();
        match event {
            PluginEvent::ModuleShutdownFailed { module_id, error } => {
                assert_eq!(module_id, "shut_fail");
                assert!(error.contains("shutdown_boom"));
            }
            _ => panic!("Expected ModuleShutdownFailed event, got {:?}", event),
        }
    }
}
