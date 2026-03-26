# Plugin Bus + AppContext Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a unified Plugin Bus + AppContext service locator that manages module lifecycle with phased initialization, event bus, and shutdown rollback — proving the pattern by converting 3 existing modules.

**Architecture:** Create `AppContext` (type-keyed service locator with broadcast event bus) and `PluginBus` (phase-ordered lifecycle manager implementing `PluginModule` trait). Wrap 3 existing modules (`health_check`, `trajectory`, `circuit_breaker`) as `PluginModule` adapters to prove the pattern. Partially wire into `main.rs` daemon startup alongside existing code (non-breaking).

**Tech Stack:** Rust, tokio (broadcast channel), async-trait, anyhow, std::any::{Any, TypeId}

**Spec:** `docs/superpowers/specs/2026-03-21-phantom-mesh-app-platform-design.md` — Sections 1.1–1.7

---

## File Structure

| Action | Path | Responsibility |
|--------|------|---------------|
| Create | `src/app_context.rs` | AppContext service locator + PluginEvent enum (~120 lines) |
| Create | `src/plugin_bus.rs` | PluginModule trait + PluginBus lifecycle manager (~200 lines) |
| Modify | `src/lib.rs:1-108` | Add `pub mod app_context; pub mod plugin_bus;` |
| Modify | `src/health_check.rs` | Add `HealthCheckPlugin` adapter (~40 lines at end) |
| Modify | `src/trajectory.rs` | Add `TrajectoryPlugin` adapter (~50 lines at end) |
| Modify | `src/circuit_breaker.rs` | Add `CircuitBreakerPlugin` adapter (~40 lines at end) |
| Create | `tests/plugin_bus_integration.rs` | Full lifecycle integration test (~150 lines) |

---

## Task 1: AppContext — Service Locator + Event Bus

**Files:**
- Create: `src/app_context.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Write failing test for register/get roundtrip**

Add to `src/app_context.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_and_get_roundtrip() {
        let ctx = AppContext::new();
        ctx.register(Arc::new(42u32));
        let retrieved = ctx.get::<u32>().unwrap();
        assert_eq!(*retrieved, 42);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib app_context::tests::test_register_and_get_roundtrip`
Expected: FAIL — module `app_context` does not exist

- [ ] **Step 3: Implement AppContext with PluginEvent**

Write `src/app_context.rs`:

```rust
//! AppContext — type-keyed service locator with broadcast event bus.
//!
//! All PluginModule implementations receive an `AppContext` during `init()`.
//! They register services (via `register<T>()`) and retrieve dependencies
//! (via `get<T>()`). Inter-module communication uses `emit()` / `subscribe()`.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use serde::Serialize;
use tokio::sync::broadcast;

use crate::health_check::HealthStatus;

// ---------------------------------------------------------------------------
// PluginEvent
// ---------------------------------------------------------------------------

/// Events emitted on the Plugin Bus event channel.
#[derive(Debug, Clone, Serialize)]
pub enum PluginEvent {
    /// A module completed initialization successfully.
    ModuleInitialized { module_id: String },
    /// A module has been shut down.
    ModuleShutdown { module_id: String },
    /// A module failed during initialization.
    ModuleFailed { module_id: String, error: String },
    /// A module's health status changed.
    HealthChanged {
        module_id: String,
        status: HealthStatus,
    },
    /// Custom event from any module.
    Custom {
        source: String,
        kind: String,
        payload: String,
    },
}

// ---------------------------------------------------------------------------
// AppContext
// ---------------------------------------------------------------------------

/// Service locator shared across all plugin modules.
///
/// Follows the DI pattern established by `AgentRuntime` (`Option<Arc<T>>`
/// setters), generalized to a type-keyed HashMap so any module can register
/// and retrieve services without compile-time coupling.
///
/// Thread-safe: inner `RwLock` allows concurrent reads, exclusive writes.
/// Clone: shares the same underlying storage (Arc).
#[derive(Clone)]
pub struct AppContext {
    services: Arc<RwLock<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>>,
    event_tx: broadcast::Sender<PluginEvent>,
}

impl AppContext {
    /// Create a new, empty context with a broadcast channel (capacity 256).
    pub fn new() -> Self {
        let (event_tx, _) = broadcast::channel(256);
        Self {
            services: Arc::new(RwLock::new(HashMap::new())),
            event_tx,
        }
    }

    /// Register a service. Overwrites any previous registration of the same type.
    pub fn register<T: Send + Sync + 'static>(&self, service: Arc<T>) {
        self.services
            .write()
            .expect("AppContext lock poisoned")
            .insert(TypeId::of::<T>(), service);
    }

    /// Retrieve a previously registered service, or `None`.
    pub fn get<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        self.services
            .read()
            .expect("AppContext lock poisoned")
            .get(&TypeId::of::<T>())
            .and_then(|s| s.clone().downcast::<T>().ok())
    }

    /// Subscribe to the event bus.
    pub fn subscribe(&self) -> broadcast::Receiver<PluginEvent> {
        self.event_tx.subscribe()
    }

    /// Emit an event. Returns the number of active receivers.
    pub fn emit(&self, event: PluginEvent) -> usize {
        self.event_tx.send(event).unwrap_or(0)
    }

    /// Number of registered services (for diagnostics).
    pub fn service_count(&self) -> usize {
        self.services
            .read()
            .expect("AppContext lock poisoned")
            .len()
    }
}

