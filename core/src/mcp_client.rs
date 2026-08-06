//! `spectyn mcp` — CLIENT mode.
//!
//! Spawns external MCP servers as child processes, performs the JSON-RPC
//! handshake over stdio, and re-exposes their tools to spectyn's own agent
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
    name: String,
    config: McpServerConfig,
    inner: Mutex<ClientInner>,
    /// Cached tool list from the most recent `tools/list` call.
    tools: Mutex<Vec<Value>>,
}

struct ClientInner {
    child: Child,
    stdin: ChildStdin,
    pending: Arc<Mutex<HashMap<i64, tokio::sync::oneshot::Sender<Value>>>>,
    next_id: i64,
}

impl McpClient {
    /// Spawn the child process, perform the `initialize` handshake, and return
    /// a ready-to-use client.
    pub async fn spawn(cfg: McpServerConfig) -> anyhow::Result<Self> {
        let inner = Self::spawn_inner(&cfg).await?;
        let client = Self {
            name: cfg.name.clone(),
            config: cfg,
            inner: Mutex::new(inner),
            tools: Mutex::new(Vec::new()),
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
        let mut child = cmd
            .spawn()
            .map_err(|e| anyhow::anyhow!("failed to spawn MCP server '{}': {}", cfg.name, e))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("MCP server '{}' has no stdin", cfg.name))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("MCP server '{}' has no stdout", cfg.name))?;
        // stderr was piped above (so the child can't pollute *our* stderr)
        // — we MUST drain it. The OS pipe buffer is small (Linux ~64 KiB,
        // macOS ~16 KiB, Windows ~4 KiB) and once full the child blocks on
        // its next write(2), freezing every subsequent stdout response too.
        // (audit V4 HIGH-5)
        let stderr = child.stderr.take();

