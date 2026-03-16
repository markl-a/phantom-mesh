//! Response Cache — LRU cache for LLM responses to avoid duplicate queries.
//! Caches based on the last user message + tool set hash.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tracing::debug;

use crate::providers::{ChatMessage, ChatResponse};

/// Configuration for the response cache
#[derive(Debug, Clone)]
pub struct ResponseCacheConfig {
    /// Maximum number of cache entries (default: 128)
    pub max_entries: usize,
    /// Time-to-live for cache entries (default: 5 minutes)
    pub ttl: Duration,
}

impl Default for ResponseCacheConfig {
    fn default() -> Self {
        Self {
            max_entries: 128,
            ttl: Duration::from_secs(300),
        }
    }
}

/// Cache statistics
#[derive(Debug, Clone, Default)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub entries: usize,
}

/// Internal cache entry
struct CacheEntry {
    response: ChatResponse,
    created_at: Instant,
}

/// LRU response cache with TTL expiration.
pub struct ResponseCache {
    entries: Mutex<HashMap<u64, CacheEntry>>,
    lru_order: Mutex<Vec<u64>>,
    config: ResponseCacheConfig,
    stats: Mutex<CacheStats>,
}

impl ResponseCache {
    pub fn new(config: ResponseCacheConfig) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            lru_order: Mutex::new(Vec::new()),
            config,
            stats: Mutex::new(CacheStats::default()),
        }
    }

    /// Generate a cache key from messages and tool names.
    /// Uses the last user message content + sorted tool names.
    pub fn cache_key(messages: &[ChatMessage], tool_names: &[String]) -> u64 {
        let mut hasher = DefaultHasher::new();

        // Hash the last user message
        if let Some(last_user) = messages.iter().rev().find(|m| m.role == "user") {
            last_user.content.hash(&mut hasher);
        }

        // Hash sorted tool names
        let mut sorted_tools: Vec<&String> = tool_names.iter().collect();
        sorted_tools.sort();
        sorted_tools.hash(&mut hasher);

        // Also hash the system prompt for context
        if let Some(sys) = messages.first() {
            if sys.role == "system" {
                // Only hash first 200 chars of system prompt (enough for differentiation)
                let prefix: String = sys.content.chars().take(200).collect();
                prefix.hash(&mut hasher);
            }
        }

        hasher.finish()
    }

    /// Try to get a cached response. Returns None if expired or not found.
    pub fn get(&self, key: u64) -> Option<ChatResponse> {
        let mut entries = self.entries.lock().unwrap();
        let mut stats = self.stats.lock().unwrap();

        if let Some(entry) = entries.get(&key) {
            if entry.created_at.elapsed() < self.config.ttl {
                stats.hits += 1;
                // Update LRU order
                let mut lru = self.lru_order.lock().unwrap();
                lru.retain(|k| *k != key);
                lru.push(key);
                debug!("Cache hit (key: {:#018x})", key);
                return Some(entry.response.clone());
            } else {
                // Expired — remove
                entries.remove(&key);
                let mut lru = self.lru_order.lock().unwrap();
                lru.retain(|k| *k != key);
                debug!("Cache expired (key: {:#018x})", key);
            }
        }

        stats.misses += 1;
        None
    }

    /// Store a response in the cache. Evicts LRU entries if at capacity.
    pub fn put(&self, key: u64, response: ChatResponse) {
        let mut entries = self.entries.lock().unwrap();
        let mut lru = self.lru_order.lock().unwrap();
        let mut stats = self.stats.lock().unwrap();

        // Evict if at capacity
        while entries.len() >= self.config.max_entries && !lru.is_empty() {
            let evict_key = lru.remove(0);
            entries.remove(&evict_key);
            stats.evictions += 1;
            debug!("Cache evicted (key: {:#018x})", evict_key);
        }

        entries.insert(key, CacheEntry {
            response,
            created_at: Instant::now(),
        });
        lru.retain(|k| *k != key);
        lru.push(key);
        stats.entries = entries.len();

        debug!("Cache put (key: {:#018x}, size: {})", key, entries.len());
    }

    /// Get cache statistics.
    pub fn stats(&self) -> CacheStats {
        let stats = self.stats.lock().unwrap();
        let entries = self.entries.lock().unwrap();
        CacheStats {
            entries: entries.len(),
            ..stats.clone()
        }
    }

    /// Clear all cache entries.
    pub fn clear(&self) {
        self.entries.lock().unwrap().clear();
        self.lru_order.lock().unwrap().clear();
        let mut stats = self.stats.lock().unwrap();
        stats.entries = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::{ChatMessage, ChatResponse, TokenUsage};

    fn make_messages(user_msg: &str) -> Vec<ChatMessage> {
        vec![
            ChatMessage {
                role: "system".into(),
                content: "You are helpful".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            ChatMessage {
                role: "user".into(),
                content: user_msg.into(),
                tool_calls: None,
                tool_call_id: None,
            },
        ]
    }

    fn make_response(content: &str) -> ChatResponse {
        ChatResponse {
            message: ChatMessage {
                role: "assistant".into(),
                content: content.into(),
                tool_calls: None,
                tool_call_id: None,
            },
            usage: Some(TokenUsage {
                prompt_tokens: 10,
                completion_tokens: 20,
                total_tokens: 30,
            }),
        }
    }

    #[test]
    fn test_cache_put_get() {
        let cache = ResponseCache::new(ResponseCacheConfig::default());
        let msgs = make_messages("hello");
        let tools: Vec<String> = vec!["shell".into()];
        let key = ResponseCache::cache_key(&msgs, &tools);
        let resp = make_response("world");

        cache.put(key, resp.clone());
        let cached = cache.get(key).unwrap();
        assert_eq!(cached.message.content, "world");
    }

    #[test]
    fn test_cache_miss() {
        let cache = ResponseCache::new(ResponseCacheConfig::default());
        assert!(cache.get(12345).is_none());
    }

    #[test]
    fn test_cache_ttl_expiry() {
        let config = ResponseCacheConfig {
            max_entries: 10,
            ttl: Duration::from_millis(1), // 1ms TTL
        };
        let cache = ResponseCache::new(config);
        let key = 42;
        cache.put(key, make_response("test"));

        // Wait for TTL to expire
        std::thread::sleep(Duration::from_millis(10));
        assert!(cache.get(key).is_none());
    }

    #[test]
    fn test_cache_lru_eviction() {
        let config = ResponseCacheConfig {
            max_entries: 2,
            ttl: Duration::from_secs(300),
        };
        let cache = ResponseCache::new(config);

        cache.put(1, make_response("first"));
        cache.put(2, make_response("second"));
        cache.put(3, make_response("third")); // Should evict key 1

        assert!(cache.get(1).is_none()); // Evicted
        assert!(cache.get(2).is_some());
        assert!(cache.get(3).is_some());
    }

    #[test]
    fn test_cache_lru_access_updates_order() {
        let config = ResponseCacheConfig {
            max_entries: 2,
            ttl: Duration::from_secs(300),
        };
        let cache = ResponseCache::new(config);

        cache.put(1, make_response("first"));
        cache.put(2, make_response("second"));

        // Access key 1 to make it most recently used
        cache.get(1);

        // Adding key 3 should evict key 2 (least recently used)
        cache.put(3, make_response("third"));

        assert!(cache.get(1).is_some()); // Recently accessed
        assert!(cache.get(2).is_none()); // Evicted
        assert!(cache.get(3).is_some());
    }

    #[test]
    fn test_cache_key_different_messages() {
        let tools: Vec<String> = vec!["shell".into()];
        let key1 = ResponseCache::cache_key(&make_messages("hello"), &tools);
        let key2 = ResponseCache::cache_key(&make_messages("world"), &tools);
        assert_ne!(key1, key2);
    }

    #[test]
    fn test_cache_key_different_tools() {
        let msgs = make_messages("hello");
        let key1 = ResponseCache::cache_key(&msgs, &vec!["shell".into()]);
        let key2 = ResponseCache::cache_key(&msgs, &vec!["file_read".into()]);
        assert_ne!(key1, key2);
    }

    #[test]
    fn test_cache_key_same_inputs() {
        let msgs = make_messages("hello");
        let tools: Vec<String> = vec!["shell".into()];
        let key1 = ResponseCache::cache_key(&msgs, &tools);
        let key2 = ResponseCache::cache_key(&msgs, &tools);
        assert_eq!(key1, key2);
    }

    #[test]
    fn test_cache_stats() {
        let cache = ResponseCache::new(ResponseCacheConfig::default());
        cache.put(1, make_response("test"));
        cache.get(1); // hit
        cache.get(2); // miss

        let stats = cache.stats();
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 1);
        assert_eq!(stats.entries, 1);
    }

    #[test]
    fn test_cache_clear() {
        let cache = ResponseCache::new(ResponseCacheConfig::default());
        cache.put(1, make_response("a"));
        cache.put(2, make_response("b"));
        cache.clear();

        assert!(cache.get(1).is_none());
        assert!(cache.get(2).is_none());
        assert_eq!(cache.stats().entries, 0);
    }

    #[test]
    fn test_cache_thread_safety() {
        use std::sync::Arc;

        let cache = Arc::new(ResponseCache::new(ResponseCacheConfig::default()));
        let cache2 = cache.clone();

        let t1 = std::thread::spawn(move || {
            for i in 0..50 {
                cache2.put(i, make_response(&format!("resp_{}", i)));
            }
        });

        let cache3 = cache.clone();
        let t2 = std::thread::spawn(move || {
            for i in 0..50 {
                let _ = cache3.get(i);
            }
        });

        t1.join().unwrap();
        t2.join().unwrap();

        // Should not panic and stats should be reasonable
        let stats = cache.stats();
        assert!(stats.entries <= 128);
    }
}
