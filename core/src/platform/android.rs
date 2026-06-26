// core/src/platform/android.rs
//
// Android platform adapter. Currently a thin shim: no sandbox (Landlock
// is Linux-only; Termux/APK contexts already run in app-sandbox), no
// /proc parsing (Termux exposes it but the previous code path returned
// the "Unknown" default — preserved here to avoid behavior change).
//
// Future work (epic P-AND-2): Termux detection, app-container path
// mapping, foreground-service lifecycle.

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
        let mut c = Command::new("sh");
        c.args(["-c", cmd]);
        c
    }

    // Pre-extraction: `ram_mb()` had no Android cfg branch and fell
    // through to `0`. Preserved.
    fn ram_mb(&self) -> u64 {
        0
    }

    // Pre-extraction: `cpu_name()` had no Android cfg branch and
    // returned `"Unknown CPU"`. Preserved.
    fn cpu_name(&self) -> String {
        "Unknown CPU".into()
    }

    // Pre-extraction: `os_name()` had no Android cfg branch and fell
    // through to `"Unknown OS"`. Preserved — P-AND-2 will switch this
    // to "Android <version>".
    fn os_name(&self) -> String {
        "Unknown OS".into()
    }

    fn dist_binary_name(&self) -> &'static str {
        "phantom-aarch64-linux-android"
    }

    // Pre-extraction: `config_dir()` had no Android cfg branch and
    // fell through to `~/.phantom-mesh`. Preserved — P-AND-2 will
    // switch this to the Termux app-container path when running
    // under Termux.
    fn config_dir(&self) -> std::path::PathBuf {
        crate::cli_config::phantom_data_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from(".").join(".phantom-mesh"))
    }
}