        let pending: Arc<Mutex<HashMap<i64, tokio::sync::oneshot::Sender<Value>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        // Background stderr drain task: read lines and forward at debug level
        // so MCP server diagnostics still surface (via RUST_LOG=spectyn=debug)
        // without ever blocking the child on a full pipe buffer.
        if let Some(stderr) = stderr {
            let server_name = cfg.name.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr).lines();
                loop {
                    match reader.next_line().await {
                        Ok(Some(line)) => {
                            // Skip empty noise but always *consume* the bytes.
                            if !line.trim().is_empty() {
                                tracing::debug!(target: "mcp::stderr",
                                                "mcp[{}] {}", server_name, line);
                            }
                        }
                        Ok(None) => {
                            tracing::debug!(target: "mcp::stderr",
                                            "mcp[{}] stderr closed", server_name);
                            break;
                        }
                        Err(e) => {
                            tracing::warn!(target: "mcp::stderr",
                                           "mcp[{}] stderr read error: {}", server_name, e);
                            break;
                        }
                    }
                }
            });
        }

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
                            if line.is_empty() {
                                continue;
                            }
                            let v: Value = match serde_json::from_str(line) {
                                Ok(v) => v,
                                Err(e) => {
                                    tracing::warn!(
                                        "mcp[{}] bad JSON from server: {} (line: {})",
                                        server_name,
                                        e,
                                        line
                                    );
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

        // Send `initialize` and wait for response.  We do this inline (rather
        // than via the McpClient::request_raw method) because the McpClient
        // wrapper hasn't been built yet — we still own ClientInner directly.
        let init_id = inner.next_id;
        inner.next_id += 1;
        let (init_tx, init_rx) = tokio::sync::oneshot::channel();
        inner.pending.lock().await.insert(init_id, init_tx);
        let init_req = json!({
            "jsonrpc": "2.0",
            "id":      init_id,
            "method":  "initialize",
            "params":  json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities":    {},
                "clientInfo":      { "name": "spectyn-mesh", "version": env!("CARGO_PKG_VERSION") },
            }),
        });
        let mut init_line = init_req.to_string();
        init_line.push('\n');
        inner
            .stdin
            .write_all(init_line.as_bytes())
            .await
            .map_err(|e| anyhow::anyhow!("MCP write failed: {}", e))?;
        inner.stdin.flush().await.ok();
        let init_resp =
            match tokio::time::timeout(std::time::Duration::from_secs(30), init_rx).await {
                Ok(Ok(v)) => v,
                Ok(Err(_)) => anyhow::bail!("MCP initialize: response channel closed"),
                Err(_) => {
                    inner.pending.lock().await.remove(&init_id);
                    anyhow::bail!("MCP server '{}' initialize timed out", cfg.name);
                }
            };
        if init_resp.get("error").is_some() {
            anyhow::bail!(
                "MCP server '{}' initialize failed: {}",
                cfg.name,
                init_resp["error"]
            );
        }
        // Send `notifications/initialized` (no response expected).
        let line = json!({
            "jsonrpc": "2.0",
            "method":  "notifications/initialized",
            "params":  {},
        })
        .to_string()
            + "\n";
        let _ = inner.stdin.write_all(line.as_bytes()).await;
        let _ = inner.stdin.flush().await;

        Ok(inner)
    }

    /// Send a JSON-RPC request and await its response. Times out after 30s.
    ///
    /// Concurrency contract (audit V4 HIGH-4): the `inner` mutex is held ONLY
    /// for the brief send phase (id allocation + pending insertion + stdin
    /// write + flush). The await on the response oneshot is performed AFTER
    /// the mutex is dropped, so a slow tool call no longer blocks every other
    /// in-flight request to the same server.
    ///
    /// `pending` is a separate `Arc<Mutex<...>>` shared with the background
    /// reader task — no other lock is held while waiting.
    async fn request_raw(&self, method: &str, params: Value) -> anyhow::Result<Value> {
        let (id, rx, pending) = {
            // ── send phase: short critical section ────────────────────────
            let mut inner = self.inner.lock().await;
            let id = inner.next_id;
            inner.next_id += 1;
            let (tx, rx) = tokio::sync::oneshot::channel();
            inner.pending.lock().await.insert(id, tx);

            let req = json!({
                "jsonrpc": "2.0",
                "id":      id,
                "method":  method,
                "params":  params,
            });
            let mut line = req.to_string();
            line.push('\n');
            inner
                .stdin
                .write_all(line.as_bytes())
                .await
                .map_err(|e| anyhow::anyhow!("MCP write failed: {}", e))?;
            inner.stdin.flush().await.ok();

            // Clone the Arc<Mutex<...>> so the timeout cleanup path doesn't
            // need to re-acquire the inner lock.
            (id, rx, inner.pending.clone())
            // ── inner mutex drops here ────────────────────────────────────
        };

        match tokio::time::timeout(std::time::Duration::from_secs(30), rx).await {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(_)) => Err(anyhow::anyhow!("MCP response channel closed")),
            Err(_) => {
                // Drop the pending entry so we don't leak.  This uses the
                // pending Arc directly — does NOT re-acquire `inner`.
                pending.lock().await.remove(&id);
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
        let resp = self.request_raw("tools/list", json!({})).await?;
        if let Some(err) = resp.get("error") {
            anyhow::bail!("tools/list error from {}: {}", self.name, err);
        }
        let tools = resp["result"]["tools"]
            .as_array()
            .cloned()
            .unwrap_or_default();
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
        let resp = self
            .request_raw(
                "tools/call",
                json!({
                    "name":      tool_name,
                    "arguments": args,
                }),
            )
            .await?;
        if let Some(err) = resp.get("error") {
            anyhow::bail!("tools/call error from {}: {}", self.name, err);
        }
        // Concatenate every text content block.
        let mut out = String::new();
        if let Some(content) = resp["result"]["content"].as_array() {
            for part in content {
                if let Some(t) = part["text"].as_str() {
                    if !out.is_empty() {
                        out.push('\n');
                    }
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
                Ok(Some(_status)) => true, // exited
                Ok(None) => false,         // still running
                Err(_) => true,            // weird — try restart
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
    /// single broken server doesn't block spectyn from starting.
    pub async fn build(servers: &[McpServerConfig]) -> Self {
        let mut clients = HashMap::new();
        for cfg in servers {
            if cfg.name.is_empty() || cfg.command.is_empty() {
                tracing::warn!("skipping mcp server with empty name/command");
                continue;
            }
            match McpClient::spawn(cfg.clone()).await {
                Ok(c) => {
                    tracing::info!(
                        "mcp client '{}' started ({} tools)",
                        cfg.name,
                        c.cached_tools().await.len()
                    );
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
                if original.is_empty() {
                    continue;
                }
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
        self.tool_defs()
            .await
            .iter()
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
                    Ok(s) => s,
                    Err(e) => format!("[mcp:{} error] {}", server, e),
                });
            }
        }
        None
    }

    /// Re-fetch a server's `tools/list` (used by the `/mcp test <server>`
    /// debug command).
    pub async fn ping_server(&self, server: &str) -> anyhow::Result<Vec<Value>> {
        let c = self
            .clients
            .get(server)
            .ok_or_else(|| anyhow::anyhow!("unknown mcp server '{}'", server))?
            .clone();
        c.list_tools().await
    }
}

/// Initialise the global registry. Subsequent calls are ignored.
pub async fn init_global(servers: &[McpServerConfig]) {
    if servers.is_empty() {
        return;
    }
    if REGISTRY.get().is_some() {
        return;
    }
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
    /// We use a system Python for portability; if it's missing the test
    /// is skipped (CI containers without python should still pass `cargo build`).
    ///
    /// Tries `python3` first, then falls back to `python` (Windows ships
    /// `python.exe` rather than `python3.exe`). Returns the working command
    /// name, or None if neither works.
    fn python_command() -> Option<&'static str> {
        for cmd in &["python3", "python"] {
            let ok = std::process::Command::new(cmd)
                .arg("--version")
                .output()
                .map(|o| {
                    o.status.success() && !String::from_utf8_lossy(&o.stdout).trim().is_empty()
                })
                .unwrap_or(false);
            if ok {
                return Some(*cmd);
            }
        }
        None
    }

    fn python_available() -> bool {
        python_command().is_some()
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
            name: "selftest".into(),
            command: python_command().unwrap_or("python3").into(),
            args: vec!["-c".into(), script.into()],
            env: HashMap::new(),
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
        let out = client
            .call_tool("echo", &json!({"msg": "hi"}))
            .await
            .expect("call");
        assert_eq!(out, "echo:hi");
    }

    #[tokio::test]
    async fn dogfood_spectyn_mcp_as_child() {
        // Eat our own dog food: spawn `spectyn mcp` as the MCP server and call
        // its `shell` tool. Skipped when the release binary hasn't been built.
        let bin = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/release/spectyn");
        if !bin.exists() {
            eprintln!(
                "release binary missing — skipping dogfood test ({})",
                bin.display()
            );
            return;
        }
        let cfg = McpServerConfig {
            name: "selftest".into(),
            command: bin.to_string_lossy().to_string(),
            args: vec!["mcp".into()],
            env: HashMap::new(),
        };
        let client = McpClient::spawn(cfg).await.expect("spawn spectyn mcp");
        let tools = client.list_tools().await.expect("list");
        assert!(
            tools.len() >= 40,
            "expected >=40 tools, got {}",
            tools.len()
        );
        let out = client
            .call_tool(
                "shell",
                &json!({
                    "command": "echo OK from spectyn mcp"
                }),
            )
            .await
            .expect("shell call");
        assert!(
            out.contains("OK from spectyn mcp"),
            "unexpected output: {}",
            out
        );
    }

    #[tokio::test]
    async fn registry_dispatch_routes_by_prefix() {
        if !python_available() {
            eprintln!("python3 not available — skipping mcp_client integration test");
            return;
        }
        let reg = McpRegistry::build(&[fake_server_cfg()]).await;
        let names = reg.tool_names().await;
        assert!(
            names.iter().any(|n| n == "selftest_echo"),
            "expected selftest_echo in {:?}",
            names
        );

        // Dispatch via prefix.
        let out = reg.dispatch("selftest_echo", &json!({"msg":"world"})).await;
        assert_eq!(out.as_deref(), Some("echo:world"));

        // Non-MCP tool name returns None (so callers can fall through).
        assert!(reg.dispatch("file_read", &json!({})).await.is_none());
    }

    // ── HIGH-4 (audit V4): mutex must NOT be held across full RPC timeout ────

    /// A fake MCP server with a per-call configurable delay. The `delay_ms`
    /// argument tells the server how long to sleep BEFORE writing the
    /// response, simulating a slow tool. Used to prove that a slow call
    /// doesn't block other in-flight calls on the same client.
    fn delay_server_cfg() -> McpServerConfig {
        let script = r#"
import sys, json, time, threading
def respond(msg):
    rid = msg['id']
    method = msg.get('method', '')
    if method == 'initialize':
        out = {'jsonrpc':'2.0','id':rid,'result':{
            'protocolVersion':'2024-11-05',
            'capabilities':{'tools':{}},
            'serverInfo':{'name':'delay','version':'0'}}}
    elif method == 'tools/list':
        out = {'jsonrpc':'2.0','id':rid,'result':{'tools':[
            {'name':'sleep','description':'sleeps then echoes',
             'inputSchema':{'type':'object'}}
        ]}}
    elif method == 'tools/call':
        args = msg.get('params',{}).get('arguments',{})
        delay_ms = int(args.get('delay_ms', 0))
        tag      = args.get('tag', '')
        if delay_ms > 0:
            time.sleep(delay_ms / 1000.0)
        out = {'jsonrpc':'2.0','id':rid,'result':{
            'content':[{'type':'text','text':'done:'+tag}],
            'isError':False}}
    else:
        out = {'jsonrpc':'2.0','id':rid,'error':{'code':-32601,'message':'not found'}}
    sys.stdout.write(json.dumps(out)+'\n')
    sys.stdout.flush()
for line in sys.stdin:
    line = line.strip()
    if not line: continue
    try:
        msg = json.loads(line)
    except Exception:
        continue
    if 'id' not in msg:
        continue
    # Spawn a thread per request so a slow request doesn't block reading
    # subsequent ones. This matches what a real MCP server would do.
    threading.Thread(target=respond, args=(msg,), daemon=True).start()
"#;
        McpServerConfig {
            name: "delaytest".into(),
            command: python_command().unwrap_or("python3").into(),
            args: vec!["-c".into(), script.into()],
            env: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn concurrent_calls_do_not_serialise_on_inner_mutex() {
        // Before the HIGH-4 fix the McpClient held its inner mutex across the
        // full 30s timeout, so two concurrent calls to the same client would
        // execute strictly serially. With the fix the lock is dropped after
        // sending; a fast call must overtake an in-flight slow call.
        if !python_available() {
            eprintln!("python3 not available — skipping mcp_client concurrency test");
            return;
        }
        let client =
            std::sync::Arc::new(McpClient::spawn(delay_server_cfg()).await.expect("spawn"));

        let c1 = client.clone();
        let c2 = client.clone();
        let slow = tokio::spawn(async move {
            let t0 = std::time::Instant::now();
            let r = c1
                .call_tool("sleep", &json!({"delay_ms": 800, "tag": "slow"}))
                .await;
            (r, t0.elapsed())
        });
        // Give the slow request time to grab the lock + start its sleep on
        // the server side. 100 ms is enough; the slow call sleeps for 800 ms.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let t0 = std::time::Instant::now();
        let fast_out = c2
            .call_tool("sleep", &json!({"delay_ms": 0, "tag": "fast"}))
            .await
            .expect("fast call");
        let fast_elapsed = t0.elapsed();

        // Sanity: outputs are correct.
        assert_eq!(fast_out, "done:fast");
        let (slow_res, slow_elapsed) = slow.await.expect("slow join");
        assert_eq!(slow_res.expect("slow call"), "done:slow");

        // The fast call must complete well before the slow one. Without the
        // mutex split, fast_elapsed would be ~700 ms (slow 800 ms - 100 ms
        // initial sleep). With the split, fast_elapsed should be <300 ms.
        assert!(
            fast_elapsed < std::time::Duration::from_millis(500),
            "fast call took {:?} — head-of-line blocking by slow call ({:?}) detected; \
             mutex split (HIGH-4) regressed",
            fast_elapsed,
            slow_elapsed
        );
    }

    // ── HIGH-5 (audit V4): child stderr must be drained or chatty servers
    //                       deadlock at ~64 KiB ────────────────────────────────

    /// A fake MCP server that emits a sizable chunk of stderr on every
    /// request. Without a stderr drain task, the OS pipe buffer fills (~64
    /// KiB on Linux, smaller on Windows) and the server blocks on its next
    /// `write(2)`, freezing all subsequent RPC.
    fn stderr_spam_server_cfg() -> McpServerConfig {
        let script = r#"
import sys, json
# 4 KiB of stderr per request. After 16+ calls this would exceed the typical
# 64 KiB pipe buffer.
SPAM = 'x' * 4096
def line():
    return sys.stdin.readline()
while True:
    raw = line()
    if not raw: break
    raw = raw.strip()
    if not raw: continue
    try:
        msg = json.loads(raw)
    except Exception:
        continue
    if 'id' not in msg:
        continue
    sys.stderr.write(SPAM + '\n')
    sys.stderr.flush()
    method = msg.get('method', '')
    rid = msg['id']
    if method == 'initialize':
        out = {'jsonrpc':'2.0','id':rid,'result':{
            'protocolVersion':'2024-11-05',
            'capabilities':{'tools':{}},
            'serverInfo':{'name':'spam','version':'0'}}}
    elif method == 'tools/list':
        out = {'jsonrpc':'2.0','id':rid,'result':{'tools':[
            {'name':'noop','description':'noop','inputSchema':{'type':'object'}}
        ]}}
    elif method == 'tools/call':
        out = {'jsonrpc':'2.0','id':rid,'result':{
            'content':[{'type':'text','text':'ok'}],
            'isError':False}}
    else:
        out = {'jsonrpc':'2.0','id':rid,'error':{'code':-32601,'message':'?'}}
    sys.stdout.write(json.dumps(out)+'\n')
    sys.stdout.flush()
"#;
        McpServerConfig {
            name: "spamtest".into(),
            command: python_command().unwrap_or("python3").into(),
            args: vec!["-c".into(), script.into()],
            env: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn chatty_stderr_does_not_deadlock_rpc() {
        if !python_available() {
            eprintln!("python3 not available — skipping mcp_client stderr drain test");
            return;
        }
        let client = McpClient::spawn(stderr_spam_server_cfg())
            .await
            .expect("spawn");

        // Make enough calls that the *un-drained* pipe buffer would fill on
        // every common platform (Linux 64 KiB, macOS 16 KiB, Windows 4 KiB).
        // 32 calls × 4 KiB = 128 KiB of stderr. Each call is also wrapped in
        // its own short timeout so a regression manifests as a per-call
        // failure rather than a 30 s test hang.
        for i in 0..32 {
            let args = json!({"i": i});
            let fut = client.call_tool("noop", &args);
            let out = tokio::time::timeout(std::time::Duration::from_secs(3), fut)
                .await
                .unwrap_or_else(|_| {
                    panic!(
                        "RPC #{} hung — child likely blocked on full stderr pipe; \
                     drain task (HIGH-5) regressed",
                        i
                    )
                })
                .unwrap_or_else(|e| panic!("RPC #{} failed: {}", i, e));
            assert_eq!(out, "ok", "RPC #{} returned unexpected output", i);
        }
    }
}