impl Default for AppContext {
    fn default() -> Self {
        Self::new()
    }
}
```

Add to `src/lib.rs` (after existing `pub mod plugin_loader;` line):

```rust
pub mod app_context;
```

> **Note:** `pub mod plugin_bus;` will be added in Task 2 Step 3 when `plugin_bus.rs` is created.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib app_context::tests::test_register_and_get_roundtrip`
Expected: PASS

- [ ] **Step 5: Write remaining unit tests**

Append to the `#[cfg(test)] mod tests` block:

```rust
    #[test]
    fn test_get_returns_none_for_unregistered() {
        let ctx = AppContext::new();
        assert!(ctx.get::<String>().is_none());
    }

    #[test]
    fn test_register_overwrites_previous() {
        let ctx = AppContext::new();
        ctx.register(Arc::new(1u32));
        ctx.register(Arc::new(2u32));
        assert_eq!(*ctx.get::<u32>().unwrap(), 2);
    }

    #[test]
    fn test_multiple_types() {
        let ctx = AppContext::new();
        ctx.register(Arc::new(42u32));
        ctx.register(Arc::new("hello".to_string()));
        assert_eq!(*ctx.get::<u32>().unwrap(), 42);
        assert_eq!(*ctx.get::<String>().unwrap(), "hello");
    }

    #[test]
    fn test_clone_shares_state() {
        let ctx1 = AppContext::new();
        let ctx2 = ctx1.clone();
        ctx1.register(Arc::new(99u32));
        assert_eq!(*ctx2.get::<u32>().unwrap(), 99);
    }

    #[test]
    fn test_service_count() {
        let ctx = AppContext::new();
        assert_eq!(ctx.service_count(), 0);
        ctx.register(Arc::new(1u32));
        assert_eq!(ctx.service_count(), 1);
        ctx.register(Arc::new("s".to_string()));
        assert_eq!(ctx.service_count(), 2);
    }

    #[tokio::test]
    async fn test_emit_and_subscribe() {
        let ctx = AppContext::new();
        let mut rx = ctx.subscribe();
        ctx.emit(PluginEvent::ModuleInitialized {
            module_id: "test".to_string(),
        });
        let event = rx.recv().await.unwrap();
        match event {
            PluginEvent::ModuleInitialized { module_id } => {
                assert_eq!(module_id, "test");
            }
            _ => panic!("unexpected event variant"),
        }
    }

    #[test]
    fn test_emit_returns_zero_with_no_subscribers() {
        let ctx = AppContext::new();
        let count = ctx.emit(PluginEvent::ModuleInitialized {
            module_id: "x".to_string(),
        });
        assert_eq!(count, 0);
    }
```

- [ ] **Step 6: Run all AppContext tests**

Run: `cargo test --lib app_context::tests`
Expected: 8 tests PASS

- [ ] **Step 7: Commit**

```bash
git add src/app_context.rs src/lib.rs
git commit -m "feat: add AppContext service locator with type-keyed registry and event bus"
```

---

## Task 2: PluginModule Trait + PluginBus

**Files:**
- Create: `src/plugin_bus.rs`

- [ ] **Step 1: Write failing test for PluginBus register + init**

Add to `src/plugin_bus.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal test plugin
    struct DummyPlugin {
        id: String,
        init_called: Arc<std::sync::atomic::AtomicBool>,
        shutdown_called: Arc<std::sync::atomic::AtomicBool>,
    }

    impl DummyPlugin {
        fn new(id: &str) -> Self {
            Self {
                id: id.to_string(),
                init_called: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                shutdown_called: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            }
        }
    }

    #[async_trait]
    impl PluginModule for DummyPlugin {
        fn id(&self) -> &str { &self.id }
        fn version(&self) -> &str { "0.1.0" }
        fn capabilities(&self) -> Vec<String> { vec!["test".into()] }
        async fn init(&self, _ctx: &AppContext) -> anyhow::Result<()> {
            self.init_called.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
        async fn shutdown(&self) -> anyhow::Result<()> {
            self.shutdown_called.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
        fn health(&self) -> HealthStatus { HealthStatus::Healthy }
    }

    #[tokio::test]
    async fn test_init_all_calls_init_on_all_modules() {
        let p1 = Arc::new(DummyPlugin::new("p1"));
        let p2 = Arc::new(DummyPlugin::new("p2"));

        let mut bus = PluginBus::new();
        bus.register(1, p1.clone());
        bus.register(1, p2.clone());

        bus.init_all().await.unwrap();

        assert!(p1.init_called.load(std::sync::atomic::Ordering::SeqCst));
        assert!(p2.init_called.load(std::sync::atomic::Ordering::SeqCst));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib plugin_bus::tests::test_init_all_calls_init_on_all_modules`
