//! `phantom mcp` — CLIENT mode.
//!
//! Spawns external MCP servers as child processes, performs the JSON-RPC
//! handshake over stdio, and re-exposes their tools to phantom's own agent
//! runtime so the LLM can call them just like built-in tools.
//!
//! Naming: external tools are surfaced with a `<server>_<tool>` prefix so they
//! don't collide with built-ins. e.g. an MCP server named `filesystem` that
//! offers a `read_file` tool becomes `filesystem_read_file` to the LLM.
//!
//! Lifecycle: clients are owned by a process-wide `OnceCell` registry. On
//! `init_global()` each `[[mcp_servers]]` entry is spawned and asked for its
//! `tools/list`. Subsequent `tools/call` requests are routed to the matching
//! client. On drop the child process is killed.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::OnceLock;

use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::Mutex;

const PROTOCOL_VERSION: &str = "2024-11-05";

// ── Config struct (referenced from config::AgentsConfig.mcp_servers) ──────────

/// One `[[mcp_servers]]` block in agents.toml.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

// ── Per-server client ─────────────────────────────────────────────────────────

/// A running MCP child process plus the request/response plumbing.
///
/// `next_id` increments per JSON-RPC call. Pending responses are read line by
/// line from a single background reader task into a shared map keyed by id.
pub struct McpClient {
    name:      String,
    config:    McpServerConfig,
    inner:     Mutex<ClientInner>,
    /// Cached tool list from the most recent `tools/list` call.
    tools:     Mutex<Vec<Value>>,
}

struct ClientInner {
    child:    Child,
    stdin:    ChildStdin,
    pending:  Arc<Mutex<HashMap<i64, tokio::sync::oneshot::Sender<Value>>>>,
    next_id:  i64,
}

impl McpClient {
    /// Spawn the child process, perform the `initialize` handshake, and return
    /// a ready-to-use client.
    pub async fn spawn(cfg: McpServerConfig) -> anyhow::Result<Self> {
        let inner = Self::spawn_inner(&cfg).await?;
        let client = Self {
            name:   cfg.name.clone(),
            config: cfg,
            inner:  Mutex::new(inner),
            tools:  Mutex::new(Vec::new()),
        };
        // Best-effort: do an initial `tools/list` so the registry has tool
        // schemas to advertise to the LLM. Errors are non-fatal — caller may
        // retry via `list_tools()` directly.
        let _ = client.list_tools_inner().await;
        Ok(client)
    }

    async fn spawn_inner(cfg: &McpServerConfig) -> anyhow::Result<ClientInner> {
        let mut cmd = Command::new(&cfg.command);
        cmd.args(&cfg.args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        for (k, v) in &cfg.env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn()
            .map_err(|e| anyhow::anyhow!("failed to spawn MCP server '{}': {}", cfg.name, e))?;
        let stdin = child.stdin.take()
            .ok_or_else(|| anyhow::anyhow!("MCP server '{}' has no stdin", cfg.name))?;
        let stdout = child.stdout.take()
            .ok_or_else(|| anyhow::anyhow!("MCP server '{}' has no stdout", cfg.name))?;

        let pending: Arc<Mutex<HashMap<i64, tokio::sync::oneshot::Sender<Value>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        // Background reader task: parse one JSON-RPC response per line.
        {
            let pending = pending.clone();
            let server_name = cfg.name.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(stdout).lines();
                loop {
                    match reader.next_line().await {
                        Ok(Some(line)) => {
                            let line = line.trim();
                            if line.is_empty() { continue; }
                            let v: Value = match serde_json::from_str(line) {
                                Ok(v) => v,
                                Err(e) => {
                                    tracing::warn!("mcp[{}] bad JSON from server: {} (line: {})",
                                                   server_name, e, line);
                                    continue;
                                }
                            };
                            let id = match v.get("id").and_then(|i| i.as_i64()) {
                                Some(id) => id,
                                None => continue, // notification — ignore
                            };
                            let tx_opt = { pending.lock().await.remove(&id) };
                            if let Some(tx) = tx_opt {
                                let _ = tx.send(v);
                            }
                        }
                        Ok(None) => {
                            tracing::warn!("mcp[{}] stdout closed", server_name);
                            break;
                        }
                        Err(e) => {
                            tracing::warn!("mcp[{}] stdout read error: {}", server_name, e);
                            break;
                        }
                    }
                }
            });
        }

