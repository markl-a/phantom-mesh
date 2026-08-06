//! Hardware and capability detection for the local host.
//!
//! Inspects CPU, GPU/VRAM, RAM, the operating system, hostname, and a few
//! optional runtime capabilities (Ollama, Docker, macOS Metal/TCC, Linux
//! display server) so the cluster hub can advertise what this machine can
//! actually do.
//!
//! # Heuristics only
//!
//! Every probe here is **best-effort and heuristic** — it answers "is this
//! capability *plausibly* available" rather than guaranteeing it works:
//!
//! * **GPU/VRAM** is parsed from `system_profiler` (macOS) or `wmic`
//!   (Windows). Apple Silicon reports unified memory as `shared_mb`, not
//!   dedicated VRAM. Linux currently reports no discrete GPU here.
//! * **CPU/RAM/OS/hostname** come from [`crate::platform`] and the
//!   `hostname` command / `$HOSTNAME`, falling back to `"unknown"`.
//! * **Ollama** is probed with a 3-second HTTP request to the default local
//!   port; a timeout or any non-success is reported as `offline`.
//! * **Docker / Metal / TCC / display-server** detectors are deliberately
//!   side-effect free (env-var and filesystem reads only) so they can run on
//!   the synchronous `scan()` hot path without spawning a runtime. They may
//!   over-report (e.g. a stale `docker` CLI with no reachable daemon);
//!   downstream layers do the authoritative round-trip before scheduling
//!   work. See each function's doc comment for its exact contract.

use crate::platform;
use serde::Serialize;
use serde_json::Value;

/// Snapshot of the local host's detected hardware and capabilities.
///
/// Produced by [`scan`] and serialized for reporting to the cluster hub.
/// All fields are best-effort; see the module docs for heuristic caveats.
#[derive(Debug, Clone, Serialize)]
pub struct HardwareScanResult {
    /// Name of the primary GPU, or `"CPU-only"` when none is detected.
    pub gpu: String,
    /// Dedicated VRAM of the primary GPU in megabytes (`0` if unknown).
    pub vram_mb: u64,
    /// All detected GPUs (may be empty on hosts with no discrete GPU probe).
    pub gpus: Vec<GpuInfo>,
    /// Detected NPUs (neural processing units); reserved, currently empty.
    pub npus: Vec<Value>,
    /// Total system RAM in megabytes.
    pub ram_mb: u64,
    /// Human-readable CPU name/model string.
    pub cpu: String,
    /// Human-readable operating-system name.
    pub os: String,
    /// Host name, or `"unknown"` if it could not be determined.
    pub hostname: String,
    /// Ollama daemon status: `"online"` or `"offline"`.
    pub ollama_status: String,
    /// Model names reported by a reachable Ollama daemon (empty otherwise).
    pub ollama_models: Vec<String>,
    /// Path to the spectyn daemon binary, when known.
    pub daemon_binary_path: Option<String>,
    /// Default port the daemon should listen on.
    pub available_port: u16,
}

/// A single detected GPU and its memory characteristics.
#[derive(Debug, Clone, Serialize)]
pub struct GpuInfo {
    /// GPU model name (e.g. `"Apple Silicon (Macmini)"`).
    pub name: String,
    /// Dedicated video memory in megabytes (`0` for unified-memory GPUs).
    pub dedicated_mb: u64,
    /// Shared/unified memory in megabytes (Apple Silicon reports RAM here).
    pub shared_mb: u64,
}

