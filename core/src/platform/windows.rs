// core/src/platform/windows.rs
//
// Windows platform adapter. Uses `cmd.exe /C <raw>` for tools that need
// shell features (redirects, built-ins like `dir`); spawns the binary
// directly for cross-platform CLIs that ship as native `.exe`.

use super::PlatformAdapter;
use tokio::process::Command;

/// Native Windows binaries that behave identically across platforms — no
/// `cmd.exe` wrapper needed. These are tools that ship as standalone
/// `.exe` and take their args via argv unchanged.
fn is_cross_platform_bin(program: &str) -> bool {
    matches!(
        program,
        "cargo"
            | "rustc"
            | "rustfmt"
            | "clippy"
            | "git"
            | "node"
            | "npm"
            | "npx"
            | "yarn"
            | "pnpm"
            | "python"
            | "python3"
            | "pip"
            | "pip3"
            | "deno"
            | "bun"
            | "docker"
            | "kubectl"
            | "helm"
            | "go"
            | "java"
            | "javac"
            | "mvn"
            | "gradle"
            | "dotnet"
            | "cmake"
            | "ninja"
            | "tsc"
            | "make"
            | "jq"
            | "curl"
            | "wget"
    )
}

pub struct Platform;
pub static PLATFORM: Platform = Platform;

impl PlatformAdapter for Platform {
    fn make_command(&self, program: &str, args: &[String], raw_cmd: &str) -> Command {
        if is_cross_platform_bin(program) {
            let mut c = Command::new(program);
            c.args(args);
            c
        } else {
            let mut c = Command::new("cmd.exe");
            c.args(["/C", raw_cmd]);
            c
        }
    }

    fn shell_command(&self, cmd: &str) -> Command {
        let mut c = Command::new("cmd.exe");
        c.args(["/C", cmd]);
        c
    }

    fn ram_mb(&self) -> u64 {
        if let Ok(out) = std::process::Command::new("wmic")
            .args(["computersystem", "get", "TotalPhysicalMemory", "/value"])
            .output()
        {
            if let Ok(s) = String::from_utf8(out.stdout) {
                for line in s.lines() {
                    if let Some(val) = line.strip_prefix("TotalPhysicalMemory=") {
                        if let Ok(bytes) = val.trim().parse::<u64>() {
                            return bytes / (1024 * 1024);
                        }
                    }
                }
            }
        }
        0
    }

    fn cpu_name(&self) -> String {
        if let Ok(out) = std::process::Command::new("wmic")
            .args(["cpu", "get", "Name", "/value"])
            .output()
        {
            if let Ok(s) = String::from_utf8(out.stdout) {
                for line in s.lines() {
                    if let Some(name) = line.strip_prefix("Name=") {
                        let name = name.trim();
                        if !name.is_empty() {
                            return name.to_string();
                        }
                    }
                }
            }
        }
        "Unknown CPU".into()
    }

    fn os_name(&self) -> String {
        "Windows".into()
    }

    fn dist_binary_name(&self) -> &'static str {
        "phantom-windows-x86_64.exe"
    }

    fn config_dir(&self) -> std::path::PathBuf {
        if let Ok(appdata) = std::env::var("APPDATA") {
            return std::path::PathBuf::from(appdata).join("phantom-mesh");
        }
        crate::cli_config::phantom_data_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from(".").join(".phantom-mesh"))
    }
}
