//! Worker Installer — one-command worker setup for all platforms.
//!
//! Provides a CLI-driven installer flow that:
//! 1. Validates configuration (hub reachability, port availability, etc.)
//! 2. Generates a TOML config file for the worker
//! 3. Generates platform-specific service files (systemd, Windows, launchd)
//!
//! Usage:
//!   `clawtex-core install-worker --hub http://100.x.x.x:7878 --name my-worker --port 7879`

use serde::{Deserialize, Serialize};
use std::net::TcpListener;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Installer configuration — everything needed to set up a new worker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallerConfig {
    /// Hub URL to connect to (e.g., "http://10.0.2.1:7878").
    pub hub_url: String,
    /// Unique worker name (e.g., "acer", "m1-mac").
    pub worker_name: String,
    /// Bearer token for hub authentication.
    #[serde(default)]
    pub auth_token: Option<String>,
    /// Port for the worker to listen on.
    #[serde(default = "default_port")]
    pub port: u16,
    /// Capabilities to advertise to the hub.
    #[serde(default = "default_capabilities")]
    pub capabilities: Vec<String>,
    /// Device type: "full", "light", or "mobile".
    #[serde(default = "default_device_type")]
    pub device_type: String,
    /// Install directory for the worker binary and config.
    #[serde(default = "default_install_dir")]
    pub install_dir: String,
    /// Path to the worker binary (auto-detected if not set).
    #[serde(default)]
    pub binary_path: Option<String>,
}

fn default_port() -> u16 {
    7879
}

fn default_capabilities() -> Vec<String> {
    vec!["tools".to_string()]
}

fn default_device_type() -> String {
    "full".to_string()
}

fn default_install_dir() -> String {
    "/opt/clawtex-worker".to_string()
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Validation issue found during config check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationIssue {
    pub field: String,
    pub message: String,
    pub severity: IssueSeverity,
}

/// How severe a validation issue is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IssueSeverity {
    Error,
    Warning,
}

/// Validate an installer config — returns a list of issues (empty = all good).
///
/// Checks:
/// - hub_url is non-empty and looks like a URL
/// - worker_name is non-empty and alphanumeric (plus hyphens/underscores)
/// - port is not 0
/// - port is available (not already bound)
/// - device_type is one of the known types
/// - capabilities is non-empty
pub fn validate_config(config: &InstallerConfig) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    // Hub URL checks
    if config.hub_url.is_empty() {
        issues.push(ValidationIssue {
            field: "hub_url".to_string(),
            message: "Hub URL is required".to_string(),
            severity: IssueSeverity::Error,
        });
    } else if !config.hub_url.starts_with("http://") && !config.hub_url.starts_with("https://") {
        issues.push(ValidationIssue {
            field: "hub_url".to_string(),
            message: "Hub URL must start with http:// or https://".to_string(),
            severity: IssueSeverity::Error,
        });
    }

    // Worker name checks
    if config.worker_name.is_empty() {
        issues.push(ValidationIssue {
            field: "worker_name".to_string(),
            message: "Worker name is required".to_string(),
            severity: IssueSeverity::Error,
        });
    } else if !config
        .worker_name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        issues.push(ValidationIssue {
            field: "worker_name".to_string(),
            message: "Worker name must be alphanumeric (hyphens and underscores allowed)".to_string(),
            severity: IssueSeverity::Error,
        });
    }

    // Port checks
    if config.port == 0 {
        issues.push(ValidationIssue {
            field: "port".to_string(),
            message: "Port must be greater than 0".to_string(),
            severity: IssueSeverity::Error,
        });
    } else if !is_port_available(config.port) {
        issues.push(ValidationIssue {
            field: "port".to_string(),
            message: format!("Port {} is already in use", config.port),
            severity: IssueSeverity::Error,
        });
    }

    // Device type checks
    let valid_types = ["full", "light", "mobile", "npu"];
    if !valid_types.contains(&config.device_type.as_str()) {
        issues.push(ValidationIssue {
            field: "device_type".to_string(),
            message: format!(
                "Unknown device type '{}'. Expected one of: {}",
                config.device_type,
                valid_types.join(", ")
            ),
            severity: IssueSeverity::Warning,
        });
    }

    // Capabilities checks
    if config.capabilities.is_empty() {
        issues.push(ValidationIssue {
            field: "capabilities".to_string(),
            message: "At least one capability should be specified".to_string(),
            severity: IssueSeverity::Warning,
        });
    }

    issues
}

