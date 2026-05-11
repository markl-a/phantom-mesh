//! Unified hardware detection: GPU, NPU, RAM, port scanning.
//! Platform-specific strategies consolidated into a single module.

use serde::Serialize;
use std::net::TcpListener;
use sysinfo::System;

// ── Types ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct GpuInfo {
    pub name: String,
    pub dedicated_mb: u64,
    pub shared_mb: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct NpuInfo {
    pub name: String,
    pub tops: u32,
    pub device_id: String,
}

/// Combined result from a single hardware detection pass.
#[derive(Debug, Clone, Serialize)]
pub struct HardwareInfo {
    pub gpus: Vec<GpuInfo>,
    pub npus: Vec<NpuInfo>,
    pub ram_mb: u64,
    pub available_port: u16,
}

// ── Public API ────────────────────────────────────────────

/// Run all hardware detection in one call.
pub fn detect_all(preferred_port: u16) -> HardwareInfo {
    let mut sys = System::new_all();
    sys.refresh_all();
    let ram_mb = sys.total_memory() / (1024 * 1024);

    let (gpus, npus) = detect_accelerators();
    let available_port = find_available_port(preferred_port);

    HardwareInfo {
        gpus,
        npus,
        ram_mb,
        available_port,
    }
}

pub fn find_available_port(preferred: u16) -> u16 {
    for port in preferred..preferred + 100 {
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
    preferred
}

// ── Unified Accelerator Detection ─────────────────────────

/// Detect GPUs and NPUs together. On Windows this is a single PowerShell call.
fn detect_accelerators() -> (Vec<GpuInfo>, Vec<NpuInfo>) {
    #[cfg(target_os = "windows")]
    {
        if let Some(result) = detect_accelerators_windows() {
            return result;
        }
    }

    #[cfg(target_os = "macos")]
    {
        return detect_accelerators_macos();
    }

    #[cfg(target_os = "linux")]
    {
        return detect_accelerators_linux();
    }

    #[allow(unreachable_code)]
    (vec![GpuInfo { name: "CPU-only".into(), dedicated_mb: 0, shared_mb: 0 }], vec![])
}

// ── Windows ───────────────────────────────────────────────

#[cfg(target_os = "windows")]
fn detect_accelerators_windows() -> Option<(Vec<GpuInfo>, Vec<NpuInfo>)> {
    // Try bundled unified script first (GPU + NPU in one call)
    if let Some(result) = run_unified_script() {
        return Some(result);
    }

    // Fallback: separate inline detection
    let gpus = detect_gpus_wmi();
    let npus = detect_npus_pnp();
    Some((gpus, npus))
}

#[cfg(target_os = "windows")]
fn find_script_path() -> Option<std::path::PathBuf> {
    // Production: next to executable
    let exe_relative = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|p| p.join("scripts/detect_hardware.ps1")));

    if let Some(ref p) = exe_relative {
        if p.exists() { return exe_relative; }
    }

    // Dev: relative to src-tauri working directory
    let dev = std::path::PathBuf::from("scripts/detect_hardware.ps1");
    if dev.exists() { return Some(dev); }

    None
}

#[cfg(target_os = "windows")]
fn run_unified_script() -> Option<(Vec<GpuInfo>, Vec<NpuInfo>)> {
    let script_path = find_script_path()?;

    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(&script_path)
        .output()
        .ok()?;

    let json_str = String::from_utf8_lossy(&output.stdout);
    let val: serde_json::Value = serde_json::from_str(&json_str).ok()?;

    let gpus = parse_gpu_array(&val["gpus"]);
    let npus = parse_npu_array(&val["npus"]);

    if gpus.is_empty() { return None; }
    Some((gpus, npus))
}

#[cfg(target_os = "windows")]
fn detect_gpus_wmi() -> Vec<GpuInfo> {
    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command",
            "Get-CimInstance Win32_VideoController | Where-Object { $_.Name -notmatch 'Microsoft Basic' } | Select-Object Name,AdapterRAM | ConvertTo-Json -Compress"])
        .output();

    if let Ok(output) = output {
        let json = String::from_utf8_lossy(&output.stdout);
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&json) {
            let items = as_array(&val);
            let result: Vec<GpuInfo> = items.iter().filter_map(|item| {
                Some(GpuInfo {
                    name: item["Name"].as_str()?.to_string(),
                    dedicated_mb: item["AdapterRAM"].as_u64().unwrap_or(0) / (1024 * 1024),
                    shared_mb: 0,
                })
            }).collect();
            if !result.is_empty() { return result; }
        }
    }

    vec![GpuInfo { name: "CPU-only".into(), dedicated_mb: 0, shared_mb: 0 }]
}

#[cfg(target_os = "windows")]
fn detect_npus_pnp() -> Vec<NpuInfo> {
    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command",
            "Get-CimInstance Win32_PnPEntity | Where-Object { $_.PNPClass -eq 'ComputeAccelerator' } | Select-Object Name, DeviceID | ConvertTo-Json -Compress"])
        .output();

    if let Ok(output) = output {
        let json = String::from_utf8_lossy(&output.stdout);
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&json) {
            let items = as_array(&val);
            return items.iter().filter_map(|item| {
                let name = item["Name"].as_str()?.trim().to_string();
                let device_id = item["DeviceID"].as_str().unwrap_or("").to_string();
                let tops = lookup_npu_tops(&device_id);
                Some(NpuInfo { name, tops, device_id })
            }).collect();
        }
    }

    vec![]
}

// ── macOS ─────────────────────────────────────────────────

