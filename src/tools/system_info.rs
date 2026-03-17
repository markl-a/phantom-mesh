//! System information tool — retrieve OS, CPU, memory, and disk info.
//! Uses platform-specific commands: wmic/systeminfo/PowerShell (Windows),
//! sysctl/vm_stat/df (macOS), /proc/* or free/lscpu/df (Linux).

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Duration;

use super::{Tool, ToolResult};

pub struct SystemInfoTool;

impl SystemInfoTool {
    pub fn new() -> Self {
        Self
    }

    /// Run a command and return its stdout (trimmed). Returns empty string on error.
    async fn run_cmd(program: &str, args: &[&str]) -> String {
        match tokio::time::timeout(
            Duration::from_secs(15),
            tokio::process::Command::new(program)
                .args(args)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .output(),
        )
        .await
        {
            Ok(Ok(output)) if output.status.success() => {
                String::from_utf8_lossy(&output.stdout).trim().to_string()
            }
            _ => String::new(),
        }
    }

    /// Gather overview: OS name, version, hostname, uptime.
    async fn gather_overview() -> Value {
        #[cfg(target_os = "windows")]
        {
            let os_name = Self::run_cmd(
                "powershell",
                &["-NoProfile", "-Command", "(Get-CimInstance Win32_OperatingSystem).Caption"],
            )
            .await;
            let version = Self::run_cmd(
                "powershell",
                &["-NoProfile", "-Command", "(Get-CimInstance Win32_OperatingSystem).Version"],
            )
            .await;
            let hostname = Self::run_cmd("hostname", &[]).await;
            let uptime = Self::run_cmd(
                "powershell",
                &[
                    "-NoProfile",
                    "-Command",
                    "(Get-Date) - (gcim Win32_OperatingSystem).LastBootUpTime | Select-Object -ExpandProperty TotalHours",
                ],
            )
            .await;
            let architecture = Self::run_cmd(
                "powershell",
                &["-NoProfile", "-Command", "[System.Environment]::Is64BitOperatingSystem"],
            )
            .await;

            json!({
                "os": if os_name.is_empty() { "Windows".to_string() } else { os_name },
                "version": version,
                "hostname": hostname,
                "uptime_hours": uptime,
                "architecture": if architecture == "True" { "x64" } else { "x86" },
            })
        }

        #[cfg(target_os = "macos")]
        {
            let os_name = Self::run_cmd("sw_vers", &["-productName"]).await;
            let version = Self::run_cmd("sw_vers", &["-productVersion"]).await;
            let hostname = Self::run_cmd("hostname", &[]).await;
            let uptime = Self::run_cmd("uptime", &[]).await;
            let arch = Self::run_cmd("uname", &["-m"]).await;
            json!({
                "os": os_name,
                "version": version,
                "hostname": hostname,
                "uptime": uptime,
                "architecture": arch,
            })
        }

        #[cfg(target_os = "linux")]
        {
            let os_name = Self::run_cmd("uname", &["-s"]).await;
            let version = Self::run_cmd("uname", &["-r"]).await;
            let hostname = Self::run_cmd("hostname", &[]).await;
            let uptime = Self::run_cmd("uptime", &["-p"]).await;
            let arch = Self::run_cmd("uname", &["-m"]).await;
            // Try to get distro info
            let distro = Self::run_cmd("sh", &["-c", "cat /etc/os-release 2>/dev/null | grep PRETTY_NAME | cut -d= -f2 | tr -d '\"'"]).await;
            json!({
                "os": if distro.is_empty() { os_name } else { distro },
                "kernel": version,
                "hostname": hostname,
                "uptime": uptime,
                "architecture": arch,
            })
        }

        #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
        {
            json!({ "os": std::env::consts::OS, "note": "Detailed info not supported on this platform" })
        }
    }

