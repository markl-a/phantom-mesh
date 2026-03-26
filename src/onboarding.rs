//! WorkerOnboarder — automated onboarding system for new cluster workers.
//!
//! Handles the full lifecycle of adding a new worker to the cluster:
//! 1. SSH connectivity check (PC workers)
//! 2. Deploy worker binary or Python script via SCP
//! 3. Generate and deploy worker configuration
//! 4. Start the worker process remotely
//! 5. Wait for worker registration with the hub
//! 6. Run health check
//! 7. Report success/failure with details
//!
//! Mobile workers skip SSH steps and instead receive a deep link or
//! QR-code-compatible URL containing the hub URL and auth token.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::info;

use crate::cluster::ClusterRegistry;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration payload for onboarding a new worker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnboardConfig {
    /// Unique name for the worker (e.g., "acer", "m1-mac", "rog6").
    pub worker_name: String,
    /// SSH host or IP address (ignored for mobile workers).
    #[serde(default)]
    pub ssh_host: Option<String>,
    /// SSH user (ignored for mobile workers).
    #[serde(default)]
    pub ssh_user: Option<String>,
    /// SSH port (default 22, ignored for mobile workers).
    #[serde(default = "default_ssh_port")]
    pub ssh_port: u16,
    /// Worker type: "pc" or "mobile".
    #[serde(default = "default_worker_type")]
    pub worker_type: String,
    /// Hub URL that the worker should connect to.
    pub hub_url: String,
    /// Bearer auth token for the hub API.
    #[serde(default)]
    pub auth_token: Option<String>,
    /// Port the worker should listen on (default 7879).
    #[serde(default = "default_worker_port")]
    pub worker_port: u16,
    /// Device type reported to hub: "full", "light", or "mobile" (auto-detected from worker_type).
    #[serde(default)]
    pub device_type: Option<String>,
    /// Capabilities to advertise (default: ["tools"]).
    #[serde(default)]
    pub capabilities: Option<Vec<String>>,
    /// Path to the local worker binary to deploy (if not set, uses Python worker script).
    #[serde(default)]
    pub binary_path: Option<String>,
    /// Path to the local Python worker script to deploy.
    #[serde(default)]
    pub python_script_path: Option<String>,
    /// Remote directory to deploy into (default: ~/phantom-mesh-worker).
    #[serde(default = "default_remote_dir")]
    pub remote_dir: String,
    /// Python interpreter path on the remote machine.
    #[serde(default)]
    pub remote_python: Option<String>,
}

fn default_ssh_port() -> u16 {
    22
}
fn default_worker_type() -> String {
    "pc".to_string()
}
fn default_worker_port() -> u16 {
    7879
}
fn default_remote_dir() -> String {
    "~/phantom-mesh-worker".to_string()
}

// ---------------------------------------------------------------------------
// Onboarding result types
// ---------------------------------------------------------------------------

/// Result of a complete onboarding operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnboardResult {
    pub worker_name: String,
    pub success: bool,
    pub steps: Vec<OnboardStep>,
    /// For mobile workers: the deep link URL.
    pub mobile_link: Option<String>,
    pub error: Option<String>,
    pub duration_ms: u64,
}

/// A single onboarding step with its outcome.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnboardStep {
    pub name: String,
    pub status: StepStatus,
    pub message: String,
    pub duration_ms: u64,
}

/// Status of an individual onboarding step.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum StepStatus {
    Success,
    Failed,
    Skipped,
}

/// Health status returned by verify_worker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthStatus {
    pub worker_name: String,
    pub reachable: bool,
    pub registered: bool,
    pub status: String,
    pub capabilities: Vec<String>,
    pub device_type: String,
    pub cpu_load: f32,
    pub error: Option<String>,
}

/// Tracks the ongoing status of an onboarding operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnboardingStatus {
    pub worker_name: String,
    pub state: OnboardingState,
    pub current_step: String,
    pub steps_completed: Vec<OnboardStep>,
    pub started_at_unix: u64,
    pub error: Option<String>,
}

/// High-level state of an onboarding operation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum OnboardingState {
    InProgress,
    Completed,
    Failed,
}

// ---------------------------------------------------------------------------
// WorkerOnboarder
// ---------------------------------------------------------------------------

/// Central struct that manages worker onboarding operations.
pub struct WorkerOnboarder {
    registry: Arc<ClusterRegistry>,
    http_client: reqwest::Client,
    /// Tracks in-progress and recently completed onboarding operations.
    statuses: Mutex<HashMap<String, OnboardingStatus>>,
}

impl WorkerOnboarder {
    /// Create a new onboarder backed by the given cluster registry.
    pub fn new(registry: Arc<ClusterRegistry>) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_default();

