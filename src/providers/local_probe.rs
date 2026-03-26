//! Local LLM Latency Probe — periodic measurement of local model response times.
//!
//! Probes local LLM providers (Ollama, llama.cpp, etc.) with a minimal request
//! and classifies the speed as Fast/Medium/Slow. This feeds into the TierRouter
//! to decide whether local models should be prioritized over free-tier cloud APIs.
//!
//! # Probe cycle
//!
//! `LocalProbeManager` runs a configurable probe cycle (default 10 minutes).
//! Each cycle sends a trivial prompt to each registered local provider and
//! measures the round-trip time.

use super::tier::LocalSpeed;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

// ---------------------------------------------------------------------------
// LocalProbe
// ---------------------------------------------------------------------------

/// Probe state for a single local LLM provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalProbe {
    /// Provider name (e.g. "ollama")
    pub provider_name: String,
    /// Endpoint URL for probing (e.g. "http://127.0.0.1:11434")
    pub endpoint: String,
    /// Last measured latency in milliseconds (0 = not yet probed)
    pub last_latency_ms: u64,
    /// Unix timestamp of last successful probe
    pub last_probed: u64,
    /// Classified speed from last probe
    pub speed: LocalSpeed,
}

impl LocalProbe {
    pub fn new(provider_name: &str, endpoint: &str) -> Self {
        Self {
            provider_name: provider_name.to_string(),
            endpoint: endpoint.to_string(),
            last_latency_ms: 0,
            last_probed: 0,
            speed: LocalSpeed::Unknown,
        }
    }

    /// Update probe results with a new latency measurement.
    pub fn update(&mut self, latency_ms: u64) {
        self.last_latency_ms = latency_ms;
        self.last_probed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.speed = LocalSpeed::from_latency_ms(latency_ms);
        debug!(
            "LocalProbe [{}]: {}ms → {:?}",
            self.provider_name, latency_ms, self.speed
        );
    }

    /// Mark probe as failed (slow/unknown).
    pub fn mark_failed(&mut self) {
        self.speed = LocalSpeed::Slow;
        self.last_latency_ms = u64::MAX;
        self.last_probed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        warn!("LocalProbe [{}]: probe failed, marking as Slow", self.provider_name);
    }

    /// Whether this probe is stale (older than `max_age_secs`).
    pub fn is_stale(&self, max_age_secs: u64) -> bool {
        if self.last_probed == 0 {
            return true;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        now.saturating_sub(self.last_probed) > max_age_secs
    }
}

// ---------------------------------------------------------------------------
// LocalProbeManager
// ---------------------------------------------------------------------------

/// Manages probes for all local LLM providers.
pub struct LocalProbeManager {
    probes: Arc<RwLock<HashMap<String, LocalProbe>>>,
    /// Probe interval in seconds (default: 600 = 10 minutes)
    pub probe_interval_secs: u64,
    client: reqwest::Client,
}

impl LocalProbeManager {
    pub fn new(probe_interval_secs: u64) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_default();
        Self {
            probes: Arc::new(RwLock::new(HashMap::new())),
            probe_interval_secs,
            client,
        }
    }

    /// Register a local provider for probing.
    pub async fn register(&self, provider_name: &str, endpoint: &str) {
        let probe = LocalProbe::new(provider_name, endpoint);
        self.probes.write().await.insert(provider_name.to_string(), probe);
        info!("LocalProbeManager: registered '{}'", provider_name);
    }

    /// Remove a provider from probing.
    pub async fn unregister(&self, provider_name: &str) -> bool {
        self.probes.write().await.remove(provider_name).is_some()
    }

    /// Get the current classified speed for a provider.
    pub async fn current_speed(&self, provider_name: &str) -> LocalSpeed {
        let probes = self.probes.read().await;
        probes
            .get(provider_name)
            .map(|p| p.speed)
            .unwrap_or(LocalSpeed::Unknown)
    }

    /// Get the overall speed — uses the fastest local provider.
    pub async fn overall_speed(&self) -> LocalSpeed {
        let probes = self.probes.read().await;
        let mut best = LocalSpeed::Unknown;
        for probe in probes.values() {
            match (best, probe.speed) {
                (LocalSpeed::Unknown, s) => best = s,
                (_, LocalSpeed::Fast) => return LocalSpeed::Fast,
                (LocalSpeed::Slow, LocalSpeed::Medium) => best = LocalSpeed::Medium,
                _ => {}
            }
        }
        best
    }

    /// Update a provider's probe result.
    pub async fn update_probe(&self, provider_name: &str, latency_ms: u64) {
        let mut probes = self.probes.write().await;
        if let Some(probe) = probes.get_mut(provider_name) {
            probe.update(latency_ms);
        }
    }

    /// Mark a provider's probe as failed.
    pub async fn mark_failed(&self, provider_name: &str) {
        let mut probes = self.probes.write().await;
        if let Some(probe) = probes.get_mut(provider_name) {
            probe.mark_failed();
        }
    }

    /// Get all probe states (for diagnostics).
    pub async fn all_probes(&self) -> Vec<LocalProbe> {
        self.probes.read().await.values().cloned().collect()
    }

    /// Get providers that need re-probing (stale probes).
    pub async fn stale_providers(&self) -> Vec<String> {
        let probes = self.probes.read().await;
        probes
            .values()
            .filter(|p| p.is_stale(self.probe_interval_secs))
            .map(|p| p.provider_name.clone())
            .collect()
    }

