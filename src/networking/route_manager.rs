//! Route manager — selects the best network path to reach a cluster node.
//!
//! Priority: mDNS (LAN) → Iroh (QUIC) → Relay → HTTP (legacy).
//! Routes are cached with a configurable TTL (default 60 s).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use super::discovery::{ConnectionLayer, ServiceDiscovery};
use super::transport::MeshTransport;

/// A resolved route to a cluster node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedRoute {
    /// Target node name.
    pub node_name: String,
    /// Full HTTP URL to reach the node.
    pub url: String,
    /// Which layer provides the connection.
    pub layer: ConnectionLayer,
    /// Estimated latency in milliseconds (0 = unknown).
    pub latency_ms: u64,
}

/// Internal cache entry.
#[derive(Debug, Clone)]
struct CachedRoute {
    route: ResolvedRoute,
    #[allow(dead_code)]
    inserted_at: Instant,
    expires_at: Instant,
}

/// A single route entry exposed for status/debug purposes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteEntry {
    pub node_name: String,
    pub url: String,
    pub layer: ConnectionLayer,
    pub latency_ms: u64,
    pub cached: bool,
}

/// Manages routing decisions across all networking layers.
pub struct RouteManager {
    /// Discovery backends, ordered by priority (mDNS first, then others).
    discoveries: Vec<Arc<dyn ServiceDiscovery>>,
    /// Mesh transports (Iroh, Tailscale, etc.).
    #[allow(dead_code)]
    transports: Vec<Arc<dyn MeshTransport>>,
    /// Cached routes (node_name → cached entry).
    cache: Arc<RwLock<HashMap<String, CachedRoute>>>,
    /// Cache TTL.
    cache_ttl: Duration,
}

impl RouteManager {
    /// Create a new RouteManager.
    pub fn new(cache_ttl: Duration) -> Self {
        Self {
            discoveries: Vec::new(),
            transports: Vec::new(),
            cache: Arc::new(RwLock::new(HashMap::new())),
            cache_ttl,
        }
    }

    /// Create with default 60-second cache TTL.
    pub fn default_ttl() -> Self {
        Self::new(Duration::from_secs(60))
    }

    /// Register a discovery backend (ordered by priority — first added = highest priority).
    pub fn add_discovery(&mut self, d: Arc<dyn ServiceDiscovery>) {
        self.discoveries.push(d);
    }

    /// Register a mesh transport backend.
    pub fn add_transport(&mut self, t: Arc<dyn MeshTransport>) {
        self.transports.push(t);
    }

    /// Find the best route to a named node.
    ///
    /// Priority: discovery (mDNS > others) → transport (Iroh) → cache fallback.
    /// Discovery backends are checked in registration order (first = highest priority).
    pub async fn best_route(&self, node_name: &str) -> Option<ResolvedRoute> {
        // 1. Query discovery backends first (highest priority — real-time data)
        for discovery in &self.discoveries {
            let nodes = discovery.discovered_nodes().await;
            if let Some(node) = nodes.iter().find(|n| n.name == node_name) {
                let route = ResolvedRoute {
                    node_name: node_name.to_string(),
                    url: node.http_url(),
                    layer: node.layer,
                    latency_ms: 0, // will be measured later
                };
                self.cache_route(&route).await;
                return Some(route);
            }
        }

        // 2. Query mesh transports for QUIC routes
        for transport in &self.transports {
            let peers = transport.peers().await;
            if let Some(peer) = peers.iter().find(|p| {
                p.name.as_deref() == Some(node_name)
            }) {
                let route = ResolvedRoute {
                    node_name: node_name.to_string(),
                    url: format!("iroh://{}", peer.id),
                    layer: ConnectionLayer::Iroh,
                    latency_ms: peer.latency_ms.unwrap_or(0),
                };
                self.cache_route(&route).await;
                return Some(route);
            }
        }

        // 3. Fallback to cache (e.g. static HTTP routes, previously resolved routes)
        {
            let cache = self.cache.read().await;
            if let Some(entry) = cache.get(node_name) {
                if Instant::now() < entry.expires_at {
                    return Some(entry.route.clone());
                }
            }
        }

        None
    }

