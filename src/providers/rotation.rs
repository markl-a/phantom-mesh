//! Provider Rotation Engine — tracks cooldown state per provider,
//! automatically switches to next available provider on rate limit.
//! Uses exponential backoff with configurable limits.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;
use serde::Serialize;
use tracing::{debug, info, warn};

/// Configuration for the rotation engine
#[derive(Debug, Clone)]
pub struct RotationConfig {
    /// Base cooldown duration in seconds (default: 60)
    pub base_cooldown_secs: u64,
    /// Maximum cooldown duration in seconds (default: 600)
    pub max_cooldown_secs: u64,
    /// Backoff multiplier (default: 2.0)
    pub backoff_multiplier: f64,
    /// Priority order for provider selection
    pub priority_order: Vec<String>,
}

impl Default for RotationConfig {
    fn default() -> Self {
        Self {
            base_cooldown_secs: 15,
            max_cooldown_secs: 120,
            backoff_multiplier: 2.0,
            priority_order: Vec::new(),
        }
    }
}

/// Internal cooldown state for a single provider
#[derive(Debug)]
struct ProviderCooldown {
    /// When the cooldown expires (None = not cooling down)
    until: Option<Instant>,
    /// Number of consecutive rate-limit hits (for exponential backoff)
    consecutive_hits: u32,
    /// Total successful calls
    success_count: u64,
    /// Total rate-limit events
    rate_limit_count: u64,
}

impl Default for ProviderCooldown {
    fn default() -> Self {
        Self {
            until: None,
            consecutive_hits: 0,
            success_count: 0,
            rate_limit_count: 0,
        }
    }
}

/// Status snapshot for a single provider (for /status endpoint)
#[derive(Debug, Clone, Serialize)]
pub struct ProviderRotationStatus {
    pub provider: String,
    pub cooling_down: bool,
    pub cooldown_remaining_secs: u64,
    pub consecutive_hits: u32,
    pub success_count: u64,
    pub rate_limit_count: u64,
}

/// Provider Rotation Engine — tracks cooldowns and selects available providers.
pub struct ProviderRotation {
    cooldowns: Mutex<HashMap<String, ProviderCooldown>>,
    config: RotationConfig,
}

impl ProviderRotation {
    pub fn new(config: RotationConfig) -> Self {
        Self {
            cooldowns: Mutex::new(HashMap::new()),
            config,
        }
    }

    /// Record a successful call to a provider — resets consecutive hits.
    pub fn record_success(&self, provider: &str) {
        let mut map = self.cooldowns.lock().unwrap();
        let entry = map.entry(provider.to_string()).or_default();
        entry.consecutive_hits = 0;
        entry.until = None;
        entry.success_count += 1;
        debug!("Rotation: provider '{}' success (total: {})", provider, entry.success_count);
    }

    /// Record a rate-limit event — starts or extends cooldown with exponential backoff.
    pub fn record_rate_limit(&self, provider: &str) {
        let mut map = self.cooldowns.lock().unwrap();
        let entry = map.entry(provider.to_string()).or_default();
        entry.consecutive_hits += 1;
        entry.rate_limit_count += 1;

        // Exponential backoff: base * multiplier^(hits-1), capped at max
        let backoff_secs = (self.config.base_cooldown_secs as f64
            * self.config.backoff_multiplier.powi((entry.consecutive_hits - 1) as i32))
            as u64;
        let cooldown_secs = backoff_secs.min(self.config.max_cooldown_secs);

        entry.until = Some(Instant::now() + std::time::Duration::from_secs(cooldown_secs));

        warn!(
            "Rotation: provider '{}' rate-limited (hit #{}, cooldown {}s)",
            provider, entry.consecutive_hits, cooldown_secs
        );
    }

    /// Check if a provider is currently in cooldown.
    pub fn is_cooling_down(&self, provider: &str) -> bool {
        let map = self.cooldowns.lock().unwrap();
        if let Some(entry) = map.get(provider) {
            if let Some(until) = entry.until {
                return Instant::now() < until;
            }
        }
        false
    }

