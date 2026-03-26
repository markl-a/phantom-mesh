//! Integration test: PluginBus manages 3 real module adapters through
//! init -> service registration -> cross-module access -> shutdown lifecycle.

use std::sync::Arc;

use phantom_mesh::app_context::AppContext;
use phantom_mesh::circuit_breaker::{BreakerConfig, CircuitBreakerPlugin, ProviderCircuitBreaker};
use phantom_mesh::health_check::{HealthCheckPlugin, HealthStatus};
use phantom_mesh::plugin_bus::{PluginBus, PluginModule};
use phantom_mesh::trajectory::{TrajectoryLogger, TrajectoryPlugin};

#[tokio::test]
async fn test_full_plugin_bus_lifecycle() {
    let dir = std::env::temp_dir().join(format!(
        "plugin-bus-integration-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();

    let mut bus = PluginBus::new();

    // Phase 1: infrastructure
    bus.register(1, Arc::new(HealthCheckPlugin::new()));

    // Phase 2: data layer
    bus.register(
        2,
        Arc::new(TrajectoryPlugin::new(
            dir.join("traj.db").to_str().unwrap(),
        )),
    );

    // Phase 4: engine
    bus.register(
        4,
        Arc::new(CircuitBreakerPlugin::new(BreakerConfig::default())),
    );

    // Init all
    bus.init_all().await.unwrap();

    assert_eq!(bus.initialized_ids().len(), 3);
    assert_eq!(bus.system_health(), HealthStatus::Healthy);

    // Cross-module service access
    let ctx = bus.context();

    let traj = ctx.get::<TrajectoryLogger>().unwrap();
    assert_eq!(traj.count_for_hand("nonexistent").unwrap(), 0);

    let breaker = ctx.get::<ProviderCircuitBreaker>().unwrap();
    assert!(breaker.is_available("test-provider"));

    let health = ctx.get::<HealthCheckPlugin>().unwrap();
    assert_eq!(health.id(), "health-check");

    // Shutdown all
    bus.shutdown_all().await.unwrap();
    assert!(bus.initialized_ids().is_empty());
}

#[tokio::test]
async fn test_plugin_bus_event_stream() {
    let mut bus = PluginBus::new();
    bus.register(1, Arc::new(HealthCheckPlugin::new()));

    let mut rx = bus.context().subscribe();

    bus.init_all().await.unwrap();

    let event = rx.recv().await.unwrap();
    match event {
        phantom_mesh::app_context::PluginEvent::ModuleInitialized { module_id } => {
            assert_eq!(module_id, "health-check");
        }
        _ => panic!("expected ModuleInitialized event"),
    }
}

#[tokio::test]
async fn test_plugin_bus_init_rollback_integration() {
    /// A plugin that always fails init.
    struct FailPlugin;
    #[async_trait::async_trait]
    impl PluginModule for FailPlugin {
        fn id(&self) -> &str { "always-fail" }
        fn version(&self) -> &str { "0.0.0" }
        fn capabilities(&self) -> Vec<String> { vec![] }
        async fn init(&self, _ctx: &AppContext) -> anyhow::Result<()> {
            Err(anyhow::anyhow!("intentional failure"))
        }
        async fn shutdown(&self) -> anyhow::Result<()> { Ok(()) }
        fn health(&self) -> HealthStatus { HealthStatus::Unhealthy }
    }

    let mut bus = PluginBus::new();
    bus.register(1, Arc::new(HealthCheckPlugin::new()));
    bus.register(2, Arc::new(FailPlugin));

    let result = bus.init_all().await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("intentional failure"));

    // HealthCheckPlugin should have been rolled back
    assert!(bus.initialized_ids().is_empty());
}
