use crate::platform;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
pub struct HardwareScanResult {
    pub gpu: String,
    pub vram_mb: u64,
    pub gpus: Vec<GpuInfo>,
    pub npus: Vec<Value>,
    pub ram_mb: u64,
    pub cpu: String,
    pub os: String,
    pub hostname: String,
    pub ollama_status: String,
    pub ollama_models: Vec<String>,
    pub daemon_binary_path: Option<String>,
    pub available_port: u16,
}

#[derive(Debug, Clone, Serialize)]
pub struct GpuInfo {
    pub name: String,
    pub dedicated_mb: u64,
    pub shared_mb: u64,
}

pub async fn scan() -> HardwareScanResult {
    let ram_mb = platform::ram_mb();
    let cpu = platform::cpu_name();
    let hostname = get_hostname();
    let os = platform::os_name();
    let gpus = get_gpus();
    let gpu = gpus.first().map(|g| g.name.clone()).unwrap_or_else(|| "CPU-only".into());
    let vram_mb = gpus.first().map(|g| g.dedicated_mb).unwrap_or(0);
    let (ollama_status, ollama_models) = probe_ollama().await;

    HardwareScanResult {
        gpu,
        vram_mb,
        gpus,
        npus: vec![],
        ram_mb,
        cpu,
        os,
        hostname,
        ollama_status,
        ollama_models,
        daemon_binary_path: None,
        available_port: 7878,
    }
}

fn get_hostname() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| {
            std::process::Command::new("hostname").output().ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string())
                .ok_or(())
        })
        .unwrap_or_else(|_| "unknown".into())
}

fn get_gpus() -> Vec<GpuInfo> {
    #[cfg(target_os = "macos")]
    {
        if let Ok(out) = std::process::Command::new("system_profiler")
            .args(["SPDisplaysDataType", "-json"]).output()
        {
            if let Ok(json) = serde_json::from_slice::<Value>(&out.stdout) {
                let mut gpus = Vec::new();
                if let Some(displays) = json["SPDisplaysDataType"].as_array() {
                    for d in displays {
                        let name = d["sppci_model"].as_str()
                            .or_else(|| d["_name"].as_str())
                            .unwrap_or("Unknown GPU").to_string();
                        let vram_str = d["spdisplays_vram"].as_str().unwrap_or("0");
                        let vram_mb = parse_vram(vram_str);
                        gpus.push(GpuInfo { name, dedicated_mb: vram_mb, shared_mb: 0 });
                    }
                }
                if !gpus.is_empty() { return gpus; }
            }
        }
        // Apple Silicon — unified memory, report as GPU
        if let Ok(out) = std::process::Command::new("sysctl").arg("-n").arg("hw.model").output() {
            if let Ok(model) = String::from_utf8(out.stdout) {
                let model = model.trim();
                if model.contains("Mac") || model.contains("Apple") {
                    return vec![GpuInfo {
                        name: format!("Apple Silicon ({})", model),
                        dedicated_mb: 0,
                        shared_mb: platform::ram_mb(),
                    }];
                }
            }
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Ok(out) = std::process::Command::new("wmic")
            .args(["path", "win32_VideoController", "get", "Name,AdapterRAM", "/value"])
            .output()
        {
            if let Ok(s) = String::from_utf8(out.stdout) {
                let mut name = String::new();
                let mut vram_mb = 0u64;
                for line in s.lines() {
                    if let Some(v) = line.strip_prefix("Name=") {
                        name = v.trim().to_string();
                    }
                    if let Some(v) = line.strip_prefix("AdapterRAM=") {
                        vram_mb = v.trim().parse::<u64>().unwrap_or(0) / (1024 * 1024);
                    }
                }
                if !name.is_empty() {
                    return vec![GpuInfo { name, dedicated_mb: vram_mb, shared_mb: 0 }];
                }
            }
        }
    }
    vec![]
}

#[cfg(target_os = "macos")]
fn parse_vram(s: &str) -> u64 {
    let num: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
    let val: u64 = num.parse().unwrap_or(0);
    if s.contains("GB") { val * 1024 } else { val }
}

async fn probe_ollama() -> (String, Vec<String>) {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap_or_default();

    match client.get("http://localhost:11434/api/tags").send().await {
        Ok(resp) if resp.status().is_success() => {
            if let Ok(body) = resp.json::<Value>().await {
                let models = body["models"].as_array()
                    .map(|arr| arr.iter()
                        .filter_map(|m| m["name"].as_str().map(String::from))
                        .collect())
                    .unwrap_or_default();
                ("online".into(), models)
            } else {
                ("online".into(), vec![])
            }
        }
        _ => ("offline".into(), vec![]),
    }
}
