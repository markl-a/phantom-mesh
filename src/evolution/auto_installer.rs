//! M5.2 AutoSkillInstaller — capability-driven auto-installation of skills/plugins.
//!
//! When an agent needs a capability it doesn't have, AutoSkillInstaller searches
//! the package registry and installs verified packages automatically (or flags
//! community packages for approval).

use crate::evolution::registry::{PackageInfo, PackageRegistry, RegistryError};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::info;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum AutoInstallError {
    #[error("auto-install disabled")]
    Disabled,
    #[error("daily install limit reached ({0})")]
    DailyLimitReached(u32),
    #[error("registry error: {0}")]
    Registry(#[from] RegistryError),
    #[error("approval required for community package: {0}")]
    NeedsApproval(String),
}

pub type AutoInstallResult<T> = Result<T, AutoInstallError>;

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoInstallConfig {
    pub enabled: bool,
    pub auto_install_verified: bool,
    pub auto_install_community: bool,
    pub max_installs_per_day: u32,
}

impl Default for AutoInstallConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_install_verified: true,
            auto_install_community: false,
            max_installs_per_day: 10,
        }
    }
}

// ---------------------------------------------------------------------------
// Result enum
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum CapabilityResult {
    /// The capability is already present locally.
    AlreadyInstalled,
    /// A package was automatically installed (contains the package id).
    AutoInstalled(String),
    /// A community package was found but requires manual approval (contains the package id).
    NeedsApproval(String),
    /// No package in the registry provides this capability.
    NotAvailable,
}

// ---------------------------------------------------------------------------
// AutoSkillInstaller
// ---------------------------------------------------------------------------

pub struct AutoSkillInstaller {
    config: AutoInstallConfig,
    installs_today: AtomicU32,
    installed_packages: Arc<RwLock<Vec<(String, Vec<String>)>>>,
}

