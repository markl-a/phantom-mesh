//! Three-layer auto-networking for Phantom Mesh cluster.
//!
//! - Layer 1: mDNS/LAN — zero-config local discovery (`_phantom_mesh._tcp.local`)
//! - Layer 2: Iroh/QUIC — NAT-traversing mesh transport (E2E encrypted)
//! - Layer 3: Cloud Relay — fallback relay with higher latency
//!
//! Each layer is pluggable via the [`ServiceDiscovery`] and [`MeshTransport`] traits.

pub mod discovery;
pub mod transport;
pub mod route_manager;
pub mod mdns;
pub mod iroh_transport;

pub use discovery::{ServiceDiscovery, DiscoveredNode, ConnectionLayer};
pub use transport::{MeshTransport, PeerInfo, TransportChannel, PeerId};
pub use route_manager::{RouteManager, ResolvedRoute, RouteEntry};
