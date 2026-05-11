// Manual LLM-provider key entry — fallback path that bypasses broker
// login entirely. iOS users who can't / don't want to go through Google
// OAuth on phantommesh.io can paste API keys directly here.
//
// Same target file as broker_sync_from_vault: ~/.phantom-mesh/env, KEY=
// VALUE per line. Each set is followed by std::env::set_var() so the
// running process picks it up immediately.
//
// This module also seeds a minimal agents.toml on iOS when one's
// missing — the agent runtime can't dispatch to any provider without
// that file mapping env-var names to provider-block configs (groq,
// opencode, gemini, etc.). Without seeding, even a fully-populated
// env file is dead code.

use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Allowlisted env keys — same set the phantommesh.io vault recognises.
/// Limiting writes to this list prevents arbitrary process-env pollution.
pub const ALLOWED_KEYS: &[&str] = &[
    "OPENCODE_API_KEY",
    "GROQ_API_KEY",
    "ANTHROPIC_API_KEY",
    "OPENAI_API_KEY",
    "GEMINI_API_KEY",
    "OPENROUTER_API_KEY",
    "CEREBRAS_API_KEY",
    "DEEPSEEK_API_KEY",
    "MISTRAL_API_KEY",
    "TOGETHER_API_KEY",
    "NVIDIA_NIM_API_KEY",
    "CLUSTER_SECRET",
];

fn env_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".phantom-mesh")
        .join("env")
}

fn read_env_file() -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Ok(text) = std::fs::read_to_string(env_path()) else {
        return out;
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            out.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    out
}

fn write_env_file(env: &BTreeMap<String, String>) -> Result<(), String> {
    let path = env_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {parent:?}: {e}"))?;
    }
    let mut buf = String::new();
    for (k, v) in env {
        buf.push_str(k);
        buf.push('=');
        buf.push_str(v);
        buf.push('\n');
    }
    std::fs::write(&path, buf).map_err(|e| format!("write {path:?}: {e}"))?;
    Ok(())
}

#[derive(Serialize)]
pub struct LocalKeysSnapshot {
    /// Per-key presence + masked preview. Caller is the iOS settings UI
    /// — it never needs the raw value, just "is this key set?".
    pub keys: Vec<KeyStatus>,
    pub env_path: String,
}

#[derive(Serialize)]
pub struct KeyStatus {
    pub name: String,
    pub set: bool,
    pub preview: Option<String>, // masked: first 4 + "…" + last 4
}

#[tauri::command]
pub fn list_provider_keys() -> LocalKeysSnapshot {
    let env = read_env_file();
    let keys = ALLOWED_KEYS
        .iter()
        .map(|name| {
            let value = env.get(*name);
            let set = value.map(|v| !v.is_empty()).unwrap_or(false);
            let preview = value.and_then(|v| {
                if v.len() < 8 {
                    None
                } else {
                    Some(format!("{}…{}", &v[..4], &v[v.len() - 4..]))
                }
            });
            KeyStatus {
                name: name.to_string(),
                set,
                preview,
            }
        })
        .collect();
    LocalKeysSnapshot {
        keys,
        env_path: env_path().display().to_string(),
    }
}

/// Set / overwrite one key. Empty value = delete (so user can wipe a
/// key by clearing the input and tapping save).
#[tauri::command]
pub fn set_provider_key(name: String, value: String) -> Result<LocalKeysSnapshot, String> {
    let name_str: &str = name.as_str();
    if !ALLOWED_KEYS.iter().any(|k| *k == name_str) {
        return Err(format!(
            "key '{name}' not in allowlist; expected one of: {}",
            ALLOWED_KEYS.join(", ")
        ));
    }
    let mut env = read_env_file();
    let trimmed = value.trim().to_string();
    if trimmed.is_empty() {
        env.remove(&name);
        std::env::remove_var(&name);
    } else {
        env.insert(name.clone(), trimmed.clone());
        // Push to running process env so the next chat call picks it up
        // without an app restart.
        std::env::set_var(&name, &trimmed);
    }
    write_env_file(&env)?;
    Ok(list_provider_keys())
}

