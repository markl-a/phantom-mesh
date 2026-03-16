//! MCP (Model Context Protocol) Client
//! Manages connections to external MCP servers via stdio JSON-RPC 2.0.

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, Mutex, RwLock};
use tracing::{debug, error, info, warn};

use crate::tools::{Tool, ToolResult};

/// MCP server configuration from agents.toml
#[derive(Debug, Clone, Deserialize)]
pub struct McpServerConfig {
    pub command: Vec<String>,
    #[serde(default)]
    pub auto_start: bool,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

fn default_timeout() -> u64 { 30 }

/// JSON-RPC 2.0 message types
#[derive(Debug, Serialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: u64,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcResponse {
    #[allow(dead_code)]
    jsonrpc: String,
    id: Option<u64>,
    result: Option<Value>,
    error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

/// An active MCP server connection
struct McpConnection {
    #[allow(dead_code)]
    child: Child,
    stdin_tx: mpsc::Sender<String>,
    pending: Arc<RwLock<HashMap<u64, tokio::sync::oneshot::Sender<Result<Value>>>>>,
    next_id: AtomicU64,
    tools: Vec<McpToolDef>,
}

/// Tool definition from an MCP server
#[derive(Debug, Clone, Deserialize)]
pub struct McpToolDef {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub input_schema: Value,
}

/// MCP Bridge — manages multiple MCP server connections
pub struct McpBridge {
    connections: RwLock<HashMap<String, Arc<Mutex<McpConnection>>>>,
    configs: HashMap<String, McpServerConfig>,
}

impl McpBridge {
    pub fn new(configs: HashMap<String, McpServerConfig>) -> Self {
        Self {
            connections: RwLock::new(HashMap::new()),
            configs,
        }
    }

    /// Start an MCP server and perform initialization handshake
    pub async fn start_server(&self, name: &str) -> Result<()> {
        let config = self.configs.get(name)
            .ok_or_else(|| anyhow!("Unknown MCP server: {}", name))?;

        if config.command.is_empty() {
            return Err(anyhow!("MCP server '{}' has empty command", name));
        }

        info!("Starting MCP server '{}': {:?}", name, config.command);

        let mut cmd = Command::new(&config.command[0]);
        if config.command.len() > 1 {
            cmd.args(&config.command[1..]);
        }
        for (k, v) in &config.env {
            cmd.env(k, v);
        }
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        let mut child = cmd.spawn()
            .map_err(|e| anyhow!("Failed to spawn MCP server '{}': {}", name, e))?;

        let stdin = child.stdin.take().ok_or_else(|| anyhow!("No stdin for MCP server"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("No stdout for MCP server"))?;

        let pending: Arc<RwLock<HashMap<u64, tokio::sync::oneshot::Sender<Result<Value>>>>> =
            Arc::new(RwLock::new(HashMap::new()));
        let pending_clone = pending.clone();

        // Spawn stdin writer
        let (stdin_tx, mut stdin_rx) = mpsc::channel::<String>(32);
        tokio::spawn(async move {
            let mut stdin = stdin;
            while let Some(msg) = stdin_rx.recv().await {
                if let Err(e) = stdin.write_all(msg.as_bytes()).await {
                    error!("MCP stdin write error: {}", e);
                    break;
                }
                if let Err(e) = stdin.write_all(b"\n").await {
                    error!("MCP stdin newline error: {}", e);
                    break;
                }
                let _ = stdin.flush().await;
            }
        });

        // Spawn stdout reader
        let server_name = name.to_string();
        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<JsonRpcResponse>(&line) {
                    Ok(resp) => {
                        if let Some(id) = resp.id {
                            let mut pending = pending_clone.write().await;
                            if let Some(tx) = pending.remove(&id) {
                                let result = if let Some(err) = resp.error {
                                    Err(anyhow!("MCP error {}: {}", err.code, err.message))
                                } else {
                                    Ok(resp.result.unwrap_or(Value::Null))
                                };
                                let _ = tx.send(result);
                            }
                        }
                    }
                    Err(e) => {
                        debug!("MCP '{}' non-JSON line: {} ({})", server_name, &line[..line.len().min(100)], e);
                    }
                }
            }
        });

        let conn = McpConnection {
            child,
            stdin_tx: stdin_tx.clone(),
            pending,
            next_id: AtomicU64::new(1),
            tools: Vec::new(),
        };

        let conn = Arc::new(Mutex::new(conn));

