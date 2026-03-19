//! Hand Result Cache — prevents duplicate cron-triggered runs.
//!
//! When a cron job fires a hand that was already run with the same input recently,
//! the cache returns the previous result instead of re-executing. This saves LLM
//! tokens and avoids redundant tool calls (e.g., duplicate emails, duplicate reports).
//!
//! Uses a bounded HashMap with SHA-256 input hashing and per-entry TTL.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use sha2::{Digest, Sha256};
use tracing::{debug, info};

use super::HandResult;

/// Default time-to-live for cached results: 1 hour (3600 seconds).
const DEFAULT_TTL_SECS: u64 = 3600;

/// Default maximum number of entries in the cache.
const DEFAULT_MAX_ENTRIES: usize = 256;

/// Cache key: (hand_name, input_hash).
type CacheKey = (String, String);

/// A cached hand result with metadata.
#[derive(Debug, Clone)]
struct CacheEntry {
    /// The cached hand execution result.
    result: HandResult,
    /// When this entry was stored.
    created_at: Instant,
    /// Number of times this entry has been returned from cache.
    hit_count: u64,
}

/// Statistics about cache usage.
#[derive(Debug, Clone, Default)]
pub struct CacheStats {
    /// Total number of cache hits (get returned Some).
    pub hits: u64,
    /// Total number of cache misses (get returned None).
    pub misses: u64,
    /// Current number of entries in the cache.
    pub size: usize,
    /// Maximum capacity of the cache.
    pub max_size: usize,
    /// Number of entries evicted due to capacity limits.
    pub evictions: u64,
    /// Number of entries expired by TTL.
    pub expirations: u64,
}

impl CacheStats {
    /// Hit rate as a percentage (0.0 - 100.0). Returns 0.0 if no lookups yet.
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            (self.hits as f64 / total as f64) * 100.0
        }
    }
}

/// Thread-safe LRU-like cache for hand execution results.
///
/// Keyed by `(hand_name, sha256(input))`. Entries expire after a configurable TTL.
/// When the cache is full, the oldest entry is evicted.
pub struct HandResultCache {
    inner: Mutex<CacheInner>,
}

/// Interior state protected by the mutex.
struct CacheInner {
    /// The actual cache storage.
    entries: HashMap<CacheKey, CacheEntry>,
    /// Default TTL in seconds for new entries.
    default_ttl_secs: u64,
    /// Maximum number of entries.
    max_entries: usize,
    /// Cumulative hit count.
    hits: u64,
    /// Cumulative miss count.
    misses: u64,
    /// Cumulative eviction count.
    evictions: u64,
    /// Cumulative expiration count.
    expirations: u64,
}