Expected: FAIL — `PluginBus` not found

- [ ] **Step 3: Implement PluginModule trait + PluginBus**

Write `src/plugin_bus.rs`:

```rust
//! Plugin Bus — unified module lifecycle manager with phased initialization.
//!
//! Manages `PluginModule` registration, ordered init (by phase), event emission,
//! and shutdown with rollback on failure.
//!
//! See spec Section 1.1–1.6 for design.

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

// Add `pub mod plugin_bus;` to src/lib.rs (after `pub mod app_context;`).

/// The universal interface for all phantom-mesh modules.
///
/// NOTE: Spec defines `version() -> semver::Version`, but we use `&str` to
/// avoid adding the `semver` dependency. No code currently parses versions.
/// Upgrade to `semver::Version` when version comparison is needed.
///
/// Every module that participates in the Plugin Bus must implement this trait.
/// Modules are initialized in phase order (see `PluginBus::register(phase, …)`)
/// and can register/retrieve services through `AppContext`.
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
    /// Phase → modules registered in that phase.
    phases: BTreeMap<u8, Vec<Arc<dyn PluginModule>>>,
    /// Shared service locator.
    ctx: AppContext,
    /// IDs of successfully initialized modules, in init order.
    initialized: Vec<String>,
}

impl PluginBus {
    /// Create a new bus with an empty AppContext.
    pub fn new() -> Self {
        Self {
            phases: BTreeMap::new(),
            ctx: AppContext::new(),
            initialized: Vec::new(),
        }
    }

    /// Register a module in the given phase (1 = first, 7 = last).
    pub fn register(&mut self, phase: u8, module: Arc<dyn PluginModule>) {
        self.phases.entry(phase).or_default().push(module);
    }

    /// Get a reference to the shared AppContext.
    pub fn context(&self) -> &AppContext {
        &self.ctx
    }

    /// Initialize all modules in phase order.
    ///
    /// On failure, rolls back by calling `shutdown()` on all already-initialized
    /// modules in reverse order.
    pub async fn init_all(&mut self) -> Result<()> {
        for (phase, modules) in &self.phases {
            for module in modules {
                info!(
                    "[PluginBus] Phase {} — initializing '{}'...",
                    phase,
                    module.id()
                );
                match module.init(&self.ctx).await {
                    Ok(()) => {
                        self.initialized.push(module.id().to_string());
                        self.ctx.emit(PluginEvent::ModuleInitialized {
                            module_id: module.id().to_string(),
                        });
                        info!("[PluginBus] '{}' initialized OK", module.id());
                    }
                    Err(e) => {
                        error!(
                            "[PluginBus] '{}' init failed: {} — rolling back",
                            module.id(),
                            e
                        );
                        self.ctx.emit(PluginEvent::ModuleFailed {
                            module_id: module.id().to_string(),
                            error: e.to_string(),
                        });
                        self.rollback_shutdown().await;
                        return Err(anyhow::anyhow!(
                            "Module '{}' failed to initialize: {}",
                            module.id(),
                            e
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    /// Shut down all initialized modules in reverse order.
    pub async fn shutdown_all(&mut self) -> Result<()> {
        let mut errors = Vec::new();
        for module_id in self.initialized.iter().rev() {
            if let Some(module) = self.find_module(module_id) {
                info!("[PluginBus] Shutting down '{}'...", module_id);
                if let Err(e) = module.shutdown().await {
                    error!("[PluginBus] '{}' shutdown error: {}", module_id, e);
                    errors.push(format!("{}: {}", module_id, e));
                }
                self.ctx.emit(PluginEvent::ModuleShutdown {
                    module_id: module_id.clone(),
                });
            }
        }
        self.initialized.clear();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(anyhow::anyhow!("Shutdown errors: {}", errors.join("; ")))
        }
    }

    /// List all registered module IDs.
    pub fn module_ids(&self) -> Vec<String> {
        self.phases
            .values()
            .flat_map(|modules| modules.iter().map(|m| m.id().to_string()))
            .collect()
    }

    /// List IDs of successfully initialized modules.
    pub fn initialized_ids(&self) -> &[String] {
        &self.initialized
    }

    /// Get aggregated health: worst status across all initialized modules.
    pub fn system_health(&self) -> HealthStatus {
        let mut worst = HealthStatus::Healthy;
        for module_id in &self.initialized {
            if let Some(module) = self.find_module(module_id) {
                worst = worst.worse(module.health());
            }
        }
        worst
    }

    // -- internals ------------------------------------------------------------

    async fn rollback_shutdown(&mut self) {
        info!(
            "[PluginBus] Rolling back {} initialized modules...",
            self.initialized.len()
        );
        for module_id in self.initialized.iter().rev() {
            if let Some(module) = self.find_module(module_id) {
                if let Err(e) = module.shutdown().await {
                    error!("[PluginBus] Rollback: '{}' shutdown error: {}", module_id, e);
                }
            }
        }
        self.initialized.clear();
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
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib plugin_bus::tests::test_init_all_calls_init_on_all_modules`
Expected: PASS

