use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

pub struct KeyPool {
    keys: Vec<KeyEntry>,
    current: AtomicUsize,
}

struct KeyEntry {
    key: String,
    cooldown_until: RwLock<Option<Instant>>,
    request_count: AtomicU64,
}

impl KeyPool {
    pub fn new(keys: Vec<String>) -> Self {
        let entries = keys.into_iter().map(|k| KeyEntry {
            key: k,
            cooldown_until: RwLock::new(None),
            request_count: AtomicU64::new(0),
        }).collect();
        Self { keys: entries, current: AtomicUsize::new(0) }
    }

    pub async fn next_key(&self) -> Option<&str> {
        let len = self.keys.len();
        if len == 0 { return None; }
        let start = self.current.fetch_add(1, Ordering::Relaxed) % len;
        for i in 0..len {
            let idx = (start + i) % len;
            let entry = &self.keys[idx];
            let guard = entry.cooldown_until.read().await;
            if let Some(until) = *guard {
                if Instant::now() < until { continue; }
            }
            entry.request_count.fetch_add(1, Ordering::Relaxed);
            return Some(&entry.key);
        }
        None // All in cooldown
    }

    pub async fn record_rate_limit(&self, key: &str) {
        self.record_rate_limit_duration(key, Duration::from_secs(60)).await;
    }

    pub async fn record_rate_limit_duration(&self, key: &str, duration: Duration) {
        for entry in &self.keys {
            if entry.key == key {
                let mut guard = entry.cooldown_until.write().await;
                *guard = Some(Instant::now() + duration);
                break;
            }
        }
    }

    /// Clear cooldown for a specific key, making it immediately available.
    pub async fn reset_cooldown(&self, key: &str) {
        for entry in &self.keys {
            if entry.key == key {
                let mut guard = entry.cooldown_until.write().await;
                *guard = None;
                break;
            }
        }
    }

    /// Return the list of keys that are currently available (not in cooldown).
    /// Useful for startup validation to confirm which API keys are ready.
    pub async fn warmup(&self) -> Vec<&str> {
        let now = Instant::now();
        let mut available = Vec::new();
        for entry in &self.keys {
            let guard = entry.cooldown_until.read().await;
            let is_cooling = match *guard {
                Some(until) => now < until,
                None => false,
            };
            if !is_cooling {
                available.push(entry.key.as_str());
            }
        }
        available
    }

    /// Return the cumulative request count for a given key.
    pub fn request_count(&self, key: &str) -> Option<u64> {
        self.keys.iter()
            .find(|e| e.key == key)
            .map(|e| e.request_count.load(Ordering::Relaxed))
    }

    pub fn len(&self) -> usize { self.keys.len() }
    pub fn is_empty(&self) -> bool { self.keys.is_empty() }

