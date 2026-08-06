//! Key management primitives.
//!
//! Read and edit `~/.spectyn-mesh/agents.toml` non-destructively
//! (preserves comments, ordering, formatting via `toml_edit`).
//! REPL slash commands `/keys` / `/keys add` / `/keys remove` /
//! `/keys test` are thin wrappers around these.

use std::fs;
use std::path::{Path, PathBuf};

/// Returns the canonical path to agents.toml — `~/.spectyn-mesh/agents.toml`.
pub fn agents_toml_path() -> PathBuf {
    crate::cli_config::spectyn_data_dir()
        .unwrap_or_else(|_| PathBuf::from(".").join(".spectyn-mesh"))
        .join("agents.toml")
}

/// Quick view of one provider's key state — surfaced by `/keys` slash.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum KeyState {
    /// `api_key = "..."` is set inline (literal value or `${ENV_VAR}` that resolved).
    Inline,
    /// `api_key_env = "VAR"` is set, and `VAR` is set in process env.
    EnvResolved { var: String },
    /// `api_key_env = "VAR"` is set but `VAR` is empty / unset.
    EnvMissing { var: String },
    /// Neither `api_key` nor `api_key_env` is set.
    NotConfigured,
}

/// Walk an `AgentsConfig` (already loaded) and report each provider's state.
/// Pure-fn — does no I/O. Caller passes the config it already has.
pub fn snapshot_states(cfg: &crate::config::AgentsConfig) -> Vec<(String, KeyState)> {
    let mut out: Vec<(String, KeyState)> = cfg
        .providers
        .iter()
        .map(|(name, ent)| {
            let state = if let Some(key) = &ent.api_key {
                if key.is_empty() {
                    // Empty after env interpolation — treat as missing.
                    KeyState::NotConfigured
                } else {
                    KeyState::Inline
                }
            } else if let Some(var) = &ent.api_key_env {
                match std::env::var(var) {
                    Ok(v) if !v.is_empty() => KeyState::EnvResolved { var: var.clone() },
                    _ => KeyState::EnvMissing { var: var.clone() },
                }
            } else {
                KeyState::NotConfigured
            };
            (name.clone(), state)
        })
        .collect();
    out.sort_by(|(a, _), (b, _)| a.cmp(b));
    out
}

/// Remove the `api_key` field from `[providers.<name>]` in agents.toml.
/// Preserves all other formatting (comments, ordering, spacing).
/// Errors if the file is missing or the provider isn't found.
pub fn remove_api_key(path: &Path, provider: &str) -> anyhow::Result<()> {
    let content =
        fs::read_to_string(path).map_err(|e| anyhow::anyhow!("read {}: {}", path.display(), e))?;
    let mut doc: toml_edit::DocumentMut = content
        .parse()
        .map_err(|e| anyhow::anyhow!("parse toml: {}", e))?;

    let providers = doc
        .get_mut("providers")
        .and_then(|v| v.as_table_mut())
        .ok_or_else(|| anyhow::anyhow!("no [providers] section in {}", path.display()))?;

    let entry = providers
        .get_mut(provider)
        .and_then(|v| v.as_table_mut())
        .ok_or_else(|| anyhow::anyhow!("provider '{}' not found", provider))?;

    if entry.remove("api_key").is_none() {
        anyhow::bail!("provider '{}' has no api_key set", provider);
    }

    write_atomic(path, &doc.to_string())?;
    Ok(())
}

