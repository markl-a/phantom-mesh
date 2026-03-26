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
    /// A module failed during shutdown.
    ModuleShutdownFailed { module_id: String, error: String },
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

    /// Remove a previously registered service.
    pub fn deregister<T: Send + Sync + 'static>(&self) -> bool {
        self.services
            .write()
            .expect("AppContext lock poisoned")
            .remove(&TypeId::of::<T>())
            .is_some()
    }

    /// Remove all registered services.
    pub fn clear(&self) {
        self.services
            .write()
            .expect("AppContext lock poisoned")
            .clear();
    }
}

impl Default for AppContext {
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

    #[test]
    fn test_register_and_get_roundtrip() {
        let ctx = AppContext::new();
        ctx.register(Arc::new(42u32));
        let retrieved = ctx.get::<u32>().unwrap();
        assert_eq!(*retrieved, 42);
    }

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

    #[test]
    fn test_deregister_existing_service() {
        let ctx = AppContext::new();
        ctx.register(Arc::new(42u32));
        assert_eq!(ctx.service_count(), 1);
        assert!(ctx.deregister::<u32>(), "deregister should return true for existing service");
        assert_eq!(ctx.service_count(), 0);
        assert!(ctx.get::<u32>().is_none(), "get should return None after deregister");
    }

    #[test]
    fn test_deregister_nonexistent_service() {
        let ctx = AppContext::new();
        assert!(!ctx.deregister::<u32>(), "deregister should return false for non-existent service");
    }

    #[test]
    fn test_deregister_does_not_affect_other_types() {
        let ctx = AppContext::new();
        ctx.register(Arc::new(42u32));
        ctx.register(Arc::new("hello".to_string()));
        assert_eq!(ctx.service_count(), 2);
        ctx.deregister::<u32>();
        assert_eq!(ctx.service_count(), 1);
        assert!(ctx.get::<u32>().is_none());
        assert_eq!(*ctx.get::<String>().unwrap(), "hello");
    }

    #[test]
    fn test_clear_removes_all_services() {
        let ctx = AppContext::new();
        ctx.register(Arc::new(42u32));
        ctx.register(Arc::new("hello".to_string()));
        ctx.register(Arc::new(3.14f64));
        assert_eq!(ctx.service_count(), 3);
        ctx.clear();
        assert_eq!(ctx.service_count(), 0);
        assert!(ctx.get::<u32>().is_none());
        assert!(ctx.get::<String>().is_none());
        assert!(ctx.get::<f64>().is_none());
    }

    #[test]
    fn test_clear_on_empty_context() {
        let ctx = AppContext::new();
        ctx.clear(); // should not panic
        assert_eq!(ctx.service_count(), 0);
    }
}