        // Initialize handshake
        let init_result = Self::send_request_inner(
            &conn,
            "initialize",
            Some(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "clawtex", "version": env!("CARGO_PKG_VERSION")},
            })),
        ).await?;

        debug!("MCP '{}' initialized: {:?}", name, init_result);

        // Send initialized notification
        let notify_msg = serde_json::to_string(&json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
        }))?;
        stdin_tx.send(notify_msg).await
            .map_err(|e| anyhow!("Failed to send notification: {}", e))?;

        // List tools
        let tools_result = Self::send_request_inner(&conn, "tools/list", None).await?;
        let tools: Vec<McpToolDef> = tools_result.get("tools")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        info!("MCP '{}' provides {} tools: {:?}", name, tools.len(),
            tools.iter().map(|t| &t.name).collect::<Vec<_>>());

        conn.lock().await.tools = tools;

        self.connections.write().await.insert(name.to_string(), conn);
        Ok(())
    }

    async fn send_request_inner(
        conn: &Arc<Mutex<McpConnection>>,
        method: &str,
        params: Option<Value>,
    ) -> Result<Value> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let id;

        {
            let conn = conn.lock().await;
            id = conn.next_id.fetch_add(1, Ordering::Relaxed);
            conn.pending.write().await.insert(id, tx);

            let request = JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id,
                method: method.to_string(),
                params,
            };
            let msg = serde_json::to_string(&request)?;
            conn.stdin_tx.send(msg).await
                .map_err(|e| anyhow!("Failed to send MCP request: {}", e))?;
        }

        match tokio::time::timeout(
            std::time::Duration::from_secs(30),
            rx,
        ).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(anyhow!("MCP response channel closed")),
            Err(_) => {
                // Clean up pending request
                let conn = conn.lock().await;
                conn.pending.write().await.remove(&id);
                Err(anyhow!("MCP request timed out"))
            }
        }
    }

    /// Call a tool on an MCP server
    pub async fn call_tool(&self, server: &str, tool_name: &str, arguments: Value) -> Result<String> {
        let connections = self.connections.read().await;
        let conn = connections.get(server)
            .ok_or_else(|| anyhow!("MCP server '{}' not connected", server))?;

        let result = Self::send_request_inner(
            conn,
            "tools/call",
            Some(json!({
                "name": tool_name,
                "arguments": arguments,
            })),
        ).await?;

        // Extract text content from result
        if let Some(content) = result.get("content").and_then(|v| v.as_array()) {
            let texts: Vec<&str> = content.iter()
                .filter_map(|c| {
                    if c.get("type").and_then(|v| v.as_str()) == Some("text") {
                        c.get("text").and_then(|v| v.as_str())
                    } else {
                        None
                    }
                })
                .collect();
            Ok(texts.join("\n"))
        } else {
            Ok(serde_json::to_string_pretty(&result)?)
        }
    }

    /// Get all tools from all connected MCP servers
    pub async fn all_tools(&self) -> Vec<(String, McpToolDef)> {
        let connections = self.connections.read().await;
        let mut all = Vec::new();
        for (server_name, conn) in connections.iter() {
            let conn = conn.lock().await;
            for tool in &conn.tools {
                all.push((server_name.clone(), tool.clone()));
            }
        }
        all
    }

    /// Start all auto_start servers
    pub async fn start_auto(&self) {
        for (name, config) in &self.configs {
            if config.auto_start {
                if let Err(e) = self.start_server(name).await {
                    warn!("Failed to start MCP server '{}': {}", name, e);
                }
            }
        }
    }

    /// Stop all MCP servers
    pub async fn stop_all(&self) {
        let mut connections = self.connections.write().await;
        for (name, conn) in connections.drain() {
            info!("Stopping MCP server '{}'", name);
            let mut conn = conn.lock().await;
            let _ = conn.child.kill().await;
        }
    }

    /// Get connected server names
    pub async fn connected_servers(&self) -> Vec<String> {
        self.connections.read().await.keys().cloned().collect()
    }
}

/// Proxy tool that wraps an MCP tool as a clawtex Tool
pub struct McpToolProxy {
    server_name: String,
    tool_def: McpToolDef,
    bridge: Arc<McpBridge>,
}

impl McpToolProxy {
    pub fn new(server_name: String, tool_def: McpToolDef, bridge: Arc<McpBridge>) -> Self {
        Self { server_name, tool_def, bridge }
    }
}

#[async_trait]
impl Tool for McpToolProxy {
    fn name(&self) -> &str {
        // Will be registered with prefixed name like "mcp:filesystem:read_file"
        &self.tool_def.name
    }

    fn description(&self) -> &str {
        &self.tool_def.description
    }

    fn parameters_schema(&self) -> Value {
        self.tool_def.input_schema.clone()
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        match self.bridge.call_tool(&self.server_name, &self.tool_def.name, args).await {
            Ok(output) => Ok(ToolResult { success: true, output }),
            Err(e) => Ok(ToolResult { success: false, output: format!("MCP tool error: {}", e) }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_bridge_empty() {
        let bridge = McpBridge::new(HashMap::new());
        assert!(bridge.configs.is_empty());
    }

    #[test]
    fn test_mcp_server_config_defaults() {
        let config: McpServerConfig = serde_json::from_value(json!({
            "command": ["echo", "test"]
        })).unwrap();
        assert!(!config.auto_start);
        assert_eq!(config.timeout_secs, 30);
    }

    #[test]
    fn test_mcp_tool_def_deserialize() {
        let def: McpToolDef = serde_json::from_value(json!({
            "name": "read_file",
            "description": "Read a file",
            "input_schema": {"type": "object", "properties": {"path": {"type": "string"}}}
        })).unwrap();
        assert_eq!(def.name, "read_file");
        assert!(!def.description.is_empty());
    }

    #[test]
    fn test_mcp_tool_proxy_name() {
        let bridge = Arc::new(McpBridge::new(HashMap::new()));
        let tool_def = McpToolDef {
            name: "read_file".into(),
            description: "Read a file".into(),
            input_schema: json!({}),
        };
        let proxy = McpToolProxy::new("filesystem".into(), tool_def, bridge);
        assert_eq!(proxy.name(), "read_file");
    }

    #[test]
    fn test_json_rpc_request_serialization() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: 1,
            method: "initialize".into(),
            params: Some(json!({"test": true})),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("jsonrpc"));
        assert!(json.contains("initialize"));
    }

    #[test]
    fn test_json_rpc_request_no_params() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: 1,
            method: "tools/list".into(),
            params: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("params"));
    }
}
