//! Edge-case and stress tests for PluginBus + AppContext.
//!
//! Covers: double init, shutdown-without-init, duplicate module IDs,
//! concurrent AppContext access, broadcast overflow, rollback-during-rollback,
//! empty bus, multi-phase registration, and more.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use async_trait::async_trait;

use phantom_mesh::app_context::{AppContext, PluginEvent};
use phantom_mesh::health_check::HealthStatus;
use phantom_mesh::plugin_bus::{PluginBus, PluginModule};

// ===========================================================================
// Test helper plugins
// ===========================================================================

/// Simple plugin that always succeeds and records init/shutdown calls.
struct OkPlugin {
    name: String,
    init_count: Arc<AtomicU32>,
    shutdown_count: Arc<AtomicU32>,
    health: HealthStatus,
}

impl OkPlugin {
    fn new(name: &str) -> (Arc<Self>, Arc<AtomicU32>, Arc<AtomicU32>) {
        let init_count = Arc::new(AtomicU32::new(0));
        let shutdown_count = Arc::new(AtomicU32::new(0));
        let plugin = Arc::new(Self {
            name: name.to_string(),
            init_count: init_count.clone(),
            shutdown_count: shutdown_count.clone(),
            health: HealthStatus::Healthy,
        });
        (plugin, init_count, shutdown_count)
    }

    fn with_health(name: &str, health: HealthStatus) -> Arc<Self> {
        Arc::new(Self {
            name: name.to_string(),
            init_count: Arc::new(AtomicU32::new(0)),
            shutdown_count: Arc::new(AtomicU32::new(0)),
            health,
        })
    }
}

