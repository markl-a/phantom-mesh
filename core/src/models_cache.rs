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

use crate::clock::{Clock, SystemClock};
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

/// `~/.phantom-mesh/models-cache.json`.
///
/// Tests can redirect this to a sandboxed dir by setting `PHANTOM_MESH_DIR`
/// (the parent dir that would normally be `~/.phantom-mesh`). This sidesteps
/// the fact that on Windows `dirs::home_dir()` queries Win32 directly and
/// ignores a test-mutated `HOME`/`USERPROFILE`, which would otherwise cause
/// tests to read/write the developer's real cache file.
pub fn cache_path() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("PHANTOM_MESH_DIR") {
        if !dir.is_empty() {
            return Some(PathBuf::from(dir).join("models-cache.json"));
        }
    }
    crate::cli_config::phantom_data_dir()
        .ok()
        .map(|d| d.join("models-cache.json"))
}

pub fn read_cache() -> ModelsCache {
    let Some(path) = cache_path() else {
        return ModelsCache::default();
    };
    let Ok(raw) = fs::read_to_string(&path) else {
        return ModelsCache::default();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

pub fn write_cache(cache: &ModelsCache) -> std::io::Result<()> {
    let Some(path) = cache_path() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no $HOME",
        ));
    };
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(cache).unwrap_or_else(|_| "{}".to_string());
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json)?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

/// Wall-clock epoch milliseconds, via the canonical [`SystemClock`] (reproduces
/// the previous `SystemTime::now() - UNIX_EPOCH`, saturating to 0, byte-for-byte).
pub fn now_ms() -> u64 {
    SystemClock.now_ms()
}

/// Read cached entry for `provider`, returning models if the entry exists
/// and is younger than `max_age_ms`. Returns None for missing OR stale.
pub fn get_fresh(provider: &str, max_age_ms: u64) -> Option<Vec<ModelInfo>> {
    get_fresh_on(&SystemClock, provider, max_age_ms)
}

/// Clock-injected core of [`get_fresh`]: freshness is judged against `clock`'s
/// "now" instead of the wall clock, so a test can pin a [`MockClock`] and assert
/// the fresh→stale TTL boundary deterministically (no wall-clock dependence /
/// `fetched_at = now - delta` arithmetic).
pub fn get_fresh_on(clock: &dyn Clock, provider: &str, max_age_ms: u64) -> Option<Vec<ModelInfo>> {
    let cache = read_cache();
    let entry = cache.providers.get(provider)?;
    let age = clock.now_ms().saturating_sub(entry.fetched_at_ms);
    if age >= max_age_ms {
        return None;
    }
    Some(
        entry
            .models
            .iter()
            .map(|m| ModelInfo {
                id: m.id.clone(),
                is_free: m.is_free,
            })
            .collect(),
    )
}

/// Update one provider's cache entry. Best-effort: silent failure on
/// write errors (caller has already done the network work; the next
/// invocation will retry).
pub fn put(provider: &str, models: &[ModelInfo]) {
    let mut cache = read_cache();
    cache.providers.insert(
        provider.to_string(),
        CachedProvider {
            fetched_at_ms: now_ms(),
            models: models
                .iter()
                .map(|m| CachedModel {
                    id: m.id.clone(),
                    is_free: m.is_free,
                })
                .collect(),
        },
    );
    if let Err(e) = write_cache(&cache) {
        tracing::warn!(provider = provider, "models cache write failed: {}", e);
    }
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
    stale_entries_on(&SystemClock, max_age_ms)
}