/// Set the `api_key` field on `[providers.<name>]`. Creates the
/// `[providers.<name>]` table if it doesn't exist yet (so `/keys add` for a
/// brand-new provider works without hand-editing).
pub fn set_api_key(path: &Path, provider: &str, key: &str) -> anyhow::Result<()> {
    let content = if path.exists() {
        fs::read_to_string(path).map_err(|e| anyhow::anyhow!("read {}: {}", path.display(), e))?
    } else {
        // Fresh file — start with an empty `[providers]` table.
        String::from("# spectyn agents.toml\n\n[providers]\n")
    };

    let mut doc: toml_edit::DocumentMut = content
        .parse()
        .map_err(|e| anyhow::anyhow!("parse toml: {}", e))?;

    // Ensure [providers] exists (and is implicit so it doesn't print empty).
    if doc.get("providers").is_none() {
        let mut t = toml_edit::Table::new();
        t.set_implicit(true);
        doc.insert("providers", toml_edit::Item::Table(t));
    }
    let providers = doc
        .get_mut("providers")
        .and_then(|v| v.as_table_mut())
        .ok_or_else(|| anyhow::anyhow!("[providers] is not a table"))?;
    providers.set_implicit(true);

    if providers.get(provider).is_none() {
        let mut child = toml_edit::Table::new();
        // Default the type/url for known providers — saves the user
        // from having to remember the exact base_url.
        if let Some((ty, url)) = default_provider_meta(provider) {
            child.insert("type", toml_edit::value(ty));
            child.insert("url", toml_edit::value(url));
        }
        providers.insert(provider, toml_edit::Item::Table(child));
    }
    let entry = providers
        .get_mut(provider)
        .and_then(|v| v.as_table_mut())
        .ok_or_else(|| anyhow::anyhow!("[providers.{}] is not a table", provider))?;
    // apex P4: seal the key at rest when SPECTYN_ENCRYPT_AGENTS is on. When OFF
    // this returns `key` unchanged, so the written bytes are byte-identical to
    // today. Fail closed: if sealing is enabled but no EventKey is available,
    // refuse the write instead of persisting the key in plaintext.
    let stored = crate::skillbank::agents_seal::seal_api_key_for_save(key)
        .map_err(|e| anyhow::anyhow!("seal api_key for provider '{}': {}", provider, e))?;
    entry.insert("api_key", toml_edit::value(stored));

    // Make sure the parent dir exists (first-run case).
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).ok();
    }
    write_atomic(path, &doc.to_string())?;
    Ok(())
}

/// Set a secret string `field` on a top-level table (`[tools]` or `[core]`) of
/// agents.toml, sealing it at rest under the SAME seam as [`set_api_key`].
///
/// apex P4 follow-up: covers the non-provider secrets
/// (`[tools].brave_search_api_key`, `[tools].todoist_api_token`,
/// `[core].hub_api_key`). Creates the `[table]` if absent. Preserves all other
/// formatting (comments, ordering) via `toml_edit`.
///
/// Sealing semantics are identical to `set_api_key`: with
/// `SPECTYN_ENCRYPT_AGENTS` OFF the written bytes are byte-identical to today
/// (plaintext); with it ON the value is sealed via `agents_seal`, and a missing
/// `EventKey` makes the write FAIL CLOSED rather than persisting plaintext.
pub fn set_table_secret(path: &Path, table: &str, field: &str, value: &str) -> anyhow::Result<()> {
    let content = if path.exists() {
        fs::read_to_string(path).map_err(|e| anyhow::anyhow!("read {}: {}", path.display(), e))?
    } else {
        // Fresh file — a bare header; the table is inserted below.
        String::from("# spectyn agents.toml\n")
    };

    let mut doc: toml_edit::DocumentMut = content
        .parse()
        .map_err(|e| anyhow::anyhow!("parse toml: {}", e))?;

    if doc.get(table).is_none() {
        doc.insert(table, toml_edit::Item::Table(toml_edit::Table::new()));
    }
    let tbl = doc
        .get_mut(table)
        .and_then(|v| v.as_table_mut())
        .ok_or_else(|| anyhow::anyhow!("[{}] is not a table", table))?;

    // apex P4: seal the secret at rest when SPECTYN_ENCRYPT_AGENTS is on. When
    // OFF this returns `value` unchanged, so the written bytes are byte-identical
    // to today. Fail closed: if sealing is enabled but no EventKey is available,
    // refuse the write instead of persisting the secret in plaintext.
    let stored = crate::skillbank::agents_seal::seal_api_key_for_save(value)
        .map_err(|e| anyhow::anyhow!("seal {}.{}: {}", table, field, e))?;
    tbl.insert(field, toml_edit::value(stored));

    // Make sure the parent dir exists (first-run case).
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).ok();
    }
    write_atomic(path, &doc.to_string())?;
    Ok(())
}