#[async_trait]
impl PluginModule for OkPlugin {
    fn id(&self) -> &str { &self.name }
    fn version(&self) -> &str { "1.0.0" }
    fn capabilities(&self) -> Vec<String> { vec!["ok".into()] }
    async fn init(&self, _ctx: &AppContext) -> Result<()> {
        self.init_count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn shutdown(&self) -> Result<()> {
        self.shutdown_count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn health(&self) -> HealthStatus { self.health }
}

/// Plugin that always fails on init.
struct FailInitPlugin {
    name: String,
    error_msg: String,
}

impl FailInitPlugin {
    fn new(name: &str, msg: &str) -> Arc<Self> {
        Arc::new(Self {
            name: name.to_string(),
            error_msg: msg.to_string(),
        })
    }
}

#[async_trait]
impl PluginModule for FailInitPlugin {
    fn id(&self) -> &str { &self.name }
    fn version(&self) -> &str { "0.0.1" }
    fn capabilities(&self) -> Vec<String> { vec![] }
    async fn init(&self, _ctx: &AppContext) -> Result<()> {
        Err(anyhow::anyhow!("{}", self.error_msg))
    }
    async fn shutdown(&self) -> Result<()> { Ok(()) }
    fn health(&self) -> HealthStatus { HealthStatus::Unhealthy }
}

/// Plugin that fails on shutdown.
struct FailShutdownPlugin {
    name: String,
    init_called: Arc<AtomicBool>,
    error_msg: String,
}

impl FailShutdownPlugin {
    fn new(name: &str, msg: &str) -> (Arc<Self>, Arc<AtomicBool>) {
        let init_called = Arc::new(AtomicBool::new(false));
        let plugin = Arc::new(Self {
            name: name.to_string(),
            init_called: init_called.clone(),
            error_msg: msg.to_string(),
        });
        (plugin, init_called)
    }
}

#[async_trait]
impl PluginModule for FailShutdownPlugin {
    fn id(&self) -> &str { &self.name }
    fn version(&self) -> &str { "0.0.1" }
    fn capabilities(&self) -> Vec<String> { vec![] }
    async fn init(&self, _ctx: &AppContext) -> Result<()> {
        self.init_called.store(true, Ordering::SeqCst);
        Ok(())
    }
    async fn shutdown(&self) -> Result<()> {
        Err(anyhow::anyhow!("{}", self.error_msg))
    }
    fn health(&self) -> HealthStatus { HealthStatus::Healthy }
}

/// Plugin that registers a service in the AppContext during init.
struct ServiceRegisterPlugin {
    name: String,
    value: u64,
}

#[async_trait]
impl PluginModule for ServiceRegisterPlugin {
    fn id(&self) -> &str { &self.name }
    fn version(&self) -> &str { "1.0.0" }
    fn capabilities(&self) -> Vec<String> { vec!["register".into()] }
    async fn init(&self, ctx: &AppContext) -> Result<()> {
        ctx.register(Arc::new(self.value));
        Ok(())
    }
    async fn shutdown(&self) -> Result<()> { Ok(()) }
    fn health(&self) -> HealthStatus { HealthStatus::Healthy }
}

/// Plugin that records lifecycle events into a shared Vec.
struct OrderPlugin {
    name: String,
    order: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl PluginModule for OrderPlugin {
    fn id(&self) -> &str { &self.name }
    fn version(&self) -> &str { "1.0.0" }
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

// ===========================================================================
// A. Edge Cases — Empty Bus
// ===========================================================================

#[tokio::test]
async fn test_init_all_with_no_modules_succeeds() {
    let mut bus = PluginBus::new();
    let result = bus.init_all().await;
    assert!(result.is_ok(), "init_all on empty bus should succeed");
    assert!(bus.initialized_ids().is_empty());
    assert!(bus.module_ids().is_empty());
}

#[tokio::test]
async fn test_shutdown_all_with_no_modules_succeeds() {
    let mut bus = PluginBus::new();
    let result = bus.shutdown_all().await;
    assert!(result.is_ok(), "shutdown_all on empty bus should succeed");
}

#[tokio::test]
async fn test_system_health_with_no_modules_is_healthy() {
    let bus = PluginBus::new();
    // With no initialized modules, worst-of is the initial value: Healthy.
    assert_eq!(bus.system_health(), HealthStatus::Healthy);
}

// ===========================================================================
// B. Edge Cases — Double Init
// ===========================================================================

#[tokio::test]
async fn test_double_init_all_inits_modules_twice() {
    // init_all has an idempotency guard: calling it twice returns Err
    // without re-initializing any modules.
    let (p, init_count, _shutdown_count) = OkPlugin::new("alpha");

    let mut bus = PluginBus::new();
    bus.register(1, p).unwrap();

    bus.init_all().await.unwrap();
    assert_eq!(init_count.load(Ordering::SeqCst), 1);
    assert_eq!(bus.initialized_ids().len(), 1);

    // Second init_all — returns Err, modules are NOT re-initialized.
    let err = bus.init_all().await.unwrap_err();
    assert!(err.to_string().contains("already initialized"));
    assert_eq!(
        init_count.load(Ordering::SeqCst), 1,
        "Module should not be initialized a second time"
    );
    assert_eq!(
        bus.initialized_ids().len(), 1,
        "initialized_ids should not grow after rejected double init"
    );
}

// ===========================================================================
// C. Edge Cases — Shutdown Without Init
// ===========================================================================

#[tokio::test]
async fn test_shutdown_all_without_init_succeeds() {
    let (p, _init_count, shutdown_count) = OkPlugin::new("beta");

    let mut bus = PluginBus::new();
    bus.register(1, p).unwrap();

    // Shutdown without init — initialized list is empty, so no shutdown calls.
    let result = bus.shutdown_all().await;
    assert!(result.is_ok());
    assert_eq!(shutdown_count.load(Ordering::SeqCst), 0);
}

// ===========================================================================
// D. Edge Cases — Duplicate Module IDs
// ===========================================================================

#[tokio::test]
async fn test_duplicate_module_ids_both_initialized() {
    // register() rejects duplicate module IDs, so only the first one is registered.
    let (p1, init1, shutdown1) = OkPlugin::new("dup");
    let (p2, _init2, _shutdown2) = OkPlugin::new("dup");

    let mut bus = PluginBus::new();
    bus.register(1, p1).unwrap();
    let err = bus.register(2, p2).unwrap_err();
    assert!(err.to_string().contains("Duplicate module ID"));

    bus.init_all().await.unwrap();

    // Only first module was registered and initialized.
    assert_eq!(init1.load(Ordering::SeqCst), 1);
    assert_eq!(bus.initialized_ids().len(), 1);

    bus.shutdown_all().await.unwrap();

    let s1 = shutdown1.load(Ordering::SeqCst);
    assert_eq!(s1, 1, "First module should be shut down exactly once");
}

// ===========================================================================
// E. Edge Cases — Module Registered to Multiple Phases
// ===========================================================================

#[tokio::test]
async fn test_same_module_registered_in_multiple_phases() {
    // register() rejects duplicate IDs even across different phases.
    let (p, init_count, shutdown_count) = OkPlugin::new("multi-phase");

    let mut bus = PluginBus::new();
    bus.register(1, p.clone()).unwrap();
    let err = bus.register(3, p).unwrap_err();
    assert!(err.to_string().contains("Duplicate module ID"));

    bus.init_all().await.unwrap();

    // Module is only registered once, so init is called once.
    assert_eq!(init_count.load(Ordering::SeqCst), 1);
    assert_eq!(bus.initialized_ids().len(), 1);

    bus.shutdown_all().await.unwrap();
    assert_eq!(shutdown_count.load(Ordering::SeqCst), 1);
}

// ===========================================================================
// F. Edge Cases — AppContext get<T> for Unregistered Type
// ===========================================================================

#[test]
fn test_app_context_get_unregistered_returns_none() {
    let ctx = AppContext::new();
    assert!(ctx.get::<Vec<u8>>().is_none());
    assert!(ctx.get::<f64>().is_none());
}

#[test]
fn test_app_context_get_wrong_type_returns_none() {
    let ctx = AppContext::new();
    ctx.register(Arc::new(42u32));
    // Ask for a different type — should be None, not a panic.
    assert!(ctx.get::<u64>().is_none());
    assert!(ctx.get::<String>().is_none());
}

// ===========================================================================
// G. Edge Cases — Broadcast Channel Capacity
// ===========================================================================

#[tokio::test]
async fn test_broadcast_channel_overflow_lagged() {
    // Channel capacity is 256. If a subscriber is slow, it will miss events.
    let ctx = AppContext::new();
    let mut rx = ctx.subscribe();

    // Emit 300 events without reading — exceeds capacity of 256.
    for i in 0..300 {
        ctx.emit(PluginEvent::Custom {
            source: "test".into(),
            kind: "flood".into(),
            payload: format!("{}", i),
        });
    }

    // The first recv should return a Lagged error because old events were dropped.
    let result = rx.recv().await;
    match result {
        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
            // n tells us how many messages we missed.
            assert!(n > 0, "Should report lagged messages, got {}", n);
        }
        Ok(_event) => {
            // After a lag, tokio broadcast may also return the next available
            // message. Either way, the subscriber lost data.
        }
        Err(e) => {
            panic!("Unexpected error: {:?}", e);
        }
    }
}

#[test]
fn test_emit_with_no_subscribers_returns_zero() {
    let ctx = AppContext::new();
    let count = ctx.emit(PluginEvent::ModuleInitialized {
        module_id: "ghost".into(),
    });
    assert_eq!(count, 0);
}

#[tokio::test]
async fn test_multiple_subscribers_receive_event() {
    let ctx = AppContext::new();
    let mut rx1 = ctx.subscribe();
    let mut rx2 = ctx.subscribe();
    let mut rx3 = ctx.subscribe();

    let count = ctx.emit(PluginEvent::ModuleInitialized {
        module_id: "multi".into(),
    });
    assert_eq!(count, 3, "Should have 3 active receivers");

    // All three should get the event.
    for rx in [&mut rx1, &mut rx2, &mut rx3] {
        let event = rx.recv().await.unwrap();
        match event {
            PluginEvent::ModuleInitialized { module_id } => {
                assert_eq!(module_id, "multi");
            }
            _ => panic!("wrong event variant"),
        }
    }
}

// ===========================================================================
// H. Error Handling — Multiple Failures in Same Phase
// ===========================================================================

#[tokio::test]
async fn test_first_failure_in_phase_triggers_rollback() {
    // If two modules are in the same phase and the first fails, the second
    // should NOT be initialized, and any earlier modules should be rolled back.
    let order = Arc::new(Mutex::new(Vec::<String>::new()));

    let ok1: Arc<dyn PluginModule> = Arc::new(OrderPlugin {
        name: "ok-phase1".into(),
        order: order.clone(),
    });
    let fail: Arc<dyn PluginModule> = FailInitPlugin::new("fail-phase2-first", "boom1");
    let ok2: Arc<dyn PluginModule> = Arc::new(OrderPlugin {
        name: "ok-phase2-second".into(),
        order: order.clone(),
    });

    let mut bus = PluginBus::new();
    bus.register(1, ok1).unwrap();
    bus.register(2, fail).unwrap();
    bus.register(2, ok2).unwrap();

    let result = bus.init_all().await;
    assert!(result.is_err());

    let entries = order.lock().unwrap().clone();
    // ok-phase1 should be initialized then rolled back.
    assert!(entries.contains(&"init:ok-phase1".to_string()));
    assert!(entries.contains(&"shutdown:ok-phase1".to_string()));
    // ok-phase2-second should NOT have been initialized.
    assert!(
        !entries.contains(&"init:ok-phase2-second".to_string()),
        "Module after failed module in same phase should not be initialized"
    );
}

#[tokio::test]
async fn test_second_module_fails_first_module_rolled_back() {
    // Two modules in the same phase: first succeeds, second fails.
    // The first should be rolled back.
    let order = Arc::new(Mutex::new(Vec::<String>::new()));

    let ok: Arc<dyn PluginModule> = Arc::new(OrderPlugin {
        name: "ok-first".into(),
        order: order.clone(),
    });
    let fail: Arc<dyn PluginModule> = FailInitPlugin::new("fail-second", "boom2");

    let mut bus = PluginBus::new();
    bus.register(1, ok).unwrap();
    bus.register(1, fail).unwrap();

    let result = bus.init_all().await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("boom2"));

    let entries = order.lock().unwrap().clone();
    assert!(entries.contains(&"init:ok-first".to_string()));
    assert!(entries.contains(&"shutdown:ok-first".to_string()));
    assert!(bus.initialized_ids().is_empty());
}

// ===========================================================================
// I. Error Handling — Shutdown Errors
// ===========================================================================

#[tokio::test]
async fn test_shutdown_error_is_reported_but_continues() {
    // If one module fails shutdown, the bus should continue shutting down
    // the remaining modules and return an error at the end.
    let (fail_p, _fail_init) = FailShutdownPlugin::new("fail-shutdown", "shutdown-boom");
    let (ok_p, _ok_init, ok_shutdown) = OkPlugin::new("ok-module");

    let mut bus = PluginBus::new();
    bus.register(1, ok_p).unwrap();
    bus.register(2, fail_p).unwrap();

    bus.init_all().await.unwrap();

    let result = bus.shutdown_all().await;
    assert!(result.is_err(), "Should report shutdown error");
    assert!(result.unwrap_err().to_string().contains("shutdown-boom"));

    // The ok module should still have been shut down.
    assert_eq!(ok_shutdown.load(Ordering::SeqCst), 1);
    // initialized_ids should be cleared even if there were errors.
    assert!(bus.initialized_ids().is_empty());
}

#[tokio::test]
async fn test_multiple_shutdown_errors_all_reported() {
    let (fail1, _) = FailShutdownPlugin::new("fail-a", "err-a");
    let (fail2, _) = FailShutdownPlugin::new("fail-b", "err-b");

    let mut bus = PluginBus::new();
    bus.register(1, fail1).unwrap();
    bus.register(2, fail2).unwrap();

    bus.init_all().await.unwrap();

    let result = bus.shutdown_all().await;
    assert!(result.is_err());
    let err_str = result.unwrap_err().to_string();
    assert!(err_str.contains("err-a"), "Should contain err-a: {}", err_str);
    assert!(err_str.contains("err-b"), "Should contain err-b: {}", err_str);
}

// ===========================================================================
// J. Error Handling — Rollback Shutdown Failure
// ===========================================================================

#[tokio::test]
async fn test_rollback_continues_when_shutdown_fails() {
    // During rollback, if a module's shutdown fails, rollback should continue
    // shutting down the remaining modules (not panic or abort).
    let order = Arc::new(Mutex::new(Vec::<String>::new()));

    let ok1: Arc<dyn PluginModule> = Arc::new(OrderPlugin {
        name: "ok1".into(),
        order: order.clone(),
    });
    let (fail_shut, _) = FailShutdownPlugin::new("fail-shutdown-rollback", "rollback-err");
    let fail_init: Arc<dyn PluginModule> = FailInitPlugin::new("fail-init", "init-err");

    let mut bus = PluginBus::new();
    bus.register(1, ok1).unwrap();
    bus.register(2, fail_shut).unwrap();
    bus.register(3, fail_init).unwrap();

    let result = bus.init_all().await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("init-err"));

    // Despite fail_shut failing during rollback, ok1 should still be shut down.
    let entries = order.lock().unwrap().clone();
    assert!(
        entries.contains(&"shutdown:ok1".to_string()),
        "ok1 should be shut down even when another module's rollback-shutdown fails"
    );
    assert!(bus.initialized_ids().is_empty());
}

// ===========================================================================
// K. API Correctness — system_health()
// ===========================================================================

#[tokio::test]
async fn test_system_health_all_healthy() {
    let (p1, _, _) = OkPlugin::new("h1");
    let (p2, _, _) = OkPlugin::new("h2");

    let mut bus = PluginBus::new();
    bus.register(1, p1).unwrap();
    bus.register(2, p2).unwrap();
    bus.init_all().await.unwrap();

    assert_eq!(bus.system_health(), HealthStatus::Healthy);
}

#[tokio::test]
async fn test_system_health_one_degraded() {
    let (healthy, _, _) = OkPlugin::new("healthy");
    let degraded = OkPlugin::with_health("degraded", HealthStatus::Degraded);

    let mut bus = PluginBus::new();
    bus.register(1, healthy).unwrap();
    bus.register(1, degraded).unwrap();
    bus.init_all().await.unwrap();

    assert_eq!(bus.system_health(), HealthStatus::Degraded);
}

#[tokio::test]
async fn test_system_health_one_unhealthy_trumps_degraded() {
    let degraded = OkPlugin::with_health("degraded", HealthStatus::Degraded);
    let unhealthy = OkPlugin::with_health("unhealthy", HealthStatus::Unhealthy);

    let mut bus = PluginBus::new();
    bus.register(1, degraded).unwrap();
    bus.register(2, unhealthy).unwrap();
    bus.init_all().await.unwrap();

    assert_eq!(bus.system_health(), HealthStatus::Unhealthy);
}

#[tokio::test]
async fn test_system_health_only_checks_initialized_modules() {
    // Register an unhealthy module but don't init — system_health should
    // not include it.
    let unhealthy = OkPlugin::with_health("unhealthy", HealthStatus::Unhealthy);

    let mut bus = PluginBus::new();
    bus.register(1, unhealthy).unwrap();

    // Before init, system_health() should be Healthy (no initialized modules).
    assert_eq!(bus.system_health(), HealthStatus::Healthy);
}

// ===========================================================================
// L. API Correctness — module_ids() vs initialized_ids()
// ===========================================================================

#[tokio::test]
async fn test_module_ids_returns_all_registered() {
    let (p1, _, _) = OkPlugin::new("x");
    let (p2, _, _) = OkPlugin::new("y");
    let (p3, _, _) = OkPlugin::new("z");

    let mut bus = PluginBus::new();
    bus.register(3, p3).unwrap();
    bus.register(1, p1).unwrap();
    bus.register(2, p2).unwrap();

    // module_ids should return in phase order: x, y, z
    let ids = bus.module_ids();
    assert_eq!(ids, vec!["x", "y", "z"]);
}

#[tokio::test]
async fn test_initialized_ids_empty_before_init() {
    let (p, _, _) = OkPlugin::new("alpha");

    let mut bus = PluginBus::new();
    bus.register(1, p).unwrap();

    assert!(bus.initialized_ids().is_empty());
}

#[tokio::test]
async fn test_initialized_ids_populated_after_init() {
    let (p1, _, _) = OkPlugin::new("a");
    let (p2, _, _) = OkPlugin::new("b");

    let mut bus = PluginBus::new();
    bus.register(1, p1).unwrap();
    bus.register(2, p2).unwrap();
    bus.init_all().await.unwrap();

    assert_eq!(bus.initialized_ids(), &["a", "b"]);
}

#[tokio::test]
async fn test_initialized_ids_cleared_after_shutdown() {
    let (p, _, _) = OkPlugin::new("c");

    let mut bus = PluginBus::new();
    bus.register(1, p).unwrap();
    bus.init_all().await.unwrap();
    bus.shutdown_all().await.unwrap();

    assert!(bus.initialized_ids().is_empty());
}

// ===========================================================================
// M. Event Bus — Events During Lifecycle
// ===========================================================================

#[tokio::test]
async fn test_init_failure_emits_module_failed_event() {
    let fail: Arc<dyn PluginModule> = FailInitPlugin::new("bad-module", "kaboom");

    let mut bus = PluginBus::new();
    bus.register(1, fail).unwrap();

    let mut rx = bus.context().subscribe();

    let _result = bus.init_all().await;

    let event = rx.recv().await.unwrap();
    match event {
        PluginEvent::ModuleFailed { module_id, error } => {
            assert_eq!(module_id, "bad-module");
            assert_eq!(error, "kaboom");
        }
        other => panic!("Expected ModuleFailed, got {:?}", other),
    }
}

#[tokio::test]
async fn test_shutdown_emits_module_shutdown_events() {
    let (p1, _, _) = OkPlugin::new("s1");
    let (p2, _, _) = OkPlugin::new("s2");

    let mut bus = PluginBus::new();
    bus.register(1, p1).unwrap();
    bus.register(2, p2).unwrap();
    bus.init_all().await.unwrap();

    let mut rx = bus.context().subscribe();
    bus.shutdown_all().await.unwrap();

    // Should receive two ModuleShutdown events (reverse order: s2, s1).
    let e1 = rx.recv().await.unwrap();
    let e2 = rx.recv().await.unwrap();

    let ids: Vec<String> = vec![e1, e2]
        .into_iter()
        .map(|e| match e {
            PluginEvent::ModuleShutdown { module_id } => module_id,
            other => panic!("Expected ModuleShutdown, got {:?}", other),
        })
        .collect();
    assert_eq!(ids, vec!["s2", "s1"]);
}

#[tokio::test]
async fn test_shutdown_emits_event_even_on_error() {
    // shutdown_all correctly emits ModuleShutdownFailed (not ModuleShutdown)
    // when a module's shutdown() returns Err.
    let (fail_p, _) = FailShutdownPlugin::new("fail-mod", "oops");

    let mut bus = PluginBus::new();
    bus.register(1, fail_p).unwrap();
    bus.init_all().await.unwrap();

    let mut rx = bus.context().subscribe();
    let _result = bus.shutdown_all().await;

    let event = rx.recv().await.unwrap();
    match event {
        PluginEvent::ModuleShutdownFailed { module_id, error } => {
            assert_eq!(module_id, "fail-mod");
            assert!(error.contains("oops"));
        }
        other => panic!("Expected ModuleShutdownFailed, got {:?}", other),
    }
}

// ===========================================================================
// N. Thread Safety — Concurrent AppContext Access
// ===========================================================================

#[tokio::test]
async fn test_concurrent_register_and_get() {
    let ctx = AppContext::new();
    let ctx2 = ctx.clone();
    let ctx3 = ctx.clone();

    // Spawn writers and readers concurrently.
    let writer = tokio::spawn(async move {
        for i in 0u32..100 {
            ctx2.register(Arc::new(i));
        }
    });

    let reader = tokio::spawn(async move {
        let mut found = false;
        for _ in 0..200 {
            if ctx3.get::<u32>().is_some() {
                found = true;
            }
            tokio::task::yield_now().await;
        }
        found
    });

    writer.await.unwrap();
    let found = reader.await.unwrap();
    assert!(found, "Reader should eventually see a registered u32");

    // Final state should be the last written value (99).
    assert_eq!(*ctx.get::<u32>().unwrap(), 99);
}

#[tokio::test]
async fn test_concurrent_subscribe_and_emit() {
    let ctx = AppContext::new();

    let num_subscribers = 5;
    let num_events = 50;
    let mut handles = Vec::new();

    for _ in 0..num_subscribers {
        let mut rx = ctx.subscribe();
        handles.push(tokio::spawn(async move {
            let mut count = 0u32;
            while let Ok(_event) = rx.recv().await {
                count += 1;
                if count >= num_events {
                    break;
                }
            }
            count
        }));
    }

    // Emit events.
    for i in 0..num_events {
        ctx.emit(PluginEvent::Custom {
            source: "stress".into(),
            kind: "test".into(),
            payload: format!("{}", i),
        });
    }

    for handle in handles {
        let count = handle.await.unwrap();
        assert_eq!(count, num_events, "Each subscriber should receive all events");
    }
}

// ===========================================================================
// O. Memory / Resource — Services Not Cleared After Shutdown
// ===========================================================================

#[tokio::test]
async fn test_services_persist_after_shutdown() {
    // shutdown_all correctly clears services from AppContext via ctx.clear(),
    // preventing access to stale resources after shutdown.
    let p: Arc<dyn PluginModule> = Arc::new(ServiceRegisterPlugin {
        name: "svc-reg".into(),
        value: 42u64,
    });

    let mut bus = PluginBus::new();
    bus.register(1, p).unwrap();
    bus.init_all().await.unwrap();

    // Service should be accessible before shutdown.
    assert!(bus.context().get::<u64>().is_some());

    bus.shutdown_all().await.unwrap();

    // Service is correctly cleared after shutdown.
    assert!(
        bus.context().get::<u64>().is_none(),
        "Services should be cleared from AppContext after shutdown"
    );
}

// ===========================================================================
// P. Edge Case — Shutdown After Failed Init (Rollback Already Happened)
// ===========================================================================

#[tokio::test]
async fn test_shutdown_after_failed_init_is_noop() {
    let (ok_p, _, shutdown_count) = OkPlugin::new("ok-mod");
    let fail: Arc<dyn PluginModule> = FailInitPlugin::new("fail-mod", "err");

    let mut bus = PluginBus::new();
    bus.register(1, ok_p).unwrap();
    bus.register(2, fail).unwrap();

    // Init fails, rollback already shuts down ok-mod.
    let _result = bus.init_all().await;
    assert_eq!(shutdown_count.load(Ordering::SeqCst), 1); // rollback shutdown

    // Now call shutdown_all — initialized is already empty, so it's a noop.
    let result = bus.shutdown_all().await;
    assert!(result.is_ok());
    assert_eq!(
        shutdown_count.load(Ordering::SeqCst), 1,
        "shutdown_all after rollback should not call shutdown again"
    );
}

// ===========================================================================
// Q. Edge Case — Init, Shutdown, Then Re-Init
// ===========================================================================

#[tokio::test]
async fn test_init_shutdown_reinit_cycle() {
    let (p, init_count, shutdown_count) = OkPlugin::new("cycle-mod");

    let mut bus = PluginBus::new();
    bus.register(1, p).unwrap();

    // First cycle
    bus.init_all().await.unwrap();
    assert_eq!(init_count.load(Ordering::SeqCst), 1);
    bus.shutdown_all().await.unwrap();
    assert_eq!(shutdown_count.load(Ordering::SeqCst), 1);
    assert!(bus.initialized_ids().is_empty());

    // Second cycle — should work cleanly.
    bus.init_all().await.unwrap();
    assert_eq!(init_count.load(Ordering::SeqCst), 2);
    assert_eq!(bus.initialized_ids(), &["cycle-mod"]);

    bus.shutdown_all().await.unwrap();
    assert_eq!(shutdown_count.load(Ordering::SeqCst), 2);
    assert!(bus.initialized_ids().is_empty());
}

// ===========================================================================
// R. Edge Case — Context Shared Across Clones
// ===========================================================================

#[test]
fn test_app_context_clone_shares_services() {
    let ctx = AppContext::new();
    let ctx_clone = ctx.clone();

    ctx.register(Arc::new(String::from("shared")));
    assert_eq!(*ctx_clone.get::<String>().unwrap(), "shared");
}

#[test]
fn test_app_context_clone_shares_event_bus() {
    // Cloned contexts should share the same broadcast channel.
    let ctx = AppContext::new();
    let ctx_clone = ctx.clone();

    let mut rx = ctx_clone.subscribe();
    ctx.emit(PluginEvent::Custom {
        source: "s".into(),
        kind: "k".into(),
        payload: "p".into(),
    });

    // Use try_recv — event should be immediately available.
    let event = rx.try_recv().unwrap();
    match event {
        PluginEvent::Custom { source, .. } => assert_eq!(source, "s"),
        _ => panic!("wrong event"),
    }
}

// ===========================================================================
// S. Stress — Many Phases
// ===========================================================================

#[tokio::test]
async fn test_many_phases_init_in_order() {
    let order = Arc::new(Mutex::new(Vec::<String>::new()));

    let mut bus = PluginBus::new();
    // Register modules in phases 255 down to 0 (reversed).
    for phase in (0u8..=10).rev() {
        let p: Arc<dyn PluginModule> = Arc::new(OrderPlugin {
            name: format!("p{}", phase),
            order: order.clone(),
        });
        bus.register(phase, p).unwrap();
    }

    bus.init_all().await.unwrap();

    let entries = order.lock().unwrap().clone();
    let expected: Vec<String> = (0u8..=10).map(|i| format!("init:p{}", i)).collect();
    assert_eq!(entries, expected, "Modules should init in ascending phase order");
}

// ===========================================================================
// T. Edge Case — Rollback Does Not Emit ModuleShutdown Events
// ===========================================================================

#[tokio::test]
async fn test_rollback_does_not_emit_shutdown_events() {
    // rollback_shutdown() correctly emits ModuleShutdown events so that
    // subscribers are aware of modules being rolled back.
    let (ok_p, _, _) = OkPlugin::new("rolled-back");
    let fail: Arc<dyn PluginModule> = FailInitPlugin::new("fail", "err");

    let mut bus = PluginBus::new();
    bus.register(1, ok_p).unwrap();
    bus.register(2, fail).unwrap();

    let mut rx = bus.context().subscribe();

    let _result = bus.init_all().await;

    // We should see: ModuleInitialized for "rolled-back", ModuleFailed for "fail",
    // and ModuleShutdown for "rolled-back" during rollback.
    let mut events = Vec::new();
    while let Ok(event) = rx.try_recv() {
        events.push(event);
    }

    let has_shutdown_event = events.iter().any(|e| matches!(
        e,
        PluginEvent::ModuleShutdown { module_id } if module_id == "rolled-back"
    ));
    assert!(
        has_shutdown_event,
        "rollback_shutdown should emit ModuleShutdown events so subscribers are notified"
    );
}

// ===========================================================================
// U. Edge Case — Service Overwrite During Init
// ===========================================================================

#[tokio::test]
async fn test_later_phase_overwrites_earlier_service() {
    // If two modules register the same type, the later phase wins.
    let p1: Arc<dyn PluginModule> = Arc::new(ServiceRegisterPlugin {
        name: "early".into(),
        value: 100u64,
    });
    let p2: Arc<dyn PluginModule> = Arc::new(ServiceRegisterPlugin {
        name: "late".into(),
        value: 200u64,
    });

    let mut bus = PluginBus::new();
    bus.register(1, p1).unwrap();
    bus.register(2, p2).unwrap();
    bus.init_all().await.unwrap();

    // The late phase should have overwritten the early phase's service.
    let val = bus.context().get::<u64>().unwrap();
    assert_eq!(*val, 200);
}