/// Scan the local host and return a [`HardwareScanResult`].
///
/// Gathers CPU/RAM/OS/hostname synchronously and probes Ollama
/// asynchronously over HTTP. Best-effort: missing or unreadable signals
/// fall back to defaults rather than erroring (see the module docs).
pub async fn scan() -> HardwareScanResult {
    let ram_mb = platform::ram_mb();
    let cpu = platform::cpu_name();
    let hostname = get_hostname();
    let os = platform::os_name();
    let gpus = get_gpus();
    let gpu = gpus
        .first()
        .map(|g| g.name.clone())
        .unwrap_or_else(|| "CPU-only".into());
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
            std::process::Command::new("hostname")
                .output()
                .ok()
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
            .args(["SPDisplaysDataType", "-json"])
            .output()
        {
            if let Ok(json) = serde_json::from_slice::<Value>(&out.stdout) {
                let mut gpus = Vec::new();
                if let Some(displays) = json["SPDisplaysDataType"].as_array() {
                    for d in displays {
                        let name = d["sppci_model"]
                            .as_str()
                            .or_else(|| d["_name"].as_str())
                            .unwrap_or("Unknown GPU")
                            .to_string();
                        let vram_str = d["spdisplays_vram"].as_str().unwrap_or("0");
                        let vram_mb = parse_vram(vram_str);
                        gpus.push(GpuInfo {
                            name,
                            dedicated_mb: vram_mb,
                            shared_mb: 0,
                        });
                    }
                }
                if !gpus.is_empty() {
                    return gpus;
                }
            }
        }
        // Apple Silicon — unified memory, report as GPU
        if let Ok(out) = std::process::Command::new("sysctl")
            .arg("-n")
            .arg("hw.model")
            .output()
        {
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
            .args([
                "path",
                "win32_VideoController",
                "get",
                "Name,AdapterRAM",
                "/value",
            ])
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
                    return vec![GpuInfo {
                        name,
                        dedicated_mb: vram_mb,
                        shared_mb: 0,
                    }];
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
    if s.contains("GB") {
        val * 1024
    } else {
        val
    }
}

/// Linux display server detected at runtime (V5 / P-LIN-1).
///
/// Distinguishing Wayland from X11 matters for picking the right capture
/// backend (e.g. `pipewire` portal vs. `xdotool`/`xrandr`) and for honest
/// capability reporting back to the cluster hub.
///
/// Heuristic: `$WAYLAND_DISPLAY` is the canonical Wayland signal (set by
/// every compositor and by `Xwayland`'s parent shell). `$DISPLAY` is the
/// X11 signal; under `Xwayland` both are set, in which case Wayland wins.
/// When neither is set we report `Headless` (typical for a TTY login,
/// a systemd service, a CI runner, or an SSH session without forwarding).
///
/// This function is intentionally side-effect free — it only reads
/// environment variables — so it can be called from `detect()` paths
/// without a tokio runtime.
#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum LinuxDisplayServer {
    Wayland,
    X11,
    Headless,
}

/// See [`LinuxDisplayServer`].
#[cfg(target_os = "linux")]
pub fn detect_linux_display_server() -> LinuxDisplayServer {
    let has_wayland = std::env::var_os("WAYLAND_DISPLAY")
        .map(|v| !v.is_empty())
        .unwrap_or(false);
    let has_x11 = std::env::var_os("DISPLAY")
        .map(|v| !v.is_empty())
        .unwrap_or(false);

    match (has_wayland, has_x11) {
        (true, _) => LinuxDisplayServer::Wayland,
        (false, true) => LinuxDisplayServer::X11,
        (false, false) => LinuxDisplayServer::Headless,
    }
}

/// Detect whether a usable Docker engine is reachable from this host (V5 /
/// P-LIN-1). Returning `true` means the cluster hub can advertise the
/// `container.docker` capability and the operator may launch sandboxed
/// workloads via the Docker socket.
///
/// Heuristic — both signals must be cheap and side-effect free, since this
/// runs inside `scan()` on the daemon hot path:
///   1. **Unix socket present**: `/var/run/docker.sock` exists. This is the
///      strongest positive signal — a running Docker Engine, rootful or
///      rootless-with-the-classic-path, always exposes it.
///   2. **CLI on PATH**: `docker` is found in `$PATH`. This catches the
///      common WSL2 case where Docker Desktop on Windows forwards the
///      CLI into the WSL distro *but the socket lives at a non-standard
///      path* (`/mnt/wsl/shared-docker/docker.sock` or a TCP endpoint),
///      so the sock-existence check alone under-reports. It also catches
///      hosts using `DOCKER_HOST=tcp://…` for a remote daemon.
///
/// Either signal alone is sufficient — we OR them. False positives (a
/// stale CLI with no reachable daemon) are acceptable here because the
/// next layer (`scenarios::docker_*`) does a real `docker version` round
/// trip before scheduling work; this function is the cheap, synchronous
/// capability advertiser.
///
/// This function is intentionally side-effect free — it does not invoke
/// `docker` or open the socket — so it can be called from `detect()` paths
/// without a tokio runtime and without surprising the user with prompts.
#[cfg(target_os = "linux")]
pub fn detect_docker() -> bool {
    if std::path::Path::new("/var/run/docker.sock").exists() {
        return true;
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            if dir.join("docker").is_file() {
                return true;
            }
        }
    }
    false
}

/// Detect whether TCC (Transparency, Consent, and Control) is reachable
/// from this process on macOS — i.e. whether the system framework that
/// gates Camera / Microphone / Screen Recording / Full Disk Access /
/// Accessibility prompts is present.
///
/// **Important**: this does *not* answer "is my app granted permission
/// X" — that requires either reading the per-user TCC.db (SIP-protected
/// on every supported macOS) or calling `AVAuthorizationStatus` /
/// `IOHIDCheckAccess` via FFI. Honest per-permission status detection
/// from a Rust CLI without Swift bindings is bounded by Apple's design.
///
/// What this *does* tell us: whether the framework binary is on disk.
/// Returning `false` means a broken / partial macOS install (the same
/// signal as `detect_metal() == false`).
#[cfg(target_os = "macos")]
pub fn detect_tcc_framework() -> bool {
    // TCC.framework is a PrivateFramework on every Apple-shipped macOS
    // since ~10.14. The path is stable; even on Recovery / Internet
    // Recovery boot it's there.
    std::path::Path::new("/System/Library/PrivateFrameworks/TCC.framework").is_dir()
        // tccutil is the user-facing CLI; ships in /usr/bin since
        // 10.15. Belt-and-braces fallback for hosts where the
        // PrivateFramework layout may have shifted.
        || std::path::Path::new("/usr/bin/tccutil").is_file()
}