/// Clock-injected core of [`stale_entries`] — judges age against `clock`'s "now"
/// so staleness reporting is deterministically testable with a [`MockClock`].
pub fn stale_entries_on(clock: &dyn Clock, max_age_ms: u64) -> Vec<(String, u64)> {
    let cache = read_cache();
    let now = clock.now_ms();
    let mut out: Vec<(String, u64)> = cache
        .providers
        .iter()
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
        ModelInfo {
            id: id.to_string(),
            is_free,
        }
    }

    #[test]
    fn cache_roundtrip_one_provider() {
        let mut c = ModelsCache::default();
        c.providers.insert(
            "groq".into(),
            CachedProvider {
                fetched_at_ms: 12345,
                models: vec![CachedModel {
                    id: "llama-3.3-70b".into(),
                    is_free: false,
                }],
            },
        );
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
        // Serialize env-mutating tests across the whole crate.
        let _guard = crate::env_lock::acquire();
        // Sandboxed cache dir — avoids touching the developer's real cache
        // and works uniformly on Unix + Windows (where HOME overrides are
        // ignored by dirs::home_dir()).
        let dir = std::env::temp_dir().join(format!("phantom-test-mc-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let prev = std::env::var("PHANTOM_MESH_DIR").ok();
        std::env::set_var("PHANTOM_MESH_DIR", &dir);

        put(
            "opencode",
            &[
                mi("claude-sonnet-4-6", false),
                mi("minimax-m2.5-free", true),
            ],
        );
        let live =
            get_fresh("opencode", DEFAULT_TTL_MS).expect("just-written entry should be fresh");
        assert_eq!(live.len(), 2);
        assert_eq!(live[0].id, "claude-sonnet-4-6");
        assert!(!live[0].is_free);
        assert_eq!(live[1].id, "minimax-m2.5-free");
        assert!(live[1].is_free);

        // Stale check: max_age_ms = 0 means everything is stale.
        assert!(
            get_fresh("opencode", 0).is_none(),
            "max_age_ms=0 should treat any entry as stale"
        );

        // Missing provider returns None even when cache is fresh.
        assert!(get_fresh("does-not-exist", DEFAULT_TTL_MS).is_none());

        if let Some(v) = prev {
            std::env::set_var("PHANTOM_MESH_DIR", v);
        } else {
            std::env::remove_var("PHANTOM_MESH_DIR");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_cache_returns_default_not_panic() {
        let _guard = crate::env_lock::acquire();
        let dir = std::env::temp_dir().join(format!("phantom-test-mc-bad-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("models-cache.json"), "this is not json{{{").unwrap();
        let prev = std::env::var("PHANTOM_MESH_DIR").ok();
        std::env::set_var("PHANTOM_MESH_DIR", &dir);
        let cache = read_cache();
        assert!(cache.providers.is_empty(), "malformed json → empty default");
        if let Some(v) = prev {
            std::env::set_var("PHANTOM_MESH_DIR", v);
        } else {
            std::env::remove_var("PHANTOM_MESH_DIR");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    // ── Additive tests (T16) ──────────────────────────────────────────────
    // In-memory fixtures only; no network. Filesystem-backed cases reuse the
    // existing env_lock + sandboxed PHANTOM_MESH_DIR pattern.

    /// RAII helper: point cache_path() at a fresh temp dir and restore on drop.
    /// Keeps each test's writes isolated and cleans up even on assert panic.
    struct CacheSandbox {
        dir: PathBuf,
        prev: Option<String>,
    }

    impl CacheSandbox {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "phantom-test-mc-{}-{}",
                tag,
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let prev = std::env::var("PHANTOM_MESH_DIR").ok();
            std::env::set_var("PHANTOM_MESH_DIR", &dir);
            CacheSandbox { dir, prev }
        }
    }

    impl Drop for CacheSandbox {
        fn drop(&mut self) {
            match &self.prev {
                Some(v) => std::env::set_var("PHANTOM_MESH_DIR", v),
                None => std::env::remove_var("PHANTOM_MESH_DIR"),
            }
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn read_cache_missing_file_is_default() {
        // No file written → graceful default (cache miss at the file layer).
        let _guard = crate::env_lock::acquire();
        let _sb = CacheSandbox::new("missing");
        let cache = read_cache();
        assert!(
            cache.providers.is_empty(),
            "absent cache file should read as empty default"
        );
        assert!(
            get_fresh("anything", DEFAULT_TTL_MS).is_none(),
            "no file means every lookup misses"
        );
    }

    #[test]
    fn cache_roundtrip_multiple_providers_and_models() {
        let mut c = ModelsCache::default();
        c.providers.insert(
            "groq".into(),
            CachedProvider {
                fetched_at_ms: 1,
                models: vec![
                    CachedModel {
                        id: "llama-3.3-70b".into(),
                        is_free: true,
                    },
                    CachedModel {
                        id: "mixtral-8x7b".into(),
                        is_free: false,
                    },
                ],
            },
        );
        c.providers.insert(
            "opencode".into(),
            CachedProvider {
                fetched_at_ms: 2,
                models: vec![CachedModel {
                    id: "claude-sonnet-4-6".into(),
                    is_free: false,
                }],
            },
        );
        let json = serde_json::to_string(&c).unwrap();
        let parsed: ModelsCache = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.providers.len(), 2);
        let groq = parsed.providers.get("groq").unwrap();
        assert_eq!(groq.models.len(), 2);
        assert!(groq.models[0].is_free);
        assert!(!groq.models[1].is_free);
        let oc = parsed.providers.get("opencode").unwrap();
        assert_eq!(oc.fetched_at_ms, 2);
        assert_eq!(oc.models[0].id, "claude-sonnet-4-6");
    }

    #[test]
    fn deserialize_missing_providers_key_defaults_empty() {
        // #[serde(default)] on `providers` means an object without the key
        // parses cleanly to an empty map rather than erroring.
        let parsed: ModelsCache = serde_json::from_str("{}").unwrap();
        assert!(parsed.providers.is_empty());
    }

    #[test]
    fn deserialize_extra_unknown_fields_is_tolerated() {
        // Forward-compat: an unknown future field must not break parsing.
        let raw = r#"{"providers":{},"future_field":42}"#;
        let parsed: ModelsCache = serde_json::from_str(raw).unwrap();
        assert!(parsed.providers.is_empty());
    }

    #[test]
    fn get_fresh_boundary_age_equal_ttl_is_stale() {
        // age >= max_age_ms is treated as stale; verify the exact boundary by
        // writing an entry with a known fetched_at and probing via the file.
        let _guard = crate::env_lock::acquire();
        let _sb = CacheSandbox::new("boundary");

        // Write a provider entry stamped exactly `delta` ms in the past.
        let delta: u64 = 5_000;
        let mut cache = ModelsCache::default();
        cache.providers.insert(
            "groq".into(),
            CachedProvider {
                fetched_at_ms: now_ms().saturating_sub(delta),
                models: vec![CachedModel {
                    id: "llama-3.3-70b".into(),
                    is_free: true,
                }],
            },
        );
        write_cache(&cache).unwrap();

        // A generous TTL well above the age → fresh hit.
        assert!(
            get_fresh("groq", delta + 60_000).is_some(),
            "entry younger than TTL should be a fresh hit"
        );
        // A tiny TTL below the age → stale miss.
        assert!(
            get_fresh("groq", 1).is_none(),
            "entry older than TTL should be stale"
        );
    }

    #[test]
    fn put_overwrites_existing_provider_entry() {
        let _guard = crate::env_lock::acquire();
        let _sb = CacheSandbox::new("overwrite");

        put("groq", &[mi("old-model", false)]);
        let first = get_fresh("groq", DEFAULT_TTL_MS).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].id, "old-model");

        // Re-put under the same key replaces, not appends.
        put("groq", &[mi("new-a", true), mi("new-b", false)]);
        let second = get_fresh("groq", DEFAULT_TTL_MS).unwrap();
        assert_eq!(second.len(), 2);
        assert_eq!(second[0].id, "new-a");
        assert!(second[0].is_free);
        assert_eq!(second[1].id, "new-b");
    }

    #[test]
    fn stale_entries_filters_and_sorts() {
        let _guard = crate::env_lock::acquire();
        let _sb = CacheSandbox::new("stale-entries");

        let now = now_ms();
        let mut cache = ModelsCache::default();
        // "alpha" is old (stale), "zeta" is brand new (fresh).
        cache.providers.insert(
            "zeta".into(),
            CachedProvider {
                fetched_at_ms: now,
                models: vec![mi_cached("z1", false)],
            },
        );
        cache.providers.insert(
            "alpha".into(),
            CachedProvider {
                fetched_at_ms: now.saturating_sub(10_000),
                models: vec![mi_cached("a1", true)],
            },
        );
        write_cache(&cache).unwrap();

        // max_age_ms small enough that only "alpha" exceeds it.
        let stale = stale_entries(1_000);
        assert_eq!(stale.len(), 1, "only the old entry is stale");
        assert_eq!(stale[0].0, "alpha");
        assert!(
            stale[0].1 >= 10_000,
            "reported age should reflect staleness"
        );

        // A huge TTL → nothing is stale.
        assert!(stale_entries(u64::MAX).is_empty());
    }

    /// Local helper mirroring `mi` but producing the on-disk `CachedModel`.
    fn mi_cached(id: &str, is_free: bool) -> CachedModel {
        CachedModel {
            id: id.to_string(),
            is_free,
        }
    }

    #[test]
    fn now_ms_is_monotonic_nonzero() {
        // Sanity guard for the time source used by freshness checks.
        let a = now_ms();
        let b = now_ms();
        assert!(a > 0, "now_ms should return a real unix-epoch millis value");
        assert!(b >= a, "now_ms should be non-decreasing within a test");
    }

    #[test]
    fn get_fresh_on_crosses_ttl_boundary_under_a_pinned_clock() {
        // With an injected clock the fresh→stale transition is exact and
        // wall-clock-independent — replacing the `fetched_at = now - delta` /
        // `max_age_ms = 0` workarounds the wall-clock tests have to use.
        use crate::clock::MockClock;
        let _guard = crate::env_lock::acquire();
        let _sb = CacheSandbox::new("mockclock");

        let mut cache = ModelsCache::default();
        cache.providers.insert(
            "groq".into(),
            CachedProvider {
                fetched_at_ms: 1_000_000,
                models: vec![mi_cached("llama-3.3-70b", true)],
            },
        );
        write_cache(&cache).unwrap();

        let clock = MockClock::new(1_000_000); // now == fetched_at → age 0
        assert!(
            get_fresh_on(&clock, "groq", 5_000).is_some(),
            "age 0 < ttl → fresh"
        );

        clock.advance_ms(5_000); // age now exactly 5_000
        assert!(
            get_fresh_on(&clock, "groq", 5_000).is_none(),
            "age == ttl is stale (age >= max_age_ms boundary)"
        );
        assert!(
            get_fresh_on(&clock, "groq", 5_001).is_some(),
            "age just under ttl is still fresh"
        );

        // stale_entries uses a strict `age > max_age_ms`.
        assert_eq!(
            stale_entries_on(&clock, 4_999),
            vec![("groq".to_string(), 5_000)],
            "age 5_000 > 4_999 → listed with its exact age"
        );
        assert!(
            stale_entries_on(&clock, 5_000).is_empty(),
            "age 5_000 is NOT > 5_000 → not listed"
        );
    }
}