/// Check if a port is available for binding.
fn is_port_available(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_ok()
}

// ---------------------------------------------------------------------------
// Config file generation
// ---------------------------------------------------------------------------

/// Generate a TOML configuration file for the worker.
pub fn generate_config_file(config: &InstallerConfig) -> String {
    let mut out = String::new();
    out.push_str("# Clawtex Worker Configuration\n");
    out.push_str("# Generated by clawtex-core installer\n\n");
    out.push_str("[worker]\n");
    out.push_str(&format!("hub_url = \"{}\"\n", config.hub_url));
    out.push_str(&format!("node_name = \"{}\"\n", config.worker_name));
    out.push_str(&format!("port = {}\n", config.port));
    out.push_str(&format!("device_type = \"{}\"\n", config.device_type));

    if let Some(ref token) = config.auth_token {
        out.push_str(&format!("auth_token = \"{}\"\n", token));
    }

    // Capabilities as TOML array
    let caps: Vec<String> = config
        .capabilities
        .iter()
        .map(|c| format!("\"{}\"", c))
        .collect();
    out.push_str(&format!("capabilities = [{}]\n", caps.join(", ")));

    out.push_str("\n[worker.heartbeat]\n");
    out.push_str("interval_secs = 30\n");
    out.push_str("timeout_secs = 10\n");

    out.push_str("\n[worker.resources]\n");
    out.push_str("max_concurrent_tasks = 4\n");
    out.push_str("cpu_report_interval_secs = 10\n");

    out
}

// ---------------------------------------------------------------------------
// Platform service generators
// ---------------------------------------------------------------------------

/// Generate a systemd unit file for Linux.
pub fn generate_systemd_unit(config: &InstallerConfig) -> String {
    let binary = config
        .binary_path
        .clone()
        .unwrap_or_else(|| format!("{}/clawtex-core", config.install_dir));

    let mut unit = String::new();
    unit.push_str("[Unit]\n");
    unit.push_str(&format!(
        "Description=Clawtex Worker ({})\n",
        config.worker_name
    ));
    unit.push_str("After=network-online.target\n");
    unit.push_str("Wants=network-online.target\n\n");

    unit.push_str("[Service]\n");
    unit.push_str("Type=simple\n");
    unit.push_str(&format!(
        "ExecStart={binary} worker --hub {} --name {} --port {}\n",
        config.hub_url, config.worker_name, config.port
    ));
    unit.push_str("Restart=always\n");
    unit.push_str("RestartSec=5\n");
    unit.push_str(&format!("WorkingDirectory={}\n", config.install_dir));
    unit.push_str(&format!(
        "Environment=CLAWTEX_CONFIG={}/worker.toml\n",
        config.install_dir
    ));

    if let Some(ref token) = config.auth_token {
        unit.push_str(&format!("Environment=CLAWTEX_AUTH_TOKEN={}\n", token));
    }

    unit.push_str("StandardOutput=journal\n");
    unit.push_str("StandardError=journal\n\n");

    unit.push_str("[Install]\n");
    unit.push_str("WantedBy=multi-user.target\n");

    unit
}

/// Generate a Windows service install script (PowerShell).
pub fn generate_windows_service(config: &InstallerConfig) -> String {
    let binary = config
        .binary_path
        .clone()
        .unwrap_or_else(|| format!("{}\\clawtex-core.exe", config.install_dir));

    let mut script = String::new();
    script.push_str("# Clawtex Worker — Windows Service Install Script\n");
    script.push_str(&format!(
        "# Worker: {} — Hub: {}\n\n",
        config.worker_name, config.hub_url
    ));

    script.push_str(&format!(
        "$ServiceName = \"ClawtexWorker_{}\"\n",
        config.worker_name
    ));
    script.push_str(&format!(
        "$DisplayName = \"Clawtex Worker ({})\"\n",
        config.worker_name
    ));
    script.push_str(&format!("$BinaryPath = \"{}\"\n", binary));
    script.push_str(&format!(
        "$Arguments = \"worker --hub {} --name {} --port {}\"\n\n",
        config.hub_url, config.worker_name, config.port
    ));

    script.push_str("# Create the service using sc.exe\n");
    script.push_str(
        "sc.exe create $ServiceName binPath= \"$BinaryPath $Arguments\" start= auto DisplayName= $DisplayName\n",
    );
    script.push_str("sc.exe description $ServiceName \"Clawtex distributed worker node\"\n");
    script.push_str("sc.exe start $ServiceName\n\n");

    script.push_str("Write-Host \"Service $ServiceName installed and started.\"\n");
    script.push_str(
        "Write-Host \"To check status: sc.exe query $ServiceName\"\n",
    );
    script.push_str(
        "Write-Host \"To stop: sc.exe stop $ServiceName\"\n",
    );
    script.push_str(
        "Write-Host \"To remove: sc.exe delete $ServiceName\"\n",
    );

    script
}

