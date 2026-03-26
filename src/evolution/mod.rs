pub mod registry;
pub mod auto_installer;
pub use registry::{PackageRegistry, PackageInfo, PackageVersion, RegistryIndex, HttpRegistry, LocalRegistry};
pub use auto_installer::{AutoSkillInstaller, AutoInstallConfig, AutoInstallError, AutoInstallResult, CapabilityResult};

pub mod architecture_adaptor;
pub use architecture_adaptor::{ArchitectureAdaptor, Adaptation, AdaptationRisk, SystemMetrics, PendingAdaptation};

pub mod manager;
pub use manager::{EvolutionManager, EvolutionConfig, EvolutionStatus};

pub mod cluster_sync;
pub use cluster_sync::{CapabilitySyncManager, CapabilitySyncMessage, NodeManifest};
