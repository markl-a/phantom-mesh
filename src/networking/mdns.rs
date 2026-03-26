//! mDNS/LAN service discovery (Layer 1).
//!
//! Uses the `_phantom_mesh._tcp.local` service type for zero-config LAN discovery.
//! Built on the `mdns-sd` crate for cross-platform mDNS (Bonjour/Avahi compatible).

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use tokio::sync::RwLock;

use super::discovery::{ConnectionLayer, DiscoveredNode, ServiceDiscovery};

/// mDNS service type for Phantom Mesh cluster nodes.
const SERVICE_TYPE: &str = "_phantom_mesh._tcp.local.";

/// Default stale-node timeout (90 seconds — 3× the 30-sec heartbeat).
const DEFAULT_STALE_TIMEOUT_SECS: u64 = 90;

/// mDNS-based service discovery for LAN nodes.
pub struct MdnsDiscovery {
    /// Our node name to advertise.
    node_name: String,
    /// Port to advertise.
    port: u16,
    /// Capabilities to advertise in TXT records.
    capabilities: Vec<String>,
    /// Device type to advertise.
    device_type: Option<String>,
    /// Discovered peers (keyed by instance name for dedup).
    nodes: Arc<RwLock<HashMap<String, DiscoveredNode>>>,
    /// Callbacks for new node events.
    callbacks: Arc<RwLock<Vec<Box<dyn Fn(DiscoveredNode) + Send + Sync>>>>,
    /// Whether the discovery loop is running.
    running: Arc<RwLock<bool>>,
    /// mDNS daemon handle (created on start, dropped on stop).
    daemon: Arc<RwLock<Option<ServiceDaemon>>>,
    /// Background browse task handle.
    browse_handle: Arc<RwLock<Option<tokio::task::JoinHandle<()>>>>,
}

impl MdnsDiscovery {
    /// Create a new mDNS discovery instance.
    pub fn new(node_name: String, port: u16, capabilities: Vec<String>) -> Self {
        Self {
            node_name,
            port,
            capabilities,
            device_type: Some("desktop".into()),
            nodes: Arc::new(RwLock::new(HashMap::new())),
            callbacks: Arc::new(RwLock::new(Vec::new())),
            running: Arc::new(RwLock::new(false)),
            daemon: Arc::new(RwLock::new(None)),
            browse_handle: Arc::new(RwLock::new(None)),
        }
    }

    /// Create with explicit device type.
    pub fn with_device_type(mut self, device_type: &str) -> Self {
        self.device_type = Some(device_type.to_string());
        self
    }

    /// Add a discovered node (used internally by the browse loop and for testing).
    pub async fn add_node(&self, node: DiscoveredNode) {
        let is_new = {
            let mut nodes = self.nodes.write().await;
            let is_new = !nodes.contains_key(&node.name);
            nodes.insert(node.name.clone(), node.clone());
            is_new
        }; // write lock dropped here before calling callbacks

        if is_new {
            let callbacks = self.callbacks.read().await;
            for cb in callbacks.iter() {
                cb(node.clone());
            }
        }
    }

    /// Remove a node by name.
    pub async fn remove_node(&self, name: &str) {
        let mut nodes = self.nodes.write().await;
        nodes.remove(name);
    }

    /// Remove nodes not seen within the given timeout (seconds).
    pub async fn evict_stale(&self, timeout_secs: u64) {
        let now = unix_now();
        let mut nodes = self.nodes.write().await;
        nodes.retain(|_, n| now.saturating_sub(n.last_seen) < timeout_secs);
    }

    /// Get our advertised node info.
    pub fn local_node_info(&self) -> (String, u16, Vec<String>) {
        (self.node_name.clone(), self.port, self.capabilities.clone())
    }

