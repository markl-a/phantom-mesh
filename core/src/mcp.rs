//! `phantom mcp` — MCP (Model Context Protocol) stdio server.
//!
//! Implements the 2024-11-05 MCP spec over the stdio transport.
//! Each newline-delimited JSON-RPC 2.0 message is read from stdin;
//! responses are written to stdout.  stderr is reserved for diagnostics.
//!
//! Supported methods:
//!   initialize                  → server capabilities / version handshake
//!   notifications/initialized   → client ready notification (no-op)
//!   tools/list                  → return all 40 phantom-mesh tool schemas
//!   tools/call                  → execute a tool and return text content
//!   ping                        → liveness check
//!
//! Once this server is running you can register it in any MCP host:
//!
//!   Claude Code ~/.claude/settings.json:
//!     { "mcpServers": { "phantom": { "command": "phantom", "args": ["mcp"] } } }
//!
//!   Codex:
//!     codex --mcp-server "phantom mcp"
//!
//!   Goose:
//!     goose session --with-extension "phantom mcp"

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::config::ToolsConfig;

const PROTOCOL_VERSION: &str = "2024-11-05";

// ── Entry point ───────────────────────────────────────────────────────────────

pub async fn run_stdio(tools_config: ToolsConfig) -> anyhow::Result<()> {
    let stdin  = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let mut reader = BufReader::new(stdin).lines();
    let mut writer = tokio::io::BufWriter::new(stdout);

    tracing::info!("phantom MCP server started (stdio)");

    while let Some(line) = reader.next_line().await? {
        let line = line.trim().to_string();
        if line.is_empty() { continue; }

        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                send(&mut writer, json!({
                    "jsonrpc": "2.0",
                    "id":      null,
                    "error":   { "code": -32700, "message": format!("Parse error: {}", e) }
                })).await?;
                continue;
            }
        };

        let method = msg["method"].as_str().unwrap_or("").to_string();
        let id     = msg["id"].clone();

        // Client notifications have no "id" → no response required.
        if msg.get("id").is_none() {
            tracing::debug!(method, "MCP client notification");
            continue;
        }

        let outcome = handle(&method, &msg["params"], &tools_config).await;
        match outcome {
            Ok(result) => {
                send(&mut writer, json!({ "jsonrpc": "2.0", "id": id, "result": result })).await?;
            }
            Err((code, message)) => {
                send(&mut writer, json!({
                    "jsonrpc": "2.0",
                    "id":      id,
                    "error":   { "code": code, "message": message }
                })).await?;
            }
        }
    }

    Ok(())
}

// ── Method dispatcher ─────────────────────────────────────────────────────────

/// Public entry point for `phantom serve`'s POST /mcp endpoint.
pub async fn handle_http(
    method: &str,
    params: &Value,
    tools_config: &ToolsConfig,
) -> Result<Value, (i64, String)> {
    handle(method, params, tools_config).await
}

async fn handle(
    method: &str,
    params: &Value,
    tools_config: &ToolsConfig,
) -> Result<Value, (i64, String)> {
    match method {
        // ── Handshake ─────────────────────────────────────────────────────
        "initialize" => Ok(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {
                "tools": { "listChanged": false },
            },
            "serverInfo": {
                "name":    "phantom-mesh",
                "version": env!("CARGO_PKG_VERSION"),
            }
        })),

        // ── Tool discovery ────────────────────────────────────────────────
        "tools/list" => {
            let mut tools: Vec<Value> = crate::tools::all_tool_names()
                .iter()
                .filter_map(|name| to_mcp_tool(name))
                .collect();
            // Add distributed cluster tools
            tools.push(json!({
                "name": "phantom_swarm",
                "description": "Send a prompt to ALL cluster nodes in parallel and synthesize results. Best for analysis, code review, or tasks that benefit from multiple perspectives.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "prompt": { "type": "string", "description": "The task to run on all nodes" },
                        "agent": { "type": "string", "description": "Agent name (default: master)", "default": "master" }
                    },
                    "required": ["prompt"]
                }
            }));
            tools.push(json!({
                "name": "phantom_evolve_distributed",
                "description": "Decompose a goal into subtasks and run them on all cluster nodes in parallel, then synthesize. Best for large tasks: refactoring, multi-file changes, analysis across a codebase.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "goal": { "type": "string", "description": "The goal to decompose and execute across the cluster" },
                        "rounds": { "type": "integer", "description": "Max evolve rounds per node (default: 5)", "default": 5 }
                    },
                    "required": ["goal"]
                }
            }));
            Ok(json!({ "tools": tools }))
        }

        // ── Tool execution ────────────────────────────────────────────────
        "tools/call" => {
            let name = params["name"].as_str().unwrap_or("");
            if name.is_empty() {
                return Err((-32602, "Missing required parameter: name".into()));
            }

            // Handle distributed cluster tools
            if name == "phantom_swarm" || name == "phantom_evolve_distributed" {
                let phantom_bin = std::env::current_exe()
                    .unwrap_or_else(|_| std::path::PathBuf::from("phantom"));
                let phantom_bin = phantom_bin.to_string_lossy().to_string();

                let output = if name == "phantom_swarm" {
                    let prompt = params["arguments"]["prompt"].as_str().unwrap_or("").to_string();
                    let agent = params["arguments"]["agent"].as_str().unwrap_or("master").to_string();
                    tokio::process::Command::new(&phantom_bin)
                        .args(["swarm", "--agent", &agent, &prompt])
                        .output().await
                        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                        .unwrap_or_else(|e| format!("Error: {}", e))
                } else {
                    let goal = params["arguments"]["goal"].as_str().unwrap_or("").to_string();
                    let rounds = params["arguments"]["rounds"].as_u64().unwrap_or(5).to_string();
                    tokio::process::Command::new(&phantom_bin)
                        .args(["evolve", "--distributed", "--rounds", &rounds, &goal])
                        .output().await
                        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                        .unwrap_or_else(|e| format!("Error: {}", e))
                };

                return Ok(json!({
                    "content": [{ "type": "text", "text": output }],
                    "isError": false,
                }));
            }

            if !crate::tools::all_tool_names().contains(&name) {
                return Err((-32602, format!("Unknown tool: {}", name)));
            }
            let args   = params["arguments"].clone();
            let output = crate::tools::execute(name, &args, tools_config).await;
            Ok(json!({
                "content": [{ "type": "text", "text": output }],
                "isError": false,
            }))
        }

        // ── Liveness ──────────────────────────────────────────────────────
        "ping" => Ok(json!({})),

        other => Err((-32601, format!("Method not found: {}", other))),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Convert a phantom-mesh tool name to an MCP tool descriptor.