/// Return (type, base_url) for the providers we know how to default.
/// Unknown names return None — caller should still let the user proceed
/// (they'll fill in url manually later).
pub fn default_provider_meta(name: &str) -> Option<(&'static str, &'static str)> {
    match name {
        "groq" => Some(("groq", "https://api.groq.com/openai/v1")),
        "gemini" => Some(("gemini", "https://generativelanguage.googleapis.com/v1beta")),
        "openrouter" => Some(("openrouter", "https://openrouter.ai/api/v1")),
        "anthropic" => Some(("anthropic", "https://api.anthropic.com/v1")),
        "openai" => Some(("openai", "https://api.openai.com/v1")),
        // OpenCode Zen gateway — /api/v1 returns 404, /zen/v1 is live.
        "opencode" => Some(("opencode", "https://opencode.ai/zen/v1")),
        // remote-control Telegram adapter — track [O1].
        // `type` is informational; the remote_control::telegram module reads
        // the api_key directly. url points at the Bot API root for
        // any future probe path that wants to GET /bot<token>/getMe.
        "telegram_bot" => Some(("telegram_bot", "https://api.telegram.org")),
        _ => None,
    }
}

/// Outcome of a `/keys test` probe — surfaced to the user.
#[derive(Debug)]
pub struct ProbeResult {
    pub ok: bool,
    pub status: u16,
    pub model_count: Option<usize>,
    pub message: String,
    pub elapsed_ms: u128,
}

/// Probe a provider by hitting its `/models` endpoint with the supplied
/// API key. 3-s timeout. Never logs the key.
///
/// Returns Ok(ProbeResult) on any HTTP outcome — even 401 or 403 are
/// "the network worked, the key just isn't accepted." Errors are
/// reserved for transport-level failures (DNS, connection refused).
pub async fn probe_provider(
    provider: &str,
    base_url: &str,
    key: &str,
) -> anyhow::Result<ProbeResult> {
    use std::time::Instant;
    let t0 = Instant::now();

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()?;

    let (url, headers): (String, Vec<(&'static str, String)>) = match provider {
        // Standard OpenAI-compatible providers — Bearer auth + GET /models
        "groq" | "openai" | "openrouter" | "opencode" => (
            format!("{}/models", base_url.trim_end_matches('/')),
            vec![("Authorization", format!("Bearer {}", key))],
        ),
        // Anthropic — same path but uses x-api-key header + version
        "anthropic" => (
            format!("{}/models", base_url.trim_end_matches('/')),
            vec![
                ("x-api-key", key.to_string()),
                ("anthropic-version", "2023-06-01".to_string()),
            ],
        ),
        // Gemini — query string ?key=...
        "gemini" => (
            format!("{}/models?key={}", base_url.trim_end_matches('/'), key),
            vec![],
        ),
        // Unknown provider type — try the generic /models with Bearer.
        _ => (
            format!("{}/models", base_url.trim_end_matches('/')),
            vec![("Authorization", format!("Bearer {}", key))],
        ),
    };

    let mut req = client.get(&url);
    for (h, v) in &headers {
        req = req.header(*h, v);
    }
    let resp = req.send().await?;
    let status = resp.status();
    let body_text = match resp.text().await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(url = %url, status = %status, "probe response body read failed; reporting empty: {}", e);
            String::new()
        }
    };
    let elapsed_ms = t0.elapsed().as_millis();

    let model_count = parse_model_count(&body_text);
    let (ok, message) = match status.as_u16() {
        200..=299 => (
            true,
            format!(
                "{} {}",
                status,
                model_count
                    .map(|n| format!("({} models)", n))
                    .unwrap_or_else(|| "(parsed OK)".into())
            ),
        ),
        401 | 403 => (false, format!("{} — key rejected", status)),
        429 => (false, format!("{} — rate limited", status)),
        other => (
            false,
            format!(
                "{} — {}",
                other,
                body_text.chars().take(200).collect::<String>().trim()
            ),
        ),
    };

    Ok(ProbeResult {
        ok,
        status: status.as_u16(),
        model_count,
        message,
        elapsed_ms,
    })
}

/// Try to extract the count of models from a /models response.
/// Most providers return either `{"data": [...]}` (OpenAI-style) or
/// `{"models": [...]}` (Gemini, Anthropic).
fn parse_model_count(body: &str) -> Option<usize> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    if let Some(arr) = v.get("data").and_then(|d| d.as_array()) {
        return Some(arr.len());
    }
    if let Some(arr) = v.get("models").and_then(|d| d.as_array()) {
        return Some(arr.len());
    }
    None
}

/// One row in a provider's `/v1/models` response, augmented with a free/paid
/// classification. Returned by `fetch_models_annotated`.
#[derive(Debug, Clone)]
pub struct ModelInfo {
    pub id: String,
    /// Heuristic from `is_likely_free_model`. NOT authoritative — providers
    /// don't reliably expose pricing in their /models endpoint, so this can
    /// mislabel. Confirm with the provider's billing dashboard.
    pub is_free: bool,
}