- [ ] **Step 5: Write remaining PluginBus unit tests**

Append to `#[cfg(test)] mod tests`:

```rust
    #[tokio::test]
    async fn test_shutdown_all_calls_shutdown_in_reverse() {
        use std::sync::Mutex;
        let order = Arc::new(Mutex::new(Vec::<String>::new()));

        struct OrderedPlugin {
            id: String,
            order: Arc<Mutex<Vec<String>>>,
        }
        #[async_trait]
        impl PluginModule for OrderedPlugin {
            fn id(&self) -> &str { &self.id }
            fn version(&self) -> &str { "0.1.0" }
            fn capabilities(&self) -> Vec<String> { vec![] }
            async fn init(&self, _ctx: &AppContext) -> Result<()> { Ok(()) }
            async fn shutdown(&self) -> Result<()> {
                self.order.lock().unwrap().push(self.id.clone());
                Ok(())
            }
            fn health(&self) -> HealthStatus { HealthStatus::Healthy }
        }

        let mut bus = PluginBus::new();
        bus.register(1, Arc::new(OrderedPlugin { id: "a".into(), order: order.clone() }));
        bus.register(2, Arc::new(OrderedPlugin { id: "b".into(), order: order.clone() }));
        bus.register(3, Arc::new(OrderedPlugin { id: "c".into(), order: order.clone() }));

        bus.init_all().await.unwrap();
        bus.shutdown_all().await.unwrap();

        let shutdown_order = order.lock().unwrap().clone();
        assert_eq!(shutdown_order, vec!["c", "b", "a"]);
    }

    #[tokio::test]
    async fn test_rollback_on_init_failure() {
        use std::sync::Mutex;
        let shutdown_log = Arc::new(Mutex::new(Vec::<String>::new()));

        struct OkPlugin {
            id: String,
            log: Arc<Mutex<Vec<String>>>,
        }
        #[async_trait]
        impl PluginModule for OkPlugin {
            fn id(&self) -> &str { &self.id }
            fn version(&self) -> &str { "0.1.0" }
            fn capabilities(&self) -> Vec<String> { vec![] }
            async fn init(&self, _ctx: &AppContext) -> Result<()> { Ok(()) }
            async fn shutdown(&self) -> Result<()> {
                self.log.lock().unwrap().push(self.id.clone());
                Ok(())
            }
            fn health(&self) -> HealthStatus { HealthStatus::Healthy }
        }

        struct FailPlugin;
        #[async_trait]
        impl PluginModule for FailPlugin {
            fn id(&self) -> &str { "fail" }
            fn version(&self) -> &str { "0.1.0" }
            fn capabilities(&self) -> Vec<String> { vec![] }
            async fn init(&self, _ctx: &AppContext) -> Result<()> {
                Err(anyhow::anyhow!("boom"))
            }
            async fn shutdown(&self) -> Result<()> { Ok(()) }
            fn health(&self) -> HealthStatus { HealthStatus::Unhealthy }
        }

        let mut bus = PluginBus::new();
        bus.register(1, Arc::new(OkPlugin { id: "a".into(), log: shutdown_log.clone() }));
        bus.register(1, Arc::new(OkPlugin { id: "b".into(), log: shutdown_log.clone() }));
        bus.register(2, Arc::new(FailPlugin));

        let result = bus.init_all().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("boom"));

        // a and b should have been rolled back (shutdown called)
        let log = shutdown_log.lock().unwrap().clone();
        assert_eq!(log, vec!["b", "a"]); // reverse order
        assert!(bus.initialized_ids().is_empty());
    }

    #[tokio::test]
    async fn test_phase_ordering() {
        use std::sync::Mutex;
        let init_order = Arc::new(Mutex::new(Vec::<String>::new()));

        struct PhasePlugin {
            id: String,
            order: Arc<Mutex<Vec<String>>>,
        }
        #[async_trait]
        impl PluginModule for PhasePlugin {
            fn id(&self) -> &str { &self.id }
            fn version(&self) -> &str { "0.1.0" }
            fn capabilities(&self) -> Vec<String> { vec![] }
            async fn init(&self, _ctx: &AppContext) -> Result<()> {
                self.order.lock().unwrap().push(self.id.clone());
                Ok(())
            }
            async fn shutdown(&self) -> Result<()> { Ok(()) }
            fn health(&self) -> HealthStatus { HealthStatus::Healthy }
        }

        let mut bus = PluginBus::new();
        // Register out of order
        bus.register(3, Arc::new(PhasePlugin { id: "phase3".into(), order: init_order.clone() }));
        bus.register(1, Arc::new(PhasePlugin { id: "phase1".into(), order: init_order.clone() }));
        bus.register(2, Arc::new(PhasePlugin { id: "phase2".into(), order: init_order.clone() }));

        bus.init_all().await.unwrap();

        let order = init_order.lock().unwrap().clone();
        assert_eq!(order, vec!["phase1", "phase2", "phase3"]);
    }

    #[tokio::test]
    async fn test_system_health_worst_of() {
        struct HealthPlugin {
            id: String,
            status: HealthStatus,
        }
        #[async_trait]
        impl PluginModule for HealthPlugin {
            fn id(&self) -> &str { &self.id }
            fn version(&self) -> &str { "0.1.0" }
            fn capabilities(&self) -> Vec<String> { vec![] }
            async fn init(&self, _ctx: &AppContext) -> Result<()> { Ok(()) }
            async fn shutdown(&self) -> Result<()> { Ok(()) }
            fn health(&self) -> HealthStatus { self.status }
        }

        let mut bus = PluginBus::new();
        bus.register(1, Arc::new(HealthPlugin { id: "ok".into(), status: HealthStatus::Healthy }));
        bus.register(1, Arc::new(HealthPlugin { id: "bad".into(), status: HealthStatus::Degraded }));

        bus.init_all().await.unwrap();
        assert_eq!(bus.system_health(), HealthStatus::Degraded);
    }

    #[tokio::test]
    async fn test_module_ids_and_initialized_ids() {
        let mut bus = PluginBus::new();
        bus.register(1, Arc::new(DummyPlugin::new("a")));
        bus.register(2, Arc::new(DummyPlugin::new("b")));

        assert_eq!(bus.module_ids(), vec!["a", "b"]);
        assert!(bus.initialized_ids().is_empty());

        bus.init_all().await.unwrap();
        assert_eq!(bus.initialized_ids(), &["a", "b"]);
    }

    #[tokio::test]
    async fn test_init_emits_events() {
        let mut bus = PluginBus::new();
        bus.register(1, Arc::new(DummyPlugin::new("x")));

        let mut rx = bus.context().subscribe();
        bus.init_all().await.unwrap();

        let event = rx.recv().await.unwrap();
        match event {
            PluginEvent::ModuleInitialized { module_id } => {
                assert_eq!(module_id, "x");
            }
            _ => panic!("expected ModuleInitialized"),
        }
    }
```

