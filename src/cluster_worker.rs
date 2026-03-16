//! ClusterWorker — runs on worker nodes, registers with hub, executes tools.
//! Start with: `clawtex-core worker --hub http://100.x.x.x:7878 --name m1`

use anyhow::Result;
use axum::{extract::State, http::StatusCode, response::Json, routing::{get, post}, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::{error, info, warn};

use crate::tools::ToolRegistry;

/// Worker configuration
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    pub hub_url: String,
    pub node_name: String,
    pub capabilities: Vec<String>,
    pub device_type: String,
    pub port: u16,
}

/// Worker state shared across HTTP handlers
#[derive(Clone)]
pub struct WorkerState {
    pub config: WorkerConfig,
    pub tool_registry: Arc<ToolRegistry>,
    http_client: reqwest::Client,
}

/// Tool execution request from hub
#[derive(Debug, Deserialize)]
struct ExecuteRequest {
    tool: String,
    input: Value,
}

/// Tool execution response to hub
#[derive(Debug, Serialize)]
struct ExecuteResponse {
    success: bool,
    output: String,
}

/// ClusterWorker — registers with hub, runs local HTTP server, executes tools
pub struct ClusterWorker {
    state: WorkerState,
}

impl ClusterWorker {
    pub fn new(config: WorkerConfig, tool_registry: Arc<ToolRegistry>) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_default();

        Self {
            state: WorkerState {
                config,
                tool_registry,
                http_client,
            },
        }
    }

    /// Register this worker with the hub
    pub async fn register(&self) -> Result<()> {
        let url = format!("{}/cluster/register", self.state.config.hub_url);
        let payload = json!({
            "name": self.state.config.node_name,
            "host": detect_local_ip(&self.state.config.hub_url),
            "port": self.state.config.port,
            "capabilities": self.state.config.capabilities,
            "device_type": self.state.config.device_type,
        });

        info!("Registering with hub at {}", url);
        let resp = self.state.http_client
            .post(&url)
            .json(&payload)
            .send()
            .await?;

        if resp.status().is_success() {
            info!("Registered with hub as '{}'", self.state.config.node_name);
            Ok(())
        } else {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Registration failed ({}): {}", status, body);
        }
    }

    /// Start the background heartbeat loop with exponential backoff reconnect.
    /// Normal interval: 15s. On failure: backs off up to 120s, re-registers on recovery.
    pub fn start_heartbeat(state: WorkerState) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let base_interval = std::time::Duration::from_secs(15);
            let max_backoff = std::time::Duration::from_secs(120);
            let mut current_interval = base_interval;
            let mut consecutive_failures: u32 = 0;
            let mut was_disconnected = false;

            loop {
                tokio::time::sleep(current_interval).await;
                let cpu_load = get_cpu_load();
                let url = format!("{}/cluster/heartbeat", state.config.hub_url);
                let payload = json!({
                    "name": state.config.node_name,
                    "cpu_load": cpu_load,
                });

                match state.http_client.post(&url).json(&payload).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        if was_disconnected {
                            info!("Reconnected to hub after {} failures — re-registering", consecutive_failures);
                            // Re-register with hub after reconnection
                            let reg_url = format!("{}/cluster/register", state.config.hub_url);
                            let reg_payload = json!({
                                "name": state.config.node_name,
                                "host": detect_local_ip(&state.config.hub_url),
                                "port": state.config.port,
                                "capabilities": state.config.capabilities,
                                "device_type": state.config.device_type,
                            });
                            match state.http_client.post(&reg_url).json(&reg_payload).send().await {
                                Ok(r) if r.status().is_success() => {
                                    info!("Re-registered with hub successfully");
                                }
                                Ok(r) => warn!("Re-registration failed: {}", r.status()),
                                Err(e) => warn!("Re-registration error: {}", e),
                            }
                        }
                        consecutive_failures = 0;
                        was_disconnected = false;
                        current_interval = base_interval;
                    }
                    Ok(resp) => {
                        consecutive_failures += 1;
                        was_disconnected = true;
                        current_interval = std::cmp::min(
                            base_interval * 2u32.saturating_pow(consecutive_failures.min(4)),
                            max_backoff,
                        );
                        warn!("Heartbeat failed ({}x): {} — next retry in {:?}",
                            consecutive_failures, resp.status(), current_interval);
                    }
                    Err(e) => {
                        consecutive_failures += 1;
                        was_disconnected = true;
                        current_interval = std::cmp::min(
                            base_interval * 2u32.saturating_pow(consecutive_failures.min(4)),
                            max_backoff,
                        );
                        warn!("Heartbeat error ({}x): {} — next retry in {:?}",
                            consecutive_failures, e, current_interval);
                    }
                }
            }
        })
    }

    /// Start the worker HTTP server
    pub async fn start_server(self) -> Result<()> {
        let port = self.state.config.port;
        let state = self.state.clone();

        // Start CPU monitoring background thread
        start_cpu_monitor();

        // Register with hub first
        if let Err(e) = self.register().await {
            warn!("Initial registration failed (will retry via heartbeat): {}", e);
        }

        // Start heartbeat
        Self::start_heartbeat(state.clone());

        let app = Router::new()
            .route("/worker/execute", post(handle_execute))
            .route("/health", get(handle_health))
            .route("/worker/status", get(handle_status))
            .with_state(state);

        let addr = format!("0.0.0.0:{}", port);
        let listener = TcpListener::bind(&addr).await?;
        info!("Worker '{}' listening on http://{}", self.state.config.node_name, addr);

        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = tokio::signal::ctrl_c().await;
                info!("Worker shutting down...");
            })
            .await?;

        Ok(())
    }
}