/// Heuristic: is this model likely on a free tier?
/// Rules (first match wins):
///   1. Local endpoint (ollama, 127.0.0.1, localhost, :11434) → free.
///   2. Model id has `-free`, `:free`, `/free`, or `-free-` marker.
///   3. Per-provider whitelist (OpenCode Zen's known 5 free preview models).
///   4. Otherwise: paid.
/// Conservative: quota-limited tiers (Groq, Cerebras, Mistral) where the
/// model id has no marker are reported as `paid` so the user isn't lulled
/// into thinking the call is unlimited.
pub fn is_likely_free_model(provider_type: &str, base_url: Option<&str>, model_id: &str) -> bool {
    if let Some(url) = base_url {
        let u = url.to_lowercase();
        if u.contains("127.0.0.1") || u.contains("localhost") || u.contains(":11434") {
            return true;
        }
    }
    if provider_type.eq_ignore_ascii_case("ollama") {
        return true;
    }
    let id = model_id.to_lowercase();
    if id.ends_with("-free")
        || id.contains(":free")
        || id.contains("/free")
        || id.contains("-free-")
    {
        return true;
    }
    if provider_type == "opencode" {
        const OPENCODE_ZEN_FREE: &[&str] = &[
            "minimax-m2.5-free",
            "hy3-preview-free",
            "ling-2.6-flash-free",
            "trinity-large-preview-free",
            "nemotron-3-super-free",
        ];
        if OPENCODE_ZEN_FREE.contains(&model_id) {
            return true;
        }
    }
    false
}

/// Same as `fetch_models` but returns annotated rows with free/paid flags.
pub async fn fetch_models_annotated(
    provider_type: &str,
    base_url: &str,
    key: &str,
) -> anyhow::Result<Vec<ModelInfo>> {
    let ids = fetch_models(provider_type, base_url, key).await?;
    Ok(ids
        .into_iter()
        .map(|id| ModelInfo {
            is_free: is_likely_free_model(provider_type, Some(base_url), &id),
            id,
        })
        .collect())
}

/// Fetch the actual list of model names from a provider — used by
/// `/model fetch`. Reuses the same auth dispatch as probe_provider.
/// Returns sorted, deduplicated model identifiers; empty Vec on
/// transport failure (errors logged via tracing, not bubbled).
///
/// For UIs that want a free/paid annotation per model, prefer
/// `fetch_models_annotated` which wraps this and adds the heuristic flag.
pub async fn fetch_models(
    provider: &str,
    base_url: &str,
    key: &str,
) -> anyhow::Result<Vec<String>> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()?;

    let (url, headers): (String, Vec<(&'static str, String)>) = match provider {
        "groq" | "openai" | "openrouter" | "opencode" => (
            format!("{}/models", base_url.trim_end_matches('/')),
            vec![("Authorization", format!("Bearer {}", key))],
        ),
        "anthropic" => (
            format!("{}/models", base_url.trim_end_matches('/')),
            vec![
                ("x-api-key", key.to_string()),
                ("anthropic-version", "2023-06-01".to_string()),
            ],
        ),
        "gemini" => (
            format!("{}/models?key={}", base_url.trim_end_matches('/'), key),
            vec![],
        ),
        _ => (
            format!("{}/models", base_url.trim_end_matches('/')),
            vec![("Authorization", format!("Bearer {}", key))],
        ),
    };

    let mut req = client.get(&url);
    for (h, v) in &headers {
        req = req.header(*h, v);
    }
    let resp = req.send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("HTTP {} from {}/models", resp.status(), provider);
    }
    let body = resp.text().await?;
    let v: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| anyhow::anyhow!("bad JSON from {}: {}", provider, e))?;

    let mut ids: Vec<String> = Vec::new();
    // OpenAI-style: { data: [{id: "..."}, ...] }
    if let Some(arr) = v.get("data").and_then(|d| d.as_array()) {
        for item in arr {
            if let Some(id) = item.get("id").and_then(|s| s.as_str()) {
                ids.push(id.to_string());
            }
        }
    }
    // Gemini / Anthropic-style: { models: [{name: "..."} | {id: "..."}, ...] }
    if let Some(arr) = v.get("models").and_then(|d| d.as_array()) {
        for item in arr {
            if let Some(name) = item
                .get("name")
                .and_then(|s| s.as_str())
                .or_else(|| item.get("id").and_then(|s| s.as_str()))
            {
                // Gemini prefixes "models/" — strip for cleaner display.
                let cleaned = name.strip_prefix("models/").unwrap_or(name).to_string();
                ids.push(cleaned);
            }
        }
    }
    ids.sort();
    ids.dedup();
    Ok(ids)
}

