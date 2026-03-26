//! Iroh/QUIC mesh transport (Layer 2) — stub implementation.
//!
//! This will be fully implemented when the `iroh` crate is integrated.
//! For now, provides the struct and trait impl with placeholder bodies.

use std::sync::Arc;

use tokio::sync::RwLock;

use super::transport::{MeshTransport, PeerId, PeerInfo, TransportChannel};

/// Iroh-based QUIC mesh transport with NAT traversal.
///
/// **Status**: Stub — requires `iroh` crate integration (M3.5).
pub struct IrohTransport {
    /// Known peers.
    peers: Arc<RwLock<Vec<PeerInfo>>>,
    /// Whether the transport is listening.
    running: Arc<RwLock<bool>>,
}

impl IrohTransport {
    /// Create a new (stub) Iroh transport.
    pub fn new() -> Self {
        Self {
            peers: Arc::new(RwLock::new(Vec::new())),
            running: Arc::new(RwLock::new(false)),
        }
    }
}

impl Default for IrohTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl MeshTransport for IrohTransport {
    async fn listen(&self) -> anyhow::Result<()> {
        let mut running = self.running.write().await;
        if *running {
            return Ok(());
        }
        *running = true;
        tracing::warn!("IrohTransport::listen() — stub, Iroh crate not yet integrated");
        Ok(())
    }

    async fn stop(&self) -> anyhow::Result<()> {
        let mut running = self.running.write().await;
        *running = false;
        Ok(())
    }

    async fn connect(&self, peer: &PeerId) -> anyhow::Result<TransportChannel> {
        // TODO(M3.5): Implement real Iroh QUIC connection
        anyhow::bail!(
            "IrohTransport::connect({}) — not yet implemented (stub)",
            peer
        )
    }

    async fn peers(&self) -> Vec<PeerInfo> {
        self.peers.read().await.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn iroh_stub_listen_and_stop() {
        let transport = IrohTransport::new();
        transport.listen().await.unwrap();
        transport.stop().await.unwrap();
    }

    #[tokio::test]
    async fn iroh_stub_connect_fails() {
        let transport = IrohTransport::new();
        let result = transport.connect(&PeerId("test".into())).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn iroh_stub_peers_empty() {
        let transport = IrohTransport::new();
        assert!(transport.peers().await.is_empty());
    }
}
