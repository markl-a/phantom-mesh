use anyhow::Result;
use clap::Subcommand;
use std::io::{self, BufRead, Write};
use tracing::info;

use crate::llm_router::LlmRouter;
use crate::agent_runtime::AgentRuntime;
use crate::tools::ToolRegistry;

/// CLI subcommands for clawtex
#[derive(Subcommand, Debug)]
pub enum CliCommand {
    /// Start the daemon (HTTP server + Telegram bot)
    Daemon,

    /// Run a single prompt and exit
    Run {
        /// The prompt to execute
        prompt: String,
        /// Agent to use (default: master)
        #[arg(long, default_value = "master")]
        agent: String,
    },

    /// Start interactive REPL mode
    Interactive {
        /// Agent to use (default: master)
        #[arg(long, default_value = "master")]
        agent: String,
    },

    /// Display current configuration
    Config,

    /// Show status of providers, tools, and MCP servers
    Status,

    /// Encrypt a secret value for use in config
    EncryptSecret {
        /// The value to encrypt
        value: String,
    },
}

/// Execute `clawtex run "prompt"` — single-shot execution
pub async fn run_once(
    prompt: &str,
    agent_name: &str,
    router: &LlmRouter,
    runtime: &AgentRuntime,
    tool_registry: &ToolRegistry,
) -> Result<()> {
    info!("CLI run: agent={}, prompt={}...", agent_name, &prompt[..prompt.len().min(50)]);

    let result = runtime
        .run(agent_name, prompt, &[], router, tool_registry, None)
        .await?;

    println!("{}", result.output);
    eprintln!(
        "\n--- {} | {:.1}s | {} tool calls | {} tokens ---",
        result.agent_name, result.elapsed_secs, result.tool_calls_made, result.total_tokens
    );

    Ok(())
}

/// Execute `clawtex interactive` — REPL mode
pub async fn run_interactive(
    agent_name: &str,
    router: &LlmRouter,
    runtime: &AgentRuntime,
    tool_registry: &ToolRegistry,
) -> Result<()> {
    println!("clawtex interactive mode (agent: {})", agent_name);
    println!("Type your prompt, or 'quit'/'exit' to leave.\n");

    let stdin = io::stdin();
    let mut history = Vec::new();

    loop {
        print!(">>> ");
        io::stdout().flush()?;

        let mut line = String::new();
        if stdin.lock().read_line(&mut line)? == 0 {
            break; // EOF
        }
        let line = line.trim().to_string();

        if line.is_empty() {
            continue;
        }
        if line == "quit" || line == "exit" || line == "/quit" || line == "/exit" {
            println!("Goodbye!");
            break;
        }
        if line == "/clear" {
            history.clear();
            println!("Conversation cleared.");
            continue;
        }
        if line == "/help" {
            println!("Commands: /clear, /quit, /exit, /help");
            continue;
        }

        match runtime.run(agent_name, &line, &history, router, tool_registry, None).await {
            Ok(result) => {
                println!("\n{}", result.output);
                eprintln!(
                    "--- {:.1}s | {} tools | {} tokens ---\n",
                    result.elapsed_secs, result.tool_calls_made, result.total_tokens
                );

                // Add to history
                history.push(crate::providers::ChatMessage {
                    role: "user".into(),
                    content: line,
                    tool_calls: None,
                    tool_call_id: None,
                });
                history.push(crate::providers::ChatMessage {
                    role: "assistant".into(),
                    content: result.output,
                    tool_calls: None,
                    tool_call_id: None,
                });
            }
            Err(e) => {
                eprintln!("Error: {}", e);
            }
        }
    }

    Ok(())
}

/// Execute `clawtex config` — display configuration
pub fn show_config(config_path: &str) {
    println!("Configuration file: {}", config_path);
    if std::path::Path::new(config_path).exists() {
        match std::fs::read_to_string(config_path) {
            Ok(content) => println!("{}", content),
            Err(e) => eprintln!("Error reading config: {}", e),
        }
    } else {
        println!("(file not found — using defaults)");
    }
}

/// Execute `clawtex status` — show system status
pub async fn show_status(
    router: &LlmRouter,
    runtime: &AgentRuntime,
    tool_registry: &ToolRegistry,
) {
    println!("=== clawtex-core status ===\n");

    // Providers
    println!("Providers:");
    for name in router.provider_names() {
        let alive = router.is_alive(&name).await;
        let status = if alive { "UP" } else { "DOWN" };
        println!("  {} — {}", name, status);
    }

    // Agents
    println!("\nAgents:");
    for name in runtime.list_agents() {
        let config = runtime.get_config(&name);
        let provider = config.and_then(|c| c.provider.as_deref()).unwrap_or("auto");
        let model = config.and_then(|c| c.model.as_deref()).unwrap_or("default");
        println!("  {} — provider={}, model={}", name, provider, model);
    }

    // Tools
    println!("\nTools:");
    for name in tool_registry.names() {
        println!("  {}", name);
    }

    println!();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_commands_debug() {
        // Just verify the enum is constructible
        let cmd = CliCommand::Config;
        assert!(format!("{:?}", cmd).contains("Config"));
    }

    #[test]
    fn test_show_config_nonexistent() {
        // Should not panic
        show_config("/nonexistent/path.toml");
    }
}