    /// Build the mDNS TXT properties for our service advertisement.
    fn build_txt_properties(&self) -> HashMap<String, String> {
        let mut props = HashMap::new();
        props.insert("node".to_string(), self.node_name.clone());
        if !self.capabilities.is_empty() {
            props.insert("caps".to_string(), self.capabilities.join(","));
        }
        if let Some(ref dt) = self.device_type {
            props.insert("device".to_string(), dt.clone());
        }
        props
    }

    /// Register our service with the mDNS daemon.
    fn register_service(&self, daemon: &ServiceDaemon) -> anyhow::Result<()> {
        let instance_name = format!("{}.{}", self.node_name, SERVICE_TYPE);

        let props_owned = self.build_txt_properties();
        let props_refs: Vec<(&str, &str)> = props_owned
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();

        let local_ip = detect_local_ip().unwrap_or(IpAddr::from([127, 0, 0, 1]));

        let service_info = ServiceInfo::new(
            SERVICE_TYPE,
            &self.node_name,
            &format!("{}.local.", self.node_name),
            local_ip,
            self.port,
            &props_refs[..],
        )
        .map_err(|e| anyhow::anyhow!("Failed to create ServiceInfo: {}", e))?;

        daemon
            .register(service_info)
            .map_err(|e| anyhow::anyhow!("Failed to register mDNS service: {}", e))?;

        tracing::info!(
            "mDNS: registered service {} at {}:{}",
            instance_name,
            local_ip,
            self.port
        );

        Ok(())
    }

    /// Spawn a background task that processes mDNS browse events.
    fn spawn_browse_loop(
        &self,
        daemon: &ServiceDaemon,
    ) -> anyhow::Result<tokio::task::JoinHandle<()>> {
        let receiver = daemon
            .browse(SERVICE_TYPE)
            .map_err(|e| anyhow::anyhow!("Failed to start mDNS browse: {}", e))?;

        let nodes = Arc::clone(&self.nodes);
        let callbacks = Arc::clone(&self.callbacks);
        let running = Arc::clone(&self.running);
        let our_name = self.node_name.clone();

        let handle = tokio::task::spawn_blocking(move || {
            loop {
                // Check if we should stop
                if let Ok(guard) = running.try_read() {
                    if !*guard {
                        break;
                    }
                }

                match receiver.recv_timeout(std::time::Duration::from_secs(2)) {
                    Ok(event) => {
                        match event {
                            ServiceEvent::ServiceResolved(info) => {
                                let name = info
                                    .get_property_val_str("node")
                                    .unwrap_or_else(|| info.get_fullname().split('.').next().unwrap_or("unknown"))
                                    .to_string();

                                // Skip our own advertisement
                                if name == our_name {
                                    continue;
                                }

                                let addr = info
                                    .get_addresses()
                                    .iter()
                                    .next()
                                    .copied()
                                    .unwrap_or(IpAddr::from([0, 0, 0, 0]));

                                let capabilities: Vec<String> = info
                                    .get_property_val_str("caps")
                                    .map(|s| s.split(',').map(|c| c.trim().to_string()).collect())
                                    .unwrap_or_default();

                                let device_type = info
                                    .get_property_val_str("device")
                                    .map(|s| s.to_string());

                                let node = DiscoveredNode {
                                    name: name.clone(),
                                    addr,
                                    port: info.get_port(),
                                    layer: ConnectionLayer::Mdns,
                                    capabilities,
                                    device_type,
                                    last_seen: unix_now(),
                                };

                                tracing::info!("mDNS: discovered node {} at {}:{}", name, addr, info.get_port());

                                // Update or insert (blocking write since we're in spawn_blocking)
                                let rt = tokio::runtime::Handle::try_current();
                                if let Ok(handle) = rt {
                                    let nodes = nodes.clone();
                                    let callbacks = callbacks.clone();
                                    let node_clone = node.clone();
                                    handle.spawn(async move {
                                        let is_new = {
                                            let mut guard = nodes.write().await;
                                            let is_new = !guard.contains_key(&node_clone.name);
                                            guard.insert(node_clone.name.clone(), node_clone.clone());
                                            is_new
                                        };
                                        if is_new {
                                            let cbs = callbacks.read().await;
                                            for cb in cbs.iter() {
                                                cb(node_clone.clone());
                                            }
                                        }
                                    });
                                }
                            }
                            ServiceEvent::ServiceRemoved(_typ, fullname) => {
                                let name = fullname.split('.').next().unwrap_or("").to_string();
                                tracing::info!("mDNS: node removed: {}", name);
                                if let Ok(handle) = tokio::runtime::Handle::try_current() {
                                    let nodes = nodes.clone();
                                    handle.spawn(async move {
                                        let mut guard = nodes.write().await;
                                        guard.remove(&name);
                                    });
                                }
                            }
                            _ => {} // SearchStarted, SearchStopped, etc.
                        }
                    }
                    Err(flume::RecvTimeoutError::Timeout) => continue,
                    Err(flume::RecvTimeoutError::Disconnected) => break,
                }
            }
            tracing::info!("mDNS browse loop ended");
        });

        Ok(handle)
    }
}