        let mut inner = ClientInner {
            child,
            stdin,
            pending,
            next_id: 1,
        };

        // Send `initialize` and wait for response.
        let init_resp = Self::request_raw(&mut inner, "initialize", json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities":    {},
            "clientInfo":      { "name": "phantom-mesh", "version": env!("CARGO_PKG_VERSION") },
        })).await?;
        if init_resp.get("error").is_some() {
            anyhow::bail!("MCP server '{}' initialize failed: {}",
                          cfg.name, init_resp["error"]);
        }
        // Send `notifications/initialized` (no response expected).
        let line = json!({
            "jsonrpc": "2.0",
            "method":  "notifications/initialized",
            "params":  {},
        }).to_string() + "\n";
        let _ = inner.stdin.write_all(line.as_bytes()).await;
        let _ = inner.stdin.flush().await;

        Ok(inner)
    }

    /// Send a JSON-RPC request and await its response. Times out after 30s.
    async fn request_raw(inner: &mut ClientInner, method: &str, params: Value)
        -> anyhow::Result<Value>
    {
        let id = inner.next_id;
        inner.next_id += 1;
        let (tx, rx) = tokio::sync::oneshot::channel();
        { inner.pending.lock().await.insert(id, tx); }

        let req = json!({
            "jsonrpc": "2.0",
            "id":      id,
            "method":  method,
            "params":  params,
        });
        let mut line = req.to_string();
        line.push('\n');
        inner.stdin.write_all(line.as_bytes()).await
            .map_err(|e| anyhow::anyhow!("MCP write failed: {}", e))?;
        inner.stdin.flush().await.ok();

        match tokio::time::timeout(std::time::Duration::from_secs(30), rx).await {
            Ok(Ok(v))  => Ok(v),
            Ok(Err(_)) => Err(anyhow::anyhow!("MCP response channel closed")),
            Err(_)     => {
                // Drop the pending entry so we don't leak.
                inner.pending.lock().await.remove(&id);
                Err(anyhow::anyhow!("MCP request '{}' timed out", method))
            }
        }
    }

    /// Public list_tools — fetches via JSON-RPC and refreshes cache.
    pub async fn list_tools(&self) -> anyhow::Result<Vec<Value>> {
        self.list_tools_inner().await
    }

    async fn list_tools_inner(&self) -> anyhow::Result<Vec<Value>> {
        // If the child died, attempt one re-spawn before giving up.
        self.ensure_alive().await?;
        let resp = {
            let mut inner = self.inner.lock().await;
            Self::request_raw(&mut inner, "tools/list", json!({})).await?
        };
        if let Some(err) = resp.get("error") {
            anyhow::bail!("tools/list error from {}: {}", self.name, err);
        }
        let tools = resp["result"]["tools"].as_array().cloned().unwrap_or_default();
        *self.tools.lock().await = tools.clone();
        Ok(tools)
    }

    /// Cached view of tools without making a JSON-RPC call.
    pub async fn cached_tools(&self) -> Vec<Value> {
        self.tools.lock().await.clone()
    }

    /// Call a tool on this server. `tool_name` is the *unprefixed* tool name as
    /// reported by `tools/list` (without the `<server>_` namespace).
    pub async fn call_tool(&self, tool_name: &str, args: &Value) -> anyhow::Result<String> {
        self.ensure_alive().await?;
        let resp = {
            let mut inner = self.inner.lock().await;
            Self::request_raw(&mut inner, "tools/call", json!({
                "name":      tool_name,
                "arguments": args,
            })).await?
        };
        if let Some(err) = resp.get("error") {
            anyhow::bail!("tools/call error from {}: {}", self.name, err);
        }
        // Concatenate every text content block.
        let mut out = String::new();
        if let Some(content) = resp["result"]["content"].as_array() {
            for part in content {
                if let Some(t) = part["text"].as_str() {
                    if !out.is_empty() { out.push('\n'); }
                    out.push_str(t);
                }
            }
        }
        if out.is_empty() {
            // Return raw result as last resort so the model gets *something*.
            out = resp["result"].to_string();
        }
        Ok(out)
    }

    /// Re-spawn the child if it has exited. Best-effort keepalive.
    async fn ensure_alive(&self) -> anyhow::Result<()> {
        let needs_respawn = {
            let mut inner = self.inner.lock().await;
            match inner.child.try_wait() {
                Ok(Some(_status)) => true,        // exited
                Ok(None)          => false,       // still running
                Err(_)            => true,        // weird — try restart
            }
        };
        if needs_respawn {
            tracing::warn!("mcp[{}] child exited, re-spawning", self.name);
            let new_inner = Self::spawn_inner(&self.config).await?;
            *self.inner.lock().await = new_inner;
        }
        Ok(())
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        // tokio Child has kill_on_drop set; nothing else to do.
    }
}

