//! Shared HTTP connection pool for all providers.
//!
//! Provides a singleton `HttpPool` wrapping a `reqwest::Client` with
//! keep-alive, connection pooling, and sensible timeout defaults.
//! Providers can call `HttpPool::global().client()` instead of each
//! building their own `reqwest::Client`.

use once_cell::sync::Lazy;
use reqwest::Client;
use std::time::Duration;

// ── Default constants ──────────────────────────────────────────────

const DEFAULT_MAX_IDLE_PER_HOST: usize = 5;
const DEFAULT_CONNECT_TIMEOUT_SECS: u64 = 10;
const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 120;
const USER_AGENT: &str = "clawtex-core/0.1.0";

// ── PoolConfig ─────────────────────────────────────────────────────

/// Configuration knobs for the HTTP connection pool.
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// Maximum number of idle connections kept alive per host.
    pub max_idle_per_host: usize,
    /// TCP connect timeout in seconds.
    pub connect_timeout_secs: u64,
    /// Overall request timeout in seconds (includes connect + transfer).
    pub request_timeout_secs: u64,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_idle_per_host: DEFAULT_MAX_IDLE_PER_HOST,
            connect_timeout_secs: DEFAULT_CONNECT_TIMEOUT_SECS,
            request_timeout_secs: DEFAULT_REQUEST_TIMEOUT_SECS,
        }
    }
}

// ── HttpPool ───────────────────────────────────────────────────────

/// A shared HTTP connection pool backed by a single `reqwest::Client`.
///
/// The `Client` is configured with:
/// - Connection keep-alive (enabled by default in reqwest/hyper)
/// - Pool max idle per host (default 5)
/// - Connect timeout (default 10 s)
/// - Request timeout (default 120 s)
/// - User-Agent header: `clawtex-core/0.1.0`
pub struct HttpPool {
    client: Client,
    config: PoolConfig,
}

/// Process-wide singleton pool, created lazily on first access.
static GLOBAL_POOL: Lazy<HttpPool> = Lazy::new(|| HttpPool::with_config(PoolConfig::default()));

impl HttpPool {
    // ── Constructors ───────────────────────────────────────────────

    /// Build a pool with custom configuration.
    pub fn with_config(config: PoolConfig) -> Self {
        let client = Client::builder()
            .pool_max_idle_per_host(config.max_idle_per_host)
            .connect_timeout(Duration::from_secs(config.connect_timeout_secs))
            .timeout(Duration::from_secs(config.request_timeout_secs))
            .user_agent(USER_AGENT)
            // Keep-alive is the default in reqwest / hyper; we don't
            // disable it, so connections are reused automatically.
            .build()
            .expect("Failed to build shared reqwest::Client");

        Self { client, config }
    }

    // ── Singleton ──────────────────────────────────────────────────

    /// Return a reference to the process-wide default pool.
    ///
    /// The pool is lazily initialised on first call with `PoolConfig::default()`.
    pub fn global() -> &'static HttpPool {
        &GLOBAL_POOL
    }

    // ── Accessors ──────────────────────────────────────────────────

    /// Get the underlying `reqwest::Client`.
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Return a clone of the `PoolConfig` this pool was built with.
    pub fn config(&self) -> &PoolConfig {
        &self.config
    }
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // 1. Default PoolConfig values
    #[test]
    fn test_pool_config_defaults() {
        let cfg = PoolConfig::default();
        assert_eq!(cfg.max_idle_per_host, 5);
        assert_eq!(cfg.connect_timeout_secs, 10);
        assert_eq!(cfg.request_timeout_secs, 120);
    }

    // 2. Custom PoolConfig propagates
    #[test]
    fn test_custom_pool_config() {
        let cfg = PoolConfig {
            max_idle_per_host: 20,
            connect_timeout_secs: 5,
            request_timeout_secs: 60,
        };
        let pool = HttpPool::with_config(cfg.clone());
        assert_eq!(pool.config().max_idle_per_host, 20);
        assert_eq!(pool.config().connect_timeout_secs, 5);
        assert_eq!(pool.config().request_timeout_secs, 60);
    }

    // 3. Singleton returns the same reference every time
    #[test]
    fn test_global_singleton_identity() {
        let a = HttpPool::global();
        let b = HttpPool::global();
        // Both must be the exact same static address.
        assert!(std::ptr::eq(a, b));
    }

    // 4. client() is accessible and non-null-like (can be used)
    #[test]
    fn test_client_accessor() {
        let pool = HttpPool::with_config(PoolConfig::default());
        // Just verify we can take a reference without panic.
        let _client: &Client = pool.client();
    }

    // 5. Global pool uses default config values
    #[test]
    fn test_global_uses_default_config() {
        let pool = HttpPool::global();
        let cfg = pool.config();
        assert_eq!(cfg.max_idle_per_host, DEFAULT_MAX_IDLE_PER_HOST);
        assert_eq!(cfg.connect_timeout_secs, DEFAULT_CONNECT_TIMEOUT_SECS);
        assert_eq!(cfg.request_timeout_secs, DEFAULT_REQUEST_TIMEOUT_SECS);
    }

    // 6. with_config produces a usable client (not the global one)
    #[test]
    fn test_with_config_separate_from_global() {
        let custom = HttpPool::with_config(PoolConfig {
            max_idle_per_host: 99,
            connect_timeout_secs: 1,
            request_timeout_secs: 30,
        });
        // The custom pool should have its own config, independent of global.
        assert_eq!(custom.config().max_idle_per_host, 99);
        assert_eq!(HttpPool::global().config().max_idle_per_host, 5);
    }

    // 7. PoolConfig Debug derive works
    #[test]
    fn test_pool_config_debug() {
        let cfg = PoolConfig::default();
        let dbg = format!("{:?}", cfg);
        assert!(dbg.contains("max_idle_per_host"));
        assert!(dbg.contains("connect_timeout_secs"));
        assert!(dbg.contains("request_timeout_secs"));
    }

    // 8. PoolConfig Clone produces equal values
    #[test]
    fn test_pool_config_clone() {
        let cfg = PoolConfig {
            max_idle_per_host: 42,
            connect_timeout_secs: 7,
            request_timeout_secs: 200,
        };
        let cloned = cfg.clone();
        assert_eq!(cloned.max_idle_per_host, cfg.max_idle_per_host);
        assert_eq!(cloned.connect_timeout_secs, cfg.connect_timeout_secs);
        assert_eq!(cloned.request_timeout_secs, cfg.request_timeout_secs);
    }

    // 9. Zero max_idle_per_host is accepted (reqwest allows it)
    #[test]
    fn test_zero_max_idle_per_host() {
        let pool = HttpPool::with_config(PoolConfig {
            max_idle_per_host: 0,
            connect_timeout_secs: 10,
            request_timeout_secs: 120,
        });
        assert_eq!(pool.config().max_idle_per_host, 0);
    }

    // 10. Very large timeout values are accepted
    #[test]
    fn test_large_timeout_values() {
        let pool = HttpPool::with_config(PoolConfig {
            max_idle_per_host: 1,
            connect_timeout_secs: 3600,
            request_timeout_secs: 7200,
        });
        assert_eq!(pool.config().connect_timeout_secs, 3600);
        assert_eq!(pool.config().request_timeout_secs, 7200);
    }
}