impl HandResultCache {
    /// Create a new cache with default settings (TTL=3600s, max=256 entries).
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(CacheInner {
                entries: HashMap::new(),
                default_ttl_secs: DEFAULT_TTL_SECS,
                max_entries: DEFAULT_MAX_ENTRIES,
                hits: 0,
                misses: 0,
                evictions: 0,
                expirations: 0,
            }),
        }
    }

    /// Create a new cache with custom TTL and max entries.
    pub fn with_config(default_ttl_secs: u64, max_entries: usize) -> Self {
        Self {
            inner: Mutex::new(CacheInner {
                entries: HashMap::new(),
                default_ttl_secs,
                max_entries: max_entries.max(1), // at least 1
                hits: 0,
                misses: 0,
                evictions: 0,
                expirations: 0,
            }),
        }
    }

    /// Compute the SHA-256 hash of the input string (used as cache key component).
    fn hash_input(input: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(input.as_bytes());
        let result = hasher.finalize();
        format!("{:x}", result)
    }

    /// Build the cache key from hand name and input.
    fn make_key(hand_name: &str, input: &str) -> CacheKey {
        (hand_name.to_string(), Self::hash_input(input))
    }

    /// Get a cached result if it exists and is still fresh (within default TTL).
    ///
    /// Returns `Some(HandResult)` on cache hit, `None` on miss or expired entry.
    pub fn get(&self, hand_name: &str, input: &str) -> Option<HandResult> {
        self.get_with_ttl(hand_name, input, None)
    }

    /// Get a cached result with a custom TTL override.
    ///
    /// If `max_age_secs` is `None`, uses the cache's default TTL.
    pub fn get_with_ttl(
        &self,
        hand_name: &str,
        input: &str,
        max_age_secs: Option<u64>,
    ) -> Option<HandResult> {
        let key = Self::make_key(hand_name, input);
        let mut inner = self.inner.lock().unwrap();

        let ttl = max_age_secs.unwrap_or(inner.default_ttl_secs);

        // Check if entry exists and is fresh
        let found = inner.entries.get(&key).map(|entry| {
            let age = entry.created_at.elapsed().as_secs();
            (age <= ttl, age, entry.result.clone())
        });

        match found {
            Some((true, age, result)) => {
                // Entry is fresh — update counters
                if let Some(entry) = inner.entries.get_mut(&key) {
                    entry.hit_count += 1;
                }
                inner.hits += 1;
                debug!(
                    "HandResultCache HIT: hand={}, input_hash={}, age={}s",
                    hand_name,
                    &key.1[..12],
                    age,
                );
                return Some(result);
            }
            Some((false, age, _)) => {
                // Entry expired — remove it
                debug!(
                    "HandResultCache EXPIRED: hand={}, age={}s > ttl={}s",
                    hand_name,
                    age,
                    ttl,
                );
                inner.entries.remove(&key);
                inner.expirations += 1;
            }
            None => {}
        }

        inner.misses += 1;
        debug!("HandResultCache MISS: hand={}", hand_name);
        None
    }

    /// Store a hand result in the cache.
    ///
    /// If the cache is full, the oldest entry is evicted first.
    pub fn put(&self, hand_name: &str, input: &str, result: HandResult) {
        let key = Self::make_key(hand_name, input);
        let mut inner = self.inner.lock().unwrap();

        // If key already exists, just update it
        if inner.entries.contains_key(&key) {
            inner.entries.insert(
                key.clone(),
                CacheEntry {
                    result,
                    created_at: Instant::now(),
                    hit_count: 0,
                },
            );
            debug!("HandResultCache PUT (update): hand={}", hand_name);
            return;
        }

        // Evict oldest entry if at capacity
        if inner.entries.len() >= inner.max_entries {
            Self::evict_oldest(&mut inner);
        }

        inner.entries.insert(
            key,
            CacheEntry {
                result,
                created_at: Instant::now(),
                hit_count: 0,
            },
        );
        debug!(
            "HandResultCache PUT: hand={}, size={}",
            hand_name,
            inner.entries.len()
        );
    }

    /// Evict the oldest entry from the cache (by `created_at`).
    fn evict_oldest(inner: &mut CacheInner) {
        if inner.entries.is_empty() {
            return;
        }
        let oldest_key = inner
            .entries
            .iter()
            .min_by_key(|(_, entry)| entry.created_at)
            .map(|(key, _)| key.clone());

        if let Some(key) = oldest_key {
            inner.entries.remove(&key);
            inner.evictions += 1;
            debug!(
                "HandResultCache EVICT: hand={}, evictions={}",
                key.0, inner.evictions
            );
        }
    }

    /// Check if a cached result exists and is still fresh.
    ///
    /// Unlike `get`, this does not clone the result or update hit counts —
    /// useful for cheap existence checks before deciding whether to run a hand.
    pub fn is_fresh(&self, hand_name: &str, input: &str, max_age_secs: u64) -> bool {
        let key = Self::make_key(hand_name, input);
        let inner = self.inner.lock().unwrap();

        if let Some(entry) = inner.entries.get(&key) {
            entry.created_at.elapsed().as_secs() <= max_age_secs
        } else {
            false
        }
    }

    /// Invalidate (remove) all cached entries for a specific hand.
    ///
    /// Use this when a hand's configuration changes or when you want to force
    /// re-execution regardless of cache state.
    pub fn invalidate(&self, hand_name: &str) {
        let mut inner = self.inner.lock().unwrap();
        let before = inner.entries.len();
        inner.entries.retain(|key, _| key.0 != hand_name);
        let removed = before - inner.entries.len();
        if removed > 0 {
            info!(
                "HandResultCache INVALIDATE: hand={}, removed={} entries",
                hand_name, removed
            );
        }
    }

    /// Invalidate a single cached entry for a specific hand + input combination.
    pub fn invalidate_entry(&self, hand_name: &str, input: &str) {
        let key = Self::make_key(hand_name, input);
        let mut inner = self.inner.lock().unwrap();
        if inner.entries.remove(&key).is_some() {
            debug!("HandResultCache INVALIDATE_ENTRY: hand={}", hand_name);
        }
    }

    /// Clear all entries from the cache.
    pub fn clear(&self) {
        let mut inner = self.inner.lock().unwrap();
        let size = inner.entries.len();
        inner.entries.clear();
        if size > 0 {
            info!("HandResultCache CLEAR: removed {} entries", size);
        }
    }

    /// Get cache statistics.
    pub fn stats(&self) -> CacheStats {
        let inner = self.inner.lock().unwrap();
        CacheStats {
            hits: inner.hits,
            misses: inner.misses,
            size: inner.entries.len(),
            max_size: inner.max_entries,
            evictions: inner.evictions,
            expirations: inner.expirations,
        }
    }

    /// Get the current default TTL in seconds.
    pub fn default_ttl_secs(&self) -> u64 {
        let inner = self.inner.lock().unwrap();
        inner.default_ttl_secs
    }

    /// Purge all expired entries from the cache.
    ///
    /// This is useful as periodic maintenance. Normally, expired entries are only
    /// removed on access (lazy expiration), but this method proactively cleans them.
    pub fn purge_expired(&self) {
        let mut inner = self.inner.lock().unwrap();
        let ttl = inner.default_ttl_secs;
        let before = inner.entries.len();
        inner
            .entries
            .retain(|_, entry| entry.created_at.elapsed().as_secs() <= ttl);
        let removed = before - inner.entries.len();
        inner.expirations += removed as u64;
        if removed > 0 {
            info!("HandResultCache PURGE: expired {} entries", removed);
        }
    }
}