    /// Select the first available (non-cooling-down) provider from candidates.
    /// Respects priority_order if set, otherwise uses the order of candidates.
    pub fn select_available(&self, candidates: &[String]) -> Option<String> {
        let map = self.cooldowns.lock().unwrap();
        let now = Instant::now();

        // Use priority_order if configured, filtering to only candidates
        let ordered: Vec<&String> = if !self.config.priority_order.is_empty() {
            let mut ordered = Vec::new();
            for p in &self.config.priority_order {
                if candidates.contains(p) {
                    ordered.push(p);
                }
            }
            // Add any candidates not in priority_order at the end
            for c in candidates {
                if !ordered.contains(&c) {
                    ordered.push(c);
                }
            }
            ordered
        } else {
            candidates.iter().collect()
        };

        for provider in ordered {
            let cooling = if let Some(entry) = map.get(provider.as_str()) {
                entry.until.map(|u| now < u).unwrap_or(false)
            } else {
                false
            };
            if !cooling {
                debug!("Rotation: selected provider '{}'", provider);
                return Some(provider.clone());
            }
        }

        info!("Rotation: all {} candidates are cooling down", candidates.len());
        None
    }

    /// Get status of all tracked providers.
    pub fn status(&self) -> Vec<ProviderRotationStatus> {
        let map = self.cooldowns.lock().unwrap();
        let now = Instant::now();

        map.iter().map(|(name, entry)| {
            let (cooling, remaining) = if let Some(until) = entry.until {
                if now < until {
                    (true, (until - now).as_secs())
                } else {
                    (false, 0)
                }
            } else {
                (false, 0)
            };
            ProviderRotationStatus {
                provider: name.clone(),
                cooling_down: cooling,
                cooldown_remaining_secs: remaining,
                consecutive_hits: entry.consecutive_hits,
                success_count: entry.success_count,
                rate_limit_count: entry.rate_limit_count,
            }
        }).collect()
    }

    /// Get config reference
    pub fn config(&self) -> &RotationConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> RotationConfig {
        RotationConfig {
            base_cooldown_secs: 1, // Short for tests
            max_cooldown_secs: 10,
            backoff_multiplier: 2.0,
            priority_order: vec!["groq".into(), "deepseek".into(), "cerebras".into()],
        }
    }

    #[test]
    fn test_initial_state_not_cooling() {
        let rotation = ProviderRotation::new(test_config());
        assert!(!rotation.is_cooling_down("groq"));
        assert!(!rotation.is_cooling_down("unknown"));
    }

    #[test]
    fn test_rate_limit_starts_cooldown() {
        let rotation = ProviderRotation::new(test_config());
        rotation.record_rate_limit("groq");
        assert!(rotation.is_cooling_down("groq"));
    }

    #[test]
    fn test_cooldown_expires() {
        let mut config = test_config();
        config.base_cooldown_secs = 0; // Immediate expiry
        let rotation = ProviderRotation::new(config);
        rotation.record_rate_limit("groq");
        // With 0s cooldown, should expire immediately
        std::thread::sleep(std::time::Duration::from_millis(10));
        assert!(!rotation.is_cooling_down("groq"));
    }

    #[test]
    fn test_exponential_backoff() {
        let rotation = ProviderRotation::new(test_config());
        // First hit: base * 2^0 = 1s
        rotation.record_rate_limit("groq");
        {
            let map = rotation.cooldowns.lock().unwrap();
            assert_eq!(map["groq"].consecutive_hits, 1);
        }
        // Second hit: base * 2^1 = 2s
        rotation.record_rate_limit("groq");
        {
            let map = rotation.cooldowns.lock().unwrap();
            assert_eq!(map["groq"].consecutive_hits, 2);
        }
        // Third hit: base * 2^2 = 4s
        rotation.record_rate_limit("groq");
        {
            let map = rotation.cooldowns.lock().unwrap();
            assert_eq!(map["groq"].consecutive_hits, 3);
            assert_eq!(map["groq"].rate_limit_count, 3);
        }
    }

    #[test]
    fn test_max_cooldown_cap() {
        let mut config = test_config();
        config.base_cooldown_secs = 100;
        config.max_cooldown_secs = 10;
        let rotation = ProviderRotation::new(config);
        rotation.record_rate_limit("groq");
        // Should be capped at 10s, not 100s
        assert!(rotation.is_cooling_down("groq"));
    }

