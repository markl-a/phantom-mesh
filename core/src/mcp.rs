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
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let mut reader = BufReader::new(stdin).lines();
    let mut writer = tokio::io::BufWriter::new(stdout);

    tracing::info!("phantom MCP server started (stdio)");

    while let Some(line) = reader.next_line().await? {
        let line = preprocess_line(line);
        if line.is_empty() {
            continue;
        }

        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                send(
                    &mut writer,
                    json!({
                        "jsonrpc": "2.0",
                        "id":      null,
                        "error":   { "code": -32700, "message": format!("Parse error: {}", e) }
                    }),
                )
                .await?;
                continue;
            }
        };

        let method = msg["method"].as_str().unwrap_or("").to_string();
        let id = msg["id"].clone();

        // Client notifications have no "id" → no response required.
        if msg.get("id").is_none() {
            tracing::debug!(method, "MCP client notification");
            continue;
        }

        let outcome = handle(&method, &msg["params"], &tools_config).await;
        match outcome {
            Ok(result) => {
                send(
                    &mut writer,
                    json!({ "jsonrpc": "2.0", "id": id, "result": result }),
                )
                .await?;
            }
            Err((code, message)) => {
                send(
                    &mut writer,
                    json!({
                        "jsonrpc": "2.0",
                        "id":      id,
                        "error":   { "code": code, "message": message }
                    }),
                )
                .await?;
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
                // These spawn a `phantom` subprocess instead of going through
                // tools::execute, so gate them explicitly — a deny/trust/plan
                // policy must block launching distributed agent work too.
                if let Err(reason) = crate::tools::gate_allows(name, &params["arguments"]) {
                    return Ok(json!({
                        "content": [{ "type": "text", "text": format!("[denied] {reason}") }],
                        "isError": true,
                    }));
                }
                let phantom_bin =
                    std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("phantom"));
                let phantom_bin = phantom_bin.to_string_lossy().to_string();

                let output = if name == "phantom_swarm" {
                    let prompt = params["arguments"]["prompt"]
                        .as_str()
                        .unwrap_or("")
                        .to_string();
                    let agent = params["arguments"]["agent"]
                        .as_str()
                        .unwrap_or("master")
                        .to_string();
                    tokio::process::Command::new(&phantom_bin)
                        .args(["swarm", "--agent", &agent, &prompt])
                        .output()
                        .await
                        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                        .unwrap_or_else(|e| format!("Error: {}", e))
                } else {
                    let goal = params["arguments"]["goal"]
                        .as_str()
                        .unwrap_or("")
                        .to_string();
                    let rounds = params["arguments"]["rounds"]
                        .as_u64()
                        .unwrap_or(5)
                        .to_string();
                    tokio::process::Command::new(&phantom_bin)
                        .args(["evolve", "--distributed", "--rounds", &rounds, &goal])
                        .output()
                        .await
                        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                        .unwrap_or_else(|e| format!("Error: {}", e))
                };

                let is_err = is_error_output(&output);
                return Ok(json!({
                    "content": [{ "type": "text", "text": output }],
                    "isError": is_err,
                }));
            }

            if !crate::tools::all_tool_names().contains(&name) {
                return Err((-32602, format!("Unknown tool: {}", name)));
            }
            let args = params["arguments"].clone();
            let output = crate::tools::execute(name, &args, tools_config).await;
            let is_err = is_error_output(&output);
            Ok(json!({
                "content": [{ "type": "text", "text": output }],
                "isError": is_err,
            }))
        }

        // ── Liveness ──────────────────────────────────────────────────────
        "ping" => Ok(json!({})),

        other => Err((-32601, format!("Method not found: {}", other))),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Heuristic: did a phantom tool return a textual error?
///
/// MCP 2024-11-05 §`tools/call` requires the server to set `isError: true`
/// whenever a tool call fails (blocked-shell, missing file, timeout, …).
/// Phantom tools return `String` and signal failure by prefixing the message
/// with one of the conventions below. We honour all of them so the MCP host
/// can surface failures to the user/LLM correctly.
///
/// This is intentionally conservative — we only flag strings that *clearly*
/// look like errors. False negatives are acceptable (host treats it as
/// success); false positives are not (host would suppress real output).
pub(crate) fn is_error_output(s: &str) -> bool {
    let t = s.trim_start();
    if t.is_empty() {
        return false;
    }
    // Common prefixes used across core/src/tools/*.rs
    t.starts_with("Error:")
        || t.starts_with("Error ")              // "Error reading", "Error writing", …
        || t.starts_with("[error]")
        || t.starts_with("[mcp:")               // mcp_client::McpRegistry::dispatch error envelope
        || t.starts_with("ERROR:")
}