- [ ] **Step 6: Run all PluginBus tests**

Run: `cargo test --lib plugin_bus::tests`
Expected: 7 tests PASS (init_all, shutdown_reverse, rollback, phase_ordering, system_health, ids, events)

- [ ] **Step 7: Commit**

```bash
git add src/plugin_bus.rs
git commit -m "feat: add PluginModule trait and PluginBus lifecycle manager with phase ordering"
```

---

## Task 3: HealthCheckPlugin Adapter

**Files:**
- Modify: `src/health_check.rs` (append adapter at end, before `#[cfg(test)]`)

- [ ] **Step 1: Write failing test**

Add to the existing `#[cfg(test)] mod tests` in `src/health_check.rs`:

```rust
    #[tokio::test]
    async fn test_health_check_plugin_lifecycle() {
        use crate::app_context::AppContext;
        use crate::plugin_bus::PluginModule;

        let plugin = HealthCheckPlugin::new();
        let ctx = AppContext::new();

        assert_eq!(plugin.id(), "health-check");
        assert_eq!(plugin.health(), HealthStatus::Healthy);

        plugin.init(&ctx).await.unwrap();
        // After init, plugin registers itself in AppContext
        assert!(ctx.get::<HealthCheckPlugin>().is_some());

        plugin.shutdown().await.unwrap();
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib health_check::tests::test_health_check_plugin_lifecycle`
Expected: FAIL — `HealthCheckPlugin` not found

- [ ] **Step 3: Implement HealthCheckPlugin adapter**

Add before `#[cfg(test)]` in `src/health_check.rs`:

```rust
// ---------------------------------------------------------------------------
// PluginModule adapter
// ---------------------------------------------------------------------------

use crate::app_context::AppContext;
use crate::plugin_bus::PluginModule;
use async_trait::async_trait;
use std::sync::Arc;

/// Wraps the health check system as a PluginModule.
///
/// On init, registers itself in AppContext so other modules can query
/// system health via `ctx.get::<HealthCheckPlugin>()`.
pub struct HealthCheckPlugin;

impl HealthCheckPlugin {
    pub fn new() -> Self {
        Self
    }

    /// Run all built-in checks and return aggregated SystemHealth.
    pub fn check_all(&self, db_dir: &str, workspace_dir: &str) -> SystemHealth {
        let start = Instant::now();
        let components = vec![
            check_database(db_dir),
            check_disk_space(workspace_dir),
            check_memory_usage(2 * 1024 * 1024 * 1024), // 2 GB threshold
        ];
        let status = components
            .iter()
            .fold(HealthStatus::Healthy, |worst, c| worst.worse(c.status));
        SystemHealth {
            status,
            components,
            uptime_secs: start.elapsed().as_secs(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            checked_at: Utc::now(),
        }
    }
}

#[async_trait]
impl PluginModule for HealthCheckPlugin {
    fn id(&self) -> &str {
        "health-check"
    }
    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }
    fn capabilities(&self) -> Vec<String> {
        vec!["health-monitoring".into()]
    }
    async fn init(&self, ctx: &AppContext) -> anyhow::Result<()> {
        // Register self so other modules can call check_all()
        ctx.register(Arc::new(HealthCheckPlugin::new()));
        Ok(())
    }
    async fn shutdown(&self) -> anyhow::Result<()> {
        Ok(())
    }
    fn health(&self) -> HealthStatus {
        HealthStatus::Healthy
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib health_check::tests::test_health_check_plugin_lifecycle`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/health_check.rs
git commit -m "feat: add HealthCheckPlugin adapter implementing PluginModule trait"
```

---

## Task 4: TrajectoryPlugin Adapter

**Files:**
- Modify: `src/trajectory.rs` (append adapter at end, before `#[cfg(test)]`)

- [ ] **Step 1: Write failing test**

Add to the existing `#[cfg(test)] mod tests` in `src/trajectory.rs`:

```rust
    #[tokio::test]
    async fn test_trajectory_plugin_lifecycle() {
        use crate::app_context::AppContext;
        use crate::plugin_bus::PluginModule;

        let dir = std::env::temp_dir().join(format!("traj-plugin-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("traj.db");

        let plugin = TrajectoryPlugin::new(db_path.to_str().unwrap());
        let ctx = AppContext::new();

        assert_eq!(plugin.id(), "trajectory-logger");
        assert_eq!(plugin.health(), HealthStatus::Healthy);

        plugin.init(&ctx).await.unwrap();
        // After init, TrajectoryLogger is available via AppContext
        let logger = ctx.get::<TrajectoryLogger>().unwrap();
        assert_eq!(logger.count_for_hand("nonexistent").unwrap(), 0);

        plugin.shutdown().await.unwrap();
        assert_eq!(plugin.health(), HealthStatus::Unhealthy);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib trajectory::tests::test_trajectory_plugin_lifecycle`
Expected: FAIL — `TrajectoryPlugin` not found

- [ ] **Step 3: Implement TrajectoryPlugin adapter**

Add before `#[cfg(test)]` in `src/trajectory.rs`:

```rust
// ---------------------------------------------------------------------------
// PluginModule adapter
// ---------------------------------------------------------------------------

use crate::app_context::AppContext;
use crate::health_check::HealthStatus;
use crate::plugin_bus::PluginModule;
use async_trait::async_trait;

/// Wraps TrajectoryLogger as a PluginModule.
///
/// On init, opens the SQLite database and registers `Arc<TrajectoryLogger>`
/// in AppContext for other modules (FeedbackLoop, Governor, etc.).
pub struct TrajectoryPlugin {
    db_path: String,
    logger: std::sync::RwLock<Option<Arc<TrajectoryLogger>>>,
}

impl TrajectoryPlugin {
    pub fn new(db_path: &str) -> Self {
        Self {
            db_path: db_path.to_string(),
            logger: std::sync::RwLock::new(None),
        }
    }
}

#[async_trait]
impl PluginModule for TrajectoryPlugin {
    fn id(&self) -> &str {
        "trajectory-logger"
    }
    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }
    fn capabilities(&self) -> Vec<String> {
        vec!["trajectory-logging".into(), "quality-analysis".into()]
    }
    async fn init(&self, ctx: &AppContext) -> anyhow::Result<()> {
        let logger = Arc::new(TrajectoryLogger::new(&self.db_path)?);
        ctx.register(logger.clone());
        *self.logger.write().expect("lock poisoned") = Some(logger);
        Ok(())
    }
    async fn shutdown(&self) -> anyhow::Result<()> {
        *self.logger.write().expect("lock poisoned") = None;
        Ok(())
    }
    fn health(&self) -> HealthStatus {
        match self.logger.read().expect("lock poisoned").as_ref() {
            Some(_) => HealthStatus::Healthy,
            None => HealthStatus::Unhealthy,
        }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib trajectory::tests::test_trajectory_plugin_lifecycle`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/trajectory.rs
