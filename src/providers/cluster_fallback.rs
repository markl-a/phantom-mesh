//! Cluster-aware provider fallback.
//!
//! When a node doesn't have a specific LLM provider's API key,
//! it can fall back to asking another node in the cluster to run
//! the request via RPC.

use serde::{Serialize, Deserialize};

/// Provider availability info for a node (shared via heartbeat).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderAvailability {
    pub node_id: String,
    pub providers: Vec<String>,  // provider names available on this node
}

/// Find which nodes can handle a request for a given provider.
pub fn nodes_with_provider(
    availability: &[ProviderAvailability],
    provider_name: &str,
) -> Vec<String> {
    availability.iter()
        .filter(|a| a.providers.iter().any(|p| p == provider_name))
        .map(|a| a.node_id.clone())
        .collect()
}