    /// Gather CPU information.
    async fn gather_cpu() -> Value {
        #[cfg(target_os = "windows")]
        {
            let name = Self::run_cmd(
                "powershell",
                &["-NoProfile", "-Command", "(Get-CimInstance Win32_Processor).Name | Select-Object -First 1"],
            )
            .await;
            let cores = Self::run_cmd(
                "powershell",
                &["-NoProfile", "-Command", "(Get-CimInstance Win32_Processor).NumberOfCores | Measure-Object -Sum | Select-Object -ExpandProperty Sum"],
            )
            .await;
            let logical = Self::run_cmd(
                "powershell",
                &["-NoProfile", "-Command", "(Get-CimInstance Win32_Processor).NumberOfLogicalProcessors | Measure-Object -Sum | Select-Object -ExpandProperty Sum"],
            )
            .await;
            let load = Self::run_cmd(
                "powershell",
                &["-NoProfile", "-Command", "(Get-CimInstance Win32_Processor).LoadPercentage | Measure-Object -Average | Select-Object -ExpandProperty Average"],
            )
            .await;
            json!({
                "model": name,
                "physical_cores": cores,
                "logical_cores": logical,
                "load_percent": load,
            })
        }

        #[cfg(target_os = "macos")]
        {
            let model = Self::run_cmd("sysctl", &["-n", "machdep.cpu.brand_string"]).await;
            let cores = Self::run_cmd("sysctl", &["-n", "hw.physicalcpu"]).await;
            let logical = Self::run_cmd("sysctl", &["-n", "hw.logicalcpu"]).await;
            let load = Self::run_cmd("sh", &["-c", "sysctl -n vm.loadavg | awk '{print $2}'"]).await;
            json!({
                "model": model,
                "physical_cores": cores,
                "logical_cores": logical,
                "load_1min": load,
            })
        }

        #[cfg(target_os = "linux")]
        {
            let model = Self::run_cmd("sh", &["-c", "grep 'model name' /proc/cpuinfo | head -1 | cut -d: -f2 | xargs"]).await;
            let cores = Self::run_cmd("sh", &["-c", "grep -c ^processor /proc/cpuinfo"]).await;
            let load = Self::run_cmd("sh", &["-c", "cat /proc/loadavg | awk '{print $1}'"]).await;
            json!({
                "model": model,
                "logical_cores": cores,
                "load_1min": load,
            })
        }

        #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
        {
            json!({ "note": "CPU info not supported on this platform" })
        }
    }

    /// Gather memory information.
    async fn gather_memory() -> Value {
        #[cfg(target_os = "windows")]
        {
            let total = Self::run_cmd(
                "powershell",
                &["-NoProfile", "-Command", "[math]::Round((Get-CimInstance Win32_OperatingSystem).TotalVisibleMemorySize / 1MB, 2)"],
            )
            .await;
            let free = Self::run_cmd(
                "powershell",
                &["-NoProfile", "-Command", "[math]::Round((Get-CimInstance Win32_OperatingSystem).FreePhysicalMemory / 1MB, 2)"],
            )
            .await;
            let total_gb: f64 = total.parse().unwrap_or(0.0);
            let free_gb: f64 = free.parse().unwrap_or(0.0);
            let used_gb = total_gb - free_gb;
            let used_pct = if total_gb > 0.0 { (used_gb / total_gb * 100.0).round() } else { 0.0 };
            json!({
                "total_gb": total_gb,
                "free_gb": free_gb,
                "used_gb": (used_gb * 100.0).round() / 100.0,
                "used_percent": used_pct,
            })
        }

        #[cfg(target_os = "macos")]
        {
            let total_bytes = Self::run_cmd("sysctl", &["-n", "hw.memsize"]).await;
            let total_gb: f64 = total_bytes.parse::<f64>().unwrap_or(0.0) / (1024.0 * 1024.0 * 1024.0);
            let vm_stat = Self::run_cmd("vm_stat", &[]).await;
            // Parse free pages from vm_stat
            let free_pages = vm_stat.lines()
                .find(|l| l.contains("Pages free"))
                .and_then(|l| l.split(':').nth(1))
                .and_then(|s| s.trim().trim_end_matches('.').parse::<f64>().ok())
                .unwrap_or(0.0);
            let page_size = 4096.0_f64;
            let free_gb = free_pages * page_size / (1024.0 * 1024.0 * 1024.0);
            let used_gb = total_gb - free_gb;
            json!({
                "total_gb": (total_gb * 100.0).round() / 100.0,
                "free_gb": (free_gb * 100.0).round() / 100.0,
                "used_gb": (used_gb * 100.0).round() / 100.0,
            })
        }

        #[cfg(target_os = "linux")]
        {
            let meminfo = Self::run_cmd("sh", &["-c", "cat /proc/meminfo"]).await;
            let get_kb = |label: &str| -> f64 {
                meminfo.lines()
                    .find(|l| l.starts_with(label))
                    .and_then(|l| l.split_whitespace().nth(1))
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(0.0)
            };
            let total_kb = get_kb("MemTotal:");
            let avail_kb = get_kb("MemAvailable:");
            let used_kb = total_kb - avail_kb;
            let total_gb = total_kb / (1024.0 * 1024.0);
            let avail_gb = avail_kb / (1024.0 * 1024.0);
            let used_gb = used_kb / (1024.0 * 1024.0);
            let used_pct = if total_kb > 0.0 { (used_kb / total_kb * 100.0).round() } else { 0.0 };
            json!({
                "total_gb": (total_gb * 100.0).round() / 100.0,
                "available_gb": (avail_gb * 100.0).round() / 100.0,
                "used_gb": (used_gb * 100.0).round() / 100.0,
                "used_percent": used_pct,
            })
        }

        #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
        {
            json!({ "note": "Memory info not supported on this platform" })
        }
    }