#[async_trait::async_trait]
impl ServiceDiscovery for MdnsDiscovery {
    async fn start(&self) -> anyhow::Result<()> {
        let mut running = self.running.write().await;
        if *running {
            return Ok(());
        }

        // Create mDNS daemon
        let daemon = ServiceDaemon::new()
            .map_err(|e| anyhow::anyhow!("Failed to create mDNS daemon: {}", e))?;

        // Register our service
        self.register_service(&daemon)?;

        // Start browsing for peers
        let handle = self.spawn_browse_loop(&daemon)?;

        // Store handles
        *self.daemon.write().await = Some(daemon);
        *self.browse_handle.write().await = Some(handle);
        *running = true;

        tracing::info!(
            "mDNS discovery started: {} on port {} with caps {:?}",
            self.node_name,
            self.port,
            self.capabilities,
        );

        // Spawn stale eviction loop
        let nodes = Arc::clone(&self.nodes);
        let running_flag = Arc::clone(&self.running);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
            loop {
                interval.tick().await;
                if let Ok(guard) = running_flag.try_read() {
                    if !*guard {
                        break;
                    }
                }
                let now = unix_now();
                let mut guard = nodes.write().await;
                guard.retain(|_, n| now.saturating_sub(n.last_seen) < DEFAULT_STALE_TIMEOUT_SECS);
            }
        });

        Ok(())
    }

    async fn stop(&self) -> anyhow::Result<()> {
        let mut running = self.running.write().await;
        if !*running {
            return Ok(());
        }
        *running = false;

        // Shutdown the daemon (this will close the browse receiver)
        if let Some(daemon) = self.daemon.write().await.take() {
            let _ = daemon.shutdown();
        }

        // Abort the browse handle
        if let Some(handle) = self.browse_handle.write().await.take() {
            handle.abort();
        }

        tracing::info!("mDNS discovery stopped");
        Ok(())
    }

    async fn discovered_nodes(&self) -> Vec<DiscoveredNode> {
        self.nodes.read().await.values().cloned().collect()
    }

    fn on_node_found(&self, callback: Box<dyn Fn(DiscoveredNode) + Send + Sync>) {
        if let Ok(mut cbs) = self.callbacks.try_write() {
            cbs.push(callback);
        } else {
            tracing::warn!("Failed to register on_node_found callback (lock contention)");
        }
    }

    fn layer(&self) -> ConnectionLayer {
        ConnectionLayer::Mdns
    }
}