    /// Get all currently known routes (cached + fresh discovery).
    pub async fn all_routes(&self) -> Vec<RouteEntry> {
        let mut routes = Vec::new();
        let mut seen = std::collections::HashSet::new();

        // Cached routes
        {
            let cache = self.cache.read().await;
            for (name, entry) in cache.iter() {
                if Instant::now() < entry.expires_at {
                    seen.insert(name.clone());
                    routes.push(RouteEntry {
                        node_name: name.clone(),
                        url: entry.route.url.clone(),
                        layer: entry.route.layer,
                        latency_ms: entry.route.latency_ms,
                        cached: true,
                    });
                }
            }
        }

        // Fresh discovery
        for discovery in &self.discoveries {
            for node in discovery.discovered_nodes().await {
                if seen.insert(node.name.clone()) {
                    routes.push(RouteEntry {
                        node_name: node.name.clone(),
                        url: node.http_url(),
                        layer: node.layer,
                        latency_ms: 0,
                        cached: false,
                    });
                }
            }
        }

        routes
    }

    /// Insert or update a route in the cache.
    async fn cache_route(&self, route: &ResolvedRoute) {
        let mut cache = self.cache.write().await;
        let now = Instant::now();
        cache.insert(
            route.node_name.clone(),
            CachedRoute {
                route: route.clone(),
                inserted_at: now,
                expires_at: now + self.cache_ttl,
            },
        );
    }

    /// Clear expired entries from the cache.
    pub async fn evict_expired(&self) {
        let mut cache = self.cache.write().await;
        let now = Instant::now();
        cache.retain(|_, v| now < v.expires_at);
    }

    /// Add a known HTTP route (legacy nodes that don't use discovery).
    pub async fn add_static_route(&self, node_name: &str, url: &str) {
        let route = ResolvedRoute {
            node_name: node_name.to_string(),
            url: url.to_string(),
            layer: ConnectionLayer::Http,
            latency_ms: 0,
        };
        self.cache_route(&route).await;
    }

    /// Number of registered discovery backends.
    pub fn discovery_count(&self) -> usize {
        self.discoveries.len()
    }

    /// Number of registered transport backends.
    pub fn transport_count(&self) -> usize {
        self.transports.len()
    }