/// Generate a macOS LaunchAgent plist file.
pub fn generate_launchd_plist(config: &InstallerConfig) -> String {
    let binary = config
        .binary_path
        .clone()
        .unwrap_or_else(|| format!("{}/clawtex-core", config.install_dir));

    let label = format!("com.clawtex.worker.{}", config.worker_name);

    let mut plist = String::new();
    plist.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    plist.push_str(
        "<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n",
    );
    plist.push_str("<plist version=\"1.0\">\n");
    plist.push_str("<dict>\n");

    plist.push_str("    <key>Label</key>\n");
    plist.push_str(&format!("    <string>{}</string>\n", label));

    plist.push_str("    <key>ProgramArguments</key>\n");
    plist.push_str("    <array>\n");
    plist.push_str(&format!("        <string>{}</string>\n", binary));
    plist.push_str("        <string>worker</string>\n");
    plist.push_str("        <string>--hub</string>\n");
    plist.push_str(&format!(
        "        <string>{}</string>\n",
        config.hub_url
    ));
    plist.push_str("        <string>--name</string>\n");
    plist.push_str(&format!(
        "        <string>{}</string>\n",
        config.worker_name
    ));
    plist.push_str("        <string>--port</string>\n");
    plist.push_str(&format!(
        "        <string>{}</string>\n",
        config.port
    ));
    plist.push_str("    </array>\n");

    plist.push_str("    <key>RunAtLoad</key>\n");
    plist.push_str("    <true/>\n");

    plist.push_str("    <key>KeepAlive</key>\n");
    plist.push_str("    <true/>\n");

    plist.push_str("    <key>WorkingDirectory</key>\n");
    plist.push_str(&format!(
        "    <string>{}</string>\n",
        config.install_dir
    ));

    plist.push_str("    <key>StandardOutPath</key>\n");
    plist.push_str(&format!(
        "    <string>{}/clawtex-worker.log</string>\n",
        config.install_dir
    ));

    plist.push_str("    <key>StandardErrorPath</key>\n");
    plist.push_str(&format!(
        "    <string>{}/clawtex-worker.err</string>\n",
        config.install_dir
    ));

    if let Some(ref token) = config.auth_token {
        plist.push_str("    <key>EnvironmentVariables</key>\n");
        plist.push_str("    <dict>\n");
        plist.push_str("        <key>CLAWTEX_AUTH_TOKEN</key>\n");
        plist.push_str(&format!("        <string>{}</string>\n", token));
        plist.push_str("    </dict>\n");
    }

    plist.push_str("</dict>\n");
    plist.push_str("</plist>\n");

    plist
}

// ---------------------------------------------------------------------------
// Install summary
// ---------------------------------------------------------------------------

/// Summary of what the installer generated / would do.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallSummary {
    pub worker_name: String,
    pub hub_url: String,
    pub port: u16,
    pub config_path: String,
    pub service_file_path: String,
    pub platform: String,
    pub instructions: Vec<String>,
}

