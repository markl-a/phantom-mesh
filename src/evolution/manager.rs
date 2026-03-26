//! M5.4 EvolutionManager — orchestrates all evolution subsystems.
//!
//! Coordinates the AutoSkillInstaller, ArchitectureAdaptor, and PackageRegistry
//! to provide a unified evolution cycle: analyze metrics, suggest adaptations,
//! auto-install missing capabilities, and manage approval workflows.

use crate::evolution::architecture_adaptor::{Adaptation, ArchitectureAdaptor, SystemMetrics};
use crate::evolution::auto_installer::{AutoInstallConfig, AutoInstallError, AutoSkillInstaller, CapabilityResult};
use crate::evolution::registry::PackageRegistry;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvolutionConfig {
    pub auto_check_interval_secs: u64,
    pub auto_install_minor: bool,
    pub auto_install_major: bool,
    pub registries: Vec<String>,
}

impl Default for EvolutionConfig {
    fn default() -> Self {
        Self {
            auto_check_interval_secs: 21600, // 6 hours
            auto_install_minor: true,
            auto_install_major: false,
            registries: vec!["https://registry.phantom-mesh.dev".to_string()],
        }
    }
}

// ---------------------------------------------------------------------------
// Status snapshot
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct EvolutionStatus {
    pub last_check_at: u64,
    pub pending_adaptations: usize,
    pub applied_adaptations: usize,
    pub installed_today: u32,
    pub enabled: bool,
}

// ---------------------------------------------------------------------------
// EvolutionManager
// ---------------------------------------------------------------------------

/// Orchestrates all evolution subsystems.
pub struct EvolutionManager {
    config: EvolutionConfig,
    installer: AutoSkillInstaller,
    adaptor: Arc<RwLock<ArchitectureAdaptor>>,
    last_check_at: AtomicU64,
}

impl EvolutionManager {
    pub fn new(config: EvolutionConfig, install_config: AutoInstallConfig) -> Self {
        Self {
            config,
            installer: AutoSkillInstaller::new(install_config),
            adaptor: Arc::new(RwLock::new(ArchitectureAdaptor::new())),
            last_check_at: AtomicU64::new(0),
        }
    }

    /// Run a full evolution cycle: analyze metrics + suggest adaptations.
    pub async fn evolution_cycle(&self, metrics: &SystemMetrics) -> Vec<Adaptation> {
        let mut adaptor = self.adaptor.write().await;
        let adaptations = adaptor.analyze(metrics);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.last_check_at.store(now, Ordering::Relaxed);
        info!(
            "EvolutionManager: cycle produced {} adaptations",
            adaptations.len()
        );
        adaptations
    }

    /// Ensure a capability is available, auto-installing if needed.
    pub async fn ensure_capability(
        &self,
        registry: &dyn PackageRegistry,
        required: &str,
        local_caps: &[String],
    ) -> Result<CapabilityResult, AutoInstallError> {
        self.installer
            .ensure_capability(registry, required, local_caps)
            .await
    }

    /// Get current evolution status.
    pub async fn status(&self) -> EvolutionStatus {
        let adaptor = self.adaptor.read().await;
        EvolutionStatus {
            last_check_at: self.last_check_at.load(Ordering::Relaxed),
            pending_adaptations: adaptor.pending_approvals().len(),
            applied_adaptations: adaptor.applied_adaptations().len(),
            installed_today: self.installer.installs_today(),
            enabled: self.config.auto_install_minor || self.config.auto_install_major,
        }
    }

    /// Approve a pending adaptation.
    pub async fn approve_adaptation(&self, id: u64) -> bool {
        self.adaptor.write().await.approve(id)
    }

    /// Reject a pending adaptation.
    pub async fn reject_adaptation(&self, id: u64) -> bool {
        self.adaptor.write().await.reject(id)
    }