    /// Run a single probe against a local provider endpoint.
    /// Sends a minimal request and measures round-trip time.
    pub async fn probe_endpoint(client: &reqwest::Client, endpoint: &str) -> Result<u64, String> {
        let start = Instant::now();

        // Ollama-compatible health check: GET /api/tags
        let url = format!("{}/api/tags", endpoint.trim_end_matches('/'));
        let resp = client.get(&url).send().await.map_err(|e| e.to_string())?;

        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status()));
        }

        // Consume body to measure full round-trip
        let _ = resp.bytes().await.map_err(|e| e.to_string())?;

        let elapsed = start.elapsed().as_millis() as u64;
        Ok(elapsed)
    }

    /// Probe all registered providers and update their states.
    pub async fn probe_all(&self) {
        let endpoints: Vec<(String, String)> = {
            let probes = self.probes.read().await;
            probes
                .values()
                .map(|p| (p.provider_name.clone(), p.endpoint.clone()))
                .collect()
        };

        for (name, endpoint) in endpoints {
            match Self::probe_endpoint(&self.client, &endpoint).await {
                Ok(latency_ms) => {
                    self.update_probe(&name, latency_ms).await;
                }
                Err(e) => {
                    warn!("LocalProbe [{}] failed: {}", name, e);
                    self.mark_failed(&name).await;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_probe_update() {
        let mut probe = LocalProbe::new("ollama", "http://127.0.0.1:11434");
        assert_eq!(probe.speed, LocalSpeed::Unknown);
        assert_eq!(probe.last_latency_ms, 0);

        probe.update(200);
        assert_eq!(probe.speed, LocalSpeed::Fast);
        assert_eq!(probe.last_latency_ms, 200);
        assert!(probe.last_probed > 0);
    }

    #[test]
    fn local_probe_mark_failed() {
        let mut probe = LocalProbe::new("ollama", "http://127.0.0.1:11434");
        probe.update(100);
        assert_eq!(probe.speed, LocalSpeed::Fast);

        probe.mark_failed();
        assert_eq!(probe.speed, LocalSpeed::Slow);
    }

    #[test]
    fn probe_staleness() {
        let mut probe = LocalProbe::new("ollama", "http://127.0.0.1:11434");
        assert!(probe.is_stale(600)); // never probed = stale

        probe.update(100);
        assert!(!probe.is_stale(600)); // just probed

        // Simulate old probe
        probe.last_probed -= 700;
        assert!(probe.is_stale(600));
    }

    #[test]
    fn speed_classification_thresholds() {
        let mut probe = LocalProbe::new("test", "http://test");

        probe.update(0);
        assert_eq!(probe.speed, LocalSpeed::Fast);

        probe.update(499);
        assert_eq!(probe.speed, LocalSpeed::Fast);

        probe.update(500);
        assert_eq!(probe.speed, LocalSpeed::Medium);

        probe.update(3000);
        assert_eq!(probe.speed, LocalSpeed::Medium);

        probe.update(3001);
        assert_eq!(probe.speed, LocalSpeed::Slow);
    }

    #[tokio::test]
    async fn manager_register_and_speed() {
        let mgr = LocalProbeManager::new(600);
        mgr.register("ollama", "http://127.0.0.1:11434").await;

        assert_eq!(mgr.current_speed("ollama").await, LocalSpeed::Unknown);
        assert_eq!(mgr.current_speed("nonexistent").await, LocalSpeed::Unknown);

        mgr.update_probe("ollama", 300).await;
        assert_eq!(mgr.current_speed("ollama").await, LocalSpeed::Fast);
    }

    #[tokio::test]
    async fn manager_overall_speed_picks_fastest() {
        let mgr = LocalProbeManager::new(600);
        mgr.register("ollama", "http://127.0.0.1:11434").await;
        mgr.register("llamacpp", "http://127.0.0.1:8080").await;

        // Both unknown
        assert_eq!(mgr.overall_speed().await, LocalSpeed::Unknown);

        // One slow, one fast → Fast
        mgr.update_probe("ollama", 5000).await;
        mgr.update_probe("llamacpp", 200).await;
        assert_eq!(mgr.overall_speed().await, LocalSpeed::Fast);
    }

    #[tokio::test]
    async fn manager_unregister() {
        let mgr = LocalProbeManager::new(600);
        mgr.register("ollama", "http://127.0.0.1:11434").await;
        assert_eq!(mgr.all_probes().await.len(), 1);

        assert!(mgr.unregister("ollama").await);
        assert!(!mgr.unregister("ollama").await); // already removed
        assert_eq!(mgr.all_probes().await.len(), 0);
    }

    #[tokio::test]
    async fn manager_stale_providers() {
        let mgr = LocalProbeManager::new(600);
        mgr.register("ollama", "http://127.0.0.1:11434").await;
        mgr.register("llamacpp", "http://127.0.0.1:8080").await;

        // Both are stale (never probed)
        let stale = mgr.stale_providers().await;
        assert_eq!(stale.len(), 2);

        // Probe one
        mgr.update_probe("ollama", 200).await;
        let stale = mgr.stale_providers().await;
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0], "llamacpp");
    }

    #[tokio::test]
    async fn manager_mark_failed() {
        let mgr = LocalProbeManager::new(600);
        mgr.register("ollama", "http://127.0.0.1:11434").await;
        mgr.update_probe("ollama", 100).await;
        assert_eq!(mgr.current_speed("ollama").await, LocalSpeed::Fast);

        mgr.mark_failed("ollama").await;
        assert_eq!(mgr.current_speed("ollama").await, LocalSpeed::Slow);
    }
}