/// Detect whether Metal is available on this Mac at runtime.
///
/// Checks for the Metal framework on disk. Metal has shipped in every
/// macOS release since 10.11 (2015) and on every Apple Silicon Mac
/// without exception, so on a healthy system this returns `true`.
/// Returning `false` indicates a broken / partial macOS install
/// (e.g. base system without graphics) — useful as a sanity gate
/// before launching MLX or other Metal-dependent providers.
#[cfg(target_os = "macos")]
pub fn detect_metal() -> bool {
    std::path::Path::new("/System/Library/Frameworks/Metal.framework").is_dir()
}

/// Detect whether the host CPU is Apple Silicon (M-series) at runtime.
///
/// Reads `hw.optional.arm64` via sysctl, which is `1` on every
/// M1/M2/M3/M4 Mac and `0` on Intel Macs. This is a **host** check,
/// not a binary check — an Intel `x86_64-apple-darwin` build running
/// under Rosetta on Apple Silicon still gets `true`. (A native arm64
/// build can only run on Apple Silicon, so for those `target_arch =
/// "aarch64"` ⇒ this returns `true`.)
///
/// Returns `false` on sysctl failure (best-effort).
#[cfg(target_os = "macos")]
pub fn detect_apple_silicon() -> bool {
    if let Ok(out) = std::process::Command::new("sysctl")
        .args(["-n", "hw.optional.arm64"])
        .output()
    {
        if let Ok(s) = String::from_utf8(out.stdout) {
            return s.trim() == "1";
        }
    }
    false
}

