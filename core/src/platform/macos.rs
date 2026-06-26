// core/src/platform/macos.rs
//
// macOS platform adapter. Wraps spawned shell tools with sandbox-exec
// (Seatbelt) — see the `sandbox` submodule for the inline SBPL profile.

use super::PlatformAdapter;
use tokio::process::Command;

pub struct Platform;
pub static PLATFORM: Platform = Platform;

impl PlatformAdapter for Platform {
    fn make_command(&self, program: &str, args: &[String], _raw_cmd: &str) -> Command {
        if let Some((sandboxed_program, sandboxed_args)) =
            crate::process_sandbox::macos::wrap(program, args)
        {
            let mut c = Command::new(sandboxed_program);
            c.args(sandboxed_args);
            return c;
        }
        let mut c = Command::new(program);
        c.args(args);
        c
    }

    fn shell_command(&self, cmd: &str) -> Command {
        let args = ["-c".to_string(), cmd.to_string()];
        if let Some((p, a)) = crate::process_sandbox::macos::wrap("/bin/sh", &args) {
            let mut c = Command::new(p);
            c.args(a);
            return c;
        }
        let mut c = Command::new("/bin/sh");
        c.args(["-c", cmd]);
        c
    }

    fn ram_mb(&self) -> u64 {
        if let Ok(out) = std::process::Command::new("sysctl")
            .args(["-n", "hw.memsize"])
            .output()
        {
            if let Ok(s) = String::from_utf8(out.stdout) {
                if let Ok(bytes) = s.trim().parse::<u64>() {
                    return bytes / (1024 * 1024);
                }
            }
        }
        0
    }

    fn cpu_name(&self) -> String {
        if let Ok(out) = std::process::Command::new("sysctl")
            .args(["-n", "machdep.cpu.brand_string"])
            .output()
        {
            if let Ok(s) = String::from_utf8(out.stdout) {
                let s = s.trim();
                if !s.is_empty() {
                    return s.to_string();
                }
            }
        }
        "Unknown CPU".into()
    }

    fn os_name(&self) -> String {
        if let Ok(out) = std::process::Command::new("sw_vers")
            .arg("-productVersion")
            .output()
        {
            if let Ok(v) = String::from_utf8(out.stdout) {
                return format!("macOS {}", v.trim());
            }
        }
        "macOS".into()
    }

    fn dist_binary_name(&self) -> &'static str {
        "phantom-macos-arm64"
    }

    fn config_dir(&self) -> std::path::PathBuf {
        if let Some(home) = dirs::home_dir() {
            return home.join("Library/Application Support/ai.phantommesh.app");
        }
        crate::cli_config::phantom_data_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from(".").join(".phantom-mesh"))
    }
}

// PF-6: sandbox mod (lines 85-257) moved to core/src/sandbox/macos.rs;
// callers use crate::process_sandbox::macos::wrap.