/// Handle POST /worker/execute — run a tool and return the result
async fn handle_execute(
    State(state): State<WorkerState>,
    Json(req): Json<ExecuteRequest>,
) -> Result<Json<ExecuteResponse>, StatusCode> {
    info!("Executing tool '{}' on worker '{}'", req.tool, state.config.node_name);

    match state.tool_registry.execute_tool(&req.tool, req.input).await {
        Ok(result) => {
            Ok(Json(ExecuteResponse {
                success: result.success,
                output: result.output,
            }))
        }
        Err(e) => {
            error!("Tool execution error: {}", e);
            Ok(Json(ExecuteResponse {
                success: false,
                output: format!("Tool execution error: {}", e),
            }))
        }
    }
}

/// Handle GET /health — return node health status
async fn handle_health(
    State(state): State<WorkerState>,
) -> Json<Value> {
    Json(json!({
        "status": "ok",
        "name": state.config.node_name,
        "capabilities": state.config.capabilities,
        "device_type": state.config.device_type,
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// Handle GET /worker/status — return detailed worker status
async fn handle_status(
    State(state): State<WorkerState>,
) -> Json<Value> {
    let tools: Vec<String> = state.tool_registry.specs()
        .iter()
        .map(|s| s.name.clone())
        .collect();

    Json(json!({
        "name": state.config.node_name,
        "hub": state.config.hub_url,
        "capabilities": state.config.capabilities,
        "device_type": state.config.device_type,
        "cpu_load": get_cpu_load(),
        "tools_available": tools,
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// Detect the local IP address that can reach the hub.
/// Uses UDP socket trick: connect to hub IP → OS picks the right local interface.
/// This automatically selects Tailscale IP (100.x) when hub is on Tailscale,
/// or LAN IP (192.168.x) when hub is on LAN.
fn detect_local_ip(hub_url: &str) -> String {
    // Parse hub host:port from URL
    if let Some(host_port) = hub_url
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .split('/')
        .next()
    {
        // Extract just the IP part (before port)
        let hub_host = host_port.split(':').next().unwrap_or("127.0.0.1");
        let hub_port: u16 = host_port.split(':').nth(1)
            .and_then(|p| p.parse().ok())
            .unwrap_or(7878);

        // UDP connect trick — doesn't send data, just lets OS pick the route
        if let Ok(socket) = std::net::UdpSocket::bind("0.0.0.0:0") {
            if socket.connect((hub_host, hub_port)).is_ok() {
                if let Ok(local_addr) = socket.local_addr() {
                    let ip = local_addr.ip().to_string();
                    if ip != "0.0.0.0" {
                        tracing::info!("Detected local IP: {} (route to hub {})", ip, hub_host);
                        return ip;
                    }
                }
            }
        }
    }

    // Fallback: try hostname
    if let Ok(hostname) = std::env::var("HOSTNAME") {
        return hostname;
    }
    if let Ok(name) = std::env::var("COMPUTERNAME") {
        return name;
    }
    "127.0.0.1".to_string()
}

/// Cached CPU load value, updated by background monitor thread.
/// Stored as percentage * 100 (e.g., 7500 = 75.00% = 0.75 load).
static CPU_LOAD_CACHE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1000); // default 10%

/// Start background thread that measures real CPU usage every ~15 seconds.
/// Must be called once at worker startup. Uses sysinfo crate for cross-platform support.
pub fn start_cpu_monitor() {
    std::thread::spawn(|| {
        let mut sys = sysinfo::System::new();
        loop {
            // sysinfo requires two refresh calls with delay for accurate CPU measurement
            sys.refresh_cpu_usage();
            std::thread::sleep(std::time::Duration::from_secs(1));
            sys.refresh_cpu_usage();

            let cpus = sys.cpus();
            if !cpus.is_empty() {
                let avg = cpus.iter().map(|c| c.cpu_usage()).sum::<f32>() / cpus.len() as f32;
                // Store as percentage * 100 (0-10000 range)
                CPU_LOAD_CACHE.store(
                    (avg * 100.0).clamp(0.0, 10000.0) as u32,
                    std::sync::atomic::Ordering::Relaxed,
                );
            }

            std::thread::sleep(std::time::Duration::from_secs(14)); // ~15s total cycle
        }
    });
}

/// Get current CPU load (0.0-1.0), read from background monitor cache.
fn get_cpu_load() -> f32 {
    let raw = CPU_LOAD_CACHE.load(std::sync::atomic::Ordering::Relaxed);
    (raw as f32 / 10000.0).clamp(0.0, 1.0)
}

/// Cluster configuration from agents.toml
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ClusterConfig {
    /// Role: "hub" or "worker"
    #[serde(default = "default_role")]
    pub role: String,
    /// Hub URL (required for workers)
    #[serde(default)]
    pub hub_url: Option<String>,
    /// Node name (auto-detected if not set)
    #[serde(default)]
    pub node_name: Option<String>,
}

fn default_role() -> String { "hub".to_string() }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_worker_config() {
        let config = WorkerConfig {
            hub_url: "http://100.0.0.1:7878".to_string(),
            node_name: "test-worker".to_string(),
            capabilities: vec!["tools".to_string()],
            device_type: "full".to_string(),
            port: 7879,
        };
        assert_eq!(config.node_name, "test-worker");
        assert_eq!(config.capabilities.len(), 1);
    }

    #[test]
    fn test_cluster_config_defaults() {
        let config: ClusterConfig = toml::from_str("").unwrap();
        assert_eq!(config.role, "hub");
        assert!(config.hub_url.is_none());
        assert!(config.node_name.is_none());
    }

    #[test]
    fn test_cluster_config_worker() {
        let toml_str = r#"
role = "worker"
hub_url = "http://100.0.0.1:7878"
node_name = "m1"
"#;
        let config: ClusterConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.role, "worker");
        assert_eq!(config.hub_url.unwrap(), "http://100.0.0.1:7878");
        assert_eq!(config.node_name.unwrap(), "m1");
    }

    #[test]
    fn test_cpu_load_range() {
        let load = get_cpu_load();
        assert!(load >= 0.0 && load <= 1.0);
    }

    #[test]
    fn test_execute_request_deserialize() {
        let json_str = r#"{"tool": "web_search", "input": {"query": "test"}}"#;
        let req: ExecuteRequest = serde_json::from_str(json_str).unwrap();
        assert_eq!(req.tool, "web_search");
    }
}
