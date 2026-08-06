//! Round-trip demo for track [T1] — Telegram → SpectynAgentDispatcher →
//! AgentRuntime → reply. Supersedes the [O1] echo example.
//!
//! Build + run (PowerShell):
//!   $env:TELEGRAM_BOT_API_KEY = "<paste from spectyn keys list>"
//!   $env:TELEGRAM_ALLOWED_USERS = "<your_telegram_user_id>"
//!   $env:CARGO_TARGET_DIR = "D:/tmp/telegram-dispatch-target"
//!   cargo run -p spectyn-mesh `
//!     --example remote_telegram_dispatch `
//!     --features experimental-remote-control-telegram
//!
//! Then send the bot any text from your Telegram desktop client; the
//! bot dispatches it through the configured spectyn agent and replies
//! with the agent's output. Conversation context is maintained
//! per-chat in-memory for the lifetime of this process.
//!
//! NOT a production binary. Used to produce the round-trip evidence
//! the PR's hard-gate (spec §8.5) requires.

#[cfg(not(feature = "experimental-remote-control-telegram"))]
fn main() {
    eprintln!(
        "remote_telegram_dispatch example requires --features experimental-remote-control-telegram"
    );
    std::process::exit(2);
}

#[cfg(feature = "experimental-remote-control-telegram")]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    use std::sync::Arc;

    use spectyn_mesh::agent::AgentRuntime;
    use spectyn_mesh::remote_control::telegram::{
        run_round_trip, RemoteTelegramBot, RemoteTelegramConfig,
    };
    use spectyn_mesh::remote_control::telegram_agent_dispatcher::SpectynAgentDispatcher;

    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    // Bot token: read from env (set by `spectyn keys set telegram_bot`).
    // Never logged.
    let bot_token = std::env::var("TELEGRAM_BOT_API_KEY").map_err(|_| {
        anyhow::anyhow!(
            "TELEGRAM_BOT_API_KEY env var not set. \
             Run: spectyn keys set telegram_bot <token>, then import it \
             into your shell before re-running."
        )
    })?;

    let allowed_user_ids: Vec<i64> = std::env::var("TELEGRAM_ALLOWED_USERS")
        .ok()
        .map(|s| {
            s.split(',')
                .filter_map(|p| p.trim().parse::<i64>().ok())
                .collect()
        })
        .unwrap_or_default();

    if allowed_user_ids.is_empty() {
        tracing::warn!(
            "TELEGRAM_ALLOWED_USERS is empty — bot will respond to ANY user. \
             Set TELEGRAM_ALLOWED_USERS=<your_user_id> for the round-trip demo."
        );
    }

    // Agent name: defaults to "master" (matches spectyn's default agent
    // in `agents.toml.example`). Override via SPECTYN_AGENT env var.
    let agent_name = std::env::var("SPECTYN_AGENT").unwrap_or_else(|_| "master".to_string());

    // AgentRuntime: load from the standard agents.toml location (the
    // same path the spectyn CLI uses). If that fails (file missing or
    // parse error), fall through to the empty default — startup will
    // succeed but every dispatch will return "All providers failed"
    // until the user configures providers.
    let agents_config = load_agents_config_or_default();
    let runtime = Arc::new(AgentRuntime::new(agents_config));

    let dispatcher = Arc::new(SpectynAgentDispatcher::new(runtime, agent_name.clone()));

    let cfg = RemoteTelegramConfig {
        bot_token,
        allowed_user_ids,
    };
    let bot = Arc::new(RemoteTelegramBot::new(cfg, dispatcher));

    tracing::info!(
        agent = %agent_name,
        "remote_telegram_dispatch: starting long-poll loop. Ctrl-C to quit."
    );
    run_round_trip(bot)
        .await
        .map_err(|e| anyhow::anyhow!("round-trip loop exited: {}", e))
}

/// Load `~/.spectyn-mesh/agents.toml` from the standard spectyn location.
/// On any failure (missing file, parse error, no $HOME) log a warning and
/// return `AgentsConfig::default()` so the bot still starts.
#[cfg(feature = "experimental-remote-control-telegram")]
fn load_agents_config_or_default() -> spectyn_mesh::config::AgentsConfig {
    use spectyn_mesh::cli_config::agents_toml_path;
    use spectyn_mesh::config::AgentsConfig;

    let path = match agents_toml_path() {
        Some(p) => p,
        None => {
            tracing::warn!("no $HOME — falling back to AgentsConfig::default()");
            return AgentsConfig::default();
        }
    };
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(
                error = %e,
                path = %path.display(),
                "could not read agents.toml; falling back to AgentsConfig::default()"
            );
            return AgentsConfig::default();
        }
    };
    match toml::from_str::<AgentsConfig>(&raw) {
        Ok(mut cfg) => {
            cfg.resolve_env_vars();
            cfg
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                "agents.toml parse failed; falling back to AgentsConfig::default()"
            );
            AgentsConfig::default()
        }
    }
}