/// Atomic write: write to `<path>.tmp` then rename. Sets permission 0600
/// on Unix so secrets aren't world-readable.
fn write_atomic(path: &Path, contents: &str) -> anyhow::Result<()> {
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, contents).map_err(|e| anyhow::anyhow!("write {}: {}", tmp.display(), e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o600);
        fs::set_permissions(&tmp, perms).ok();
    }
    fs::rename(&tmp, path).map_err(|e| anyhow::anyhow!("rename to {}: {}", path.display(), e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AgentsConfig;

    fn cfg_from(toml_str: &str) -> AgentsConfig {
        let mut cfg: AgentsConfig = toml::from_str(toml_str).unwrap();
        cfg.resolve_env_vars();
        cfg
    }

    #[test]
    fn snapshot_marks_inline_keys() {
        let cfg = cfg_from(
            r#"
            [providers.groq]
            type = "groq"
            api_key = "gsk_real"
        "#,
        );
        let s = snapshot_states(&cfg);
        assert_eq!(s, vec![("groq".into(), KeyState::Inline)]);
    }

    #[test]
    fn snapshot_marks_env_resolved_and_missing() {
        std::env::set_var("SPECTYN_TEST_GROQ_K6", "abc");
        std::env::remove_var("SPECTYN_TEST_GEMINI_K6");
        let cfg = cfg_from(
            r#"
            [providers.groq]
            type = "groq"
            api_key_env = "SPECTYN_TEST_GROQ_K6"
            [providers.gemini]
            type = "gemini"
            api_key_env = "SPECTYN_TEST_GEMINI_K6"
        "#,
        );
        let mut s = snapshot_states(&cfg);
        s.sort();
        assert_eq!(s.len(), 2);
        assert!(matches!(&s[0].1, KeyState::EnvMissing { var } if var == "SPECTYN_TEST_GEMINI_K6"));
        assert!(matches!(&s[1].1, KeyState::EnvResolved { var } if var == "SPECTYN_TEST_GROQ_K6"));
        std::env::remove_var("SPECTYN_TEST_GROQ_K6");
    }

    #[test]
    fn snapshot_marks_unset_provider() {
        let cfg = cfg_from(
            r#"
            [providers.opencode]
            type = "opencode"
        "#,
        );
        let s = snapshot_states(&cfg);
        assert_eq!(s, vec![("opencode".into(), KeyState::NotConfigured)]);
    }

    #[test]
    fn set_then_remove_round_trip_preserves_other_fields() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("agents.toml");
        fs::write(
            &path,
            "[providers.groq]\ntype = \"groq\"\nurl = \"https://api.groq.com/openai/v1\"\n",
        )
        .unwrap();

        set_api_key(&path, "groq", "gsk_pasted").unwrap();
        let after_add = fs::read_to_string(&path).unwrap();
        assert!(after_add.contains("api_key = \"gsk_pasted\""));
        assert!(
            after_add.contains("type = \"groq\""),
            "type field preserved"
        );
        assert!(after_add.contains("url = "), "url preserved");

        remove_api_key(&path, "groq").unwrap();
        let after_remove = fs::read_to_string(&path).unwrap();
        assert!(
            !after_remove.contains("api_key"),
            "api_key gone after remove"
        );
        assert!(after_remove.contains("type = \"groq\""), "type still there");
    }

    #[test]
    fn set_creates_provider_table_if_absent() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("agents.toml");
        fs::write(&path, "# header comment\n").unwrap();

        set_api_key(&path, "groq", "gsk_new").unwrap();

        let after = fs::read_to_string(&path).unwrap();
        // Header comment preserved
        assert!(after.contains("# header comment"));
        // [providers.groq] auto-created with sensible defaults
        assert!(after.contains("[providers.groq]"));
        assert!(after.contains("type = \"groq\""));
        assert!(after.contains("url = \"https://api.groq.com/openai/v1\""));
        assert!(after.contains("api_key = \"gsk_new\""));
    }

    #[test]
    fn remove_errors_when_provider_missing() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("agents.toml");
        fs::write(
            &path,
            "[providers.groq]\ntype = \"groq\"\napi_key = \"x\"\n",
        )
        .unwrap();

        assert!(remove_api_key(&path, "nonesuch").is_err());
        // groq's api_key is still there
        let after = fs::read_to_string(&path).unwrap();
        assert!(after.contains("api_key = \"x\""));
    }

    // ── is_likely_free_model ───────────────────────────────────────────

    #[test]
    fn free_marker_explicit_suffixes() {
        // -free suffix
        assert!(is_likely_free_model(
            "opencode",
            Some("https://opencode.ai/zen/v1"),
            "minimax-m2.5-free"
        ));
        assert!(is_likely_free_model(
            "opencode",
            Some("https://opencode.ai/zen/v1"),
            "hy3-preview-free"
        ));
        // :free marker (OpenRouter pattern)
        assert!(is_likely_free_model(
            "openrouter",
            Some("https://openrouter.ai/api/v1"),
            "meta-llama/llama-3.1-8b:free"
        ));
        // /free segment
        assert!(is_likely_free_model(
            "openrouter",
            None,
            "google/gemma-2-9b/free"
        ));
    }

    #[test]
    fn free_marker_local_endpoints() {
        assert!(is_likely_free_model(
            "openai_compat",
            Some("http://localhost:11434/v1"),
            "qwen3:8b"
        ));
        assert!(is_likely_free_model(
            "openai_compat",
            Some("http://127.0.0.1:8080"),
            "anything"
        ));
        assert!(is_likely_free_model("ollama", None, "llama3.2"));
    }

    #[test]
    fn free_marker_paid_models() {
        // Frontier models on remote providers — paid even if account has free quota.
        assert!(!is_likely_free_model(
            "opencode",
            Some("https://opencode.ai/zen/v1"),
            "claude-sonnet-4-6"
        ));
        assert!(!is_likely_free_model(
            "opencode",
            Some("https://opencode.ai/zen/v1"),
            "gpt-5.4"
        ));
        assert!(!is_likely_free_model(
            "groq",
            Some("https://api.groq.com/openai/v1"),
            "llama-3.3-70b-versatile"
        ));
        assert!(!is_likely_free_model(
            "anthropic",
            Some("https://api.anthropic.com/v1"),
            "claude-3-5-sonnet-20241022"
        ));
    }

    #[test]
    fn free_marker_opencode_known_freetier_whitelist() {
        // Models in the whitelist that DON'T have -free suffix would still
        // be marked. (Currently all 5 do have the suffix; this test just
        // documents the whitelist mechanism for future entries.)
        for id in &[
            "minimax-m2.5-free",
            "hy3-preview-free",
            "ling-2.6-flash-free",
            "trinity-large-preview-free",
            "nemotron-3-super-free",
        ] {
            assert!(
                is_likely_free_model("opencode", Some("https://opencode.ai/zen/v1"), id),
                "expected {} to be marked free",
                id
            );
        }
    }

    // ── default_provider_meta: telegram_bot (track O1) ─────────────────

    #[test]
    fn default_provider_meta_recognises_telegram_bot() {
        // Track [O1] — remote-control Telegram adapter.
        // The keys.rs flow needs an entry so that
        //   spectyn keys add telegram_bot <token>     (TUI /keys add)
        // creates a sensible [providers.telegram_bot] table without
        // forcing the user to remember the type/url strings.
        let meta = default_provider_meta("telegram_bot");
        assert_eq!(
            meta,
            Some(("telegram_bot", "https://api.telegram.org")),
            "telegram_bot should map to type=telegram_bot, url=Bot API root"
        );
    }

    #[test]
    fn set_api_key_creates_telegram_bot_table_with_defaults() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("agents.toml");
        // Fresh file
        fs::write(&path, "# header\n").unwrap();

        // Bot tokens look like 123456789:ABC-DEF... — never log this value.
        set_api_key(&path, "telegram_bot", "FAKE_TEST_TOKEN_123:abc").unwrap();

        let after = fs::read_to_string(&path).unwrap();
        assert!(after.contains("[providers.telegram_bot]"));
        assert!(after.contains("type = \"telegram_bot\""));
        assert!(after.contains("url = \"https://api.telegram.org\""));
        assert!(after.contains("api_key = \"FAKE_TEST_TOKEN_123:abc\""));
    }
}