/// Helper to get the current unix timestamp.
pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Detect our local LAN IP (same trick as cluster_worker.rs).
pub fn detect_local_ip() -> Option<IpAddr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    socket.local_addr().ok().map(|a| a.ip())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn mdns_discovery_lifecycle() {
        let disc = MdnsDiscovery::new("test-hub".into(), 7878, vec!["shell".into()]);

        // Don't call start() in tests — it requires network access.
        // Test manual add_node / discovered_nodes instead.
        let node = DiscoveredNode {
            name: "worker-1".into(),
            addr: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 101)),
            port: 7879,
            layer: ConnectionLayer::Mdns,
            capabilities: vec!["shell".into()],
            device_type: Some("desktop".into()),
            last_seen: unix_now(),
        };

        disc.add_node(node).await;
        let nodes = disc.discovered_nodes().await;
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].name, "worker-1");
    }

    #[tokio::test]
    async fn mdns_update_existing_node() {
        let disc = MdnsDiscovery::new("hub".into(), 7878, vec![]);
        let node1 = DiscoveredNode {
            name: "worker-1".into(),
            addr: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 101)),
            port: 7879,
            layer: ConnectionLayer::Mdns,
            capabilities: vec![],
            device_type: None,
            last_seen: 100,
        };
        disc.add_node(node1).await;

        // Update same node with new port/timestamp
        let node2 = DiscoveredNode {
            name: "worker-1".into(),
            addr: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 102)),
            port: 8080,
            layer: ConnectionLayer::Mdns,
            capabilities: vec!["shell".into()],
            device_type: None,
            last_seen: 200,
        };
        disc.add_node(node2).await;

        let nodes = disc.discovered_nodes().await;
        assert_eq!(nodes.len(), 1); // Still only 1 node
        let n = &nodes[0];
        assert_eq!(n.port, 8080); // Updated
        assert_eq!(n.last_seen, 200); // Updated
    }

    #[tokio::test]
    async fn mdns_callback_fires_on_new_only() {
        let disc = MdnsDiscovery::new("hub".into(), 7878, vec![]);
        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();
        disc.on_node_found(Box::new(move |_| {
            c.fetch_add(1, Ordering::Relaxed);
        }));

        let node = DiscoveredNode {
            name: "new-node".into(),
            addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: 1234,
            layer: ConnectionLayer::Mdns,
            capabilities: vec![],
            device_type: None,
            last_seen: unix_now(),
        };

        disc.add_node(node.clone()).await;
        assert_eq!(counter.load(Ordering::Relaxed), 1);

        // Adding same node again should NOT fire callback
        disc.add_node(node).await;
        assert_eq!(counter.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn evict_stale_nodes() {
        let disc = MdnsDiscovery::new("hub".into(), 7878, vec![]);
        disc.add_node(DiscoveredNode {
            name: "stale".into(),
            addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: 1234,
            layer: ConnectionLayer::Mdns,
            capabilities: vec![],
            device_type: None,
            last_seen: 1, // Very old timestamp
        })
        .await;

        assert_eq!(disc.discovered_nodes().await.len(), 1);
        disc.evict_stale(60).await;
        assert_eq!(disc.discovered_nodes().await.len(), 0);
    }

    #[tokio::test]
    async fn remove_node() {
        let disc = MdnsDiscovery::new("hub".into(), 7878, vec![]);
        disc.add_node(DiscoveredNode {
            name: "temp".into(),
            addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: 1234,
            layer: ConnectionLayer::Mdns,
            capabilities: vec![],
            device_type: None,
            last_seen: unix_now(),
        })
        .await;

        assert_eq!(disc.discovered_nodes().await.len(), 1);
        disc.remove_node("temp").await;
        assert_eq!(disc.discovered_nodes().await.len(), 0);
    }

    #[test]
    fn build_txt_properties() {
        let disc = MdnsDiscovery::new("z13".into(), 7878, vec!["shell".into(), "browser".into()]);
        let props = disc.build_txt_properties();
        assert_eq!(props.get("node"), Some(&"z13".to_string()));
        assert_eq!(props.get("caps"), Some(&"shell,browser".to_string()));
        assert_eq!(props.get("device"), Some(&"desktop".to_string()));
    }

    #[test]
    fn detect_local_ip_does_not_panic() {
        let _ = detect_local_ip();
    }
}