    /// Gather disk information.
    async fn gather_disk() -> Value {
        #[cfg(target_os = "windows")]
        {
            let output = Self::run_cmd(
                "powershell",
                &[
                    "-NoProfile",
                    "-Command",
                    "Get-PSDrive -PSProvider FileSystem | Select-Object Name,@{N='Used_GB';E={[math]::Round($_.Used/1GB,2)}},@{N='Free_GB';E={[math]::Round($_.Free/1GB,2)}} | ConvertTo-Json",
                ],
            )
            .await;
            let drives: Value = serde_json::from_str(&output).unwrap_or(json!([]));
            json!({ "drives": drives })
        }

        #[cfg(target_os = "macos")]
        {
            let output = Self::run_cmd("df", &["-h", "-l"]).await;
            let lines: Vec<Value> = output.lines().skip(1).filter_map(|line| {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 6 {
                    Some(json!({
                        "filesystem": parts[0],
                        "size": parts[1],
                        "used": parts[2],
                        "available": parts[3],
                        "use_percent": parts[4],
                        "mount": parts[5],
                    }))
                } else { None }
            }).collect();
            json!({ "filesystems": lines })
        }

        #[cfg(target_os = "linux")]
        {
            let output = Self::run_cmd("df", &["-h", "--output=source,size,used,avail,pcent,target"]).await;
            let lines: Vec<Value> = output.lines().skip(1).filter_map(|line| {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 6 {
                    Some(json!({
                        "filesystem": parts[0],
                        "size": parts[1],
                        "used": parts[2],
                        "available": parts[3],
                        "use_percent": parts[4],
                        "mount": parts[5],
                    }))
                } else { None }
            }).collect();
            json!({ "filesystems": lines })
        }

        #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
        {
            json!({ "note": "Disk info not supported on this platform" })
        }
    }
}

#[async_trait]
impl Tool for SystemInfoTool {
    fn name(&self) -> &str {
        "system_info"
    }