///
/// phantom-mesh schemas use the OpenAI function-calling envelope:
///   `{ "type":"function", "function": { "name", "description", "parameters" } }`
///
/// MCP expects:
///   `{ "name", "description", "inputSchema" }`
fn to_mcp_tool(name: &str) -> Option<Value> {
    let schema = crate::tools::schema(name)?;
    let func   = &schema["function"];
    Some(json!({
        "name":        func["name"],
        "description": func["description"],
        "inputSchema": func["parameters"],
    }))
}

async fn send(
    writer: &mut tokio::io::BufWriter<tokio::io::Stdout>,
    value: Value,
) -> anyhow::Result<()> {
    let mut line = value.to_string();
    line.push('\n');
    writer.write_all(line.as_bytes()).await?;
    writer.flush().await?;
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools;

    #[test]
    fn all_tools_have_mcp_schema() {
        let names = tools::all_tool_names();
        let mut missing = vec![];
        for name in &names {
            if to_mcp_tool(name).is_none() {
                missing.push(*name);
            }
        }
        assert!(
            missing.is_empty(),
            "tools missing MCP schema conversion: {:?}",
            missing
        );
    }

    #[test]
    fn mcp_schema_has_required_fields() {
        for name in tools::all_tool_names() {
            if let Some(tool) = to_mcp_tool(name) {
                assert!(tool["name"].is_string(),        "{}: missing name",        name);
                assert!(tool["description"].is_string(), "{}: missing description", name);
                assert!(tool["inputSchema"].is_object(), "{}: missing inputSchema", name);
            }
        }
    }

    #[tokio::test]
    async fn handle_initialize() {
        let cfg = crate::config::ToolsConfig::default();
        let result = handle("initialize", &serde_json::json!({}), &cfg).await.unwrap();
        assert_eq!(result["protocolVersion"], PROTOCOL_VERSION);
        assert!(result["serverInfo"]["name"].is_string());
    }

    #[tokio::test]
    async fn handle_tools_list() {
        let cfg = crate::config::ToolsConfig::default();
        let result = handle("tools/list", &serde_json::json!({}), &cfg).await.unwrap();
        let tools = result["tools"].as_array().unwrap();
        assert!(!tools.is_empty(), "tools list should not be empty");
        // Every entry must have name + description + inputSchema
        for t in tools {
            assert!(t["name"].is_string());
            assert!(t["description"].is_string());
            assert!(t["inputSchema"].is_object());
        }
    }

    #[tokio::test]
    async fn handle_unknown_method() {
        let cfg = crate::config::ToolsConfig::default();
        let err = handle("nonexistent/method", &serde_json::json!({}), &cfg).await.unwrap_err();
        assert_eq!(err.0, -32601);
    }

    #[tokio::test]
    async fn handle_ping() {
        let cfg = crate::config::ToolsConfig::default();
        let result = handle("ping", &serde_json::json!({}), &cfg).await.unwrap();
        assert_eq!(result, serde_json::json!({}));
    }
}
