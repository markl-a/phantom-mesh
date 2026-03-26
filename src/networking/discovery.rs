//! Service discovery trait and types for Phantom Mesh auto-networking.

use std::fmt;
use std::net::IpAddr;

use serde::{Deserialize, Serialize};

/// Which networking layer a node was discovered through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionLayer {
    /// Layer 1 — mDNS / LAN (zero-config, <1 ms).
    Mdns,
    /// Layer 2 — Iroh / QUIC (NAT traversal, E2E encrypted).
    Iroh,
    /// Layer 3 — Cloud relay (fallback, higher latency).
    Relay,
    /// Direct HTTP connection (legacy, pre-B.3).
    Http,
}

impl fmt::Display for ConnectionLayer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Mdns => write!(f, "mDNS"),
            Self::Iroh => write!(f, "Iroh"),
            Self::Relay => write!(f, "Relay"),
            Self::Http => write!(f, "HTTP"),
        }
    }
}

/// A node discovered by a [`ServiceDiscovery`] implementation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredNode {
    /// Human-readable name (e.g. "Z13", "M1-Mac").
    pub name: String,
    /// IP address (v4 or v6).
    pub addr: IpAddr,
    /// Daemon HTTP port.
    pub port: u16,
    /// Which layer discovered this node.
    pub layer: ConnectionLayer,
    /// Capabilities advertised by the node (empty = unknown).
    pub capabilities: Vec<String>,
    /// Device type hint (e.g. "desktop", "mobile").
    pub device_type: Option<String>,
    /// Unix-epoch timestamp of last successful discovery/heartbeat.
    pub last_seen: u64,
}

impl DiscoveredNode {
    /// Build a base HTTP URL for this node's daemon.
    /// Correctly brackets IPv6 addresses (e.g. `http://[::1]:7878`).
    pub fn http_url(&self) -> String {
        match self.addr {
            IpAddr::V4(v4) => format!("http://{}:{}", v4, self.port),
            IpAddr::V6(v6) => format!("http://[{}]:{}", v6, self.port),
        }
    }
}

/// Trait for pluggable service discovery backends.
///
/// Implementations: [`super::mdns::MdnsDiscovery`], (future: Tailscale, libp2p, etc.)
#[async_trait::async_trait]
pub trait ServiceDiscovery: Send + Sync {
    /// Start broadcasting our presence and browsing for peers.
    async fn start(&self) -> anyhow::Result<()>;

    /// Stop broadcasting and browsing.
    async fn stop(&self) -> anyhow::Result<()>;

    /// Return the current list of discovered peers.
    async fn discovered_nodes(&self) -> Vec<DiscoveredNode>;

    /// Register a callback invoked whenever a **new** node is found.
    /// Multiple callbacks may be registered.
    fn on_node_found(&self, callback: Box<dyn Fn(DiscoveredNode) + Send + Sync>);

    /// The layer this discovery backend represents.
    fn layer(&self) -> ConnectionLayer;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn discovered_node_http_url() {
        let node = DiscoveredNode {
            name: "test-node".into(),
            addr: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 42)),
            port: 7878,
            layer: ConnectionLayer::Mdns,
            capabilities: vec!["shell".into()],
            device_type: Some("desktop".into()),
            last_seen: 1711065600,
        };
        assert_eq!(node.http_url(), "http://192.168.1.42:7878");
    }

    #[test]
    fn discovered_node_http_url_ipv6() {
        let node = DiscoveredNode {
            name: "ipv6-node".into(),
            addr: IpAddr::V6(Ipv6Addr::LOCALHOST),
            port: 7878,
            layer: ConnectionLayer::Mdns,
            capabilities: vec![],
            device_type: None,
            last_seen: 0,
        };
        assert_eq!(node.http_url(), "http://[::1]:7878");
    }

    #[test]
    fn connection_layer_display() {
        assert_eq!(ConnectionLayer::Mdns.to_string(), "mDNS");
        assert_eq!(ConnectionLayer::Iroh.to_string(), "Iroh");
        assert_eq!(ConnectionLayer::Relay.to_string(), "Relay");
        assert_eq!(ConnectionLayer::Http.to_string(), "HTTP");
    }
}