/// Determine the current platform and return an install summary with instructions.
pub fn generate_install_summary(config: &InstallerConfig) -> InstallSummary {
    let platform = detect_platform();
    let (service_file_path, instructions) = match platform.as_str() {
        "linux" => {
            let svc_path = format!(
                "/etc/systemd/system/clawtex-worker-{}.service",
                config.worker_name
            );
            let instructions = vec![
                format!(
                    "1. Copy config to {}/worker.toml",
                    config.install_dir
                ),
                format!("2. Copy service file to {}", svc_path),
                "3. Run: sudo systemctl daemon-reload".to_string(),
                format!(
                    "4. Run: sudo systemctl enable --now clawtex-worker-{}",
                    config.worker_name
                ),
            ];
            (svc_path, instructions)
        }
        "macos" => {
            let plist_path = format!(
                "~/Library/LaunchAgents/com.clawtex.worker.{}.plist",
                config.worker_name
            );
            let instructions = vec![
                format!(
                    "1. Copy config to {}/worker.toml",
                    config.install_dir
                ),
                format!("2. Copy plist to {}", plist_path),
                format!("3. Run: launchctl load {}", plist_path),
            ];
            (plist_path, instructions)
        }
        _ => {
            // windows
            let instructions = vec![
                format!(
                    "1. Copy config to {}\\worker.toml",
                    config.install_dir
                ),
                "2. Open PowerShell as Administrator".to_string(),
                "3. Run the generated install script".to_string(),
            ];
            ("(PowerShell script)".to_string(), instructions)
        }
    };

    InstallSummary {
        worker_name: config.worker_name.clone(),
        hub_url: config.hub_url.clone(),
        port: config.port,
        config_path: format!("{}/worker.toml", config.install_dir),
        service_file_path,
        platform,
        instructions,
    }
}