/// Convert a phantom-mesh tool name to an MCP tool descriptor.
///
/// phantom-mesh schemas use the OpenAI function-calling envelope:
///   `{ "type":"function", "function": { "name", "description", "parameters" } }`
///
/// MCP expects:
///   `{ "name", "description", "inputSchema" }`
fn to_mcp_tool(name: &str) -> Option<Value> {
    let schema = crate::tools::schema(name)?;
    let func = &schema["function"];
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

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Strip a leading UTF-8 BOM (U+FEFF) plus surrounding whitespace from one
/// line of MCP stdin input.
///
/// PowerShell 5.1 prepends `EF BB BF` to stdout when piping a native
/// command on non-UTF-8 console codepages (CP950/CP932/CP949 etc., the
/// default on most localised Windows installs). Without this strip,
/// `serde_json::from_str` rejects the first line with
/// `Parse error: expected value at line 1 column 1`, making phantom-mcp
/// unreachable from default-config PowerShell clients — the standard MCP
/// transport on Windows. RFC 8259 §8.1 permits implementations to ignore
/// a leading BOM.
fn preprocess_line(line: String) -> String {
    line.trim()
        .trim_start_matches('\u{FEFF}')
        .trim()
        .to_string()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools;

    #[test]
    fn server_tolerates_utf8_bom_on_stdin() {
        // Repro from 2026-05-19 sweep (PR #254 / #256): PowerShell 5.1 on a
        // non-UTF-8 console codepage prepends EF BB BF to native-command
        // pipes. The MCP server's serde_json::from_str then rejects line 1
        // with `Parse error: expected value at line 1 column 1`, breaking
        // every default-PS MCP client on Windows.
        //
        // RFC 8259 §8.1 permits ignoring a leading BOM. preprocess_line()
        // strips it before parsing; this test locks that contract.
        let raw = "\u{FEFF}{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}".to_string();
        let cleaned = preprocess_line(raw);
        assert!(
            !cleaned.starts_with('\u{FEFF}'),
            "BOM not stripped: cleaned bytes start with {:02x?}",
            cleaned.as_bytes().iter().take(3).collect::<Vec<_>>()
        );
        let parsed: serde_json::Value =
            serde_json::from_str(&cleaned).expect("cleaned line must parse as JSON");
        assert_eq!(parsed["method"], "ping");

        // Also cover the inputs we will not change so this test catches
        // accidental over-stripping later:
        assert_eq!(preprocess_line("{\"a\":1}".to_string()), "{\"a\":1}");
        assert_eq!(preprocess_line("  {\"a\":1}  ".to_string()), "{\"a\":1}");
        assert_eq!(
            preprocess_line("\u{FEFF}\u{FEFF}{\"a\":1}".to_string()),
            "{\"a\":1}",
            "multiple leading BOMs should all be stripped"
        );
        assert_eq!(preprocess_line("".to_string()), "");
    }

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
                assert!(tool["name"].is_string(), "{}: missing name", name);
                assert!(
                    tool["description"].is_string(),
                    "{}: missing description",
                    name
                );
                assert!(
                    tool["inputSchema"].is_object(),
                    "{}: missing inputSchema",
                    name
                );
            }
        }
    }

    #[tokio::test]
    async fn handle_initialize() {
        let cfg = crate::config::ToolsConfig::default();
        let result = handle("initialize", &serde_json::json!({}), &cfg)
            .await
            .unwrap();
        assert_eq!(result["protocolVersion"], PROTOCOL_VERSION);
        assert!(result["serverInfo"]["name"].is_string());
    }

    #[tokio::test]
    async fn handle_tools_list() {
        let cfg = crate::config::ToolsConfig::default();
        let result = handle("tools/list", &serde_json::json!({}), &cfg)
            .await
            .unwrap();
        let tools = result["tools"].as_array().unwrap();
        assert!(!tools.is_empty(), "tools list should not be empty");
        // Every entry must have name + description + inputSchema
        for t in tools {
            assert!(t["name"].is_string());
            assert!(t["description"].is_string());
            assert!(t["inputSchema"].is_object());
        }
    }

    /// Regression guard for the journey "使用④ MCP 工具": the stdio server
    /// must expose a healthy tool surface, every advertised tool must be
    /// callable, and the macOS-specific tools must be present on macOS.
    /// Locks the contract that `validate-mac.sh` exercises (≈58 tools).
    #[tokio::test]
    async fn tools_list_exposes_expected_surface() {
        let cfg = crate::config::ToolsConfig::default();
        let result = handle("tools/list", &serde_json::json!({}), &cfg)
            .await
            .unwrap();
        let tools = result["tools"].as_array().unwrap();

        // (a) healthy count — registry + 2 appended distributed tools.
        assert!(
            tools.len() >= 48,
            "expected >= 48 MCP tools, got {}",
            tools.len()
        );

        let names: std::collections::HashSet<&str> =
            tools.iter().filter_map(|t| t["name"].as_str()).collect();

        // (b) key cross-platform tools present by name.
        for expected in [
            "file_read",
            "file_write",
            "shell",
            "dev_verify",
            // distributed tools appended in tools/list
            "phantom_swarm",
            "phantom_evolve_distributed",
        ] {
            assert!(
                names.contains(expected),
                "MCP tools/list missing expected tool: {} (have {} tools)",
                expected,
                tools.len()
            );
        }

        // (c) macOS-specific tools must be registered + advertised on macOS,
        //     and must NOT leak onto other platforms.
        #[cfg(target_os = "macos")]
        {
            for mac_tool in ["spotlight_search", "xcode_simctl"] {
                assert!(
                    names.contains(mac_tool),
                    "macOS MCP tools/list missing {}",
                    mac_tool
                );
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            for mac_tool in ["spotlight_search", "xcode_simctl"] {
                assert!(
                    !names.contains(mac_tool),
                    "{} should be cfg-gated off on non-macOS targets",
                    mac_tool
                );
            }
        }
    }

    #[tokio::test]
    async fn handle_unknown_method() {
        let cfg = crate::config::ToolsConfig::default();
        let err = handle("nonexistent/method", &serde_json::json!({}), &cfg)
            .await
            .unwrap_err();
        assert_eq!(err.0, -32601);
    }

    #[tokio::test]
    async fn handle_ping() {
        let cfg = crate::config::ToolsConfig::default();
        let result = handle("ping", &serde_json::json!({}), &cfg).await.unwrap();
        assert_eq!(result, serde_json::json!({}));
    }

    // ── HIGH-3 (audit V4): tools/call must report isError correctly ───────────

    #[test]
    fn is_error_output_recognises_phantom_conventions() {
        // Error-shaped strings: must be detected
        assert!(is_error_output("Error: missing 'path' argument"));
        assert!(is_error_output("Error reading /tmp/nope: not found"));
        assert!(is_error_output("Error writing foo: permission denied"));
        assert!(is_error_output("[error] something blew up"));
        assert!(is_error_output("[mcp:filesystem error] timeout"));
        assert!(is_error_output("ERROR: bad input"));
        assert!(is_error_output("   Error: leading whitespace ok"));

        // Success-shaped strings: must NOT be flagged
        assert!(!is_error_output(""));
        assert!(!is_error_output("OK"));
        assert!(!is_error_output("Written 42 bytes to /tmp/x"));
        assert!(!is_error_output("hello world"));
        assert!(!is_error_output(
            "error in lower case alone is not a prefix"
        ));
    }

    #[tokio::test]
    async fn tools_call_reports_iserror_true_on_failure() {
        // file_read with no `path` argument is the canonical "tool failed"
        // path — phantom returns "Error: missing 'path' argument". MCP spec
        // requires isError:true so the host doesn't silently feed the error
        // text back to the model as if it were real data.
        let cfg = crate::config::ToolsConfig::default();
        let result = handle(
            "tools/call",
            &serde_json::json!({
                "name": "file_read",
                "arguments": {}
            }),
            &cfg,
        )
        .await
        .unwrap();
        assert_eq!(
            result["isError"],
            serde_json::Value::Bool(true),
            "expected isError:true for failed tool call, got {}",
            result
        );
        let text = result["content"][0]["text"].as_str().unwrap_or("");
        assert!(
            text.starts_with("Error"),
            "expected Error-prefixed text, got: {}",
            text
        );
    }

    #[tokio::test]
    async fn tools_call_reports_iserror_false_on_success() {
        // todo_list with no todos is a successful call returning a non-error
        // string — must keep isError:false so the host treats it as real
        // output.
        let cfg = crate::config::ToolsConfig::default();
        let result = handle(
            "tools/call",
            &serde_json::json!({
                "name": "todo_list",
                "arguments": {}
            }),
            &cfg,
        )
        .await
        .unwrap();
        assert_eq!(
            result["isError"],
            serde_json::Value::Bool(false),
            "expected isError:false for successful call, got {}",
            result
        );
    }
}