#[cfg(target_os = "macos")]
fn detect_accelerators_macos() -> (Vec<GpuInfo>, Vec<NpuInfo>) {
    let gpus = detect_gpus_system_profiler();
    let npus = detect_npu_sysctl();
    (gpus, npus)
}

#[cfg(target_os = "macos")]
fn detect_gpus_system_profiler() -> Vec<GpuInfo> {
    if let Ok(output) = std::process::Command::new("system_profiler")
        .args(["SPDisplaysDataType", "-json"])
        .output()
    {
        if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&output.stdout) {
            if let Some(displays) = json["SPDisplaysDataType"].as_array() {
                let result: Vec<GpuInfo> = displays.iter().filter_map(|d| {
                    let name = d["sppci_model"].as_str()
                        .or_else(|| d["_name"].as_str())?;
                    let vram_str = d["sppci_vram"].as_str().unwrap_or("0");
                    let vram_mb = vram_str.split_whitespace().next()
                        .and_then(|n| n.parse::<u64>().ok())
                        .unwrap_or(0);
                    Some(GpuInfo { name: name.to_string(), dedicated_mb: vram_mb, shared_mb: 0 })
                }).collect();
                if !result.is_empty() { return result; }
            }
        }
    }
    vec![GpuInfo { name: "CPU-only".into(), dedicated_mb: 0, shared_mb: 0 }]
}

#[cfg(target_os = "macos")]
fn detect_npu_sysctl() -> Vec<NpuInfo> {
    if let Ok(output) = std::process::Command::new("sysctl")
        .args(["-n", "hw.optional.neural_engine"])
        .output()
    {
        if String::from_utf8_lossy(&output.stdout).trim() == "1" {
            return vec![NpuInfo {
                name: "Apple Neural Engine".to_string(),
                tops: 38,
                device_id: String::new(),
            }];
        }
    }
    vec![]
}

// ── Linux ─────────────────────────────────────────────────

#[cfg(target_os = "linux")]
fn detect_accelerators_linux() -> (Vec<GpuInfo>, Vec<NpuInfo>) {
    let gpus = detect_gpus_lspci();
    let npus = detect_npus_accel();
    (gpus, npus)
}

#[cfg(target_os = "linux")]
fn detect_gpus_lspci() -> Vec<GpuInfo> {
    if let Ok(output) = std::process::Command::new("lspci").output() {
        let text = String::from_utf8_lossy(&output.stdout);
        let result: Vec<GpuInfo> = text.lines()
            .filter(|l| l.contains("VGA") || l.contains("3D") || l.contains("Display"))
            .filter_map(|line| {
                let name = line.splitn(3, ':').nth(2)?.trim().to_string();
                Some(GpuInfo { name, dedicated_mb: 0, shared_mb: 0 })
            })
            .collect();
        if !result.is_empty() { return result; }
    }
    vec![GpuInfo { name: "CPU-only".into(), dedicated_mb: 0, shared_mb: 0 }]
}

#[cfg(target_os = "linux")]
fn detect_npus_accel() -> Vec<NpuInfo> {
    if let Ok(entries) = std::fs::read_dir("/sys/class/accel") {
        let result: Vec<NpuInfo> = entries.filter_map(|e| {
            let path = e.ok()?.path();
            let name = std::fs::read_to_string(path.join("device/name"))
                .or_else(|_| std::fs::read_to_string(path.join("name")))
                .unwrap_or_else(|_| path.file_name()?.to_string_lossy().to_string());
            Some(NpuInfo { name: name.trim().to_string(), tops: 0, device_id: String::new() })
        }).collect();
        if !result.is_empty() { return result; }
    }
    vec![]
}

// ── Shared Helpers ────────────────────────────────────────

/// PowerShell returns a single object (not array) when there's only one item.
fn as_array(val: &serde_json::Value) -> Vec<serde_json::Value> {
    if val.is_array() {
        val.as_array().cloned().unwrap_or_default()
    } else if val.is_null() {
        vec![]
    } else {
        vec![val.clone()]
    }
}

fn parse_gpu_array(val: &serde_json::Value) -> Vec<GpuInfo> {
    as_array(val).iter().filter_map(|g| {
        Some(GpuInfo {
            name: g["name"].as_str()?.to_string(),
            dedicated_mb: g["dedicated_mb"].as_u64().unwrap_or(0),
            shared_mb: g["shared_mb"].as_u64().unwrap_or(0),
        })
    }).collect()
}

fn parse_npu_array(val: &serde_json::Value) -> Vec<NpuInfo> {
    as_array(val).iter().filter_map(|n| {
        let name = n["name"].as_str()?.trim().to_string();
        let device_id = n["device_id"].as_str().unwrap_or("").to_string();
        let tops = lookup_npu_tops(&device_id);
        Some(NpuInfo { name, tops, device_id })
    }).collect()
}

/// Known NPU TOPS ratings by PCI vendor:device ID.
fn lookup_npu_tops(device_id: &str) -> u32 {
    let id = device_id.to_uppercase();
    // AMD XDNA / XDNA2
    if id.contains("VEN_1022") {
        if id.contains("DEV_17F0") { return 50; }  // XDNA2 (Strix Halo/Point)
        if id.contains("DEV_1502") { return 16; }  // XDNA (Hawk Point)
    }
    // Intel NPU
    if id.contains("VEN_8086") {
        if id.contains("DEV_7D1D") { return 11; }  // Meteor Lake
        if id.contains("DEV_AD1D") { return 48; }  // Lunar Lake
        if id.contains("DEV_B51D") { return 13; }  // Arrow Lake
    }
    // Qualcomm Hexagon
    if id.contains("VEN_17CB") { return 45; }
    0
}
