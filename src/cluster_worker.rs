//! ClusterWorker — runs on worker nodes, registers with hub, executes tools.
//! Start with: `clawtex-core worker --hub http://100.x.x.x:7878 --name m1`

use anyhow::Result;
use axum::{extract::State, http::StatusCode, response::Json, routing::{get, post}, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
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

/// Rich telemetry data collected from the worker node and sent alongside heartbeats.
/// Provides the hub with resource utilisation, task load, and provider health information
/// so it can make informed scheduling and routing decisions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerTelemetry {
    /// CPU utilisation percentage (0.0–100.0).
    pub cpu_pct: f32,
    /// RAM utilisation percentage (0.0–100.0).
    pub ram_pct: f32,
    /// Free disk space on the primary filesystem, in gigabytes.
    pub disk_free_gb: f32,
    /// Number of tasks currently being executed by this worker.
    pub active_tasks: usize,
    /// Names of LLM models currently loaded (e.g. via Ollama).
    pub loaded_models: Vec<String>,
    /// Per-provider reachability status (true = healthy).
    pub provider_status: HashMap<String, bool>,
    /// Seconds since the worker process started.
    pub uptime_secs: u64,
}

impl Default for WorkerTelemetry {
    fn default() -> Self {
        Self {
            cpu_pct: 0.0,
            ram_pct: 0.0,
            disk_free_gb: 0.0,
            active_tasks: 0,
            loaded_models: Vec::new(),
            provider_status: HashMap::new(),
            uptime_secs: 0,
        }
    }
}

/// Process start time — used to derive uptime_secs in telemetry.
static PROCESS_START: once_cell::sync::Lazy<std::time::Instant> =
    once_cell::sync::Lazy::new(std::time::Instant::now);

/// Collect a snapshot of system telemetry using the `sysinfo` crate.
///
/// CPU percentage is read from the background monitor cache (see [`start_cpu_monitor`]).
/// RAM and disk are sampled on the spot via `sysinfo::System` / `sysinfo::Disks`.
/// `active_tasks`, `loaded_models`, and `provider_status` are left at defaults
/// because the worker does not yet track those values internally; callers or
/// future patches should fill them in.
pub fn collect_telemetry() -> WorkerTelemetry {
    // Force-initialise the start time on first call.
    let uptime = PROCESS_START.elapsed().as_secs();

    // CPU — reuse the cached value from the background monitor thread.
    let cpu_pct = {
        let raw = CPU_LOAD_CACHE.load(std::sync::atomic::Ordering::Relaxed);
        // Cache stores percentage * 100 (0-10000).  Convert to 0.0-100.0.
        (raw as f32 / 100.0).clamp(0.0, 100.0)
    };

    // RAM — refresh memory counters (cheap, ~1 ms).
    let ram_pct = {
        let mut sys = sysinfo::System::new();
        sys.refresh_memory();
        let total = sys.total_memory(); // bytes
        if total > 0 {
            let used = sys.used_memory();
            ((used as f64 / total as f64) * 100.0) as f32
        } else {
            0.0
        }
    };

    // Disk — free space on the filesystem that contains the working directory.
    let disk_free_gb = {
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        disk_free_bytes_for(&cwd) as f32 / (1024.0 * 1024.0 * 1024.0)
    };

    WorkerTelemetry {
        cpu_pct,
        ram_pct,
        disk_free_gb,
        active_tasks: 0,
        loaded_models: Vec::new(),
        provider_status: HashMap::new(),
        uptime_secs: uptime,
    }
}