git commit -m "feat: add TrajectoryPlugin adapter implementing PluginModule trait"
```

---

## Task 5: CircuitBreakerPlugin Adapter

**Files:**
- Modify: `src/circuit_breaker.rs` (append adapter at end, before `#[cfg(test)]`)

- [ ] **Step 1: Write failing test**

Add to the existing `#[cfg(test)] mod tests` in `src/circuit_breaker.rs`:

```rust
    #[tokio::test]
    async fn test_circuit_breaker_plugin_lifecycle() {
        use crate::app_context::AppContext;
        use crate::health_check::HealthStatus;
        use crate::plugin_bus::PluginModule;

        let plugin = CircuitBreakerPlugin::new(BreakerConfig::default());
        let ctx = AppContext::new();

        assert_eq!(plugin.id(), "circuit-breaker");
        assert_eq!(plugin.health(), HealthStatus::Healthy);

        plugin.init(&ctx).await.unwrap();

        // After init, ProviderCircuitBreaker is available via AppContext
        let breaker = ctx.get::<ProviderCircuitBreaker>().unwrap();
        assert!(breaker.is_available("test-provider"));

        plugin.shutdown().await.unwrap();
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib circuit_breaker::tests::test_circuit_breaker_plugin_lifecycle`
Expected: FAIL — `CircuitBreakerPlugin` not found

- [ ] **Step 3: Implement CircuitBreakerPlugin adapter**

Add before `#[cfg(test)]` in `src/circuit_breaker.rs`:

```rust
// ---------------------------------------------------------------------------
// PluginModule adapter
// ---------------------------------------------------------------------------

use crate::app_context::AppContext;
use crate::health_check::HealthStatus;
use crate::plugin_bus::PluginModule;
use async_trait::async_trait;
use std::sync::Arc;

/// Wraps ProviderCircuitBreaker as a PluginModule.
///
/// On init, creates the circuit breaker and registers `Arc<ProviderCircuitBreaker>`
/// in AppContext for ProviderRouter and other consumers.
pub struct CircuitBreakerPlugin {
    config: BreakerConfig,
    breaker: std::sync::RwLock<Option<Arc<ProviderCircuitBreaker>>>,
}

impl CircuitBreakerPlugin {
    pub fn new(config: BreakerConfig) -> Self {
        Self {
            config,
            breaker: std::sync::RwLock::new(None),
        }
    }
}

#[async_trait]
impl PluginModule for CircuitBreakerPlugin {
    fn id(&self) -> &str {
        "circuit-breaker"
    }
    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }
    fn capabilities(&self) -> Vec<String> {
        vec!["provider-reliability".into()]
    }
    async fn init(&self, ctx: &AppContext) -> anyhow::Result<()> {
        let breaker = Arc::new(ProviderCircuitBreaker::new(self.config.clone()));
        ctx.register(breaker.clone());
        *self.breaker.write().expect("lock poisoned") = Some(breaker);
        Ok(())
    }
    async fn shutdown(&self) -> anyhow::Result<()> {
        *self.breaker.write().expect("lock poisoned") = None;
        Ok(())
    }
    fn health(&self) -> HealthStatus {
        match self.breaker.read().expect("lock poisoned").as_ref() {
            Some(_) => HealthStatus::Healthy,
            None => HealthStatus::Unhealthy,
        }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib circuit_breaker::tests::test_circuit_breaker_plugin_lifecycle`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/circuit_breaker.rs
git commit -m "feat: add CircuitBreakerPlugin adapter implementing PluginModule trait"
```

---

## Task 6: Integration Test — Full PluginBus Lifecycle

**Files:**
- Create: `tests/plugin_bus_integration.rs`

- [ ] **Step 1: Write integration test**

```rust
//! Integration test: PluginBus manages 3 real module adapters through
//! init → service registration → cross-module access → shutdown lifecycle.

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

    // --- Build bus with 3 modules across 2 phases ---
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

    // --- Init all ---
    bus.init_all().await.unwrap();

    assert_eq!(bus.initialized_ids().len(), 3);
    assert_eq!(bus.system_health(), HealthStatus::Healthy);

    // --- Cross-module service access ---
    let ctx = bus.context();

    // TrajectoryLogger registered by TrajectoryPlugin
    let traj = ctx.get::<TrajectoryLogger>().unwrap();
    assert_eq!(traj.count_for_hand("nonexistent").unwrap(), 0);

    // ProviderCircuitBreaker registered by CircuitBreakerPlugin
    let breaker = ctx.get::<ProviderCircuitBreaker>().unwrap();
    assert!(breaker.is_available("test-provider"));

    // HealthCheckPlugin registered by itself
    let health = ctx.get::<HealthCheckPlugin>().unwrap();
    assert_eq!(health.id(), "health-check");

    // --- Shutdown all ---
    bus.shutdown_all().await.unwrap();
    assert!(bus.initialized_ids().is_empty());
}