impl Default for HandResultCache {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hands::PhaseOutput;

    /// Helper: create a dummy HandResult for testing.
    fn make_result(hand_name: &str, output: &str) -> HandResult {
        HandResult {
            hand_name: hand_name.to_string(),
            phases_completed: 1,
            total_phases: 1,
            outputs: vec![PhaseOutput {
                phase_name: "test_phase".to_string(),
                output: output.to_string(),
                tool_calls: 0,
                duration_secs: 0.5,
                skipped: false,
                guardrail_issues: vec![],
                quality_score: None,
                quality_retries: 0,
            }],
            final_output: output.to_string(),
            elapsed_secs: 1.0,
            chain_to: None,
        }
    }

    #[test]
    fn test_new_cache_empty() {
        let cache = HandResultCache::new();
        let stats = cache.stats();
        assert_eq!(stats.size, 0);
        assert_eq!(stats.hits, 0);
        assert_eq!(stats.misses, 0);
        assert_eq!(stats.max_size, DEFAULT_MAX_ENTRIES);
    }

    #[test]
    fn test_put_and_get() {
        let cache = HandResultCache::new();
        let result = make_result("lead", "Found 5 leads");

        cache.put("lead", "find leads for AI", result.clone());
        let cached = cache.get("lead", "find leads for AI");

        assert!(cached.is_some());
        let cached = cached.unwrap();
        assert_eq!(cached.hand_name, "lead");
        assert_eq!(cached.final_output, "Found 5 leads");
        assert_eq!(cached.phases_completed, 1);
    }