    #[test]
    fn test_success_resets_consecutive_hits() {
        let rotation = ProviderRotation::new(test_config());
        rotation.record_rate_limit("groq");
        rotation.record_rate_limit("groq");
        assert!(rotation.is_cooling_down("groq"));
        {
            let map = rotation.cooldowns.lock().unwrap();
            assert_eq!(map["groq"].consecutive_hits, 2);
        }
        rotation.record_success("groq");
        {
            let map = rotation.cooldowns.lock().unwrap();
            assert_eq!(map["groq"].consecutive_hits, 0);
            assert_eq!(map["groq"].success_count, 1);
        }
        assert!(!rotation.is_cooling_down("groq"));
    }

    #[test]
    fn test_select_available_skips_cooldown() {
        let rotation = ProviderRotation::new(test_config());
        let candidates = vec!["groq".into(), "deepseek".into(), "cerebras".into()];

        // Without cooldown, first priority wins
        assert_eq!(rotation.select_available(&candidates), Some("groq".into()));

        // Cool down groq → next in priority
        rotation.record_rate_limit("groq");
        assert_eq!(rotation.select_available(&candidates), Some("deepseek".into()));

        // Cool down deepseek too
        rotation.record_rate_limit("deepseek");
        assert_eq!(rotation.select_available(&candidates), Some("cerebras".into()));
    }

    #[test]
    fn test_select_available_all_cooling_returns_none() {
        let rotation = ProviderRotation::new(test_config());
        let candidates = vec!["groq".into(), "deepseek".into()];
        rotation.record_rate_limit("groq");
        rotation.record_rate_limit("deepseek");
        assert_eq!(rotation.select_available(&candidates), None);
    }

    #[test]
    fn test_select_available_respects_priority_order() {
        let rotation = ProviderRotation::new(test_config());
        // Candidates in different order than priority
        let candidates = vec!["cerebras".into(), "groq".into(), "deepseek".into()];
        // Priority: groq > deepseek > cerebras
        assert_eq!(rotation.select_available(&candidates), Some("groq".into()));
    }

    #[test]
    fn test_select_available_no_priority() {
        let config = RotationConfig {
            priority_order: Vec::new(),
            ..test_config()
        };
        let rotation = ProviderRotation::new(config);
        let candidates = vec!["alpha".into(), "beta".into()];
        // Without priority, uses candidate order
        assert_eq!(rotation.select_available(&candidates), Some("alpha".into()));
    }

    #[test]
    fn test_status_reports_all_providers() {
        let rotation = ProviderRotation::new(test_config());
        rotation.record_success("groq");
        rotation.record_rate_limit("deepseek");
        let status = rotation.status();
        assert_eq!(status.len(), 2);

        let groq_status = status.iter().find(|s| s.provider == "groq").unwrap();
        assert!(!groq_status.cooling_down);
        assert_eq!(groq_status.success_count, 1);

        let ds_status = status.iter().find(|s| s.provider == "deepseek").unwrap();
        assert!(ds_status.cooling_down);
        assert_eq!(ds_status.rate_limit_count, 1);
    }

    #[test]
    fn test_multiple_providers_independent() {
        let rotation = ProviderRotation::new(test_config());
        rotation.record_rate_limit("groq");
        rotation.record_success("deepseek");

        assert!(rotation.is_cooling_down("groq"));
        assert!(!rotation.is_cooling_down("deepseek"));
    }

    #[test]
    fn test_record_success_increments_count() {
        let rotation = ProviderRotation::new(test_config());
        rotation.record_success("groq");
        rotation.record_success("groq");
        rotation.record_success("groq");
        let map = rotation.cooldowns.lock().unwrap();
        assert_eq!(map["groq"].success_count, 3);
    }

    #[test]
    fn test_select_with_unknown_candidate() {
        let rotation = ProviderRotation::new(test_config());
        // "unknown" is not in priority_order but is a valid candidate
        let candidates = vec!["unknown".into()];
        assert_eq!(rotation.select_available(&candidates), Some("unknown".into()));
    }

    #[test]
    fn test_empty_candidates() {
        let rotation = ProviderRotation::new(test_config());
        assert_eq!(rotation.select_available(&[]), None);
    }

    #[test]
    fn test_config_accessor() {
        let config = test_config();
        let rotation = ProviderRotation::new(config.clone());
        assert_eq!(rotation.config().base_cooldown_secs, 1);
        assert_eq!(rotation.config().max_cooldown_secs, 10);
    }
}
