// core/src/platform/mod.rs
//
// Per-OS platform adapters.
//
// Each supported target_os has its own module under `core/src/platform/`
// implementing the [`PlatformAdapter`] trait. The free-function API
// (`make_command`, `shell_command`, `ram_mb`, ...) is preserved as
// thin delegating wrappers around [`current()`] so existing callers
// continue to work unchanged.
//
// Adding a new target:
//   1. Create `core/src/platform/<os>.rs`:
//        pub struct Platform;
//        pub static PLATFORM: Platform = Platform;
//        impl PlatformAdapter for Platform { /* ... */ }
//   2. Add a `#[cfg(target_os = "<os>")] mod <os>;` line below.
//   3. Add the corresponding cfg arm to [`current()`].
//
// iOS: minimal stub adapter (see ios.rs). `spectyn-mesh-app` links the
// core lib as a dep for all targets (Cargo.toml line 24), so the iOS
// target needs *some* adapter or `current()` falls through to the
// compile_error! arm and breaks `package-ios.sh --sim`. Full iOS
// support — including app-container-aware paths, ATS-friendly mDNS,
// MLX provider — is deferred to v0.7.0+ per docs/tdd/INDEX.md.

use tokio::process::Command;

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "ios")]
mod ios;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

/// The set of platform-specific operations every target must provide.
/// Implementations live in `core/src/platform/<os>.rs`.
pub trait PlatformAdapter: Send + Sync + 'static {
    /// Build a `tokio::process::Command` for (program, args, raw_cmd).
    ///
    /// - Unix: `Command::new(program)` with `args`.
    /// - Windows cross-platform bins: `Command::new(program)` with `args`.
    /// - Windows everything else: `cmd.exe /C <raw_cmd>`.
    /// - macOS additionally wraps the spawn in `/usr/bin/sandbox-exec`.
    /// - Linux additionally installs a Landlock pre-exec hook.
    fn make_command(&self, program: &str, args: &[String], raw_cmd: &str) -> Command;

    /// Build a shell command for strings that contain pipes / redirects.
    /// Uses `sh -c` on Unix, `cmd.exe /C` on Windows. Per-OS sandboxing
    /// applies the same way as [`make_command`].
    fn shell_command(&self, cmd: &str) -> Command;

    /// Total system RAM in megabytes. Best-effort; returns 0 on failure.
    fn ram_mb(&self) -> u64;

    /// CPU model string. Best-effort; returns "Unknown CPU" on failure.
    fn cpu_name(&self) -> String;

    /// Human-readable OS string.
    fn os_name(&self) -> String;

    /// Platform-specific binary name for dist/ artefacts.
    fn dist_binary_name(&self) -> &'static str;

    /// User-level config/data directory.
    /// macOS: `~/Library/Application Support/ai.spectynmesh.app`
    /// Windows: `%APPDATA%\spectyn-mesh`
    /// Linux/Android/other: `~/.spectyn-mesh`
    fn config_dir(&self) -> std::path::PathBuf;
}

/// The platform adapter for the currently compiled target.
pub fn current() -> &'static dyn PlatformAdapter {
    #[cfg(target_os = "windows")]
    {
        &windows::PLATFORM
    }
    #[cfg(target_os = "linux")]
    {
        &linux::PLATFORM
    }
    #[cfg(target_os = "macos")]
    {
        &macos::PLATFORM
    }
    #[cfg(target_os = "android")]
    {
        &android::PLATFORM
    }
    #[cfg(target_os = "ios")]
    {
        &ios::PLATFORM
    }
    #[cfg(not(any(
        target_os = "windows",
        target_os = "linux",
        target_os = "macos",
        target_os = "android",
        target_os = "ios",
    )))]
    {
        compile_error!(
            "spectyn-mesh: unsupported target_os for platform adapter. \
             Add a per-OS module under core/src/platform/ and wire it \
             into platform::current()."
        );
    }
}

// ── Free-function compatibility wrappers ───────────────────────────────────
// These preserve the original API surface (`crate::platform::<fn>(...)`)
// so callers do not need to change. Each delegates to `current()`.

/// See [`PlatformAdapter::make_command`].
pub fn make_command(program: &str, args: &[String], raw_cmd: &str) -> Command {
    current().make_command(program, args, raw_cmd)
}

/// See [`PlatformAdapter::shell_command`].
pub fn shell_command(cmd: &str) -> Command {
    current().shell_command(cmd)
}

/// See [`PlatformAdapter::ram_mb`].
pub fn ram_mb() -> u64 {
    current().ram_mb()
}

/// See [`PlatformAdapter::cpu_name`].
pub fn cpu_name() -> String {
    current().cpu_name()
}

/// See [`PlatformAdapter::os_name`].
pub fn os_name() -> String {
    current().os_name()
}

/// See [`PlatformAdapter::dist_binary_name`].
pub fn dist_binary_name() -> &'static str {
    current().dist_binary_name()
}

/// See [`PlatformAdapter::config_dir`].
pub fn config_dir() -> std::path::PathBuf {
    current().config_dir()
}

// ── mDNS advertising ───────────────────────────────────────────────────────
// Async free function (not on the trait — keeps the trait sync so we
// don't need `async-trait`). Per-OS cfg dispatch inline.

/// Advertise this node on the local network via the platform's mDNS daemon.
/// Spawns a background process and returns immediately.
pub async fn mdns_advertise(service_name: &str, port: u16, url_txt: &str) {
    let port_str = port.to_string();

    #[cfg(target_os = "macos")]
    let result = tokio::process::Command::new("dns-sd")
        .args([
            "-R",
            service_name,
            "_spectyn-mesh._tcp",
            ".",
            &port_str,
            url_txt,
        ])
        .spawn();

    #[cfg(target_os = "linux")]
    let result = tokio::process::Command::new("avahi-publish-service")
        .args([service_name, "_spectyn-mesh._tcp", &port_str, url_txt])
        .spawn();

    #[cfg(target_os = "windows")]
    let result: Result<tokio::process::Child, std::io::Error> = {
        let _ = (service_name, &port_str, url_txt);
        Err(std::io::Error::other(
            "mDNS: not supported on Windows (use coordinator instead)",
        ))
    };

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    let result: Result<tokio::process::Child, std::io::Error> = {
        let _ = (service_name, &port_str, url_txt);
        Err(std::io::Error::other("mDNS: unsupported platform"))
    };

    if let Err(e) = result {
        tracing::debug!("mDNS advertise: {}", e);
    }
}