    fn description(&self) -> &str {
        "Retrieve system information. Operations: overview (OS, hostname, uptime), cpu (processor details), memory (RAM usage), disk (storage usage), all (complete system report)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "description": "One of: overview, cpu, memory, disk, all",
                    "enum": ["overview", "cpu", "memory", "disk", "all"]
                }
            },
            "required": ["operation"]
        })
    }

    fn preflight(&self, args: &Value) -> Result<()> {
        let operation = args.get("operation").and_then(|v| v.as_str()).unwrap_or("");
        if operation.is_empty() {
            anyhow::bail!("Preflight: 'operation' is required");
        }
        if !["overview", "cpu", "memory", "disk", "all"].contains(&operation) {
            anyhow::bail!(
                "Preflight: unknown operation '{}'. Use: overview, cpu, memory, disk, all",
                operation
            );
        }
        Ok(())
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let operation = args["operation"].as_str().unwrap_or("").trim();

        if operation.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: "Missing required parameter: operation".into(),
            });
        }

        let result = match operation {
            "overview" => {
                let info = Self::gather_overview().await;
                json!({ "overview": info })
            }
            "cpu" => {
                let info = Self::gather_cpu().await;
                json!({ "cpu": info })
            }
            "memory" => {
                let info = Self::gather_memory().await;
                json!({ "memory": info })
            }
            "disk" => {
                let info = Self::gather_disk().await;
                json!({ "disk": info })
            }
            "all" => {
                // Gather all info concurrently
                let (overview, cpu, memory, disk) = tokio::join!(
                    Self::gather_overview(),
                    Self::gather_cpu(),
                    Self::gather_memory(),
                    Self::gather_disk(),
                );
                json!({
                    "overview": overview,
                    "cpu": cpu,
                    "memory": memory,
                    "disk": disk,
                })
            }
            _ => {
                return Ok(ToolResult {
                    success: false,
                    output: format!("Unknown operation: '{}'. Use: overview, cpu, memory, disk, all", operation),
                });
            }
        };

        Ok(ToolResult {
            success: true,
            output: serde_json::to_string_pretty(&result)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_name() {
        let tool = SystemInfoTool::new();
        assert_eq!(tool.name(), "system_info");
    }

    #[test]
    fn test_description_not_empty() {
        let tool = SystemInfoTool::new();
        assert!(!tool.description().is_empty());
    }

    #[test]
    fn test_schema() {
        let tool = SystemInfoTool::new();
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["operation"].is_object());
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("operation")));
    }

    #[test]
    fn test_schema_enum_values() {
        let tool = SystemInfoTool::new();
        let schema = tool.parameters_schema();
        let ops = schema["properties"]["operation"]["enum"].as_array().unwrap();
        let op_strings: Vec<&str> = ops.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(op_strings.contains(&"overview"));
        assert!(op_strings.contains(&"cpu"));
        assert!(op_strings.contains(&"memory"));
        assert!(op_strings.contains(&"disk"));
        assert!(op_strings.contains(&"all"));
    }

    #[test]
    fn test_preflight_missing_operation() {
        let tool = SystemInfoTool::new();
        let result = tool.preflight(&json!({}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("operation"));
    }

    #[test]
    fn test_preflight_invalid_operation() {
        let tool = SystemInfoTool::new();
        let result = tool.preflight(&json!({"operation": "network"}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unknown operation"));
    }

    #[test]
    fn test_preflight_overview_valid() {
        let tool = SystemInfoTool::new();
        assert!(tool.preflight(&json!({"operation": "overview"})).is_ok());
    }

    #[test]
    fn test_preflight_cpu_valid() {
        let tool = SystemInfoTool::new();
        assert!(tool.preflight(&json!({"operation": "cpu"})).is_ok());
    }

    #[test]
    fn test_preflight_memory_valid() {
        let tool = SystemInfoTool::new();
        assert!(tool.preflight(&json!({"operation": "memory"})).is_ok());
    }

    #[test]
    fn test_preflight_disk_valid() {
        let tool = SystemInfoTool::new();
        assert!(tool.preflight(&json!({"operation": "disk"})).is_ok());
    }

    #[test]
    fn test_preflight_all_valid() {
        let tool = SystemInfoTool::new();
        assert!(tool.preflight(&json!({"operation": "all"})).is_ok());
    }

    #[tokio::test]
    async fn test_execute_missing_operation() {
        let tool = SystemInfoTool::new();
        let result = tool.execute(json!({})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Missing"));
    }

    #[tokio::test]
    async fn test_execute_unknown_operation() {
        let tool = SystemInfoTool::new();
        let result = tool.execute(json!({"operation": "network"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Unknown operation"));
    }

    #[tokio::test]
    async fn test_execute_overview() {
        let tool = SystemInfoTool::new();
        let result = tool.execute(json!({"operation": "overview"})).await.unwrap();
        assert!(result.success);
        let parsed: Value = serde_json::from_str(&result.output).unwrap();
        assert!(parsed["overview"].is_object());
    }

    #[tokio::test]
    async fn test_execute_cpu() {
        let tool = SystemInfoTool::new();
        let result = tool.execute(json!({"operation": "cpu"})).await.unwrap();
        assert!(result.success);
        let parsed: Value = serde_json::from_str(&result.output).unwrap();
        assert!(parsed["cpu"].is_object());
    }

    #[tokio::test]
    async fn test_execute_memory() {
        let tool = SystemInfoTool::new();
        let result = tool.execute(json!({"operation": "memory"})).await.unwrap();
        assert!(result.success);
        let parsed: Value = serde_json::from_str(&result.output).unwrap();
        assert!(parsed["memory"].is_object());
    }

    #[tokio::test]
    async fn test_execute_disk() {
        let tool = SystemInfoTool::new();
        let result = tool.execute(json!({"operation": "disk"})).await.unwrap();
        assert!(result.success);
        let parsed: Value = serde_json::from_str(&result.output).unwrap();
        assert!(parsed["disk"].is_object());
    }

    #[tokio::test]
    async fn test_execute_all() {
        let tool = SystemInfoTool::new();
        let result = tool.execute(json!({"operation": "all"})).await.unwrap();
        assert!(result.success);
        let parsed: Value = serde_json::from_str(&result.output).unwrap();
        assert!(parsed["overview"].is_object());
        assert!(parsed["cpu"].is_object());
        assert!(parsed["memory"].is_object());
        assert!(parsed["disk"].is_object());
    }

    #[test]
    fn test_spec_name_matches() {
        let tool = SystemInfoTool::new();
        let spec = tool.spec();
        assert_eq!(spec.name, "system_info");
    }
}