    /// Produce a serializable snapshot of the pool state.
    pub async fn snapshot(&self) -> KeyPoolSnapshot {
        let now = Instant::now();
        let mut entries = Vec::with_capacity(self.keys.len());
        for entry in &self.keys {
            let guard = entry.cooldown_until.read().await;
            let cooldown_remaining_ms = match *guard {
                Some(until) if until > now => {
                    (until - now).as_millis() as u64
                }
                _ => 0,
            };
            entries.push(KeyEntrySnapshot {
                key_prefix: if entry.key.len() > 8 {
                    format!("{}...", &entry.key[..8])
                } else {
                    entry.key.clone()
                },
                request_count: entry.request_count.load(Ordering::Relaxed),
                cooldown_remaining_ms,
            });
        }
        KeyPoolSnapshot {
            total_keys: self.keys.len(),
            current_index: self.current.load(Ordering::Relaxed),
            entries,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct KeyPoolSnapshot {
    pub total_keys: usize,
    pub current_index: usize,
    pub entries: Vec<KeyEntrySnapshot>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct KeyEntrySnapshot {
    pub key_prefix: String,
    pub request_count: u64,
    pub cooldown_remaining_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    // ── 1. Round-robin key selection ──────────────────────────────────

    #[tokio::test]
    async fn test_round_robin_rotation() {
        let pool = KeyPool::new(vec!["key1".into(), "key2".into(), "key3".into()]);
        let k1 = pool.next_key().await.unwrap().to_string();
        let k2 = pool.next_key().await.unwrap().to_string();
        let k3 = pool.next_key().await.unwrap().to_string();
        let k4 = pool.next_key().await.unwrap().to_string();
        assert_eq!(k1, "key1");
        assert_eq!(k2, "key2");
        assert_eq!(k3, "key3");
        assert_eq!(k4, "key1"); // Wrapped around
    }

    #[tokio::test]
    async fn test_round_robin_full_cycle_twice() {
        let pool = KeyPool::new(vec!["a".into(), "b".into()]);
        let mut sequence = Vec::new();
        for _ in 0..6 {
            sequence.push(pool.next_key().await.unwrap().to_string());
        }
        assert_eq!(sequence, vec!["a", "b", "a", "b", "a", "b"]);
    }

    // ── 2. Cooldown: mark key as cooling, verify it's skipped ────────

    #[tokio::test]
    async fn test_skip_cooled_down_key() {
        let pool = KeyPool::new(vec!["key1".into(), "key2".into()]);
        pool.record_rate_limit("key1").await;
        // Round-robin starts at key1 but it is cooling → skips to key2
        let k = pool.next_key().await.unwrap();
        assert_eq!(k, "key2");
    }

    #[tokio::test]
    async fn test_cooldown_skips_to_next_available() {
        let pool = KeyPool::new(vec!["a".into(), "b".into(), "c".into()]);
        pool.record_rate_limit("a").await;
        pool.record_rate_limit("b").await;
        // Both a and b in cooldown; round-robin starts at a, skips a, skips b, lands on c
        let k = pool.next_key().await.unwrap();
        assert_eq!(k, "c");
    }

    // ── 3. All keys in cooldown → returns None ───────────────────────

    #[tokio::test]
    async fn test_all_cooled_down_returns_none() {
        let pool = KeyPool::new(vec!["key1".into(), "key2".into()]);
        pool.record_rate_limit("key1").await;
        pool.record_rate_limit("key2").await;
        assert!(pool.next_key().await.is_none());
    }

    // ── 4. Cooldown expiry ───────────────────────────────────────────

    #[tokio::test]
    async fn test_cooldown_expiry() {
        let pool = KeyPool::new(vec!["key1".into()]);
        // Set a very short cooldown (10ms)
        pool.record_rate_limit_duration("key1", Duration::from_millis(10)).await;
        // Immediately should be in cooldown
        assert!(pool.next_key().await.is_none());
        // Wait for cooldown to expire
        tokio::time::sleep(Duration::from_millis(30)).await;
        // Now should be available
        let k = pool.next_key().await.unwrap();
        assert_eq!(k, "key1");
    }

    // ── 5. Single key pool behavior ──────────────────────────────────

    #[tokio::test]
    async fn test_single_key_pool() {
        let pool = KeyPool::new(vec!["only".into()]);
        assert_eq!(pool.len(), 1);
        assert!(!pool.is_empty());
        // Same key returned every time
        for _ in 0..5 {
            assert_eq!(pool.next_key().await.unwrap(), "only");
        }
        // After cooldown, returns None
        pool.record_rate_limit("only").await;
        assert!(pool.next_key().await.is_none());
    }

    // ── 6. Empty pool behavior ───────────────────────────────────────

    #[tokio::test]
    async fn test_empty_pool() {
        let pool = KeyPool::new(vec![]);
        assert!(pool.next_key().await.is_none());
        assert!(pool.is_empty());
        assert_eq!(pool.len(), 0);
    }

    #[tokio::test]
    async fn test_empty_pool_warmup() {
        let pool = KeyPool::new(vec![]);
        let available = pool.warmup().await;
        assert!(available.is_empty());
    }

    // ── 7. Concurrent access safety ──────────────────────────────────

    #[tokio::test]
    async fn test_concurrent_access() {
        let pool = std::sync::Arc::new(KeyPool::new(
            vec!["k1".into(), "k2".into(), "k3".into()],
        ));
        let mut handles = Vec::new();
        for _ in 0..30 {
            let p = pool.clone();
            handles.push(tokio::spawn(async move {
                p.next_key().await.map(|s| s.to_string())
            }));
        }
        let mut results = Vec::new();
        for h in handles {
            if let Some(k) = h.await.unwrap() {
                results.push(k);
            }
        }
        // All 30 calls should succeed (no panics, no None)
        assert_eq!(results.len(), 30);
        // Each key should appear roughly 10 times (round-robin)
        let k1_count = results.iter().filter(|k| *k == "k1").count();
        let k2_count = results.iter().filter(|k| *k == "k2").count();
        let k3_count = results.iter().filter(|k| *k == "k3").count();
        assert_eq!(k1_count + k2_count + k3_count, 30);
        assert!(k1_count >= 8 && k1_count <= 12, "k1 count: {k1_count}");
        assert!(k2_count >= 8 && k2_count <= 12, "k2 count: {k2_count}");
        assert!(k3_count >= 8 && k3_count <= 12, "k3 count: {k3_count}");
    }

    // ── 8. Key count / pool size ─────────────────────────────────────

    #[tokio::test]
    async fn test_pool_size() {
        let pool = KeyPool::new(vec!["a".into(), "b".into(), "c".into(), "d".into()]);
        assert_eq!(pool.len(), 4);
        assert!(!pool.is_empty());
    }

    #[tokio::test]
    async fn test_request_count_tracking() {
        let pool = KeyPool::new(vec!["x".into(), "y".into()]);
        assert_eq!(pool.request_count("x"), Some(0));
        assert_eq!(pool.request_count("y"), Some(0));
        assert_eq!(pool.request_count("z"), None); // nonexistent key
        pool.next_key().await; // x
        pool.next_key().await; // y
        pool.next_key().await; // x
        assert_eq!(pool.request_count("x"), Some(2));
        assert_eq!(pool.request_count("y"), Some(1));
    }

    // ── 9. Reset cooldown ────────────────────────────────────────────

    #[tokio::test]
    async fn test_reset_cooldown() {
        let pool = KeyPool::new(vec!["key1".into(), "key2".into()]);
        pool.record_rate_limit("key1").await;
        pool.record_rate_limit("key2").await;
        // Both cooling → None
        assert!(pool.next_key().await.is_none());
        // Reset key1
        pool.reset_cooldown("key1").await;
        let k = pool.next_key().await.unwrap();
        assert_eq!(k, "key1");
    }

    #[tokio::test]
    async fn test_reset_cooldown_nonexistent_key() {
        let pool = KeyPool::new(vec!["a".into()]);
        pool.record_rate_limit("a").await;
        // Resetting a nonexistent key should not panic or affect anything
        pool.reset_cooldown("nonexistent").await;
        // "a" should still be in cooldown
        assert!(pool.next_key().await.is_none());
    }

    // ── 10. Multiple mark_cooldown on same key ───────────────────────

    #[tokio::test]
    async fn test_multiple_cooldown_same_key() {
        let pool = KeyPool::new(vec!["key1".into()]);
        // Mark cooldown twice with different durations
        pool.record_rate_limit_duration("key1", Duration::from_millis(10)).await;
        // Second call overwrites with longer cooldown
        pool.record_rate_limit_duration("key1", Duration::from_secs(60)).await;
        // Should still be in cooldown (60s hasn't passed)
        assert!(pool.next_key().await.is_none());
    }

    #[tokio::test]
    async fn test_multiple_cooldown_overwrite_with_shorter() {
        let pool = KeyPool::new(vec!["key1".into()]);
        pool.record_rate_limit_duration("key1", Duration::from_secs(60)).await;
        assert!(pool.next_key().await.is_none());
        // Overwrite with very short cooldown
        pool.record_rate_limit_duration("key1", Duration::from_millis(10)).await;
        tokio::time::sleep(Duration::from_millis(30)).await;
        // Now the shorter cooldown has expired
        let k = pool.next_key().await.unwrap();
        assert_eq!(k, "key1");
    }

    // ── 11. Keys with different cooldown durations ───────────────────

    #[tokio::test]
    async fn test_different_cooldown_durations() {
        let pool = KeyPool::new(vec!["fast".into(), "slow".into()]);
        pool.record_rate_limit_duration("fast", Duration::from_millis(10)).await;
        pool.record_rate_limit_duration("slow", Duration::from_secs(60)).await;
        // Both cooling initially
        // After short wait, "fast" recovers but "slow" stays in cooldown
        tokio::time::sleep(Duration::from_millis(30)).await;
        let k = pool.next_key().await.unwrap();
        assert_eq!(k, "fast");
        // "slow" is still in cooldown
        pool.record_rate_limit_duration("fast", Duration::from_secs(60)).await;
        assert!(pool.next_key().await.is_none());
    }

    // ── 12. Serialization roundtrip (snapshot) ───────────────────────

    #[tokio::test]
    async fn test_snapshot_serialization_roundtrip() {
        let pool = KeyPool::new(vec!["sk-abcdefghij".into(), "sk-1234567890".into()]);
        pool.next_key().await; // increment request count on first key
        pool.next_key().await; // increment request count on second key
        pool.next_key().await; // first key again
        pool.record_rate_limit_duration("sk-1234567890", Duration::from_secs(30)).await;

        let snapshot = pool.snapshot().await;

        // Verify snapshot structure
        assert_eq!(snapshot.total_keys, 2);
        assert_eq!(snapshot.entries.len(), 2);
        // Keys are truncated to prefix
        assert_eq!(snapshot.entries[0].key_prefix, "sk-abcde...");
        assert_eq!(snapshot.entries[1].key_prefix, "sk-12345...");
        // Request counts
        assert_eq!(snapshot.entries[0].request_count, 2);
        assert_eq!(snapshot.entries[1].request_count, 1);
        // Second key has cooldown remaining
        assert!(snapshot.entries[1].cooldown_remaining_ms > 0);
        // First key has no cooldown
        assert_eq!(snapshot.entries[0].cooldown_remaining_ms, 0);

        // Serialize → deserialize roundtrip
        let json = serde_json::to_string(&snapshot).unwrap();
        let deserialized: KeyPoolSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(snapshot, deserialized);
    }

    #[tokio::test]
    async fn test_snapshot_short_key_not_truncated() {
        let pool = KeyPool::new(vec!["short".into()]);
        let snapshot = pool.snapshot().await;
        // Key shorter than 8 chars should not be truncated
        assert_eq!(snapshot.entries[0].key_prefix, "short");
    }

    // ── 13. warmup() method ──────────────────────────────────────────

    #[tokio::test]
    async fn test_warmup_all_available() {
        let pool = KeyPool::new(vec!["a".into(), "b".into(), "c".into()]);
        let available = pool.warmup().await;
        assert_eq!(available, vec!["a", "b", "c"]);
    }

    #[tokio::test]
    async fn test_warmup_partial_cooldown() {
        let pool = KeyPool::new(vec!["a".into(), "b".into(), "c".into()]);
        pool.record_rate_limit("b").await;
        let available = pool.warmup().await;
        assert_eq!(available, vec!["a", "c"]);
    }

    #[tokio::test]
    async fn test_warmup_all_in_cooldown() {
        let pool = KeyPool::new(vec!["a".into(), "b".into()]);
        pool.record_rate_limit("a").await;
        pool.record_rate_limit("b").await;
        let available = pool.warmup().await;
        assert!(available.is_empty());
    }

    #[tokio::test]
    async fn test_warmup_after_cooldown_expiry() {
        let pool = KeyPool::new(vec!["a".into(), "b".into()]);
        pool.record_rate_limit_duration("a", Duration::from_millis(10)).await;
        pool.record_rate_limit("b").await; // 60s cooldown
        tokio::time::sleep(Duration::from_millis(30)).await;
        let available = pool.warmup().await;
        // "a" has expired cooldown, "b" still cooling
        assert_eq!(available, vec!["a"]);
    }

    // ── 14. Edge cases ───────────────────────────────────────────────

    #[tokio::test]
    async fn test_record_rate_limit_nonexistent_key() {
        let pool = KeyPool::new(vec!["a".into()]);
        // Should not panic
        pool.record_rate_limit("nonexistent").await;
        // "a" should still be available
        let k = pool.next_key().await.unwrap();
        assert_eq!(k, "a");
    }

    #[tokio::test]
    async fn test_reset_then_re_cooldown() {
        let pool = KeyPool::new(vec!["key1".into()]);
        pool.record_rate_limit("key1").await;
        assert!(pool.next_key().await.is_none());
        pool.reset_cooldown("key1").await;
        assert_eq!(pool.next_key().await.unwrap(), "key1");
        pool.record_rate_limit("key1").await;
        assert!(pool.next_key().await.is_none());
    }
}
