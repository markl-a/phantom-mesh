//! Cache for `/v1/models` responses keyed by provider name.
//!
//! Why: the `/models` slash command + `/model fetch <provider>` both call
//! the provider's catalog endpoint. On free tiers each call burns quota,
//! and the catalog rarely changes between calls in the same session. A
//! 1-hour file-backed cache pays the network cost once per session/hour
//! and serves every subsequent ask instantly with the [FREE]/[paid]
//! markers already computed.
//!
//! Layout — one JSON file at `~/.phantom-mesh/models-cache.json`:
//!
//! ```json
//! {
//!   "providers": {
//!     "opencode": {
//!       "fetched_at_ms": 1777712345678,
//!       "models": [
//!         { "id": "claude-sonnet-4-6", "is_free": false },
//!         { "id": "minimax-m2.5-free", "is_free": true }
//!       ]
//!     },
//!     "groq": { ... }
//!   }
//! }
//! ```
//!
//! Atomic write to `<path>.tmp` then rename so a crash mid-write can't
//! leave the file truncated. Read errors degrade gracefully — a missing
//! or malformed cache means callers do a live fetch and overwrite.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::keys::ModelInfo;

/// Default TTL: 1 hour. Catalogs change rarely; this keeps users on the
/// fast path through a normal work session without serving stale picks
/// for days.
pub const DEFAULT_TTL_MS: u64 = 60 * 60 * 1_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedProvider {
    pub fetched_at_ms: u64,
    pub models: Vec<CachedModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedModel {
    pub id: String,
    pub is_free: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelsCache {
    #[serde(default)]
    pub providers: HashMap<String, CachedProvider>,
}

/// `~/.phantom-mesh/models-cache.json`
pub fn cache_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".phantom-mesh").join("models-cache.json"))
}

pub fn read_cache() -> ModelsCache {
    let Some(path) = cache_path() else { return ModelsCache::default(); };
    let Ok(raw) = fs::read_to_string(&path) else { return ModelsCache::default(); };
    serde_json::from_str(&raw).unwrap_or_default()
}

pub fn write_cache(cache: &ModelsCache) -> std::io::Result<()> {
    let Some(path) = cache_path() else {
        return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "no $HOME"));
    };
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(cache)
        .unwrap_or_else(|_| "{}".to_string());
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json)?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Read cached entry for `provider`, returning models if the entry exists
/// and is younger than `max_age_ms`. Returns None for missing OR stale.
pub fn get_fresh(provider: &str, max_age_ms: u64) -> Option<Vec<ModelInfo>> {
    let cache = read_cache();
    let entry = cache.providers.get(provider)?;
    let age = now_ms().saturating_sub(entry.fetched_at_ms);
    if age >= max_age_ms {
        return None;
    }
    Some(entry.models.iter().map(|m| ModelInfo {
        id: m.id.clone(),
        is_free: m.is_free,
    }).collect())
}

/// Update one provider's cache entry. Best-effort: silent failure on
/// write errors (caller has already done the network work; the next
/// invocation will retry).
pub fn put(provider: &str, models: &[ModelInfo]) {
    let mut cache = read_cache();
    cache.providers.insert(provider.to_string(), CachedProvider {
        fetched_at_ms: now_ms(),
        models: models.iter().map(|m| CachedModel {
            id: m.id.clone(),
            is_free: m.is_free,
        }).collect(),
    });
    let _ = write_cache(&cache);
}

/// Refresh one provider in the cache by hitting the wire. Returns the
/// fresh model list. Existing cache entry is overwritten on success;
/// untouched on failure.
pub async fn refresh_provider(
    provider: &str,
    provider_type: &str,
    base_url: &str,
    api_key: &str,
) -> anyhow::Result<Vec<ModelInfo>> {
    let models = crate::keys::fetch_models_annotated(provider_type, base_url, api_key).await?;
    put(provider, &models);
    Ok(models)
}

/// Convenience: report which entries in the cache are stale beyond `max_age_ms`.
/// For a future `phantom models status` UX. Sorted by name for deterministic output.
pub fn stale_entries(max_age_ms: u64) -> Vec<(String, u64)> {
    let cache = read_cache();
    let now = now_ms();
    let mut out: Vec<(String, u64)> = cache.providers.iter()
        .filter_map(|(name, entry)| {
            let age = now.saturating_sub(entry.fetched_at_ms);
            (age > max_age_ms).then(|| (name.clone(), age))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn mi(id: &str, is_free: bool) -> ModelInfo {
        ModelInfo { id: id.to_string(), is_free }
    }

    #[test]
    fn cache_roundtrip_one_provider() {
        let mut c = ModelsCache::default();
        c.providers.insert("groq".into(), CachedProvider {
            fetched_at_ms: 12345,
            models: vec![
                CachedModel { id: "llama-3.3-70b".into(), is_free: false },
            ],
        });
        let json = serde_json::to_string(&c).unwrap();
        let parsed: ModelsCache = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.providers.len(), 1);
        let g = parsed.providers.get("groq").unwrap();
        assert_eq!(g.fetched_at_ms, 12345);
        assert_eq!(g.models.len(), 1);
        assert_eq!(g.models[0].id, "llama-3.3-70b");
        assert!(!g.models[0].is_free);
    }

    #[test]
    fn put_and_get_fresh_roundtrip() {
        // Serialize HOME-mutating tests across the whole crate.
        let _guard = crate::env_lock::acquire();
        // Use a deterministic temp dir so this doesn't clash with a real cache.
        let dir = std::env::temp_dir().join(format!("phantom-test-mc-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        // Override $HOME for the duration of this test so cache_path() points here.
        let prev_home = std::env::var("HOME").ok();
        let prev_userprofile = std::env::var("USERPROFILE").ok();
        std::env::set_var("HOME", &dir);
        std::env::set_var("USERPROFILE", &dir);

        put("opencode", &[
            mi("claude-sonnet-4-6", false),
            mi("minimax-m2.5-free", true),
        ]);
        let live = get_fresh("opencode", DEFAULT_TTL_MS).expect("just-written entry should be fresh");
        assert_eq!(live.len(), 2);
        assert_eq!(live[0].id, "claude-sonnet-4-6");
        assert!(!live[0].is_free);
        assert_eq!(live[1].id, "minimax-m2.5-free");
        assert!(live[1].is_free);

        // Stale check: max_age_ms = 0 means everything is stale.
        assert!(get_fresh("opencode", 0).is_none(),
            "max_age_ms=0 should treat any entry as stale");

        // Missing provider returns None even when cache is fresh.
        assert!(get_fresh("does-not-exist", DEFAULT_TTL_MS).is_none());

        // Cleanup + restore HOME
        if let Some(h) = prev_home { std::env::set_var("HOME", h); } else { std::env::remove_var("HOME"); }
        if let Some(u) = prev_userprofile { std::env::set_var("USERPROFILE", u); } else { std::env::remove_var("USERPROFILE"); }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_cache_returns_default_not_panic() {
        let _guard = crate::env_lock::acquire();
        let dir = std::env::temp_dir().join(format!("phantom-test-mc-bad-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir.join(".phantom-mesh")).unwrap();
        fs::write(dir.join(".phantom-mesh/models-cache.json"),
                  "this is not json{{{").unwrap();
        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &dir);
        std::env::set_var("USERPROFILE", &dir);
        let cache = read_cache();
        assert!(cache.providers.is_empty(), "malformed json → empty default");
        if let Some(h) = prev_home { std::env::set_var("HOME", h); } else { std::env::remove_var("HOME"); }
        let _ = fs::remove_dir_all(&dir);
    }
}