// ── Process-wide registry ─────────────────────────────────────────────────────

static REGISTRY: OnceLock<McpRegistry> = OnceLock::new();

pub struct McpRegistry {
    /// server-name → client
    clients: HashMap<String, Arc<McpClient>>,
}

impl McpRegistry {
    /// Spawn one client per config entry. Failures are logged and skipped so a
    /// single broken server doesn't block phantom from starting.
    pub async fn build(servers: &[McpServerConfig]) -> Self {
        let mut clients = HashMap::new();
        for cfg in servers {
            if cfg.name.is_empty() || cfg.command.is_empty() {
                tracing::warn!("skipping mcp server with empty name/command");
                continue;
            }
            match McpClient::spawn(cfg.clone()).await {
                Ok(c) => {
                    tracing::info!("mcp client '{}' started ({} tools)",
                                   cfg.name, c.cached_tools().await.len());
                    clients.insert(cfg.name.clone(), Arc::new(c));
                }
                Err(e) => tracing::warn!("mcp client '{}' failed to start: {}", cfg.name, e),
            }
        }
        Self { clients }
    }

    /// Server names + cached tool counts.
    pub async fn summary(&self) -> Vec<(String, usize)> {
        let mut out = Vec::new();
        for (name, c) in &self.clients {
            out.push((name.clone(), c.cached_tools().await.len()));
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// Tool definitions (OpenAI function-calling envelope) for every external
    /// tool, with `<server>_` prefixed names. Suitable for splicing into the
    /// `tools=[...]` payload sent to the LLM.
    pub async fn tool_defs(&self) -> Vec<Value> {
        let mut defs = Vec::new();
        for (name, c) in &self.clients {
            for t in c.cached_tools().await {
                let original = t["name"].as_str().unwrap_or("").to_string();
                if original.is_empty() { continue; }
                let prefixed = format!("{}_{}", name, original);
                defs.push(json!({
                    "type": "function",
                    "function": {
                        "name":        prefixed,
                        "description": t.get("description").cloned().unwrap_or(Value::String("".into())),
                        "parameters":  t.get("inputSchema").cloned().unwrap_or(json!({"type":"object","properties":{}})),
                    }
                }));
            }
        }
        defs
    }

    /// Prefixed tool names — used by `/tools` listings and the agent system.
    pub async fn tool_names(&self) -> Vec<String> {
        self.tool_defs().await.iter()
            .filter_map(|d| d["function"]["name"].as_str().map(|s| s.to_string()))
            .collect()
    }

    /// If `name` looks like `<server>_<tool>` and `<server>` is registered,
    /// dispatch to it. Returns `Some(output)` on hit, `None` otherwise (so the
    /// caller can fall through to built-in tools).
    pub async fn dispatch(&self, name: &str, args: &Value) -> Option<String> {
        // Match the longest server-name prefix to support tool names that
        // themselves contain underscores.
        let mut keys: Vec<&String> = self.clients.keys().collect();
        keys.sort_by_key(|k| std::cmp::Reverse(k.len()));
        for server in keys {
            let prefix = format!("{}_", server);
            if let Some(rest) = name.strip_prefix(&prefix) {
                let client = self.clients.get(server)?.clone();
                return Some(match client.call_tool(rest, args).await {
                    Ok(s)  => s,
                    Err(e) => format!("[mcp:{} error] {}", server, e),
                });
            }
        }
        None
    }

    /// Re-fetch a server's `tools/list` (used by the `/mcp test <server>`
    /// debug command).
    pub async fn ping_server(&self, server: &str) -> anyhow::Result<Vec<Value>> {
        let c = self.clients.get(server)
            .ok_or_else(|| anyhow::anyhow!("unknown mcp server '{}'", server))?
            .clone();
        c.list_tools().await
    }
}

/// Initialise the global registry. Subsequent calls are ignored.
pub async fn init_global(servers: &[McpServerConfig]) {
    if servers.is_empty() { return; }
    if REGISTRY.get().is_some() { return; }
    let reg = McpRegistry::build(servers).await;
    let _ = REGISTRY.set(reg);
}

/// Borrow the global registry if it has been initialised.
pub fn global() -> Option<&'static McpRegistry> {
    REGISTRY.get()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal MCP server implemented as a one-liner shell pipeline:
    /// reads JSON-RPC requests on stdin and writes canned responses to stdout.
    /// We use the system `python3` for portability; if it's missing the test
    /// is skipped (CI containers without python should still pass `cargo build`).
    fn python_available() -> bool {
        std::process::Command::new("python3").arg("--version").output()
            .map(|o| o.status.success()).unwrap_or(false)
    }

    fn fake_server_cfg() -> McpServerConfig {
        let script = r#"
import sys, json
for line in sys.stdin:
    line = line.strip()
    if not line: continue
    try:
        msg = json.loads(line)
    except Exception:
        continue
    if 'id' not in msg:
        continue                              # notification
    method = msg.get('method', '')
    rid    = msg['id']
    if method == 'initialize':
        out = {'jsonrpc':'2.0','id':rid,'result':{
            'protocolVersion':'2024-11-05',
            'capabilities':{'tools':{}},
            'serverInfo':{'name':'fake','version':'0'}}}
    elif method == 'tools/list':
        out = {'jsonrpc':'2.0','id':rid,'result':{'tools':[
            {'name':'echo','description':'echoes its argument',
             'inputSchema':{'type':'object','properties':{'msg':{'type':'string'}}}}
        ]}}
    elif method == 'tools/call':
        msg_arg = msg.get('params',{}).get('arguments',{}).get('msg','')
        out = {'jsonrpc':'2.0','id':rid,'result':{
            'content':[{'type':'text','text':'echo:'+msg_arg}],
            'isError':False}}
    else:
        out = {'jsonrpc':'2.0','id':rid,'error':{'code':-32601,'message':'not found'}}
    sys.stdout.write(json.dumps(out)+'\n')
    sys.stdout.flush()
"#;
        McpServerConfig {
            name:    "selftest".into(),
            command: "python3".into(),
            args:    vec!["-c".into(), script.into()],
            env:     HashMap::new(),
        }
    }

    #[tokio::test]
    async fn list_and_call_against_fake_server() {
        if !python_available() {
            eprintln!("python3 not available — skipping mcp_client integration test");
            return;
        }
        let client = McpClient::spawn(fake_server_cfg()).await.expect("spawn");

        // tools/list
        let tools = client.list_tools().await.expect("list");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"].as_str(), Some("echo"));

        // tools/call
        let out = client.call_tool("echo", &json!({"msg": "hi"})).await.expect("call");
        assert_eq!(out, "echo:hi");
    }

    #[tokio::test]
    async fn dogfood_phantom_mcp_as_child() {
        // Eat our own dog food: spawn `phantom mcp` as the MCP server and call
        // its `shell` tool. Skipped when the release binary hasn't been built.
        let bin = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/release/phantom");
        if !bin.exists() {
            eprintln!("release binary missing — skipping dogfood test ({})", bin.display());
            return;
        }
        let cfg = McpServerConfig {
            name:    "selftest".into(),
            command: bin.to_string_lossy().to_string(),
            args:    vec!["mcp".into()],
            env:     HashMap::new(),
        };
        let client = McpClient::spawn(cfg).await.expect("spawn phantom mcp");
        let tools = client.list_tools().await.expect("list");
        assert!(tools.len() >= 40, "expected >=40 tools, got {}", tools.len());
        let out = client.call_tool("shell", &json!({
            "command": "echo OK from phantom mcp"
        })).await.expect("shell call");
        assert!(out.contains("OK from phantom mcp"), "unexpected output: {}", out);
    }

    #[tokio::test]
    async fn registry_dispatch_routes_by_prefix() {
        if !python_available() {
            eprintln!("python3 not available — skipping mcp_client integration test");
            return;
        }
        let reg = McpRegistry::build(&[fake_server_cfg()]).await;
        let names = reg.tool_names().await;
        assert!(names.iter().any(|n| n == "selftest_echo"),
                "expected selftest_echo in {:?}", names);

        // Dispatch via prefix.
        let out = reg.dispatch("selftest_echo", &json!({"msg":"world"})).await;
        assert_eq!(out.as_deref(), Some("echo:world"));

        // Non-MCP tool name returns None (so callers can fall through).
        assert!(reg.dispatch("file_read", &json!({})).await.is_none());
    }
}
