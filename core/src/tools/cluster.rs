//! `cluster_*` tools — read-only awareness for multi-machine orchestration.
//!
//! These let a master agent ANSWER "what targets are reachable right now?"
//! before deciding whether to delegate work via `task`/`parallel_tasks`
//! (both already accept a `node:` parameter for cross-machine dispatch).
//!
//! Pure reads — no auth, no side effects. Each wraps an existing pure
//! function in cli_config so the CLI surface (`spectyn cluster status`,
//! `spectyn sessions`) and the agent surface stay in sync.
//!
//! - `cluster_status`   → peer alive/dead + RTT (same as the slash `/cluster`)
//! - `cluster_sessions` → live TUIs across the user's mesh (same as `/cluster who`)
//! - `cluster_peers`    → static peer registry (names + URLs + capabilities)
//!                        from peers.json; lets the LLM enumerate options
//!                        without doing a network call

use serde_json::{json, Value};

/// `cluster_status` — parallel ping every configured peer, return alive/dead + RTT.
pub async fn status(_args: &Value) -> String {
    match crate::cli_config::cluster_status_lines().await {
        Ok(lines) => lines.join("\n"),
        Err(e) => format!("[cluster_status error] {}", e),
    }
}

/// `cluster_sessions` — list live TUI sessions across the user's mesh
/// (heartbeat within the last 60s).
pub async fn sessions(_args: &Value) -> String {
    match crate::cli_config::sessions_lines().await {
        Ok(lines) => lines.join("\n"),
        Err(e) => format!("[cluster_sessions error] {}", e),
    }
}

/// `cluster_peers` — return the static peer registry (names + URLs +
/// capability tags) so the agent can enumerate dispatch targets without a
/// network round-trip. Reads ~/.spectyn-mesh/peers.json directly.
pub async fn peers(_args: &Value) -> String {
    let peers = match crate::cli_config::read_peers_json() {
        Some(p) => p,
        None => {
            return "[cluster_peers] no peers.json found — run `spectyn config pull` to sync"
                .to_string()
        }
    };
    let me = crate::cli_config::resolve_self_node_name().unwrap_or_default();
    if peers.is_empty() {
        return "[cluster_peers] (empty registry)".to_string();
    }
    let json_value: Vec<Value> = peers
        .into_iter()
        .map(|p| {
            let is_self = p.name == me;
            json!({
                "name":         p.name,
                "url":          p.url,
                "label":        p.label.unwrap_or_default(),
                "capabilities": p.capabilities,
                "is_self":      is_self,
            })
        })
        .collect();
    serde_json::to_string_pretty(&json_value)
        .unwrap_or_else(|e| format!("[cluster_peers error] serialize: {}", e))
}