    /// Get the config.
    pub fn config(&self) -> &EvolutionConfig {
        &self.config
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evolution::auto_installer::AutoInstallConfig;

    fn default_evolution_config() -> EvolutionConfig {
        EvolutionConfig::default()
    }

    fn default_install_config() -> AutoInstallConfig {
        AutoInstallConfig::default()
    }

    #[tokio::test]
    async fn test_evolution_cycle() {
        let manager = EvolutionManager::new(default_evolution_config(), default_install_config());
        let mut metrics = SystemMetrics::default();

        // Healthy system — no adaptations
        let results = manager.evolution_cycle(&metrics).await;
        assert!(results.is_empty());

        // Add a missing capability — should produce an InstallCapability adaptation
        metrics.missing_capabilities.push("vision".to_string());
        let results = manager.evolution_cycle(&metrics).await;
        assert!(!results.is_empty());
        assert!(results.iter().any(
            |a| matches!(a, Adaptation::InstallCapability { capability } if capability == "vision")
        ));

        // Add provider failures — should produce DisableProvider adaptation
        metrics
            .provider_failures
            .insert("bad-provider".to_string(), 20);
        let results = manager.evolution_cycle(&metrics).await;
        assert!(results.iter().any(
            |a| matches!(a, Adaptation::DisableProvider { provider, .. } if provider == "bad-provider")
        ));
    }

    #[tokio::test]
    async fn test_status() {
        let config = EvolutionConfig {
            auto_install_minor: true,
            auto_install_major: false,
            ..default_evolution_config()
        };
        let manager = EvolutionManager::new(config, default_install_config());

        let status = manager.status().await;
        assert_eq!(status.last_check_at, 0);
        assert_eq!(status.pending_adaptations, 0);
        assert_eq!(status.applied_adaptations, 0);
        assert_eq!(status.installed_today, 0);
        assert!(status.enabled); // auto_install_minor is true

        // Run a cycle with metrics that produce adaptations
        let mut metrics = SystemMetrics::default();
        metrics.missing_capabilities.push("ocr".to_string());
        manager.evolution_cycle(&metrics).await;

        let status = manager.status().await;
        // last_check_at should now be non-zero after evolution_cycle
        assert!(status.last_check_at > 0, "last_check_at should be updated after evolution_cycle");
        // InstallCapability is Normal risk => goes to pending
        assert_eq!(status.pending_adaptations, 1);
        assert_eq!(status.applied_adaptations, 0);

        // Verify disabled config
        let disabled_config = EvolutionConfig {
            auto_install_minor: false,
            auto_install_major: false,
            ..default_evolution_config()
        };
        let disabled_manager =
            EvolutionManager::new(disabled_config, default_install_config());
        let status = disabled_manager.status().await;
        assert!(!status.enabled);
    }

    #[tokio::test]
    async fn test_approve_reject_via_manager() {
        let manager = EvolutionManager::new(default_evolution_config(), default_install_config());

        // Generate a Normal adaptation (pending approval)
        let mut metrics = SystemMetrics::default();
        metrics.missing_capabilities.push("speech".to_string());
        manager.evolution_cycle(&metrics).await;

        let status = manager.status().await;
        assert_eq!(status.pending_adaptations, 1);

        // Approve the pending adaptation (id = 1, first generated)
        assert!(manager.approve_adaptation(1).await);
        let status = manager.status().await;
        assert_eq!(status.pending_adaptations, 0);
        assert_eq!(status.applied_adaptations, 1);

        // Non-existent id returns false
        assert!(!manager.approve_adaptation(999).await);

        // Generate another pending adaptation and reject it
        metrics.missing_capabilities.clear();
        metrics
            .missing_capabilities
            .push("translate".to_string());
        manager.evolution_cycle(&metrics).await;

        let status = manager.status().await;
        assert_eq!(status.pending_adaptations, 1);

        // The next id should be 2 (since first cycle already used id=1)
        assert!(manager.reject_adaptation(2).await);
        let status = manager.status().await;
        assert_eq!(status.pending_adaptations, 0);
        // Rejected adaptations don't go to applied
        assert_eq!(status.applied_adaptations, 1);

        // Non-existent reject returns false
        assert!(!manager.reject_adaptation(999).await);
    }
}
