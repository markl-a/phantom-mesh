use chrono::{DateTime, Utc};
use std::path::PathBuf;

use super::reliable::ErrorClass;
pub use super::tier::ProviderTier;

// ── Credential Types ───────────────────────────────────────

#[derive(Debug, Clone)]
pub enum CredentialType {
    /// Standard API key (OpenAI, Anthropic, Groq, etc.)
    ApiKey { key: String },
    /// OAuth token with optional refresh (Codex, Copilot, gcloud)
    OAuth {
        access_token: String,
        refresh_token: Option<String>,
        expires_at: Option<DateTime<Utc>>,
    },
    /// Token read from a local file (Claude CLI, Copilot hosts.json)
    TokenFile {
        token: String,
        source_path: PathBuf,
    },
}

// ── Auth Profile ───────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct AuthProfile {
    pub provider_name: String,
    pub credential: CredentialType,
    pub tier: ProviderTier,
    pub usage_stats: ProfileUsageStats,
}

#[derive(Debug, Clone, Default)]
pub struct ProfileUsageStats {
    pub last_used: Option<DateTime<Utc>>,
    pub success_count: u64,
    pub failure_count: u64,
    pub last_error: Option<String>,
    pub cooldown_until: Option<DateTime<Utc>>,
}

// ── Failover Reason ────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub enum FailoverReason {
    Auth,
    Billing,
    RateLimit,
    Overload,
    ContextOverflow,
    Timeout,
}

impl FailoverReason {
    /// Convert to existing ErrorClass for retry logic compatibility
    pub fn to_error_class(&self) -> ErrorClass {
        match self {
            Self::Auth | Self::Billing => ErrorClass::NonRetryable,
            Self::RateLimit => ErrorClass::RateLimited,
            Self::Overload | Self::ContextOverflow | Self::Timeout => ErrorClass::Transient,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::reliable::ErrorClass;

    #[test]
    fn failover_auth_is_non_retryable() {
        assert!(matches!(
            FailoverReason::Auth.to_error_class(),
            ErrorClass::NonRetryable
        ));
    }

    #[test]
    fn failover_billing_is_non_retryable() {
        assert!(matches!(
            FailoverReason::Billing.to_error_class(),
            ErrorClass::NonRetryable
        ));
    }

    #[test]
    fn failover_rate_limit_is_rate_limited() {
        assert!(matches!(
            FailoverReason::RateLimit.to_error_class(),
            ErrorClass::RateLimited
        ));
    }

    #[test]
    fn failover_overload_is_transient() {
        assert!(matches!(
            FailoverReason::Overload.to_error_class(),
            ErrorClass::Transient
        ));
    }

    #[test]
    fn failover_context_overflow_is_transient() {
        assert!(matches!(
            FailoverReason::ContextOverflow.to_error_class(),
            ErrorClass::Transient
        ));
    }

    #[test]
    fn failover_timeout_is_transient() {
        assert!(matches!(
            FailoverReason::Timeout.to_error_class(),
            ErrorClass::Transient
        ));
    }

    #[test]
    fn profile_usage_stats_default() {
        let stats = ProfileUsageStats::default();
        assert_eq!(stats.success_count, 0);
        assert_eq!(stats.failure_count, 0);
        assert!(stats.last_used.is_none());
        assert!(stats.cooldown_until.is_none());
    }

    #[test]
    fn provider_tier_serde_json_roundtrip() {
        let tier = ProviderTier::PayAsYouGo;
        let json = serde_json::to_string(&tier).unwrap();
        assert_eq!(json, "\"payg\"");
        let back: ProviderTier = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, ProviderTier::PayAsYouGo));
    }

    #[test]
    fn provider_tier_serde_alias() {
        // Old format should still deserialize
        let back: ProviderTier = serde_json::from_str("\"FreeApi\"").unwrap();
        assert!(matches!(back, ProviderTier::FreeApi));
    }
}