/// Return free bytes for the filesystem containing `path` (platform-independent).
fn disk_free_bytes_for(path: &std::path::Path) -> u64 {
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut best_match: Option<u64> = None;
    let mut best_len = 0usize;

    for disk in disks.list() {
        let mount = disk.mount_point();
        let mount_str = mount.to_string_lossy();
        let canon_str = canonical.to_string_lossy();
        // Case-insensitive prefix match (important on Windows).
        if canon_str.to_lowercase().starts_with(&mount_str.to_lowercase())
            && mount_str.len() >= best_len
        {
            best_len = mount_str.len();
            best_match = Some(disk.available_space());
        }
    }
    best_match.unwrap_or(0)
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
                let telemetry = collect_telemetry();
                let url = format!("{}/cluster/heartbeat", state.config.hub_url);
                let payload = json!({
                    "name": state.config.node_name,
                    "cpu_load": cpu_load,
                    "telemetry": telemetry,
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

    // ── WorkerTelemetry tests ──────────────────────────────────────────

    #[test]
    fn test_telemetry_struct_creation() {
        let mut providers = HashMap::new();
        providers.insert("ollama".to_string(), true);
        providers.insert("gemini".to_string(), false);

        let t = WorkerTelemetry {
            cpu_pct: 42.5,
            ram_pct: 65.3,
            disk_free_gb: 128.7,
            active_tasks: 3,
            loaded_models: vec!["llama3".to_string(), "mistral".to_string()],
            provider_status: providers,
            uptime_secs: 86400,
        };

        assert_eq!(t.cpu_pct, 42.5);
        assert_eq!(t.ram_pct, 65.3);
        assert_eq!(t.disk_free_gb, 128.7);
        assert_eq!(t.active_tasks, 3);
        assert_eq!(t.loaded_models.len(), 2);
        assert_eq!(t.provider_status.len(), 2);
        assert!(t.provider_status["ollama"]);
        assert!(!t.provider_status["gemini"]);
        assert_eq!(t.uptime_secs, 86400);
    }

    #[test]
    fn test_telemetry_default_values() {
        let t = WorkerTelemetry::default();
        assert_eq!(t.cpu_pct, 0.0);
        assert_eq!(t.ram_pct, 0.0);
        assert_eq!(t.disk_free_gb, 0.0);
        assert_eq!(t.active_tasks, 0);
        assert!(t.loaded_models.is_empty());
        assert!(t.provider_status.is_empty());
        assert_eq!(t.uptime_secs, 0);
    }

    #[test]
    fn test_telemetry_serialization_roundtrip() {
        let mut providers = HashMap::new();
        providers.insert("ollama".to_string(), true);

        let original = WorkerTelemetry {
            cpu_pct: 55.5,
            ram_pct: 70.2,
            disk_free_gb: 250.0,
            active_tasks: 1,
            loaded_models: vec!["phi3".to_string()],
            provider_status: providers,
            uptime_secs: 3600,
        };

        let json_str = serde_json::to_string(&original).expect("serialize");
        let restored: WorkerTelemetry = serde_json::from_str(&json_str).expect("deserialize");

        assert_eq!(original.cpu_pct, restored.cpu_pct);
        assert_eq!(original.ram_pct, restored.ram_pct);
        assert_eq!(original.disk_free_gb, restored.disk_free_gb);
        assert_eq!(original.active_tasks, restored.active_tasks);
        assert_eq!(original.loaded_models, restored.loaded_models);
        assert_eq!(original.provider_status, restored.provider_status);
        assert_eq!(original.uptime_secs, restored.uptime_secs);
    }

    #[test]
    fn test_telemetry_json_field_names() {
        let t = WorkerTelemetry::default();
        let v: Value = serde_json::to_value(&t).expect("to_value");
        // Verify all expected keys are present in the serialised JSON.
        assert!(v.get("cpu_pct").is_some());
        assert!(v.get("ram_pct").is_some());
        assert!(v.get("disk_free_gb").is_some());
        assert!(v.get("active_tasks").is_some());
        assert!(v.get("loaded_models").is_some());
        assert!(v.get("provider_status").is_some());
        assert!(v.get("uptime_secs").is_some());
    }

    #[test]
    fn test_telemetry_cpu_pct_boundary_values() {
        // Minimum
        let t_min = WorkerTelemetry { cpu_pct: 0.0, ..Default::default() };
        assert_eq!(t_min.cpu_pct, 0.0);

        // Maximum
        let t_max = WorkerTelemetry { cpu_pct: 100.0, ..Default::default() };
        assert_eq!(t_max.cpu_pct, 100.0);

        // Mid-range
        let t_mid = WorkerTelemetry { cpu_pct: 50.0, ..Default::default() };
        assert!((0.0..=100.0).contains(&t_mid.cpu_pct));
    }

    #[test]
    fn test_telemetry_ram_pct_boundary_values() {
        let t_zero = WorkerTelemetry { ram_pct: 0.0, ..Default::default() };
        assert_eq!(t_zero.ram_pct, 0.0);

        let t_full = WorkerTelemetry { ram_pct: 100.0, ..Default::default() };
        assert_eq!(t_full.ram_pct, 100.0);

        let t_mid = WorkerTelemetry { ram_pct: 73.8, ..Default::default() };
        assert!((0.0..=100.0).contains(&t_mid.ram_pct));
    }

    #[test]
    fn test_collect_telemetry_field_ranges() {
        // collect_telemetry() reads real system state; verify returned values
        // are within physically plausible ranges.
        let t = collect_telemetry();
        assert!(t.cpu_pct >= 0.0 && t.cpu_pct <= 100.0,
            "cpu_pct out of range: {}", t.cpu_pct);
        assert!(t.ram_pct >= 0.0 && t.ram_pct <= 100.0,
            "ram_pct out of range: {}", t.ram_pct);
        assert!(t.disk_free_gb >= 0.0,
            "disk_free_gb should be non-negative: {}", t.disk_free_gb);
        // uptime_secs should be at least 0 (process just started).
        // It is u64 so cannot be negative, but sanity-check it is small-ish
        // (< 1 year) to catch overflow bugs.
        assert!(t.uptime_secs < 365 * 24 * 3600,
            "uptime_secs suspiciously large: {}", t.uptime_secs);
    }

    #[test]
    fn test_telemetry_default_serializes_to_known_json() {
        let t = WorkerTelemetry::default();
        let v: Value = serde_json::to_value(&t).unwrap();
        assert_eq!(v["cpu_pct"], 0.0);
        assert_eq!(v["ram_pct"], 0.0);
        assert_eq!(v["disk_free_gb"], 0.0);
        assert_eq!(v["active_tasks"], 0);
        assert_eq!(v["loaded_models"], json!([]));
        assert_eq!(v["provider_status"], json!({}));
        assert_eq!(v["uptime_secs"], 0);
    }
}
