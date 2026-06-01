// core/src/platform/ios.rs
//
// iOS platform adapter. Required because `phantom-mesh-app` (Tauri shell)
// links `phantom-mesh` core as a lib dep on every target including
// aarch64-apple-ios{,-sim} (Cargo.toml: `phantom-mesh = { path = "../../core",
// features = ["desktop"] }`). PF-1's per-OS dispatch in `current()` had no
// iOS arm, so `cargo build --target aarch64-apple-ios-sim` triggered the
// `compile_error!` and broke `scripts/package-ios.sh --sim`.
//
// iOS is "deferred to v0.7.0+" per docs/tdd/INDEX.md — this adapter is
// intentionally minimal:
//   - no sandbox-exec / Seatbelt at process level (iOS uses app-container
//     sandboxing via entitlements; the Tauri layer handles that)
//   - hardware introspection via sysctl is best-effort and may be
//     restricted under App Store sandbox rules
//   - dist_binary_name matches the rustup target name (the lib is what
//     ships, not a CLI binary)

use super::PlatformAdapter;
use tokio::process::Command;

pub struct Platform;
pub static PLATFORM: Platform = Platform;

impl PlatformAdapter for Platform {
    fn make_command(&self, program: &str, args: &[String], _raw_cmd: &str) -> Command {
        let mut c = Command::new(program);
        c.args(args);
        c
    }

    fn shell_command(&self, cmd: &str) -> Command {
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
        "Apple Silicon (iOS)".into()
    }

    fn os_name(&self) -> String {
        "iOS".into()
    }

    fn dist_binary_name(&self) -> &'static str {
        "phantom-aarch64-apple-ios"
    }

    fn config_dir(&self) -> std::path::PathBuf {
        if let Some(home) = dirs::home_dir() {
            return home.join("Library/Application Support/ai.phantommesh.app");
        }
        std::path::PathBuf::from(".phantom-mesh")
    }
}