/// Detect the current platform.
fn detect_platform() -> String {
    if cfg!(target_os = "linux") {
        "linux".to_string()
    } else if cfg!(target_os = "macos") {
        "macos".to_string()
    } else {
        "windows".to_string()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> InstallerConfig {
        InstallerConfig {
            hub_url: "http://10.0.2.1:7878".to_string(),
            worker_name: "test-worker".to_string(),
            auth_token: Some("clawtex-hub-2026".to_string()),
            port: 0, // will use 0 for most tests (validation tests use specific ports)
            capabilities: vec!["tools".to_string(), "web_search".to_string()],
            device_type: "full".to_string(),
            install_dir: "/opt/clawtex-worker".to_string(),
            binary_path: None,
        }
    }

    // -- InstallerConfig serde tests --

    #[test]
    fn test_config_defaults() {
        let json = r#"{
            "hub_url": "http://localhost:7878",
            "worker_name": "w1"
        }"#;
        let config: InstallerConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.port, 7879);
        assert_eq!(config.device_type, "full");
        assert_eq!(config.capabilities, vec!["tools"]);
        assert_eq!(config.install_dir, "/opt/clawtex-worker");
        assert!(config.auth_token.is_none());
        assert!(config.binary_path.is_none());
    }

    #[test]
    fn test_config_full_deserialize() {
        let json = r#"{
            "hub_url": "http://10.0.2.1:7878",
            "worker_name": "acer",
            "auth_token": "secret-token",
            "port": 7881,
            "capabilities": ["tools", "web_search", "shell"],
            "device_type": "light",
            "install_dir": "/home/user/clawtex",
            "binary_path": "/usr/local/bin/clawtex-core"
        }"#;
        let config: InstallerConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.hub_url, "http://10.0.2.1:7878");
        assert_eq!(config.worker_name, "acer");
        assert_eq!(config.auth_token.unwrap(), "secret-token");
        assert_eq!(config.port, 7881);
        assert_eq!(config.capabilities.len(), 3);
        assert_eq!(config.device_type, "light");
        assert_eq!(config.binary_path.unwrap(), "/usr/local/bin/clawtex-core");
    }

    #[test]
    fn test_config_serialize_roundtrip() {
        let config = test_config();
        let json = serde_json::to_string(&config).unwrap();
        let parsed: InstallerConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.hub_url, config.hub_url);
        assert_eq!(parsed.worker_name, config.worker_name);
        assert_eq!(parsed.capabilities, config.capabilities);
    }

    // -- Validation tests --

    #[test]
    fn test_validate_empty_hub_url() {
        let mut config = test_config();
        config.hub_url = "".to_string();
        let issues = validate_config(&config);
        assert!(issues.iter().any(|i| i.field == "hub_url" && i.severity == IssueSeverity::Error));
    }

    #[test]
    fn test_validate_bad_hub_url_scheme() {
        let mut config = test_config();
        config.hub_url = "ftp://example.com".to_string();
        let issues = validate_config(&config);
        assert!(issues
            .iter()
            .any(|i| i.field == "hub_url" && i.message.contains("http://")));
    }

    #[test]
    fn test_validate_empty_worker_name() {
        let mut config = test_config();
        config.worker_name = "".to_string();
        let issues = validate_config(&config);
        assert!(issues
            .iter()
            .any(|i| i.field == "worker_name" && i.severity == IssueSeverity::Error));
    }

    #[test]
    fn test_validate_invalid_worker_name() {
        let mut config = test_config();
        config.worker_name = "bad name!".to_string();
        let issues = validate_config(&config);
        assert!(issues
            .iter()
            .any(|i| i.field == "worker_name" && i.message.contains("alphanumeric")));
    }

    #[test]
    fn test_validate_valid_worker_name_with_hyphens() {
        let mut config = test_config();
        config.worker_name = "m1-mac_2".to_string();
        let issues = validate_config(&config);
        assert!(!issues.iter().any(|i| i.field == "worker_name"));
    }

    #[test]
    fn test_validate_port_zero() {
        let mut config = test_config();
        config.port = 0;
        let issues = validate_config(&config);
        assert!(issues
            .iter()
            .any(|i| i.field == "port" && i.severity == IssueSeverity::Error));
    }

    #[test]
    fn test_validate_unknown_device_type() {
        let mut config = test_config();
        config.device_type = "quantum".to_string();
        let issues = validate_config(&config);
        assert!(issues
            .iter()
            .any(|i| i.field == "device_type" && i.severity == IssueSeverity::Warning));
    }

    #[test]
    fn test_validate_known_device_types() {
        for dt in &["full", "light", "mobile", "npu"] {
            let mut config = test_config();
            config.device_type = dt.to_string();
            config.port = 1; // avoid port-zero error; port 1 is privileged so likely unavailable
            let issues = validate_config(&config);
            assert!(!issues.iter().any(|i| i.field == "device_type"),
                "device_type '{}' should be valid", dt);
        }
    }

    #[test]
    fn test_validate_empty_capabilities_warning() {
        let mut config = test_config();
        config.capabilities = Vec::new();
        let issues = validate_config(&config);
        assert!(issues
            .iter()
            .any(|i| i.field == "capabilities" && i.severity == IssueSeverity::Warning));
    }

    // -- Config file generation tests --

    #[test]
    fn test_generate_config_file_basic() {
        let config = test_config();
        let toml = generate_config_file(&config);
        assert!(toml.contains("[worker]"));
        assert!(toml.contains(&format!("hub_url = \"{}\"", config.hub_url)));
        assert!(toml.contains(&format!("node_name = \"{}\"", config.worker_name)));
        assert!(toml.contains(&format!("port = {}", config.port)));
        assert!(toml.contains(&format!("device_type = \"{}\"", config.device_type)));
        assert!(toml.contains("auth_token = \"clawtex-hub-2026\""));
        assert!(toml.contains("[worker.heartbeat]"));
        assert!(toml.contains("[worker.resources]"));
    }

    #[test]
    fn test_generate_config_file_no_token() {
        let mut config = test_config();
        config.auth_token = None;
        let toml = generate_config_file(&config);
        assert!(!toml.contains("auth_token"));
    }

    #[test]
    fn test_generate_config_file_capabilities() {
        let config = test_config();
        let toml = generate_config_file(&config);
        assert!(toml.contains("capabilities = [\"tools\", \"web_search\"]"));
    }

    // -- systemd unit tests --

    #[test]
    fn test_generate_systemd_unit() {
        let config = test_config();
        let unit = generate_systemd_unit(&config);
        assert!(unit.contains("[Unit]"));
        assert!(unit.contains("[Service]"));
        assert!(unit.contains("[Install]"));
        assert!(unit.contains(&format!("Description=Clawtex Worker ({})", config.worker_name)));
        assert!(unit.contains("Restart=always"));
        assert!(unit.contains("WantedBy=multi-user.target"));
        assert!(unit.contains(&format!("--hub {}", config.hub_url)));
        assert!(unit.contains(&format!("--name {}", config.worker_name)));
        assert!(unit.contains(&format!("--port {}", config.port)));
        assert!(unit.contains("CLAWTEX_AUTH_TOKEN=clawtex-hub-2026"));
    }

    #[test]
    fn test_generate_systemd_unit_custom_binary() {
        let mut config = test_config();
        config.binary_path = Some("/usr/local/bin/clawtex".to_string());
        let unit = generate_systemd_unit(&config);
        assert!(unit.contains("ExecStart=/usr/local/bin/clawtex worker"));
    }

    #[test]
    fn test_generate_systemd_unit_no_token() {
        let mut config = test_config();
        config.auth_token = None;
        let unit = generate_systemd_unit(&config);
        assert!(!unit.contains("CLAWTEX_AUTH_TOKEN"));
    }

    // -- Windows service tests --

    #[test]
    fn test_generate_windows_service() {
        let config = test_config();
        let script = generate_windows_service(&config);
        assert!(script.contains(&format!("$ServiceName = \"ClawtexWorker_{}\"", config.worker_name)));
        assert!(script.contains("sc.exe create"));
        assert!(script.contains("sc.exe start"));
        assert!(script.contains(&config.hub_url));
        assert!(script.contains(&config.worker_name));
    }

    #[test]
    fn test_generate_windows_service_custom_binary() {
        let mut config = test_config();
        config.binary_path = Some("C:\\clawtex\\clawtex-core.exe".to_string());
        let script = generate_windows_service(&config);
        assert!(script.contains("C:\\clawtex\\clawtex-core.exe"));
    }

    // -- launchd plist tests --

    #[test]
    fn test_generate_launchd_plist() {
        let config = test_config();
        let plist = generate_launchd_plist(&config);
        assert!(plist.contains("<?xml version=\"1.0\""));
        assert!(plist.contains("<plist version=\"1.0\">"));
        assert!(plist.contains(&format!(
            "<string>com.clawtex.worker.{}</string>",
            config.worker_name
        )));
        assert!(plist.contains("<key>RunAtLoad</key>"));
        assert!(plist.contains("<key>KeepAlive</key>"));
        assert!(plist.contains(&config.hub_url));
        assert!(plist.contains(&config.worker_name));
        assert!(plist.contains("<key>CLAWTEX_AUTH_TOKEN</key>"));
    }

    #[test]
    fn test_generate_launchd_plist_no_token() {
        let mut config = test_config();
        config.auth_token = None;
        let plist = generate_launchd_plist(&config);
        assert!(!plist.contains("CLAWTEX_AUTH_TOKEN"));
        assert!(!plist.contains("<key>EnvironmentVariables</key>"));
    }

    #[test]
    fn test_generate_launchd_plist_program_arguments() {
        let config = test_config();
        let plist = generate_launchd_plist(&config);
        assert!(plist.contains("<key>ProgramArguments</key>"));
        assert!(plist.contains("<string>worker</string>"));
        assert!(plist.contains("<string>--hub</string>"));
        assert!(plist.contains("<string>--name</string>"));
        assert!(plist.contains("<string>--port</string>"));
    }

    // -- Install summary tests --

    #[test]
    fn test_generate_install_summary() {
        let config = test_config();
        let summary = generate_install_summary(&config);
        assert_eq!(summary.worker_name, config.worker_name);
        assert_eq!(summary.hub_url, config.hub_url);
        assert_eq!(summary.port, config.port);
        assert!(!summary.instructions.is_empty());
        assert!(!summary.platform.is_empty());
    }

    #[test]
    fn test_install_summary_serialize() {
        let config = test_config();
        let summary = generate_install_summary(&config);
        let json = serde_json::to_string(&summary).unwrap();
        assert!(json.contains("\"worker_name\":\"test-worker\""));
        assert!(json.contains("\"platform\""));
    }

    // -- Severity equality --

    #[test]
    fn test_issue_severity_equality() {
        assert_eq!(IssueSeverity::Error, IssueSeverity::Error);
        assert_eq!(IssueSeverity::Warning, IssueSeverity::Warning);
        assert_ne!(IssueSeverity::Error, IssueSeverity::Warning);
    }

    // -- Validation issue serialize --

    #[test]
    fn test_validation_issue_serialize() {
        let issue = ValidationIssue {
            field: "port".to_string(),
            message: "Port is in use".to_string(),
            severity: IssueSeverity::Error,
        };
        let json = serde_json::to_string(&issue).unwrap();
        assert!(json.contains("\"field\":\"port\""));
        assert!(json.contains("\"severity\":\"Error\""));
    }
}