#[tokio::test]
async fn test_plugin_bus_event_stream() {
    let mut bus = PluginBus::new();
    bus.register(1, Arc::new(HealthCheckPlugin::new()));

    let mut rx = bus.context().subscribe();

    bus.init_all().await.unwrap();

    // Should receive ModuleInitialized event
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
    use phantom_mesh::plugin_bus::PluginModule;

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
    bus.register(2, Arc::new(FailPlugin)); // This will fail

    let result = bus.init_all().await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("intentional failure"));

    // HealthCheckPlugin should have been rolled back
    assert!(bus.initialized_ids().is_empty());
}
```

- [ ] **Step 2: Run integration tests**

Run: `cargo test --test plugin_bus_integration`
Expected: 3 tests PASS

- [ ] **Step 3: Commit**

```bash
git add tests/plugin_bus_integration.rs
git commit -m "test: add integration tests for PluginBus with 3 real module adapters"
```

---

## Task 7: Wire PluginBus into Daemon Startup (Partial)

**Files:**
- Modify: `src/main.rs` — daemon startup section (~lines 2885-3060)

> **Important:** This is a non-breaking, additive change. The existing manual wiring continues
> to work. We add PluginBus alongside it for the 3 converted modules only. Full migration
> happens incrementally in later phases.

- [ ] **Step 1: Add PluginBus initialization near daemon startup**

In `src/main.rs`, find the daemon startup section (around line 2885). Add the following
**after** the logging/config setup but **before** the existing component initialization:

```rust
    // --- Plugin Bus (Phase 1: partial migration) ---
    use phantom_mesh::app_context::AppContext;
    use phantom_mesh::plugin_bus::PluginBus;
    use phantom_mesh::health_check::HealthCheckPlugin;
    use phantom_mesh::trajectory::TrajectoryPlugin;
    use phantom_mesh::circuit_breaker::{CircuitBreakerPlugin, BreakerConfig};

    let mut plugin_bus = PluginBus::new();

    // Phase 1: Infrastructure
    plugin_bus.register(1, Arc::new(HealthCheckPlugin::new()));

    // Phase 2: Data layer
    // Reuse the existing `home` variable already defined in daemon startup
    let traj_db = format!("{}/.phantom-mesh/trajectories.db", home);
    plugin_bus.register(2, Arc::new(TrajectoryPlugin::new(&traj_db)));

    // Phase 4: Engine
    plugin_bus.register(4, Arc::new(CircuitBreakerPlugin::new(BreakerConfig::default())));

    // Initialize all plugins
    if let Err(e) = plugin_bus.init_all().await {
        tracing::error!("[PluginBus] Init failed: {}", e);
        // Fall through to existing manual init as fallback
    } else {
        tracing::info!(
            "[PluginBus] {} modules initialized: {:?}",
            plugin_bus.initialized_ids().len(),
            plugin_bus.initialized_ids()
        );
    }

    // Retrieve PluginBus-created instances for use by existing wiring code.
    // This avoids creating duplicate instances (e.g., two TrajectoryLoggers
    // pointing at the same SQLite file would cause locking issues).
    // The existing `let trajectory_logger = Arc::new(TrajectoryLogger::new(...))`
    // lines should be replaced with:
    //   let trajectory_logger = app_context.get::<TrajectoryLogger>().unwrap();
    //   let circuit_breaker = app_context.get::<ProviderCircuitBreaker>().unwrap();
    // For Phase 1, comment out the existing manual creation of these 2 instances
    // and use the PluginBus-created ones instead.
    let app_context = plugin_bus.context().clone();
```

- [ ] **Step 2: Verify daemon still starts**

Run: `cargo build --release`
Expected: compiles without errors

Run (manual): `cargo run -- daemon` — verify daemon starts, check logs for `[PluginBus] 3 modules initialized`

- [ ] **Step 3: Run all tests**

Run: `cargo test`
Expected: all existing 3793+ tests pass, plus new tests

- [ ] **Step 4: Commit**

```bash
git add src/main.rs
git commit -m "feat: wire PluginBus into daemon startup with 3 modules (non-breaking, alongside existing init)"
```

---

## Milestone Verification

After completing all 7 tasks, verify against spec milestones:

| Milestone | Verification |
|-----------|-------------|
| M1.1 AppContext register/get | `cargo test --lib app_context::tests` — 8 tests pass |
| M1.2 PluginModule + PluginBus, 3 modules, phase init | `cargo test --test plugin_bus_integration` — 3 tests pass |
| M1.3 Event bus emit/subscribe | `test_emit_and_subscribe` + `test_plugin_bus_event_stream` pass |
| M1.4 Shutdown rollback | `test_rollback_on_init_failure` + `test_plugin_bus_init_rollback_integration` pass |
| Stage complete | `cargo test` — all tests pass, daemon starts via PluginBus |

Run: `cargo test 2>&1 | tail -5`
Expected: `test result: ok. XXXX passed; 0 failed;`