async fn probe_ollama() -> (String, Vec<String>) {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap_or_default();

    match client.get("http://localhost:11434/api/tags").send().await {
        Ok(resp) if resp.status().is_success() => {
            if let Ok(body) = resp.json::<Value>().await {
                let models = body["models"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|m| m["name"].as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                ("online".into(), models)
            } else {
                ("online".into(), vec![])
            }
        }
        _ => ("offline".into(), vec![]),
    }
}

#[cfg(test)]
mod tests {
    //! Linux V5 P0 tests for hardware capability detection.
    //!
    //! NOTE on env-var mutation: `std::env::set_var` / `remove_var` are
    //! `unsafe` as of Rust 1.81 because they race with concurrent reads
    //! from other threads. We mitigate by:
    //!   1) requiring `--test-threads=1` (documented in the TDD runbook;
    //!      the `tdd-run.sh` helper already passes this flag), and
    //!   2) saving + restoring every variable we touch so other tests in
    //!      the same module see a clean environment.
    //!
    //! The cfg gate ensures this block is only compiled on Linux, so
    //! macOS/Windows builds don't break.

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_wayland_x11_detect() {
        use super::{detect_linux_display_server, LinuxDisplayServer};

        // Snapshot ambient values so we can restore them on exit.
        let prev_wayland = std::env::var_os("WAYLAND_DISPLAY");
        let prev_display = std::env::var_os("DISPLAY");

        // Helper that runs an assertion and panics with the same restore
        // path regardless of outcome — we want `prev_*` reinstated even
        // on failure so subsequent tests aren't poisoned.
        let restore = || {
            // SAFETY: tests are single-threaded (`--test-threads=1`); no
            // other thread is reading the environment concurrently.
            unsafe {
                match &prev_wayland {
                    Some(v) => std::env::set_var("WAYLAND_DISPLAY", v),
                    None => std::env::remove_var("WAYLAND_DISPLAY"),
                }
                match &prev_display {
                    Some(v) => std::env::set_var("DISPLAY", v),
                    None => std::env::remove_var("DISPLAY"),
                }
            }
        };

        // SAFETY: see above; tests are single-threaded.
        let assertions = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // Case 1: Wayland-only.
            unsafe {
                std::env::set_var("WAYLAND_DISPLAY", "wayland-0");
                std::env::remove_var("DISPLAY");
            }
            assert_eq!(
                detect_linux_display_server(),
                LinuxDisplayServer::Wayland,
                "WAYLAND_DISPLAY set, DISPLAY unset → Wayland"
            );

            // Case 2: X11-only.
            unsafe {
                std::env::remove_var("WAYLAND_DISPLAY");
                std::env::set_var("DISPLAY", ":0");
            }
            assert_eq!(
                detect_linux_display_server(),
                LinuxDisplayServer::X11,
                "WAYLAND_DISPLAY unset, DISPLAY=:0 → X11"
            );

            // Case 3: Both set (Xwayland). Wayland wins.
            unsafe {
                std::env::set_var("WAYLAND_DISPLAY", "wayland-1");
                std::env::set_var("DISPLAY", ":0");
            }
            assert_eq!(
                detect_linux_display_server(),
                LinuxDisplayServer::Wayland,
                "both set (Xwayland) → Wayland wins"
            );

            // Case 4: Neither set (headless TTY / systemd service / CI).
            unsafe {
                std::env::remove_var("WAYLAND_DISPLAY");
                std::env::remove_var("DISPLAY");
            }
            assert_eq!(
                detect_linux_display_server(),
                LinuxDisplayServer::Headless,
                "neither set → Headless"
            );

            // Case 5: Empty WAYLAND_DISPLAY string is treated as unset
            // (some shells leak empty vars from a parent session).
            unsafe {
                std::env::set_var("WAYLAND_DISPLAY", "");
                std::env::set_var("DISPLAY", ":1");
            }
            assert_eq!(
                detect_linux_display_server(),
                LinuxDisplayServer::X11,
                "empty WAYLAND_DISPLAY + DISPLAY=:1 → X11"
            );
        }));

        restore();
        if let Err(payload) = assertions {
            std::panic::resume_unwind(payload);
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_docker_capability_detect() {
        use super::detect_docker;

        // Compute ground truth INDEPENDENTLY of the production code path so
        // a regression in `detect_docker()` can't make this test silently
        // tautological. We replicate the two-signal OR by hand:
        //   1) classic rootful socket at the canonical path, and
        //   2) `docker` discoverable on $PATH.
        // Either one is sufficient — matches the contract documented on
        // `detect_docker()`.
        let socket_present = std::path::Path::new("/var/run/docker.sock").exists();
        let cli_on_path = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).any(|d| d.join("docker").is_file()))
            .unwrap_or(false);
        let expected = socket_present || cli_on_path;

        let actual = detect_docker();

        // The assertion is robust across machines because both sides
        // observe the SAME filesystem & $PATH at the SAME instant. On a
        // host without Docker (typical CI runner, fresh WSL2 without
        // Docker Desktop), both are `false`; on a developer box with
        // Docker Desktop forwarding the CLI but no /var/run/docker.sock,
        // both are `true` via the PATH branch; on a rootful server, both
        // are `true` via the socket branch.
        assert_eq!(
            actual, expected,
            "detect_docker() disagreed with independent ground truth: \
             socket_present={socket_present}, cli_on_path={cli_on_path}, \
             expected={expected}, actual={actual}. Either the detector's \
             heuristic changed or the filesystem mutated mid-test."
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_tcc_permission_detect() {
        use super::detect_tcc_framework;
        // Every supported macOS (10.14+) ships TCC.framework AND
        // /usr/bin/tccutil. A `false` here means a non-standard host
        // (Asahi Linux booting macOS userspace? broken Recovery?),
        // which would mean downstream Camera/Mic/Screen permission
        // probes can't work.
        assert!(
            detect_tcc_framework(),
            "neither /System/Library/PrivateFrameworks/TCC.framework \
             nor /usr/bin/tccutil exists — TCC permission flows cannot \
             be exercised from this host. Either the OS install is \
             corrupt or detect_tcc_framework() regressed."
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_metal_detect() {
        use super::detect_metal;
        // Every supported macOS (10.11+) ships Metal — and every Apple
        // Silicon Mac requires it. A `false` here means a non-standard
        // host, which would invalidate downstream MLX provider checks.
        assert!(
            detect_metal(),
            "Metal.framework missing — required by every Apple Silicon \
             Mac and every supported macOS since 10.11. Either the host \
             is broken or detect_metal() regressed."
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_apple_silicon_detect() {
        use super::detect_apple_silicon;

        let is_apple_silicon = detect_apple_silicon();

        // If the binary itself is aarch64-apple-darwin, the host MUST
        // be Apple Silicon — Rosetta cannot run native arm64 binaries
        // on Intel Macs. This is the strongest invariant we can assert
        // without pinning the test to a specific host.
        if cfg!(target_arch = "aarch64") {
            assert!(
                is_apple_silicon,
                "aarch64-apple-darwin binary running on non-Apple-Silicon host \
                 — `sysctl hw.optional.arm64` returned 0 (or failed). Either \
                 sysctl was unavailable or detection logic regressed."
            );
        }
        // For x86_64-apple-darwin builds the host may be Intel or
        // Apple-Silicon-under-Rosetta — either result is valid; we only
        // smoke that the function returned without panicking.
        let _ = is_apple_silicon;
    }
}
