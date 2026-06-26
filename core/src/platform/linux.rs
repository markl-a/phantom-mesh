// core/src/platform/linux.rs
//
// Linux platform adapter. Wraps spawned shell tools with a Landlock
// LSM ruleset (equivalent to macOS Seatbelt). Since PF-6 (PR #226) the
// ruleset implementation lives in `crate::process_sandbox::linux`; this
// file holds only the `PlatformAdapter` impl plus a handful of small
// linux-specific helpers (journald routing, etc.).

use super::PlatformAdapter;
use tokio::process::Command;

pub struct Platform;
pub static PLATFORM: Platform = Platform;

impl PlatformAdapter for Platform {
    fn make_command(&self, program: &str, args: &[String], _raw_cmd: &str) -> Command {
        let mut c = Command::new(program);
        c.args(args);
        crate::process_sandbox::linux::install_pre_exec(&mut c);
        c
    }

    fn shell_command(&self, cmd: &str) -> Command {
        let mut c = Command::new("sh");
        c.args(["-c", cmd]);
        crate::process_sandbox::linux::install_pre_exec(&mut c);
        c
    }

    fn ram_mb(&self) -> u64 {
        if let Ok(content) = std::fs::read_to_string("/proc/meminfo") {
            for line in content.lines() {
                if line.starts_with("MemTotal:") {
                    let kb: u64 = line
                        .split_whitespace()
                        .nth(1)
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                    return kb / 1024;
                }
            }
        }
        0
    }

    fn cpu_name(&self) -> String {
        if let Ok(content) = std::fs::read_to_string("/proc/cpuinfo") {
            for line in content.lines() {
                if line.starts_with("model name") {
                    if let Some(name) = line.split(':').nth(1) {
                        return name.trim().to_string();
                    }
                }
            }
        }
        "Unknown CPU".into()
    }

    fn os_name(&self) -> String {
        if let Ok(content) = std::fs::read_to_string("/etc/os-release") {
            for line in content.lines() {
                if line.starts_with("PRETTY_NAME=") {
                    return line
                        .trim_start_matches("PRETTY_NAME=")
                        .trim_matches('"')
                        .to_string();
                }
            }
        }
        "Linux".into()
    }

    fn dist_binary_name(&self) -> &'static str {
        "phantom-linux-arm64"
    }

    fn config_dir(&self) -> std::path::PathBuf {
        crate::cli_config::phantom_data_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from(".").join(".phantom-mesh"))
    }
}

// PF-6: the per-OS Landlock sandbox was extracted to
// `core/src/process_sandbox/linux.rs`; callers route through
// `crate::process_sandbox::linux::install_pre_exec`.

/// Canonical systemd-journald sockets phantom-mesh routes through when
/// the host runs systemd. We pin them as a constant rather than hard-coding
/// in callers so the journalctl integration in `serve.rs` / `service::linux`
/// has a single source of truth.
///
/// The three paths are the trio journald listens on:
///   - `/run/systemd/journal/socket`  — native journal protocol (Sd-Native)
///   - `/run/systemd/journal/stdout`  — stream-mode stdout/stderr capture
///   - `/run/systemd/journal/dev-log` — legacy `syslog(3)` compatibility
///
/// On hosts without systemd (Alpine OpenRC, busybox containers, the
/// minimal `distroless` runtime image, etc.) none of these will exist;
/// callers must handle absence gracefully and fall back to stderr logging.
/// The `journal_routing_available()` helper expresses that policy.
#[allow(dead_code)] // public journald-routing API; consumed by tests + future systemd unit installer
pub const JOURNAL_SOCKETS: &[&str] = &[
    "/run/systemd/journal/socket",
    "/run/systemd/journal/stdout",
    "/run/systemd/journal/dev-log",
];

/// True when at least one journald socket exists on this host, i.e. when
/// systemd is the running init and journald is listening. Used by
/// `serve::install_logging` (and the future systemd unit installer in
/// `service::linux`) to decide whether to inherit `$JOURNAL_STREAM`
/// semantics or fall back to plain stderr.
#[allow(dead_code)] // public journald-routing API; consumed by tests + future systemd unit installer
pub fn journal_routing_available() -> bool {
    JOURNAL_SOCKETS
        .iter()
        .any(|p| std::path::Path::new(p).exists())
}

#[cfg(test)]
mod tests {
    //! Top-level Linux platform tests.
    //!
    //! Module path here is `platform::linux::tests::*`. The Landlock
    //! kernel-version pin (formerly `platform::linux::sandbox::tests::
    //! landlock_kernel_version_check`) moved to
    //! `process_sandbox::linux::tests::*` as part of PF-6 (PR #226).

    use super::{journal_routing_available, JOURNAL_SOCKETS};

    /// LIN P0 V5: pin the journald-socket integration point.
    ///
    /// `journal_routing_available()` is the single decision used by
    /// `service::linux` and `serve::install_logging` to choose between
    /// "feed logs to journald via the inherited stream" and "fall back
    /// to plain stderr." If the canonical socket paths ever move (e.g.
    /// upstream systemd relocates `/run/systemd/journal` — unlikely but
    /// not impossible on flatcar / nix / immutable distros), this test
    /// fails loudly so the install path is audited at the same time.
    ///
    /// Three invariants:
    ///   1. The constant lists the three canonical sockets, no more, no
    ///      less. Adding a fourth is a knowing change that must update
    ///      the test in the same commit.
    ///   2. On a host that is currently running journald (WSL2 Ubuntu
    ///      22.04 ticks this box because systemd-genie / `systemd=true`
    ///      in /etc/wsl.conf is now default), `journal_routing_available()`
    ///      returns true and at least one socket file exists.
    ///   3. On a host without journald the function returns false. We
    ///      can't simulate that in a unit test without faking the
    ///      filesystem, so we just assert the cheap correctness path:
    ///      the function never panics and never lies (return value
    ///      matches actual disk state).
    #[test]
    fn journal_routing_works() {
        // Invariant 1: stable socket list.
        assert_eq!(
            JOURNAL_SOCKETS.len(),
            3,
            "JOURNAL_SOCKETS should list exactly the 3 canonical paths; \
             update this test deliberately if systemd adds a fourth."
        );
        for p in JOURNAL_SOCKETS {
            assert!(
                p.starts_with("/run/systemd/journal/"),
                "expected socket under /run/systemd/journal/, got: {}",
                p
            );
        }

        // Invariant 3 (always-true sanity): function return must match
        // disk state.
        let any_exists = JOURNAL_SOCKETS
            .iter()
            .any(|p| std::path::Path::new(p).exists());
        assert_eq!(
            journal_routing_available(),
            any_exists,
            "journal_routing_available() must reflect actual filesystem state"
        );

        // Invariant 2 (host-conditional): when we can see journald, at
        // least one of the canonical paths must be a real file. We
        // detect journald via `/run/systemd/system/` (the marker dir
        // for "systemd is PID 1"), so the assertion is gated to hosts
        // that actually run systemd and don't get a spurious failure
        // on Alpine / busybox CI runners.
        let systemd_running = std::path::Path::new("/run/systemd/system").exists();
        if systemd_running {
            assert!(
                any_exists,
                "systemd appears to be PID 1 (/run/systemd/system exists) but \
                 no journal socket was found; journald may be disabled or \
                 the socket path moved. Checked: {:?}",
                JOURNAL_SOCKETS
            );
        }
    }
}