impl AutoSkillInstaller {
    pub fn new(config: AutoInstallConfig) -> Self {
        Self {
            config,
            installs_today: AtomicU32::new(0),
            installed_packages: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Check if a capability is available locally. If not, search the registry
    /// and auto-install if policy allows.
    pub async fn ensure_capability(
        &self,
        registry: &dyn PackageRegistry,
        required: &str,
        local_capabilities: &[String],
    ) -> AutoInstallResult<CapabilityResult> {
        if !self.config.enabled {
            return Err(AutoInstallError::Disabled);
        }

        // Already present locally
        if local_capabilities.iter().any(|c| c == required) {
            return Ok(CapabilityResult::AlreadyInstalled);
        }

        // Already installed this session (check if any installed package provides the capability)
        {
            let installed = self.installed_packages.read().await;
            if installed.iter().any(|(_, caps)| caps.iter().any(|c| c == required)) {
                return Ok(CapabilityResult::AlreadyInstalled);
            }
        }

        // Search registry
        let index = registry.fetch_index().await?;
        let matches: Vec<&PackageInfo> = index.search_by_capability(required);

        if matches.is_empty() {
            return Ok(CapabilityResult::NotAvailable);
        }

        // Pick the best match (prefer verified, then first result)
        let best = matches
            .iter()
            .find(|p| p.verified)
            .or(matches.first())
            .unwrap(); // safe: matches is non-empty

        // Decision: auto-install or needs approval
        if best.verified && self.config.auto_install_verified {
            // Atomic quota reservation via compare_exchange loop
            loop {
                let current = self.installs_today.load(Ordering::Relaxed);
                if current >= self.config.max_installs_per_day {
                    return Err(AutoInstallError::DailyLimitReached(self.config.max_installs_per_day));
                }
                if self.installs_today.compare_exchange(current, current + 1, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
                    break;
                }
            }
            self.installed_packages.write().await.push((best.id.clone(), best.capabilities.clone()));
            info!(
                "AutoSkillInstaller: auto-installed verified package '{}'",
                best.id
            );
            Ok(CapabilityResult::AutoInstalled(best.id.clone()))
        } else if !best.verified && self.config.auto_install_community {
            // Atomic quota reservation via compare_exchange loop
            loop {
                let current = self.installs_today.load(Ordering::Relaxed);
                if current >= self.config.max_installs_per_day {
                    return Err(AutoInstallError::DailyLimitReached(self.config.max_installs_per_day));
                }
                if self.installs_today.compare_exchange(current, current + 1, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
                    break;
                }
            }
            self.installed_packages.write().await.push((best.id.clone(), best.capabilities.clone()));
            info!(
                "AutoSkillInstaller: auto-installed community package '{}'",
                best.id
            );
            Ok(CapabilityResult::AutoInstalled(best.id.clone()))
        } else {
            // Either verified-but-disabled or community-but-disabled: needs approval
            Ok(CapabilityResult::NeedsApproval(best.id.clone()))
        }
    }

    /// Reset the daily counter (call at midnight or start of day).
    pub fn reset_daily(&self) {
        self.installs_today.store(0, Ordering::Relaxed);
    }

    /// Get the number of installs performed today.
    pub fn installs_today(&self) -> u32 {
        self.installs_today.load(Ordering::Relaxed)
    }

    /// Get the list of package IDs installed this session.
    pub async fn installed_packages(&self) -> Vec<String> {
        self.installed_packages.read().await.iter().map(|(id, _)| id.clone()).collect()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evolution::registry::{
        PackageInfo, PackageRegistry, PackageType, PackageVersion, RegistryError, RegistryIndex,
        RegistryResult,
    };
    use async_trait::async_trait;

    // -- Mock registry -------------------------------------------------------

    struct MockRegistry {
        index: RegistryIndex,
    }

    impl MockRegistry {
        fn new() -> Self {
            let packages = vec![
                PackageInfo {
                    id: "verified-ocr".into(),
                    name: "OCR Tool".into(),
                    description: "Optical character recognition".into(),
                    package_type: PackageType::Skill,
                    capabilities: vec!["ocr".into(), "image-to-text".into()],
                    author: "phantom_mesh-team".into(),
                    verified: true,
                    versions: vec![PackageVersion {
                        version: "1.0.0".into(),
                        checksum_sha256: "abc123".into(),
                        download_url: "https://example.com/ocr-1.0.0.tar.gz".into(),
                        size_bytes: 1024,
                        released_at: "2026-01-01".into(),
                    }],
                },
                PackageInfo {
                    id: "community-translate".into(),
                    name: "Translation Pack".into(),
                    description: "Multi-language translation".into(),
                    package_type: PackageType::Plugin,
                    capabilities: vec!["translate".into(), "language-detect".into()],
                    author: "community-user".into(),
                    verified: false,
                    versions: vec![PackageVersion {
                        version: "0.3.1".into(),
                        checksum_sha256: "def456".into(),
                        download_url: "https://example.com/translate-0.3.1.tar.gz".into(),
                        size_bytes: 2048,
                        released_at: "2026-02-15".into(),
                    }],
                },
                PackageInfo {
                    id: "verified-tts".into(),
                    name: "Text-to-Speech".into(),
                    description: "TTS engine".into(),
                    package_type: PackageType::Skill,
                    capabilities: vec!["tts".into(), "speech-synthesis".into()],
                    author: "phantom_mesh-team".into(),
                    verified: true,
                    versions: vec![PackageVersion {
                        version: "2.1.0".into(),
                        checksum_sha256: "ghi789".into(),
                        download_url: "https://example.com/tts-2.1.0.tar.gz".into(),
                        size_bytes: 4096,
                        released_at: "2026-03-01".into(),
                    }],
                },
            ];

            Self {
                index: RegistryIndex {
                    packages,
                    updated_at: "2026-03-22T00:00:00Z".into(),
                },
            }
        }
    }

    #[async_trait]
    impl PackageRegistry for MockRegistry {
        async fn fetch_index(&self) -> RegistryResult<RegistryIndex> {
            Ok(self.index.clone())
        }

        async fn download(&self, _id: &str, _version: &str) -> RegistryResult<Vec<u8>> {
            Ok(vec![])
        }

        async fn verify(&self, _data: &[u8], _expected_sha256: &str) -> RegistryResult<bool> {
            Ok(true)
        }
    }

    // -- Helpers --------------------------------------------------------------

    fn default_config() -> AutoInstallConfig {
        AutoInstallConfig::default()
    }

    fn local(caps: &[&str]) -> Vec<String> {
        caps.iter().map(|s| s.to_string()).collect()
    }

    // -- Tests ----------------------------------------------------------------

    #[tokio::test]
    async fn already_installed() {
        let installer = AutoSkillInstaller::new(default_config());
        let registry = MockRegistry::new();
        let result = installer
            .ensure_capability(&registry, "ocr", &local(&["ocr", "tts"]))
            .await
            .unwrap();
        assert_eq!(result, CapabilityResult::AlreadyInstalled);
    }

    #[tokio::test]
    async fn auto_install_verified() {
        let installer = AutoSkillInstaller::new(default_config());
        let registry = MockRegistry::new();
        let result = installer
            .ensure_capability(&registry, "ocr", &local(&[]))
            .await
            .unwrap();
        assert_eq!(
            result,
            CapabilityResult::AutoInstalled("verified-ocr".into())
        );
        assert_eq!(installer.installs_today(), 1);
        let pkgs = installer.installed_packages().await;
        assert!(pkgs.contains(&"verified-ocr".to_string()));
    }

    #[tokio::test]
    async fn community_needs_approval() {
        let config = AutoInstallConfig {
            auto_install_community: false,
            ..default_config()
        };
        let installer = AutoSkillInstaller::new(config);
        let registry = MockRegistry::new();
        let result = installer
            .ensure_capability(&registry, "translate", &local(&[]))
            .await
            .unwrap();
        assert_eq!(
            result,
            CapabilityResult::NeedsApproval("community-translate".into())
        );
        assert_eq!(installer.installs_today(), 0);
    }

    #[tokio::test]
    async fn community_auto_install() {
        let config = AutoInstallConfig {
            auto_install_community: true,
            ..default_config()
        };
        let installer = AutoSkillInstaller::new(config);
        let registry = MockRegistry::new();
        let result = installer
            .ensure_capability(&registry, "translate", &local(&[]))
            .await
            .unwrap();
        assert_eq!(
            result,
            CapabilityResult::AutoInstalled("community-translate".into())
        );
        assert_eq!(installer.installs_today(), 1);
    }

    #[tokio::test]
    async fn not_available() {
        let installer = AutoSkillInstaller::new(default_config());
        let registry = MockRegistry::new();
        let result = installer
            .ensure_capability(&registry, "quantum-computing", &local(&[]))
            .await
            .unwrap();
        assert_eq!(result, CapabilityResult::NotAvailable);
    }

    #[tokio::test]
    async fn daily_limit() {
        let config = AutoInstallConfig {
            max_installs_per_day: 2,
            auto_install_community: true,
            ..default_config()
        };
        let installer = AutoSkillInstaller::new(config);
        let registry = MockRegistry::new();

        // Install 1: "ocr" -> verified-ocr (capabilities: ["ocr", "image-to-text"])
        installer
            .ensure_capability(&registry, "ocr", &local(&[]))
            .await
            .unwrap();
        // Install 2: "translate" -> community-translate (capabilities: ["translate", "language-detect"])
        installer
            .ensure_capability(&registry, "translate", &local(&[]))
            .await
            .unwrap();
        assert_eq!(installer.installs_today(), 2);

        // Install 3 — should hit limit.
        // "tts" maps to verified-tts which is not yet installed, so it will attempt
        // auto-install and hit the daily limit inside the CAS loop.
        let result = installer
            .ensure_capability(&registry, "tts", &local(&[]))
            .await;
        match result {
            Err(AutoInstallError::DailyLimitReached(2)) => {} // expected
            other => panic!("expected DailyLimitReached(2), got {:?}", other),
        }
    }

    #[tokio::test]
    async fn verified_but_disabled_needs_approval() {
        let config = AutoInstallConfig {
            auto_install_verified: false,
            ..default_config()
        };
        let installer = AutoSkillInstaller::new(config);
        let registry = MockRegistry::new();
        let result = installer
            .ensure_capability(&registry, "ocr", &local(&[]))
            .await
            .unwrap();
        assert_eq!(
            result,
            CapabilityResult::NeedsApproval("verified-ocr".into())
        );
        // NeedsApproval should not count against the daily quota
        assert_eq!(installer.installs_today(), 0);
    }

    #[tokio::test]
    async fn disabled() {
        let config = AutoInstallConfig {
            enabled: false,
            ..default_config()
        };
        let installer = AutoSkillInstaller::new(config);
        let registry = MockRegistry::new();
        let result = installer
            .ensure_capability(&registry, "ocr", &local(&[]))
            .await;
        match result {
            Err(AutoInstallError::Disabled) => {} // expected
            other => panic!("expected Disabled, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn reset_daily_counter() {
        let config = AutoInstallConfig {
            max_installs_per_day: 1,
            ..default_config()
        };
        let installer = AutoSkillInstaller::new(config);
        let registry = MockRegistry::new();

        // Install one
        installer
            .ensure_capability(&registry, "ocr", &local(&[]))
            .await
            .unwrap();
        assert_eq!(installer.installs_today(), 1);

        // Hit limit
        let result = installer
            .ensure_capability(&registry, "tts", &local(&[]))
            .await;
        assert!(matches!(result, Err(AutoInstallError::DailyLimitReached(1))));

        // Reset
        installer.reset_daily();
        assert_eq!(installer.installs_today(), 0);

        // Now install succeeds
        let result = installer
            .ensure_capability(&registry, "tts", &local(&[]))
            .await
            .unwrap();
        assert_eq!(
            result,
            CapabilityResult::AutoInstalled("verified-tts".into())
        );
        assert_eq!(installer.installs_today(), 1);
    }
}