        Self {
            registry,
            http_client,
            statuses: Mutex::new(HashMap::new()),
        }
    }

    // -----------------------------------------------------------------------
    // Public API
    // -----------------------------------------------------------------------

    /// Run the full onboarding sequence for a worker.
    ///
    /// For PC workers: SSH check -> deploy -> config -> start -> register wait -> health.
    /// For mobile workers: generate deep link / QR data (no SSH).
    pub async fn onboard_worker(&self, config: OnboardConfig) -> Result<OnboardResult> {
        let start = Instant::now();
        let worker_name = config.worker_name.clone();

        info!(worker = %worker_name, worker_type = %config.worker_type, "Starting onboarding");

        // Initialize status tracking
        {
            let mut statuses = self.statuses.lock().await;
            statuses.insert(
                worker_name.clone(),
                OnboardingStatus {
                    worker_name: worker_name.clone(),
                    state: OnboardingState::InProgress,
                    current_step: "initializing".to_string(),
                    steps_completed: Vec::new(),
                    started_at_unix: chrono::Utc::now().timestamp() as u64,
                    error: None,
                },
            );
        }

        let result = if config.worker_type == "mobile" {
            self.onboard_mobile(&config).await
        } else {
            self.onboard_pc(&config).await
        };

        let duration_ms = start.elapsed().as_millis() as u64;

        // Finalize status tracking
        let onboard_result = match result {
            Ok(mut r) => {
                r.duration_ms = duration_ms;
                let mut statuses = self.statuses.lock().await;
                if let Some(status) = statuses.get_mut(&worker_name) {
                    status.state = OnboardingState::Completed;
                    status.current_step = "done".to_string();
                    status.steps_completed = r.steps.clone();
                }
                r
            }
            Err(e) => {
                let mut statuses = self.statuses.lock().await;
                if let Some(status) = statuses.get_mut(&worker_name) {
                    status.state = OnboardingState::Failed;
                    status.current_step = "failed".to_string();
                    status.error = Some(e.to_string());
                }
                OnboardResult {
                    worker_name: worker_name.clone(),
                    success: false,
                    steps: Vec::new(),
                    mobile_link: None,
                    error: Some(e.to_string()),
                    duration_ms,
                }
            }
        };

        info!(
            worker = %worker_name,
            success = onboard_result.success,
            duration_ms = duration_ms,
            "Onboarding finished"
        );

        Ok(onboard_result)
    }

    /// Verify a worker's health and registration status.
    pub async fn verify_worker(&self, worker_name: &str) -> Result<HealthStatus> {
        // Check registry
        let node = self.registry.get_node(worker_name).await;

        match node {
            Some(n) => {
                // Try HTTP health check
                let url = format!("http://{}:{}/health", n.host, n.port);
                let reachable = match self.http_client.get(&url).send().await {
                    Ok(resp) => resp.status().is_success(),
                    Err(_) => false,
                };

                Ok(HealthStatus {
                    worker_name: worker_name.to_string(),
                    reachable,
                    registered: true,
                    status: n.status.clone(),
                    capabilities: n.capabilities.clone(),
                    device_type: n.device_type.clone(),
                    cpu_load: n.cpu_load,
                    error: if !reachable {
                        Some("Worker registered but HTTP health check failed".to_string())
                    } else {
                        None
                    },
                })
            }
            None => Ok(HealthStatus {
                worker_name: worker_name.to_string(),
                reachable: false,
                registered: false,
                status: "unknown".to_string(),
                capabilities: Vec::new(),
                device_type: "unknown".to_string(),
                cpu_load: 0.0,
                error: Some("Worker not found in registry".to_string()),
            }),
        }
    }

    /// Generate the content of a worker configuration file (TOML format).
    pub fn generate_worker_config(worker_name: &str, hub_url: &str) -> String {
        format!(
            r#"# Phantom Mesh Worker Configuration
# Generated by auto-onboarding system
# Worker: {worker_name}

[cluster]
role = "worker"
hub_url = "{hub_url}"
node_name = "{worker_name}"
"#,
        )
    }

    /// Get the current onboarding status for a worker.
    pub async fn get_status(&self, worker_name: &str) -> Option<OnboardingStatus> {
        let statuses = self.statuses.lock().await;
        statuses.get(worker_name).cloned()
    }

    /// Generate a mobile deep link URL containing hub connection details.
    pub fn generate_mobile_link(hub_url: &str, auth_token: Option<&str>, worker_name: &str) -> String {
        let mut link = format!(
            "phantom_mesh://join?hub={}&worker={}",
            urlencoding::encode(hub_url),
            urlencoding::encode(worker_name),
        );
        if let Some(token) = auth_token {
            link.push_str(&format!("&token={}", urlencoding::encode(token)));
        }
        link
    }

    // -----------------------------------------------------------------------
    // PC onboarding (SSH-based)
    // -----------------------------------------------------------------------

    async fn onboard_pc(&self, config: &OnboardConfig) -> Result<OnboardResult> {
        let worker_name = &config.worker_name;
        let mut steps: Vec<OnboardStep> = Vec::new();

        let ssh_host = config.ssh_host.as_deref()
            .ok_or_else(|| anyhow!("ssh_host is required for PC workers"))?;
        let ssh_user = config.ssh_user.as_deref()
            .ok_or_else(|| anyhow!("ssh_user is required for PC workers"))?;

        // Step 1: SSH connectivity check
        self.update_step(worker_name, "ssh_check").await;
        let step = self.step_ssh_check(ssh_user, ssh_host, config.ssh_port).await;
        let ssh_ok = step.status == StepStatus::Success;
        steps.push(step);
        if !ssh_ok {
            return Ok(OnboardResult {
                worker_name: worker_name.clone(),
                success: false,
                steps,
                mobile_link: None,
                error: Some("SSH connectivity check failed".to_string()),
                duration_ms: 0,
            });
        }

        // Step 2: Create remote directory + deploy worker binary/script
        self.update_step(worker_name, "deploy").await;
        let step = self.step_deploy(config).await;
        let deploy_ok = step.status == StepStatus::Success;
        steps.push(step);
        if !deploy_ok {
            return Ok(OnboardResult {
                worker_name: worker_name.clone(),
                success: false,
                steps,
                mobile_link: None,
                error: Some("Deployment failed".to_string()),
                duration_ms: 0,
            });
        }

        // Step 3: Generate and deploy worker config
        self.update_step(worker_name, "config_deploy").await;
        let step = self.step_deploy_config(config).await;
        let config_ok = step.status == StepStatus::Success;
        steps.push(step);
        if !config_ok {
            return Ok(OnboardResult {
                worker_name: worker_name.clone(),
                success: false,
                steps,
                mobile_link: None,
                error: Some("Config deployment failed".to_string()),
                duration_ms: 0,
            });
        }

        // Step 4: Start worker process remotely
        self.update_step(worker_name, "start_worker").await;
        let step = self.step_start_worker(config).await;
        let start_ok = step.status == StepStatus::Success;
        steps.push(step);
        if !start_ok {
            return Ok(OnboardResult {
                worker_name: worker_name.clone(),
                success: false,
                steps,
                mobile_link: None,
                error: Some("Failed to start worker process".to_string()),
                duration_ms: 0,
            });
        }

        // Step 5: Wait for worker to register with hub
        self.update_step(worker_name, "wait_registration").await;
        let step = self.step_wait_registration(worker_name).await;
        let reg_ok = step.status == StepStatus::Success;
        steps.push(step);
        if !reg_ok {
            return Ok(OnboardResult {
                worker_name: worker_name.clone(),
                success: false,
                steps,
                mobile_link: None,
                error: Some("Worker did not register with hub in time".to_string()),
                duration_ms: 0,
            });
        }

        // Step 6: Health check
        self.update_step(worker_name, "health_check").await;
        let step = self.step_health_check(worker_name).await;
        let health_ok = step.status == StepStatus::Success;
        steps.push(step);

        let success = health_ok;
        Ok(OnboardResult {
            worker_name: worker_name.clone(),
            success,
            steps,
            mobile_link: None,
            error: if !success {
                Some("Health check failed after registration".to_string())
            } else {
                None
            },
            duration_ms: 0,
        })
    }

    // -----------------------------------------------------------------------
    // Mobile onboarding (no SSH)
    // -----------------------------------------------------------------------

    async fn onboard_mobile(&self, config: &OnboardConfig) -> Result<OnboardResult> {
        let worker_name = &config.worker_name;
        let mut steps: Vec<OnboardStep> = Vec::new();

        let start = Instant::now();

        // Step 1: Generate mobile deep link
        self.update_step(worker_name, "generate_link").await;
        let link = Self::generate_mobile_link(
            &config.hub_url,
            config.auth_token.as_deref(),
            worker_name,
        );

        steps.push(OnboardStep {
            name: "generate_link".to_string(),
            status: StepStatus::Success,
            message: format!("Deep link generated: {}", link),
            duration_ms: start.elapsed().as_millis() as u64,
        });

        // Step 2: Pre-register the worker in the registry so it's known
        self.update_step(worker_name, "pre_register").await;
        let device_type = config.device_type.as_deref().unwrap_or("mobile");
        let capabilities = config.capabilities.clone().unwrap_or_else(|| {
            vec![
                "web_search".to_string(),
                "http_request".to_string(),
            ]
        });

        let step_start = Instant::now();
        match self.registry.register_full(
            worker_name,
            "0.0.0.0", // placeholder until mobile connects
            config.worker_port,
            &capabilities,
            device_type,
        ).await {
            Ok(()) => {
                steps.push(OnboardStep {
                    name: "pre_register".to_string(),
                    status: StepStatus::Success,
                    message: format!("Pre-registered '{}' as {}", worker_name, device_type),
                    duration_ms: step_start.elapsed().as_millis() as u64,
                });
            }
            Err(e) => {
                steps.push(OnboardStep {
                    name: "pre_register".to_string(),
                    status: StepStatus::Failed,
                    message: format!("Pre-registration failed: {}", e),
                    duration_ms: step_start.elapsed().as_millis() as u64,
                });
                return Ok(OnboardResult {
                    worker_name: worker_name.clone(),
                    success: false,
                    steps,
                    mobile_link: Some(link),
                    error: Some("Pre-registration failed".to_string()),
                    duration_ms: 0,
                });
            }
        }

        info!(
            worker = %worker_name,
            link = %link,
            "Mobile worker onboarding: share the deep link with the device"
        );

        Ok(OnboardResult {
            worker_name: worker_name.clone(),
            success: true,
            steps,
            mobile_link: Some(link),
            error: None,
            duration_ms: 0,
        })
    }

    // -----------------------------------------------------------------------
    // Individual step implementations
    // -----------------------------------------------------------------------

    /// Build the SSH command prefix for a given config.
    fn ssh_prefix(user: &str, host: &str, port: u16) -> Vec<String> {
        let mut args = vec![
            "ssh".to_string(),
            "-o".to_string(), "StrictHostKeyChecking=no".to_string(),
            "-o".to_string(), "ConnectTimeout=10".to_string(),
            "-o".to_string(), "BatchMode=yes".to_string(),
        ];
        if port != 22 {
            args.push("-p".to_string());
            args.push(port.to_string());
        }
        args.push(format!("{}@{}", user, host));
        args
    }

    /// Build SCP arguments for a file transfer.
    fn scp_args(user: &str, host: &str, port: u16, local: &str, remote: &str) -> Vec<String> {
        let mut args = vec![
            "scp".to_string(),
            "-o".to_string(), "StrictHostKeyChecking=no".to_string(),
            "-o".to_string(), "ConnectTimeout=10".to_string(),
            "-o".to_string(), "BatchMode=yes".to_string(),
        ];
        if port != 22 {
            args.push("-P".to_string());
            args.push(port.to_string());
        }
        args.push(local.to_string());
        args.push(format!("{}@{}:{}", user, host, remote));
        args
    }

    /// Execute a shell command asynchronously, cross-platform.
    async fn run_shell_command(command: &str) -> Result<String> {
        let output = if cfg!(target_os = "windows") {
            tokio::process::Command::new("cmd")
                .args(["/C", command])
                .output()
                .await?
        } else {
            tokio::process::Command::new("sh")
                .args(["-c", command])
                .output()
                .await?
        };

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if output.status.success() {
            Ok(stdout)
        } else {
            Err(anyhow!(
                "Command failed (exit {}): stdout={}, stderr={}",
                output.status,
                stdout.trim(),
                stderr.trim()
            ))
        }
    }

    /// Execute an SSH command on the remote host.
    async fn run_ssh_command(user: &str, host: &str, port: u16, remote_cmd: &str) -> Result<String> {
        let ssh_prefix = Self::ssh_prefix(user, host, port);
        let full_cmd = format!("{} \"{}\"", ssh_prefix.join(" "), remote_cmd);
        Self::run_shell_command(&full_cmd).await
    }

    /// Step 1: SSH connectivity check.
    async fn step_ssh_check(&self, user: &str, host: &str, port: u16) -> OnboardStep {
        let start = Instant::now();
        let result = Self::run_ssh_command(user, host, port, "echo phantom_mesh-ssh-ok").await;

        match result {
            Ok(output) if output.contains("phantom_mesh-ssh-ok") => OnboardStep {
                name: "ssh_check".to_string(),
                status: StepStatus::Success,
                message: format!("SSH connection to {}@{}:{} successful", user, host, port),
                duration_ms: start.elapsed().as_millis() as u64,
            },
            Ok(output) => OnboardStep {
                name: "ssh_check".to_string(),
                status: StepStatus::Failed,
                message: format!("SSH connected but unexpected output: {}", output.trim()),
                duration_ms: start.elapsed().as_millis() as u64,
            },
            Err(e) => OnboardStep {
                name: "ssh_check".to_string(),
                status: StepStatus::Failed,
                message: format!("SSH connection failed: {}", e),
                duration_ms: start.elapsed().as_millis() as u64,
            },
        }
    }

    /// Step 2: Deploy worker binary or Python script to remote.
    async fn step_deploy(&self, config: &OnboardConfig) -> OnboardStep {
        let start = Instant::now();
        let user = config.ssh_user.as_deref().unwrap_or("root");
        let host = config.ssh_host.as_deref().unwrap_or("127.0.0.1");
        let port = config.ssh_port;
        let remote_dir = &config.remote_dir;

        // Create remote directory
        let mkdir_cmd = format!("mkdir -p {}", remote_dir);
        if let Err(e) = Self::run_ssh_command(user, host, port, &mkdir_cmd).await {
            return OnboardStep {
                name: "deploy".to_string(),
                status: StepStatus::Failed,
                message: format!("Failed to create remote directory {}: {}", remote_dir, e),
                duration_ms: start.elapsed().as_millis() as u64,
            };
        }

        // Determine what to deploy
        if let Some(ref binary_path) = config.binary_path {
            // Deploy compiled binary via SCP
            let remote_path = format!("{}/phantom-mesh-worker", remote_dir);
            let scp_args = Self::scp_args(user, host, port, binary_path, &remote_path);
            let scp_cmd = scp_args.join(" ");

            match Self::run_shell_command(&scp_cmd).await {
                Ok(_) => {
                    // Make executable
                    let chmod_cmd = format!("chmod +x {}/phantom-mesh-worker", remote_dir);
                    let _ = Self::run_ssh_command(user, host, port, &chmod_cmd).await;

                    OnboardStep {
                        name: "deploy".to_string(),
                        status: StepStatus::Success,
                        message: format!("Binary deployed to {}:{}", host, remote_path),
                        duration_ms: start.elapsed().as_millis() as u64,
                    }
                }
                Err(e) => OnboardStep {
                    name: "deploy".to_string(),
                    status: StepStatus::Failed,
                    message: format!("SCP binary deploy failed: {}", e),
                    duration_ms: start.elapsed().as_millis() as u64,
                },
            }
        } else if let Some(ref script_path) = config.python_script_path {
            // Deploy Python worker script via SCP
            let remote_path = format!("{}/worker.py", remote_dir);
            let scp_args = Self::scp_args(user, host, port, script_path, &remote_path);
            let scp_cmd = scp_args.join(" ");

            match Self::run_shell_command(&scp_cmd).await {
                Ok(_) => OnboardStep {
                    name: "deploy".to_string(),
                    status: StepStatus::Success,
                    message: format!("Python worker script deployed to {}:{}", host, remote_path),
                    duration_ms: start.elapsed().as_millis() as u64,
                },
                Err(e) => OnboardStep {
                    name: "deploy".to_string(),
                    status: StepStatus::Failed,
                    message: format!("SCP script deploy failed: {}", e),
                    duration_ms: start.elapsed().as_millis() as u64,
                },
            }
        } else {
            // Generate a minimal Python worker inline and deploy via echo
            let py_script = Self::generate_python_worker_script(
                &config.hub_url,
                &config.worker_name,
                config.worker_port,
            );
            // Escape for shell and write via SSH
            let escaped = py_script.replace('\\', "\\\\").replace('"', "\\\"").replace('$', "\\$");
            let write_cmd = format!(
                "cat > {}/worker.py << 'PHANTOM_MESH_WORKER_EOF'\n{}\nPHANTOM_MESH_WORKER_EOF",
                remote_dir, py_script
            );
            // Use heredoc instead of escaped echo
            let _ = escaped; // suppress unused warning

            match Self::run_ssh_command(user, host, port, &write_cmd).await {
                Ok(_) => OnboardStep {
                    name: "deploy".to_string(),
                    status: StepStatus::Success,
                    message: format!("Generated Python worker deployed to {}:{}/worker.py", host, remote_dir),
                    duration_ms: start.elapsed().as_millis() as u64,
                },
                Err(e) => OnboardStep {
                    name: "deploy".to_string(),
                    status: StepStatus::Failed,
                    message: format!("Failed to deploy generated worker: {}", e),
                    duration_ms: start.elapsed().as_millis() as u64,
                },
            }
        }
    }

    /// Step 3: Generate and deploy worker configuration.
    async fn step_deploy_config(&self, config: &OnboardConfig) -> OnboardStep {
        let start = Instant::now();
        let user = config.ssh_user.as_deref().unwrap_or("root");
        let host = config.ssh_host.as_deref().unwrap_or("127.0.0.1");
        let port = config.ssh_port;
        let remote_dir = &config.remote_dir;

        let config_content = Self::generate_worker_config(&config.worker_name, &config.hub_url);
        let write_cmd = format!(
            "cat > {}/worker.toml << 'PHANTOM_MESH_CONFIG_EOF'\n{}\nPHANTOM_MESH_CONFIG_EOF",
            remote_dir, config_content
        );

        match Self::run_ssh_command(user, host, port, &write_cmd).await {
            Ok(_) => OnboardStep {
                name: "config_deploy".to_string(),
                status: StepStatus::Success,
                message: format!("Worker config deployed to {}:{}/worker.toml", host, remote_dir),
                duration_ms: start.elapsed().as_millis() as u64,
            },
            Err(e) => OnboardStep {
                name: "config_deploy".to_string(),
                status: StepStatus::Failed,
                message: format!("Config deployment failed: {}", e),
                duration_ms: start.elapsed().as_millis() as u64,
            },
        }
    }

    /// Step 4: Start the worker process remotely.
    async fn step_start_worker(&self, config: &OnboardConfig) -> OnboardStep {
        let start = Instant::now();
        let user = config.ssh_user.as_deref().unwrap_or("root");
        let host = config.ssh_host.as_deref().unwrap_or("127.0.0.1");
        let port = config.ssh_port;
        let remote_dir = &config.remote_dir;

        let start_cmd = if config.binary_path.is_some() {
            // Start the Rust binary
            format!(
                "cd {} && nohup ./phantom-mesh-worker worker --hub {} --name {} --port {} --device-type {} > worker.log 2>&1 &",
                remote_dir,
                config.hub_url,
                config.worker_name,
                config.worker_port,
                config.device_type.as_deref().unwrap_or("full"),
            )
        } else {
            // Start the Python worker
            let python = config.remote_python.as_deref().unwrap_or("python3");
            format!(
                "cd {} && nohup {} worker.py > worker.log 2>&1 &",
                remote_dir, python,
            )
        };

        match Self::run_ssh_command(user, host, port, &start_cmd).await {
            Ok(_) => OnboardStep {
                name: "start_worker".to_string(),
                status: StepStatus::Success,
                message: format!("Worker process started on {}", host),
                duration_ms: start.elapsed().as_millis() as u64,
            },
            Err(e) => OnboardStep {
                name: "start_worker".to_string(),
                status: StepStatus::Failed,
                message: format!("Failed to start worker: {}", e),
                duration_ms: start.elapsed().as_millis() as u64,
            },
        }
    }

    /// Step 5: Wait for the worker to register with the hub.
    async fn step_wait_registration(&self, worker_name: &str) -> OnboardStep {
        let start = Instant::now();
        let timeout = Duration::from_secs(60);
        let poll_interval = Duration::from_secs(3);

        loop {
            if start.elapsed() > timeout {
                return OnboardStep {
                    name: "wait_registration".to_string(),
                    status: StepStatus::Failed,
                    message: format!(
                        "Worker '{}' did not register within {}s",
                        worker_name,
                        timeout.as_secs()
                    ),
                    duration_ms: start.elapsed().as_millis() as u64,
                };
            }

            if let Some(node) = self.registry.get_node(worker_name).await {
                if node.status == "online" {
                    return OnboardStep {
                        name: "wait_registration".to_string(),
                        status: StepStatus::Success,
                        message: format!(
                            "Worker '{}' registered ({}:{})",
                            worker_name, node.host, node.port
                        ),
                        duration_ms: start.elapsed().as_millis() as u64,
                    };
                }
            }

            tokio::time::sleep(poll_interval).await;
        }
    }

    /// Step 6: Run health check against the registered worker.
    async fn step_health_check(&self, worker_name: &str) -> OnboardStep {
        let start = Instant::now();

        match self.verify_worker(worker_name).await {
            Ok(health) if health.reachable && health.registered => OnboardStep {
                name: "health_check".to_string(),
                status: StepStatus::Success,
                message: format!(
                    "Worker '{}' healthy: status={}, device={}, caps={:?}",
                    worker_name, health.status, health.device_type, health.capabilities
                ),
                duration_ms: start.elapsed().as_millis() as u64,
            },
            Ok(health) => OnboardStep {
                name: "health_check".to_string(),
                status: StepStatus::Failed,
                message: format!(
                    "Worker '{}' unhealthy: reachable={}, registered={}, error={:?}",
                    worker_name, health.reachable, health.registered, health.error
                ),
                duration_ms: start.elapsed().as_millis() as u64,
            },
            Err(e) => OnboardStep {
                name: "health_check".to_string(),
                status: StepStatus::Failed,
                message: format!("Health check error: {}", e),
                duration_ms: start.elapsed().as_millis() as u64,
            },
        }
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Update the current step for status tracking.
    async fn update_step(&self, worker_name: &str, step: &str) {
        let mut statuses = self.statuses.lock().await;
        if let Some(status) = statuses.get_mut(worker_name) {
            status.current_step = step.to_string();
        }
    }

    /// Generate a minimal Python worker script for lightweight workers.
    fn generate_python_worker_script(hub_url: &str, worker_name: &str, port: u16) -> String {
        format!(
            r#"#!/usr/bin/env python3
"""Phantom Mesh lightweight Python worker - auto-generated by onboarding system."""
import json, subprocess, time, urllib.request, http.server, threading, os, platform

HUB_URL = "{hub_url}"
WORKER_NAME = "{worker_name}"
WORKER_PORT = {port}
HEARTBEAT_INTERVAL = 15

def get_cpu_load():
    try:
        if platform.system() == "Windows":
            r = subprocess.run(
                ["wmic", "cpu", "get", "loadpercentage", "/value"],
                capture_output=True, text=True, timeout=5
            )
            for line in r.stdout.split("\n"):
                if "LoadPercentage" in line:
                    return float(line.split("=")[1].strip()) / 100.0
        else:
            return os.getloadavg()[0] / os.cpu_count()
    except Exception:
        pass
    return 0.1

def register():
    data = json.dumps({{
        "name": WORKER_NAME,
        "host": get_local_ip(),
        "port": WORKER_PORT,
        "capabilities": ["tools"],
        "device_type": "light"
    }}).encode()
    req = urllib.request.Request(f"{{HUB_URL}}/cluster/register", data=data,
                                 headers={{"Content-Type": "application/json"}})
    try:
        urllib.request.urlopen(req, timeout=10)
        print(f"Registered as {{WORKER_NAME}}")
    except Exception as e:
        print(f"Registration failed: {{e}}")

def heartbeat_loop():
    while True:
        time.sleep(HEARTBEAT_INTERVAL)
        data = json.dumps({{"name": WORKER_NAME, "cpu_load": get_cpu_load()}}).encode()
        req = urllib.request.Request(f"{{HUB_URL}}/cluster/heartbeat", data=data,
                                     headers={{"Content-Type": "application/json"}})
        try:
            urllib.request.urlopen(req, timeout=10)
        except Exception:
            pass

def get_local_ip():
    import socket
    try:
        s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        hub_host = HUB_URL.replace("http://","").replace("https://","").split("/")[0].split(":")[0]
        hub_port = int(HUB_URL.replace("http://","").replace("https://","").split("/")[0].split(":")[-1]) if ":" in HUB_URL.replace("http://","").split("/")[0] else 7878
        s.connect((hub_host, hub_port))
        ip = s.getsockname()[0]
        s.close()
        return ip
    except Exception:
        return "127.0.0.1"

class WorkerHandler(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        if self.path == "/worker/execute":
            length = int(self.headers.get("Content-Length", 0))
            body = json.loads(self.rfile.read(length)) if length else {{}}
            tool = body.get("tool", "")
            inp = body.get("input", {{}})
            result = self.execute_tool(tool, inp)
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(json.dumps(result).encode())
        else:
            self.send_response(404)
            self.end_headers()

    def do_GET(self):
        if self.path == "/health":
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(json.dumps({{"status": "ok", "name": WORKER_NAME}}).encode())
        else:
            self.send_response(404)
            self.end_headers()

    def execute_tool(self, tool, inp):
        try:
            if tool == "shell":
                cmd = inp.get("command", "echo no-command")
                r = subprocess.run(cmd, shell=True, capture_output=True, text=True, timeout=60)
                return {{"success": r.returncode == 0, "output": r.stdout + r.stderr}}
            elif tool in ("web_search", "http_request"):
                url = inp.get("url") or inp.get("query", "")
                if url.startswith("http"):
                    resp = urllib.request.urlopen(url, timeout=30)
                    return {{"success": True, "output": resp.read().decode(errors="replace")[:4000]}}
                return {{"success": False, "output": f"Invalid URL: {{url}}"}}
            else:
                return {{"success": False, "output": f"Tool '{{tool}}' not supported on this worker"}}
        except Exception as e:
            return {{"success": False, "output": str(e)}}

    def log_message(self, fmt, *args):
        pass  # suppress default logging

if __name__ == "__main__":
    register()
    threading.Thread(target=heartbeat_loop, daemon=True).start()
    server = http.server.HTTPServer(("0.0.0.0", WORKER_PORT), WorkerHandler)
    print(f"Worker '{{WORKER_NAME}}' listening on port {{WORKER_PORT}}")
    server.serve_forever()
"#,
        )
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn test_registry() -> Arc<ClusterRegistry> {
        Arc::new(
            futures_util::FutureExt::now_or_never(ClusterRegistry::new(":memory:"))
                .unwrap()
                .unwrap(),
        )
    }

    fn test_config_pc() -> OnboardConfig {
        OnboardConfig {
            worker_name: "test-worker".to_string(),
            ssh_host: Some("10.0.1.2".to_string()),
            ssh_user: Some("worker".to_string()),
            ssh_port: 22,
            worker_type: "pc".to_string(),
            hub_url: "http://100.0.0.1:7878".to_string(),
            auth_token: Some("test-token".to_string()),
            worker_port: 7879,
            device_type: Some("full".to_string()),
            capabilities: Some(vec!["tools".to_string()]),
            binary_path: None,
            python_script_path: None,
            remote_dir: "~/phantom-mesh-worker".to_string(),
            remote_python: None,
        }
    }

    fn test_config_mobile() -> OnboardConfig {
        OnboardConfig {
            worker_name: "test-phone".to_string(),
            ssh_host: None,
            ssh_user: None,
            ssh_port: 22,
            worker_type: "mobile".to_string(),
            hub_url: "http://100.0.0.1:7878".to_string(),
            auth_token: Some("mobile-token-123".to_string()),
            worker_port: 7880,
            device_type: Some("mobile".to_string()),
            capabilities: Some(vec!["web_search".to_string(), "http_request".to_string()]),
            binary_path: None,
            python_script_path: None,
            remote_dir: "~/phantom-mesh-worker".to_string(),
            remote_python: None,
        }
    }

    #[test]
    fn test_onboard_config_defaults() {
        let json_str = r#"{
            "worker_name": "w1",
            "hub_url": "http://localhost:7878"
        }"#;
        let config: OnboardConfig = serde_json::from_str(json_str).unwrap();
        assert_eq!(config.worker_name, "w1");
        assert_eq!(config.ssh_port, 22);
        assert_eq!(config.worker_type, "pc");
        assert_eq!(config.worker_port, 7879);
        assert_eq!(config.remote_dir, "~/phantom-mesh-worker");
        assert!(config.ssh_host.is_none());
        assert!(config.auth_token.is_none());
    }

    #[test]
    fn test_onboard_config_full_deserialize() {
        let json_str = r#"{
            "worker_name": "acer",
            "ssh_host": "10.0.1.3",
            "ssh_user": "worker",
            "ssh_port": 22,
            "worker_type": "pc",
            "hub_url": "http://10.0.2.1:7878",
            "auth_token": "phantom_mesh-hub-2026",
            "worker_port": 7881,
            "device_type": "light",
            "capabilities": ["tools", "web_search"],
            "binary_path": null,
            "python_script_path": "/path/to/worker.py",
            "remote_dir": "/home/user/phantom_mesh"
        }"#;
        let config: OnboardConfig = serde_json::from_str(json_str).unwrap();
        assert_eq!(config.worker_name, "acer");
        assert_eq!(config.ssh_host.unwrap(), "10.0.1.3");
        assert_eq!(config.ssh_user.unwrap(), "worker");
        assert_eq!(config.worker_port, 7881);
        assert_eq!(config.device_type.unwrap(), "light");
        assert_eq!(config.capabilities.unwrap().len(), 2);
        assert_eq!(config.python_script_path.unwrap(), "/path/to/worker.py");
    }

    #[test]
    fn test_generate_worker_config() {
        let config = WorkerOnboarder::generate_worker_config("m1-mac", "http://100.0.0.1:7878");
        assert!(config.contains("role = \"worker\""));
        assert!(config.contains("hub_url = \"http://100.0.0.1:7878\""));
        assert!(config.contains("node_name = \"m1-mac\""));
    }

    #[test]
    fn test_generate_mobile_link() {
        let link = WorkerOnboarder::generate_mobile_link(
            "http://100.0.0.1:7878",
            Some("secret-token"),
            "rog6",
        );
        assert!(link.starts_with("phantom_mesh://join?"));
        assert!(link.contains("hub="));
        assert!(link.contains("worker=rog6"));
        assert!(link.contains("token=secret-token"));
    }

    #[test]
    fn test_generate_mobile_link_no_token() {
        let link = WorkerOnboarder::generate_mobile_link(
            "http://100.0.0.1:7878",
            None,
            "iphone",
        );
        assert!(link.starts_with("phantom_mesh://join?"));
        assert!(link.contains("worker=iphone"));
        assert!(!link.contains("token="));
    }

    #[test]
    fn test_generate_python_worker_script() {
        let script = WorkerOnboarder::generate_python_worker_script(
            "http://100.0.0.1:7878",
            "acer",
            7881,
        );
        assert!(script.contains("HUB_URL = \"http://100.0.0.1:7878\""));
        assert!(script.contains("WORKER_NAME = \"acer\""));
        assert!(script.contains("WORKER_PORT = 7881"));
        assert!(script.contains("def register():"));
        assert!(script.contains("def heartbeat_loop():"));
        assert!(script.contains("class WorkerHandler"));
    }

    #[test]
    fn test_ssh_prefix_default_port() {
        let prefix = WorkerOnboarder::ssh_prefix("worker", "10.0.1.2", 22);
        assert_eq!(prefix[0], "ssh");
        assert!(prefix.contains(&"BatchMode=yes".to_string()));
        assert!(!prefix.contains(&"-p".to_string()));
        assert!(prefix.last().unwrap().contains("worker@10.0.1.2"));
    }

    #[test]
    fn test_ssh_prefix_custom_port() {
        let prefix = WorkerOnboarder::ssh_prefix("admin", "10.0.0.5", 2222);
        assert!(prefix.contains(&"-p".to_string()));
        assert!(prefix.contains(&"2222".to_string()));
        assert!(prefix.last().unwrap().contains("admin@10.0.0.5"));
    }

    #[test]
    fn test_scp_args() {
        let args = WorkerOnboarder::scp_args(
            "worker", "10.0.1.2", 22,
            "/local/worker.py", "/home/user/phantom_mesh/worker.py",
        );
        assert_eq!(args[0], "scp");
        assert!(args.contains(&"/local/worker.py".to_string()));
        assert!(args.last().unwrap().contains("worker@10.0.1.2:/home/user/phantom_mesh/worker.py"));
    }

    #[test]
    fn test_scp_args_custom_port() {
        let args = WorkerOnboarder::scp_args(
            "root", "10.0.0.1", 2222,
            "/tmp/binary", "/opt/phantom_mesh/binary",
        );
        assert!(args.contains(&"-P".to_string()));
        assert!(args.contains(&"2222".to_string()));
    }

    #[test]
    fn test_step_status_serialize() {
        let step = OnboardStep {
            name: "ssh_check".to_string(),
            status: StepStatus::Success,
            message: "Connected".to_string(),
            duration_ms: 250,
        };
        let json = serde_json::to_string(&step).unwrap();
        assert!(json.contains("\"status\":\"Success\""));
        assert!(json.contains("\"duration_ms\":250"));
    }

    #[test]
    fn test_onboard_result_serialize() {
        let result = OnboardResult {
            worker_name: "test".to_string(),
            success: true,
            steps: vec![OnboardStep {
                name: "ssh_check".to_string(),
                status: StepStatus::Success,
                message: "ok".to_string(),
                duration_ms: 100,
            }],
            mobile_link: None,
            error: None,
            duration_ms: 5000,
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"success\":true"));
        assert!(json.contains("\"worker_name\":\"test\""));
    }

    #[test]
    fn test_health_status_serialize() {
        let status = HealthStatus {
            worker_name: "w1".to_string(),
            reachable: true,
            registered: true,
            status: "online".to_string(),
            capabilities: vec!["tools".to_string()],
            device_type: "full".to_string(),
            cpu_load: 0.25,
            error: None,
        };
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("\"reachable\":true"));
        assert!(json.contains("\"cpu_load\":0.25"));
    }

    #[test]
    fn test_onboarding_status_serialize() {
        let status = OnboardingStatus {
            worker_name: "w1".to_string(),
            state: OnboardingState::InProgress,
            current_step: "ssh_check".to_string(),
            steps_completed: Vec::new(),
            started_at_unix: 1710000000,
            error: None,
        };
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("\"state\":\"InProgress\""));
        assert!(json.contains("\"current_step\":\"ssh_check\""));
    }

    #[tokio::test]
    async fn test_onboard_mobile_worker() {
        let registry = test_registry();
        let onboarder = WorkerOnboarder::new(registry.clone());

        let config = test_config_mobile();
        let result = onboarder.onboard_worker(config).await.unwrap();

        assert!(result.success);
        assert!(result.mobile_link.is_some());
        assert!(result.mobile_link.unwrap().starts_with("phantom_mesh://join?"));
        assert_eq!(result.steps.len(), 2); // generate_link + pre_register
        assert!(result.steps.iter().all(|s| s.status == StepStatus::Success));
        assert!(result.error.is_none());

        // Verify worker was pre-registered
        let node = registry.get_node("test-phone").await;
        assert!(node.is_some());
        assert_eq!(node.unwrap().device_type, "mobile");
    }

    #[tokio::test]
    async fn test_verify_worker_not_found() {
        let registry = test_registry();
        let onboarder = WorkerOnboarder::new(registry);

        let health = onboarder.verify_worker("nonexistent").await.unwrap();
        assert!(!health.reachable);
        assert!(!health.registered);
        assert_eq!(health.status, "unknown");
        assert!(health.error.is_some());
    }

    #[tokio::test]
    async fn test_verify_worker_registered() {
        let registry = test_registry();
        registry
            .register_full("w1", "127.0.0.1", 9999, &["tools".to_string()], "full")
            .await
            .unwrap();

        let onboarder = WorkerOnboarder::new(registry);
        let health = onboarder.verify_worker("w1").await.unwrap();
        assert!(health.registered);
        assert_eq!(health.status, "online");
        assert_eq!(health.device_type, "full");
        // Note: reachable will be false because there's no actual server at 127.0.0.1:9999
        // but registered should be true
    }

    #[tokio::test]
    async fn test_get_status_not_found() {
        let registry = test_registry();
        let onboarder = WorkerOnboarder::new(registry);

        let status = onboarder.get_status("nonexistent").await;
        assert!(status.is_none());
    }

    #[tokio::test]
    async fn test_get_status_after_mobile_onboard() {
        let registry = test_registry();
        let onboarder = WorkerOnboarder::new(registry);

        let config = test_config_mobile();
        let _ = onboarder.onboard_worker(config).await.unwrap();

        let status = onboarder.get_status("test-phone").await;
        assert!(status.is_some());
        let s = status.unwrap();
        assert_eq!(s.state, OnboardingState::Completed);
        assert_eq!(s.current_step, "done");
        assert!(s.error.is_none());
    }

    #[tokio::test]
    async fn test_onboard_pc_missing_ssh_host() {
        let registry = test_registry();
        let onboarder = WorkerOnboarder::new(registry);

        let mut config = test_config_pc();
        config.ssh_host = None;

        let result = onboarder.onboard_worker(config).await.unwrap();
        assert!(!result.success);
        assert!(result.error.is_some());
        assert!(result.error.unwrap().contains("ssh_host"));
    }

    #[tokio::test]
    async fn test_onboard_pc_missing_ssh_user() {
        let registry = test_registry();
        let onboarder = WorkerOnboarder::new(registry);

        let mut config = test_config_pc();
        config.ssh_user = None;

        let result = onboarder.onboard_worker(config).await.unwrap();
        assert!(!result.success);
        assert!(result.error.is_some());
        assert!(result.error.unwrap().contains("ssh_user"));
    }

    #[test]
    fn test_mobile_link_url_encoding() {
        let link = WorkerOnboarder::generate_mobile_link(
            "http://192.168.1.1:7878",
            Some("token with spaces"),
            "my worker",
        );
        // Should be URL-encoded
        assert!(link.contains("token%20with%20spaces"));
        assert!(link.contains("my%20worker"));
    }

    #[test]
    fn test_onboard_config_mobile_type() {
        let json_str = r#"{
            "worker_name": "rog6",
            "worker_type": "mobile",
            "hub_url": "http://100.0.0.1:7878",
            "auth_token": "abc123"
        }"#;
        let config: OnboardConfig = serde_json::from_str(json_str).unwrap();
        assert_eq!(config.worker_type, "mobile");
        assert_eq!(config.worker_name, "rog6");
        assert!(config.ssh_host.is_none()); // not needed for mobile
    }

    #[test]
    fn test_step_status_equality() {
        assert_eq!(StepStatus::Success, StepStatus::Success);
        assert_ne!(StepStatus::Success, StepStatus::Failed);
        assert_ne!(StepStatus::Failed, StepStatus::Skipped);
    }

    #[test]
    fn test_onboarding_state_equality() {
        assert_eq!(OnboardingState::InProgress, OnboardingState::InProgress);
        assert_ne!(OnboardingState::InProgress, OnboardingState::Completed);
        assert_ne!(OnboardingState::Completed, OnboardingState::Failed);
    }
}
