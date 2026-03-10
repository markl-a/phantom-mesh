use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;
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
        for entry in &self.keys {
            if entry.key == key {
                let mut guard = entry.cooldown_until.write().await;
                *guard = Some(Instant::now() + std::time::Duration::from_secs(60));
                break;
            }
        }
    }

    pub fn len(&self) -> usize { self.keys.len() }
    pub fn is_empty(&self) -> bool { self.keys.is_empty() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_round_robin_rotation() {
        let pool = KeyPool::new(vec!["key1".into(), "key2".into(), "key3".into()]);
        let k1 = pool.next_key().await.unwrap().to_string();
        let k2 = pool.next_key().await.unwrap().to_string();
        let k3 = pool.next_key().await.unwrap().to_string();
        let k4 = pool.next_key().await.unwrap().to_string();
        assert_ne!(k1, k2);
        assert_ne!(k2, k3);
        assert_eq!(k1, k4); // Wrapped around
    }

    #[tokio::test]
    async fn test_skip_cooled_down_key() {
        let pool = KeyPool::new(vec!["key1".into(), "key2".into()]);
        pool.record_rate_limit("key1").await;
        let k = pool.next_key().await.unwrap();
        assert_eq!(k, "key2");
    }

    #[tokio::test]
    async fn test_all_cooled_down_returns_none() {
        let pool = KeyPool::new(vec!["key1".into(), "key2".into()]);
        pool.record_rate_limit("key1").await;
        pool.record_rate_limit("key2").await;
        assert!(pool.next_key().await.is_none());
    }

    #[tokio::test]
    async fn test_empty_pool() {
        let pool = KeyPool::new(vec![]);
        assert!(pool.next_key().await.is_none());
        assert!(pool.is_empty());
    }
}
