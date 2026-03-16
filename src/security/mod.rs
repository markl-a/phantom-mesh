pub mod secrets;
pub mod autonomy;
pub mod roles;
pub mod privacy;
pub use secrets::SecretManager;
pub use autonomy::AutonomyLevel;
pub use roles::{Role, RoleRegistry};
pub use privacy::{PrivacyGuard, PrivacyConfig, PrivacyTier};
