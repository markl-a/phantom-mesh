//! Mesh transport trait and types for Phantom Mesh auto-networking (Layer 2+).

use serde::{Deserialize, Serialize};

/// Opaque peer identifier (e.g. Iroh NodeId, Tailscale IP).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PeerId(pub String);

impl std::fmt::Display for PeerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Information about a connected mesh peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfo {
    /// Unique peer identifier.
    pub id: PeerId,
    /// Human-readable name, if known.
    pub name: Option<String>,
    /// Round-trip latency in milliseconds.
    pub latency_ms: Option<u64>,
    /// Whether this peer is reachable directly (not relayed).
    pub direct: bool,
}

/// A bidirectional communication channel to a peer.
/// Wraps the underlying transport's stream/connection.
pub struct TransportChannel {
    /// Peer this channel connects to.
    pub peer: PeerId,
    /// Sender half: send bytes to the peer.
    pub tx: tokio::sync::mpsc::Sender<Vec<u8>>,
    /// Receiver half: receive bytes from the peer.
    pub rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
}

/// Trait for pluggable mesh transport backends.
///
/// Implementations: [`super::iroh_transport::IrohTransport`] (future: Tailscale, libp2p).
#[async_trait::async_trait]
pub trait MeshTransport: Send + Sync {
    /// Start listening for incoming peer connections.
    async fn listen(&self) -> anyhow::Result<()>;

    /// Stop the transport.
    async fn stop(&self) -> anyhow::Result<()>;

    /// Connect to a specific peer, returning a bidirectional channel.
    async fn connect(&self, peer: &PeerId) -> anyhow::Result<TransportChannel>;

    /// Return the currently known/connected peers.
    async fn peers(&self) -> Vec<PeerInfo>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_id_display() {
        let pid = PeerId("abc123".into());
        assert_eq!(pid.to_string(), "abc123");
    }

    #[tokio::test]
    async fn transport_channel_smoke() {
        let peer = PeerId("test".into());
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        let (_tx2, rx2) = tokio::sync::mpsc::channel(8);
        let ch = TransportChannel {
            peer: peer.clone(),
            tx,
            rx: rx2,
        };
        assert_eq!(ch.peer, peer);
    }
}