    #[test]
    fn test_miss_on_different_input() {
        let cache = HandResultCache::new();
        cache.put("lead", "input A", make_result("lead", "result A"));

        let cached = cache.get("lead", "input B");
        assert!(cached.is_none());

        let stats = cache.stats();
        assert_eq!(stats.hits, 0);
        assert_eq!(stats.misses, 1);
    }

    #[test]
    fn test_miss_on_different_hand() {
        let cache = HandResultCache::new();
        cache.put("lead", "same input", make_result("lead", "lead result"));

        let cached = cache.get("researcher", "same input");
        assert!(cached.is_none());
    }

    #[test]
    fn test_hit_miss_stats() {
        let cache = HandResultCache::new();
        cache.put("seo", "keywords", make_result("seo", "SEO report"));

        // 1 hit
        let _ = cache.get("seo", "keywords");
        // 2 misses
        let _ = cache.get("seo", "other");
        let _ = cache.get("content", "keywords");

        let stats = cache.stats();
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 2);
        assert_eq!(stats.size, 1);
    }

    #[test]
    fn test_is_fresh() {
        let cache = HandResultCache::new();
        cache.put("lead", "test input", make_result("lead", "result"));

        // Should be fresh with a generous TTL
        assert!(cache.is_fresh("lead", "test input", 3600));

        // Should be fresh with even a 0-second TTL (just inserted)
        assert!(cache.is_fresh("lead", "test input", 0));

        // Non-existent entry is not fresh
        assert!(!cache.is_fresh("lead", "other input", 3600));
        assert!(!cache.is_fresh("unknown", "test input", 3600));
    }

    #[test]
    fn test_invalidate_hand() {
        let cache = HandResultCache::new();
        cache.put("lead", "input 1", make_result("lead", "result 1"));
        cache.put("lead", "input 2", make_result("lead", "result 2"));
        cache.put("seo", "input 1", make_result("seo", "seo result"));

        assert_eq!(cache.stats().size, 3);

        cache.invalidate("lead");

        assert_eq!(cache.stats().size, 1);
        assert!(cache.get("lead", "input 1").is_none());
        assert!(cache.get("lead", "input 2").is_none());
        assert!(cache.get("seo", "input 1").is_some());
    }

    #[test]
    fn test_invalidate_entry() {
        let cache = HandResultCache::new();
        cache.put("lead", "input 1", make_result("lead", "result 1"));
        cache.put("lead", "input 2", make_result("lead", "result 2"));

        cache.invalidate_entry("lead", "input 1");

        assert!(cache.get("lead", "input 1").is_none());
        assert!(cache.get("lead", "input 2").is_some());
    }

    #[test]
    fn test_clear() {
        let cache = HandResultCache::new();
        cache.put("lead", "a", make_result("lead", "1"));
        cache.put("seo", "b", make_result("seo", "2"));
        cache.put("content", "c", make_result("content", "3"));

        assert_eq!(cache.stats().size, 3);
        cache.clear();
        assert_eq!(cache.stats().size, 0);
    }

    #[test]
    fn test_eviction_on_capacity() {
        let cache = HandResultCache::with_config(3600, 3);

        cache.put("h1", "i", make_result("h1", "r1"));
        cache.put("h2", "i", make_result("h2", "r2"));
        cache.put("h3", "i", make_result("h3", "r3"));
        assert_eq!(cache.stats().size, 3);

        // This should evict the oldest entry
        cache.put("h4", "i", make_result("h4", "r4"));
        assert_eq!(cache.stats().size, 3);
        assert_eq!(cache.stats().evictions, 1);

        // h4 should be present
        assert!(cache.get("h4", "i").is_some());
    }

    #[test]
    fn test_put_overwrites_existing() {
        let cache = HandResultCache::new();
        cache.put("lead", "input", make_result("lead", "old result"));
        cache.put("lead", "input", make_result("lead", "new result"));

        let cached = cache.get("lead", "input").unwrap();
        assert_eq!(cached.final_output, "new result");
        assert_eq!(cache.stats().size, 1);
    }

    #[test]
    fn test_default_ttl() {
        let cache = HandResultCache::new();
        assert_eq!(cache.default_ttl_secs(), DEFAULT_TTL_SECS);

        let custom = HandResultCache::with_config(7200, 100);
        assert_eq!(custom.default_ttl_secs(), 7200);
    }

    #[test]
    fn test_cache_stats_hit_rate() {
        let stats = CacheStats {
            hits: 7,
            misses: 3,
            size: 5,
            max_size: 256,
            evictions: 0,
            expirations: 0,
        };
        let rate = stats.hit_rate();
        assert!((rate - 70.0).abs() < 0.01);
    }

    #[test]
    fn test_cache_stats_hit_rate_zero() {
        let stats = CacheStats::default();
        assert_eq!(stats.hit_rate(), 0.0);
    }

    #[test]
    fn test_hash_deterministic() {
        let h1 = HandResultCache::hash_input("hello world");
        let h2 = HandResultCache::hash_input("hello world");
        assert_eq!(h1, h2);

        let h3 = HandResultCache::hash_input("hello world!");
        assert_ne!(h1, h3);
    }

    #[test]
    fn test_with_config_min_capacity() {
        // max_entries of 0 should be clamped to 1
        let cache = HandResultCache::with_config(60, 0);
        assert_eq!(cache.stats().max_size, 1);
    }

    #[test]
    fn test_invalidate_nonexistent_hand_is_noop() {
        let cache = HandResultCache::new();
        cache.put("lead", "x", make_result("lead", "y"));
        cache.invalidate("nonexistent");
        assert_eq!(cache.stats().size, 1);
    }

    #[test]
    fn test_multiple_hands_same_input() {
        let cache = HandResultCache::new();
        let input = "analyze market trends";

        cache.put("seo", input, make_result("seo", "SEO analysis"));
        cache.put("researcher", input, make_result("researcher", "Research report"));
        cache.put("content", input, make_result("content", "Blog post"));

        assert_eq!(cache.stats().size, 3);

        let seo = cache.get("seo", input).unwrap();
        assert_eq!(seo.final_output, "SEO analysis");

        let researcher = cache.get("researcher", input).unwrap();
        assert_eq!(researcher.final_output, "Research report");

        let content = cache.get("content", input).unwrap();
        assert_eq!(content.final_output, "Blog post");
    }

    #[test]
    fn test_get_with_custom_ttl() {
        let cache = HandResultCache::with_config(0, 256); // 0s default TTL
        cache.put("lead", "input", make_result("lead", "result"));

        // With default TTL (0s), entry should still be accessible at instant of insertion
        // but the get uses <=, so 0-elapsed <= 0 is true.
        // Use a generous override TTL to ensure hit.
        let cached = cache.get_with_ttl("lead", "input", Some(3600));
        assert!(cached.is_some());
    }

    #[test]
    fn test_concurrent_access_does_not_panic() {
        use std::sync::Arc;
        use std::thread;

        let cache = Arc::new(HandResultCache::new());
        let mut handles = vec![];

        for i in 0..10 {
            let cache = Arc::clone(&cache);
            let handle = thread::spawn(move || {
                let name = format!("hand_{}", i);
                let input = format!("input_{}", i);
                cache.put(&name, &input, make_result(&name, &format!("result_{}", i)));
                let _ = cache.get(&name, &input);
                let _ = cache.is_fresh(&name, &input, 3600);
                let _ = cache.stats();
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().expect("thread should not panic");
        }

        assert_eq!(cache.stats().size, 10);
    }
}
