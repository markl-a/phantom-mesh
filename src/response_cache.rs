//! Response Cache — LRU cache for LLM responses to avoid duplicate queries.
//! Caches based on the last user message + tool set hash.
//! Also supports semantic similarity matching via Jaccard similarity on tokenized words.

use std::collections::HashMap;
use std::collections::HashSet;
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
    /// Jaccard similarity threshold for semantic matching (default: 0.85)
    pub semantic_threshold: f64,
}

impl Default for ResponseCacheConfig {
    fn default() -> Self {
        Self {
            max_entries: 128,
            ttl: Duration::from_secs(300),
            semantic_threshold: 0.85,
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
    pub semantic_hits: u64,
}

/// Internal cache entry
struct CacheEntry {
    response: ChatResponse,
    created_at: Instant,
}

/// A semantic cache entry that stores tokenized words for Jaccard similarity matching.
pub struct SemanticCacheEntry {
    /// Set of lowercased, normalized words from the original message
    pub tokenized_words: HashSet<String>,
    /// The exact-hash cache key this entry corresponds to
    pub original_key: u64,
    /// Sorted tool names associated with this entry
    pub tool_signature: Vec<String>,
    /// When this entry was created
    pub timestamp: Instant,
}

/// Tokenize a string into a set of lowercased words.
///
/// Splits on whitespace and punctuation boundaries. For CJK characters,
/// each individual character becomes its own token (unigram segmentation).
fn tokenize(text: &str) -> HashSet<String> {
    let mut words = HashSet::new();
    let lower = text.to_lowercase();
    let mut current_word = String::new();

    for ch in lower.chars() {
        if is_cjk_char(ch) {
            // Flush any accumulated ASCII/Latin word
            if !current_word.is_empty() {
                words.insert(std::mem::take(&mut current_word));
            }
            // Each CJK character is its own token
            words.insert(ch.to_string());
        } else if ch.is_alphanumeric() || ch == '_' {
            current_word.push(ch);
        } else {
            // Whitespace or punctuation — flush the current word
            if !current_word.is_empty() {
                words.insert(std::mem::take(&mut current_word));
            }
        }
    }

    // Flush trailing word
    if !current_word.is_empty() {
        words.insert(current_word);
    }

    words
}

/// Returns true if the character is in a CJK Unified Ideographs block.
fn is_cjk_char(ch: char) -> bool {
    matches!(ch,
        '\u{4E00}'..='\u{9FFF}'   // CJK Unified Ideographs
        | '\u{3400}'..='\u{4DBF}' // CJK Unified Ideographs Extension A
        | '\u{F900}'..='\u{FAFF}' // CJK Compatibility Ideographs
        | '\u{2E80}'..='\u{2EFF}' // CJK Radicals Supplement
        | '\u{3000}'..='\u{303F}' // CJK Symbols and Punctuation
        | '\u{3040}'..='\u{309F}' // Hiragana
        | '\u{30A0}'..='\u{30FF}' // Katakana
        | '\u{AC00}'..='\u{D7AF}' // Hangul Syllables
    )
}

/// Compute Jaccard similarity between two word sets: |A ∩ B| / |A ∪ B|.
///
/// Returns 0.0 if both sets are empty (no meaningful comparison).
fn jaccard_similarity(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    let intersection = a.intersection(b).count();
    let union = a.union(b).count();
    if union == 0 {
        return 0.0;
    }
    intersection as f64 / union as f64
}

/// LRU response cache with TTL expiration and semantic similarity matching.
pub struct ResponseCache {
    entries: Mutex<HashMap<u64, CacheEntry>>,
    lru_order: Mutex<Vec<u64>>,
    semantic_entries: Mutex<Vec<SemanticCacheEntry>>,
    config: ResponseCacheConfig,
    stats: Mutex<CacheStats>,
}

impl ResponseCache {
    pub fn new(config: ResponseCacheConfig) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            lru_order: Mutex::new(Vec::new()),
            semantic_entries: Mutex::new(Vec::new()),
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

    /// Semantic similarity lookup. Finds the best-matching cached response
    /// by computing Jaccard similarity on the tokenized words of the last user
    /// message. Only entries with the same tool signature and similarity >= threshold
    /// are considered.
    ///
    /// Returns `Some(response)` if a match above the threshold is found, `None` otherwise.
    pub fn semantic_get(
        &self,
        messages: &[ChatMessage],
        tool_names: &[String],
        threshold: f64,
    ) -> Option<ChatResponse> {
        // Extract the last user message
        let last_user_content = messages
            .iter()
            .rev()
            .find(|m| m.role == "user")
            .map(|m| m.content.as_str())
            .unwrap_or("");

        let query_tokens = tokenize(last_user_content);
        if query_tokens.is_empty() {
            return None;
        }

        // Build sorted tool signature for comparison
        let mut sorted_tools: Vec<String> = tool_names.to_vec();
        sorted_tools.sort();

        let semantic = self.semantic_entries.lock().unwrap();
        let entries = self.entries.lock().unwrap();
        let ttl = self.config.ttl;

        let mut best_score: f64 = 0.0;
        let mut best_key: Option<u64> = None;

        for sem_entry in semantic.iter() {
            // Must have matching tool signature
            if sem_entry.tool_signature != sorted_tools {
                continue;
            }

            // Must not be expired
            if sem_entry.timestamp.elapsed() >= ttl {
                continue;
            }

            // Must still exist in the exact cache
            if !entries.contains_key(&sem_entry.original_key) {
                continue;
            }

            let score = jaccard_similarity(&query_tokens, &sem_entry.tokenized_words);
            if score >= threshold && score > best_score {
                best_score = score;
                best_key = Some(sem_entry.original_key);
            }
        }

        drop(entries);
        drop(semantic);

        if let Some(key) = best_key {
            // Use the exact-cache get path so LRU and stats are updated
            let mut entries = self.entries.lock().unwrap();
            if let Some(entry) = entries.get(&key) {
                if entry.created_at.elapsed() < ttl {
                    let mut stats = self.stats.lock().unwrap();
                    stats.semantic_hits += 1;
                    stats.hits += 1;
                    let mut lru = self.lru_order.lock().unwrap();
                    lru.retain(|k| *k != key);
                    lru.push(key);
                    debug!(
                        "Semantic cache hit (key: {:#018x}, score: {:.3})",
                        key, best_score
                    );
                    return Some(entry.response.clone());
                }
            }
            // Entry was expired or removed between checks
            let _ = entries.remove(&key);
        }

        None
    }

    /// Convenience wrapper that uses the config's default semantic threshold.
    pub fn semantic_get_default(
        &self,
        messages: &[ChatMessage],
        tool_names: &[String],
    ) -> Option<ChatResponse> {
        self.semantic_get(messages, tool_names, self.config.semantic_threshold)
    }

    /// Store a response in the cache. Evicts LRU entries if at capacity.
    /// Also stores a semantic entry for future similarity matching.
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

    /// Store a response with both exact-hash and semantic entries.
    ///
    /// `messages` and `tool_names` are used to build the semantic entry's
    /// tokenized words and tool signature.
    pub fn put_with_semantic(
        &self,
        messages: &[ChatMessage],
        tool_names: &[String],
        response: ChatResponse,
    ) {
        let key = Self::cache_key(messages, tool_names);
        self.put(key, response);

        // Build the semantic entry
        let last_user_content = messages
            .iter()
            .rev()
            .find(|m| m.role == "user")
            .map(|m| m.content.as_str())
            .unwrap_or("");

        let tokenized_words = tokenize(last_user_content);

        let mut sorted_tools: Vec<String> = tool_names.to_vec();
        sorted_tools.sort();

        let mut semantic = self.semantic_entries.lock().unwrap();

        // Remove any previous semantic entry for this key
        semantic.retain(|e| e.original_key != key);

        // Evict expired semantic entries while we are here
        let ttl = self.config.ttl;
        semantic.retain(|e| e.timestamp.elapsed() < ttl);

        // Cap semantic entries to max_entries
        while semantic.len() >= self.config.max_entries {
            semantic.remove(0);
        }

        semantic.push(SemanticCacheEntry {
            tokenized_words,
            original_key: key,
            tool_signature: sorted_tools,
            timestamp: Instant::now(),
        });
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
        self.semantic_entries.lock().unwrap().clear();
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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

    // ── Tokenizer tests ──

    #[test]
    fn test_tokenize_basic() {
        let tokens = tokenize("Hello World");
        assert!(tokens.contains("hello"));
        assert!(tokens.contains("world"));
        assert_eq!(tokens.len(), 2);
    }

    #[test]
    fn test_tokenize_punctuation_normalization() {
        let tokens = tokenize("hello, world! How's it going?");
        assert!(tokens.contains("hello"));
        assert!(tokens.contains("world"));
        assert!(tokens.contains("how"));
        assert!(tokens.contains("s"));
        assert!(tokens.contains("it"));
        assert!(tokens.contains("going"));
        // Punctuation itself should not be a token
        assert!(!tokens.contains(","));
        assert!(!tokens.contains("!"));
        assert!(!tokens.contains("?"));
    }

    #[test]
    fn test_tokenize_cjk_characters() {
        // "\u{4F60}\u{597D}" = CJK for ni-hao, plus ASCII "hello world"
        let tokens = tokenize("\u{4F60}\u{597D} hello world");
        // Each CJK character is its own token
        assert!(tokens.contains("\u{4F60}"));
        assert!(tokens.contains("\u{597D}"));
        assert!(tokens.contains("hello"));
        assert!(tokens.contains("world"));
    }

    #[test]
    fn test_tokenize_mixed_cjk_latin() {
        // Mix of ASCII and CJK: "\u{57F7}\u{884C} Run command"
        let input = "\u{57F7}\u{884C} Run command";
        let tokens = tokenize(input);
        assert!(tokens.contains("run"));
        assert!(tokens.contains("command"));
        // CJK chars as individual tokens
        assert!(tokens.contains("\u{57F7}"));
        assert!(tokens.contains("\u{884C}"));
    }

    #[test]
    fn test_tokenize_empty() {
        let tokens = tokenize("");
        assert!(tokens.is_empty());
    }

    #[test]
    fn test_tokenize_only_punctuation() {
        let tokens = tokenize("!!! ... ???");
        assert!(tokens.is_empty());
    }

    // ── Jaccard similarity tests ──

    #[test]
    fn test_jaccard_identical_sets() {
        let a: HashSet<String> = ["hello", "world"].iter().map(|s| s.to_string()).collect();
        let b = a.clone();
        assert!((jaccard_similarity(&a, &b) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_jaccard_disjoint_sets() {
        let a: HashSet<String> = ["hello", "world"].iter().map(|s| s.to_string()).collect();
        let b: HashSet<String> = ["foo", "bar"].iter().map(|s| s.to_string()).collect();
        assert!((jaccard_similarity(&a, &b) - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_jaccard_partial_overlap() {
        let a: HashSet<String> = ["hello", "world", "foo"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let b: HashSet<String> = ["hello", "world", "bar"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        // intersection = {hello, world} = 2, union = {hello, world, foo, bar} = 4
        let sim = jaccard_similarity(&a, &b);
        assert!((sim - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_jaccard_both_empty() {
        let a: HashSet<String> = HashSet::new();
        let b: HashSet<String> = HashSet::new();
        assert!((jaccard_similarity(&a, &b) - 0.0).abs() < f64::EPSILON);
    }

    // ── Semantic cache tests ──

    #[test]
    fn test_semantic_exact_match() {
        // Exact same message should always be a semantic hit
        let config = ResponseCacheConfig {
            max_entries: 128,
            ttl: Duration::from_secs(300),
            semantic_threshold: 0.85,
        };
        let cache = ResponseCache::new(config);
        let tools: Vec<String> = vec!["shell".into()];
        let msgs = make_messages("What is the weather today?");

        cache.put_with_semantic(&msgs, &tools, make_response("It's sunny."));

        let result = cache.semantic_get(&msgs, &tools, 0.85);
        assert!(result.is_some());
        assert_eq!(result.unwrap().message.content, "It's sunny.");
    }

    #[test]
    fn test_semantic_near_duplicate_threshold_met() {
        // Nearly identical messages should match with sufficient threshold
        let config = ResponseCacheConfig {
            max_entries: 128,
            ttl: Duration::from_secs(300),
            semantic_threshold: 0.6,
        };
        let cache = ResponseCache::new(config);
        let tools: Vec<String> = vec!["shell".into()];

        // Store "What is the weather today in Tokyo?"
        let msgs_original = make_messages("What is the weather today in Tokyo");
        cache.put_with_semantic(&msgs_original, &tools, make_response("It's sunny in Tokyo."));

        // Query with slightly different phrasing:
        // "What is the weather today at Tokyo"
        // Tokens: {what, is, the, weather, today, at, tokyo} vs {what, is, the, weather, today, in, tokyo}
        // intersection=6, union=8 => 0.75, above 0.6 threshold
        let msgs_query = make_messages("What is the weather today at Tokyo");
        let result = cache.semantic_get(&msgs_query, &tools, 0.6);
        assert!(result.is_some());
        assert_eq!(result.unwrap().message.content, "It's sunny in Tokyo.");
    }

    #[test]
    fn test_semantic_different_messages_threshold_not_met() {
        // Completely different messages should not match
        let config = ResponseCacheConfig {
            max_entries: 128,
            ttl: Duration::from_secs(300),
            semantic_threshold: 0.5,
        };
        let cache = ResponseCache::new(config);
        let tools: Vec<String> = vec!["shell".into()];

        let msgs_original = make_messages("What is the weather today in Tokyo");
        cache.put_with_semantic(&msgs_original, &tools, make_response("It's sunny."));

        // Completely different message
        let msgs_query = make_messages("Please compile my Rust project now");
        let result = cache.semantic_get(&msgs_query, &tools, 0.5);
        assert!(result.is_none());
    }

    #[test]
    fn test_semantic_empty_message_returns_none() {
        let cache = ResponseCache::new(ResponseCacheConfig::default());
        let tools: Vec<String> = vec!["shell".into()];

        let msgs_original = make_messages("hello world");
        cache.put_with_semantic(&msgs_original, &tools, make_response("hi"));

        // Query with empty user message
        let msgs_empty = make_messages("");
        let result = cache.semantic_get(&msgs_empty, &tools, 0.5);
        assert!(result.is_none());
    }

    #[test]
    fn test_semantic_punctuation_normalization() {
        // Messages differing only in punctuation should be treated as identical
        let cache = ResponseCache::new(ResponseCacheConfig::default());
        let tools: Vec<String> = vec!["shell".into()];

        let msgs_original = make_messages("Hello, world! How are you?");
        cache.put_with_semantic(&msgs_original, &tools, make_response("Fine."));

        // Same words, different punctuation
        let msgs_query = make_messages("Hello world -- how are you");
        let result = cache.semantic_get(&msgs_query, &tools, 0.85);
        assert!(result.is_some());
        assert_eq!(result.unwrap().message.content, "Fine.");
    }

    #[test]
    fn test_semantic_cjk_handling() {
        // CJK characters should be tokenized individually and matched
        let cache = ResponseCache::new(ResponseCacheConfig {
            semantic_threshold: 0.5,
            ..Default::default()
        });
        let tools: Vec<String> = vec![];

        // Store a message with CJK: "\u{4ECA}\u{5929}\u{5929}\u{6C23}\u{5982}\u{4F55}" (6 CJK chars)
        // plus "hello" => tokens: {hello, \u4ECA, \u5929, \u5929, \u6C23, \u5982, \u4F55}
        // HashSet dedupes \u5929, so 6 unique tokens
        let msgs_original = make_messages("hello \u{4ECA}\u{5929}\u{5929}\u{6C23}\u{5982}\u{4F55}");
        cache.put_with_semantic(&msgs_original, &tools, make_response("ok"));

        // Query with same CJK chars but "world" instead of "hello"
        // tokens: {world, \u4ECA, \u5929, \u6C23, \u5982, \u4F55}
        // intersection with original = {\u4ECA, \u5929, \u6C23, \u5982, \u4F55} = 5
        // union = {hello, world, \u4ECA, \u5929, \u6C23, \u5982, \u4F55} = 7
        // Jaccard = 5/7 ~= 0.714, above 0.5 threshold
        let msgs_query = make_messages("world \u{4ECA}\u{5929}\u{5929}\u{6C23}\u{5982}\u{4F55}");
        let result = cache.semantic_get(&msgs_query, &tools, 0.5);
        assert!(result.is_some());
        assert_eq!(result.unwrap().message.content, "ok");
    }

    #[test]
    fn test_semantic_different_tools_no_match() {
        // Same message but different tools should NOT match
        let cache = ResponseCache::new(ResponseCacheConfig {
            semantic_threshold: 0.5,
            ..Default::default()
        });
        let tools_a: Vec<String> = vec!["shell".into()];
        let tools_b: Vec<String> = vec!["file_read".into()];

        let msgs = make_messages("list files in current directory");
        cache.put_with_semantic(&msgs, &tools_a, make_response("ls output"));

        // Same message, different tools
        let result = cache.semantic_get(&msgs, &tools_b, 0.5);
        assert!(result.is_none());
    }

    #[test]
    fn test_semantic_best_match_wins() {
        // When multiple entries match, the one with the highest score should win
        let cache = ResponseCache::new(ResponseCacheConfig {
            semantic_threshold: 0.3,
            ..Default::default()
        });
        let tools: Vec<String> = vec!["shell".into()];

        // Entry 1: somewhat related
        let msgs_a = make_messages("the quick brown fox jumps over the lazy dog");
        cache.put_with_semantic(&msgs_a, &tools, make_response("fox response"));

        // Entry 2: very close to query
        let msgs_b = make_messages("the quick brown cat jumps over the lazy dog");
        cache.put_with_semantic(&msgs_b, &tools, make_response("cat response"));

        // Query is closer to entry 2 (only one word different)
        let msgs_query = make_messages("the quick brown cat leaps over the lazy dog");
        let result = cache.semantic_get(&msgs_query, &tools, 0.3);
        assert!(result.is_some());
        assert_eq!(result.unwrap().message.content, "cat response");
    }

    #[test]
    fn test_semantic_ttl_expiry() {
        let config = ResponseCacheConfig {
            max_entries: 128,
            ttl: Duration::from_millis(1),
            semantic_threshold: 0.85,
        };
        let cache = ResponseCache::new(config);
        let tools: Vec<String> = vec!["shell".into()];

        let msgs = make_messages("What is the weather");
        cache.put_with_semantic(&msgs, &tools, make_response("sunny"));

        // Wait for TTL to expire
        std::thread::sleep(Duration::from_millis(10));

        let result = cache.semantic_get(&msgs, &tools, 0.85);
        assert!(result.is_none());
    }

    #[test]
    fn test_semantic_stats_tracking() {
        let cache = ResponseCache::new(ResponseCacheConfig {
            semantic_threshold: 0.85,
            ..Default::default()
        });
        let tools: Vec<String> = vec!["shell".into()];

        let msgs = make_messages("check the system status");
        cache.put_with_semantic(&msgs, &tools, make_response("all good"));

        // Semantic hit
        let result = cache.semantic_get(&msgs, &tools, 0.85);
        assert!(result.is_some());

        let stats = cache.stats();
        assert_eq!(stats.semantic_hits, 1);
        assert!(stats.hits >= 1);
    }

    #[test]
    fn test_semantic_get_default_uses_config_threshold() {
        let cache = ResponseCache::new(ResponseCacheConfig {
            semantic_threshold: 0.99, // Very high threshold
            ..Default::default()
        });
        let tools: Vec<String> = vec!["shell".into()];

        let msgs_original = make_messages("tell me about the weather in Tokyo");
        cache.put_with_semantic(&msgs_original, &tools, make_response("sunny"));

        // Slightly different — will not meet 0.99 threshold
        let msgs_query = make_messages("tell me about the weather in Osaka");
        let result = cache.semantic_get_default(&msgs_query, &tools);
        assert!(result.is_none());

        // But exact match should still work at 0.99
        let result_exact = cache.semantic_get_default(&msgs_original, &tools);
        assert!(result_exact.is_some());
    }

    #[test]
    fn test_clear_also_clears_semantic_entries() {
        let cache = ResponseCache::new(ResponseCacheConfig::default());
        let tools: Vec<String> = vec!["shell".into()];

        let msgs = make_messages("hello world");
        cache.put_with_semantic(&msgs, &tools, make_response("hi"));

        cache.clear();

        // Semantic lookup should find nothing after clear
        let result = cache.semantic_get(&msgs, &tools, 0.5);
        assert!(result.is_none());
    }

    #[test]
    fn test_semantic_no_user_message() {
        // Messages with no user message at all
        let cache = ResponseCache::new(ResponseCacheConfig::default());
        let tools: Vec<String> = vec!["shell".into()];

        let msgs_original = make_messages("some query");
        cache.put_with_semantic(&msgs_original, &tools, make_response("answer"));

        // Query with only system message (no user message)
        let msgs_no_user = vec![ChatMessage {
            role: "system".into(),
            content: "You are helpful".into(),
            tool_calls: None,
            tool_call_id: None,
        }];
        let result = cache.semantic_get(&msgs_no_user, &tools, 0.5);
        assert!(result.is_none());
    }

    #[test]
    fn test_semantic_case_insensitivity() {
        // Upper/lower case should not matter
        let cache = ResponseCache::new(ResponseCacheConfig {
            semantic_threshold: 0.85,
            ..Default::default()
        });
        let tools: Vec<String> = vec![];

        let msgs_original = make_messages("Hello World Foo Bar");
        cache.put_with_semantic(&msgs_original, &tools, make_response("resp"));

        let msgs_query = make_messages("HELLO WORLD FOO BAR");
        let result = cache.semantic_get(&msgs_query, &tools, 0.85);
        assert!(result.is_some());
        assert_eq!(result.unwrap().message.content, "resp");
    }
}