    /// Start a background task that periodically refreshes routes and evicts stale cache.
    pub fn spawn_refresh_loop(self: &Arc<Self>, interval: Duration) -> tokio::task::JoinHandle<()> {
        let mgr = Arc::clone(self);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(interval);
            loop {
                tick.tick().await;
                mgr.evict_expired().await;
                // Touch each discovery to refresh internal state
                for d in &mgr.discoveries {
                    let _ = d.discovered_nodes().await;
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::networking::mdns::{MdnsDiscovery, unix_now};
    use crate::networking::discovery::DiscoveredNode;
    use std::net::{IpAddr, Ipv4Addr};

    #[tokio::test]
    async fn empty_route_manager_returns_none() {
        let rm = RouteManager::default_ttl();
        assert!(rm.best_route("nonexistent").await.is_none());
    }

    #[tokio::test]
    async fn all_routes_empty() {
        let rm = RouteManager::default_ttl();
        let routes = rm.all_routes().await;
        assert!(routes.is_empty());
    }

    #[tokio::test]
    async fn cache_route_and_retrieve() {
        let rm = RouteManager::default_ttl();
        let route = ResolvedRoute {
            node_name: "test-node".into(),
            url: "http://192.168.1.42:7878".into(),
            layer: ConnectionLayer::Mdns,
            latency_ms: 2,
        };
        rm.cache_route(&route).await;
        let found = rm.best_route("test-node").await;
        assert!(found.is_some());
        let found = found.unwrap();
        assert_eq!(found.node_name, "test-node");
        assert_eq!(found.url, "http://192.168.1.42:7878");
    }

    #[tokio::test]
    async fn evict_expired() {
        let rm = RouteManager::new(Duration::from_millis(1));
        let route = ResolvedRoute {
            node_name: "ephemeral".into(),
            url: "http://10.0.0.1:7878".into(),
            layer: ConnectionLayer::Http,
            latency_ms: 0,
        };
        rm.cache_route(&route).await;
        tokio::time::sleep(Duration::from_millis(5)).await;
        rm.evict_expired().await;
        assert!(rm.best_route("ephemeral").await.is_none());
    }

    #[tokio::test]
    async fn route_from_mdns_discovery() {
        let disc = Arc::new(MdnsDiscovery::new("hub".into(), 7878, vec![]));
        disc.add_node(DiscoveredNode {
            name: "worker-1".into(),
            addr: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 101)),
            port: 7879,
            layer: ConnectionLayer::Mdns,
            capabilities: vec![],
            device_type: None,
            last_seen: unix_now(),
        })
        .await;

        let mut rm = RouteManager::default_ttl();
        rm.add_discovery(disc);

        let route = rm.best_route("worker-1").await;
        assert!(route.is_some());
        let route = route.unwrap();
        assert_eq!(route.url, "http://192.168.1.101:7879");
        assert_eq!(route.layer, ConnectionLayer::Mdns);
    }

    #[tokio::test]
    async fn route_priority_mdns_over_http() {
        let disc = Arc::new(MdnsDiscovery::new("hub".into(), 7878, vec![]));
        disc.add_node(DiscoveredNode {
            name: "worker-1".into(),
            addr: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 101)),
            port: 7879,
            layer: ConnectionLayer::Mdns,
            capabilities: vec![],
            device_type: None,
            last_seen: unix_now(),
        })
        .await;

        let mut rm = RouteManager::default_ttl();
        rm.add_discovery(disc);

        // Add a static HTTP route for the same node
        rm.add_static_route("worker-1", "http://10.0.0.50:7879").await;

        // Discovery (mDNS) should take priority over cached HTTP route
        let route = rm.best_route("worker-1").await.unwrap();
        assert_eq!(route.layer, ConnectionLayer::Mdns);
    }

    #[tokio::test]
    async fn all_routes_includes_discovery_and_cache() {
        let disc = Arc::new(MdnsDiscovery::new("hub".into(), 7878, vec![]));
        disc.add_node(DiscoveredNode {
            name: "lan-node".into(),
            addr: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 101)),
            port: 7879,
            layer: ConnectionLayer::Mdns,
            capabilities: vec![],
            device_type: None,
            last_seen: unix_now(),
        })
        .await;

        let mut rm = RouteManager::default_ttl();
        rm.add_discovery(disc);
        rm.add_static_route("remote-node", "http://10.0.0.1:7878").await;

        let routes = rm.all_routes().await;
        assert_eq!(routes.len(), 2);
        assert!(routes.iter().any(|r| r.node_name == "remote-node" && r.cached));
        assert!(routes.iter().any(|r| r.node_name == "lan-node" && !r.cached));
    }

    #[tokio::test]
    async fn static_http_route() {
        let rm = RouteManager::default_ttl();
        rm.add_static_route("legacy-worker", "http://192.168.1.200:7878").await;
        let route = rm.best_route("legacy-worker").await.unwrap();
        assert_eq!(route.layer, ConnectionLayer::Http);
        assert_eq!(route.url, "http://192.168.1.200:7878");
    }

    #[test]
    fn discovery_and_transport_count() {
        let rm = RouteManager::default_ttl();
        assert_eq!(rm.discovery_count(), 0);
        assert_eq!(rm.transport_count(), 0);
    }
}