/// Bulk set — accepts a flat object {KEY: "value", ...} and applies all
/// allowlisted entries. Used when pasting a multi-line block.
#[tauri::command]
pub fn set_provider_keys_bulk(
    entries: std::collections::HashMap<String, String>,
) -> Result<LocalKeysSnapshot, String> {
    let mut env = read_env_file();
    let mut applied = 0usize;
    for (k, v) in entries {
        if !ALLOWED_KEYS.iter().any(|allowed| *allowed == k.as_str()) {
            continue; // silently drop unknown keys
        }
        let trimmed = v.trim().to_string();
        if trimmed.is_empty() {
            env.remove(&k);
            std::env::remove_var(&k);
        } else {
            env.insert(k.clone(), trimmed.clone());
            std::env::set_var(&k, &trimmed);
        }
        applied += 1;
    }
    if applied == 0 {
        return Err("no allowlisted keys in input".to_string());
    }
    write_env_file(&env)?;
    Ok(list_provider_keys())
}

// ── Default agents.toml seed ─────────────────────────────────────────────
//
// Without an agents.toml, the agent runtime knows nothing about how to
// translate `OPENAI_API_KEY` env vars into provider config (model,
// endpoint, fallback chain). This template covers the 6 free-tier
// providers with the keys vault sync ships, pinned to a master agent
// that prefers opencode → groq → cerebras (3-way failover).
//
// Sized to be useful out of the box; a power user can edit ~/.phantom-
// mesh/agents.toml later without re-running this seed.

// Schema match: AgentEntry takes singular `provider: String` + `model:
// String` (see core/src/config.rs:451). Provider-block `type` strings
// must match what core/src/providers/mod.rs registers ("opencode" /
// "groq" / "gemini" / "openai" / "anthropic"). Format mirrors the Mac
// agents.toml that's already working in production — opencode zen
// endpoint, minimax-m2.5-free model (verified to call tools).
const DEFAULT_AGENTS_TOML: &str = r#"# phantom-mesh — auto-seeded on iOS first launch.
# Edit freely; this file won't be regenerated unless you delete it.

[core]
host = "127.0.0.1"
port = 7878

[providers.opencode]
type = "opencode"
url = "https://opencode.ai/zen/v1"
api_key_env = "OPENCODE_API_KEY"
default_model = "minimax-m2.5-free"

[providers.groq]
type = "groq"
api_key_env = "GROQ_API_KEY"
default_model = "llama-3.3-70b-versatile"

[providers.gemini]
type = "gemini"
api_key_env = "GEMINI_API_KEY"
default_model = "gemini-2.0-flash"

# ── Master agent — single-provider; iOS UI dispatches chats here. ─────────
# Switch provider by editing this block: opencode | groq | gemini.
[agent.master]
provider = "opencode"
model = "minimax-m2.5-free"
instructions = "You are phantom, a helpful AI agent. Be concise and direct."
"#;

fn agents_toml_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".phantom-mesh")
        .join("agents.toml")
}

/// Idempotent — writes DEFAULT_AGENTS_TOML to ~/.phantom-mesh/agents.toml
/// only when the file is missing. Returns true if it created a new file.
/// Called from lib.rs setup() at app launch and from broker_sync_from_
/// vault after the env file is written, so a fresh user lands with both
/// keys AND a config the agent can actually use.
pub fn seed_default_agents_toml_if_missing() -> std::io::Result<bool> {
    let path = agents_toml_path();
    if path.exists() {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, DEFAULT_AGENTS_TOML)?;
    Ok(true)
}

/// Tauri command — exposed so the diagnostics UI can offer a "re-seed
/// agents.toml" button, and so the Settings panel can show whether the
/// file is present.
#[tauri::command]
pub fn agents_toml_status() -> serde_json::Value {
    let path = agents_toml_path();
    let exists = path.exists();
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    serde_json::json!({
        "exists": exists,
        "path": path.display().to_string(),
        "size": size,
    })
}

/// Tauri command — force-rewrites agents.toml from the embedded default.
/// Useful when the file is corrupt (e.g. duplicate-key TOML parse
/// errors) — same Win-side recovery hatch install.ps1 has.
#[tauri::command]
pub fn reseed_agents_toml() -> Result<serde_json::Value, String> {
    let path = agents_toml_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
    }
    std::fs::write(&path, DEFAULT_AGENTS_TOML).map_err(|e| format!("write: {e}"))?;
    Ok(serde_json::json!({
        "ok": true,
        "path": path.display().to_string(),
        "size": DEFAULT_AGENTS_TOML.len(),
    }))
}

#[cfg(test)]
fn _path_unused(_: &Path) {} // silence unused-import warning when there's no test calling Path
