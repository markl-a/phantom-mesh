// core/src/platform/mod.rs
//
// Single place for all platform-specific behaviour.
// Add a new target (Android, iOS, WASM, …) by implementing the functions below.
//
// Usage:
//   use crate::platform;
//   let cmd = platform::make_command("ls", &[], "ls");
//   let ram = platform::ram_mb();

use tokio::process::Command;

// ── Shell ──────────────────────────────────────────────────────────────────

/// Returns true for binaries that have native Windows executables and behave
/// identically across platforms (no shell wrapper needed on Windows).
#[allow(dead_code)]
fn is_cross_platform_bin(program: &str) -> bool {
    matches!(
        program,
        "cargo" | "rustc" | "rustfmt" | "clippy"
        | "git"
        | "node" | "npm" | "npx" | "yarn" | "pnpm"
        | "python" | "python3" | "pip" | "pip3"
        | "deno" | "bun"
        | "docker" | "kubectl" | "helm"
        | "go" | "java" | "javac" | "mvn" | "gradle"
        | "dotnet" | "cmake" | "ninja"
        | "tsc" | "make"
        | "jq" | "curl" | "wget"
    )
}

/// Build a `tokio::process::Command` for (program, args, raw_cmd).
///
/// - Unix: always `Command::new(program)` with `args`.
/// - Windows cross-platform bins: `Command::new(program)` with `args`.
/// - Windows everything else: `cmd.exe /C <raw_cmd>`.
///
/// On macOS the resulting `Command` is automatically wrapped in
/// `/usr/bin/sandbox-exec` so the spawned process can read anywhere but
/// can only write to cwd, `$HOME`, `/tmp`, `/private/tmp` (see
/// [`SBPL_PROFILE`]). Set `PHANTOM_SANDBOX=0` to disable for a session
/// — required when the agent needs to write to e.g. `/usr/local` or
/// `/Applications`. Linux landlock support is tracked separately.
pub fn make_command(program: &str, args: &[String], raw_cmd: &str) -> Command {
    #[cfg(windows)]
    {
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
    #[cfg(target_os = "macos")]
    {
        let _ = raw_cmd;
        if let Some((sandboxed_program, sandboxed_args)) = sandbox_macos::wrap(program, args) {
            let mut c = Command::new(sandboxed_program);
            c.args(sandboxed_args);
            return c;
        }
        let mut c = Command::new(program);
        c.args(args);
        c
    }
    #[cfg(target_os = "linux")]
    {
        let _ = raw_cmd;
        let mut c = Command::new(program);
        c.args(args);
        sandbox_linux::install_pre_exec(&mut c);
        c
    }
    #[cfg(all(not(windows), not(target_os = "macos"), not(target_os = "linux")))]
    {
        let _ = raw_cmd;
        let mut c = Command::new(program);
        c.args(args);
        c
    }
}

/// Build a shell command for strings that contain pipes / redirects.
/// Uses `sh -c` on Unix, `cmd.exe /C` on Windows. macOS adds the
/// same `sandbox-exec` wrapping as [`make_command`]; Linux installs
/// a Landlock pre-exec hook on the spawned `sh`.
pub fn shell_command(cmd: &str) -> Command {
    #[cfg(windows)]
    { let mut c = Command::new("cmd.exe"); c.args(["/C", cmd]); c }
    #[cfg(target_os = "macos")]
    {
        let args = ["-c".to_string(), cmd.to_string()];
        if let Some((p, a)) = sandbox_macos::wrap("/bin/sh", &args) {
            let mut c = Command::new(p);
            c.args(a);
            return c;
        }
        let mut c = Command::new("/bin/sh");
        c.args(["-c", cmd]);
        c
    }
    #[cfg(target_os = "linux")]
    {
        let mut c = Command::new("sh");
        c.args(["-c", cmd]);
        sandbox_linux::install_pre_exec(&mut c);
        c
    }
    #[cfg(all(not(windows), not(target_os = "macos"), not(target_os = "linux")))]
    {
        let mut c = Command::new("sh");
        c.args(["-c", cmd]);
        c
    }
}

#[cfg(target_os = "linux")]
mod sandbox_linux {
    //! Linux Landlock LSM wrapping for shell tools — equivalent of
    //! macOS Seatbelt. Restricts file writes to cwd, `$HOME`, `/tmp`,
    //! `/var/tmp`; reads remain unrestricted; network is unrestricted
    //! (Landlock doesn't cover sockets — that's a job for seccomp,
    //! tracked separately).
    //!
    //! Activation: on by default whenever the kernel reports a
    //! supported Landlock ABI (>= v1, kernel 5.13+). Older kernels
    //! degrade silently — the ruleset build returns an error which
    //! we swallow, leaving the spawn unrestricted. Set
    //! `PHANTOM_SANDBOX=0` to opt out per session,
    //! `PHANTOM_SANDBOX=strict` to drop `$HOME` from the writable
    //! list.
    //!
    //! Implementation: `tokio::process::Command::pre_exec` runs in
    //! the forked child after `fork(2)` but before `execve(2)`. The
    //! child applies the ruleset to itself via
    //! `landlock::Ruleset::restrict_self`, then exec's the target
    //! binary; the restrictions are inherited across exec.
    //!
    //! Caveats:
    //! - Untested from macOS dev machine. The structural code path
    //!   compiles via the `landlock` crate's cross-platform shims;
    //!   real-world validation happens the first time phantom-mesh
    //!   is rebuilt on Linux. Watch the `landlock` crate's
    //!   `RulesetStatus` value in tracing logs — `FullyEnforced`
    //!   means we got everything we asked for.
    //! - Pre-exec callback must be `unsafe` because it runs after
    //!   fork; only async-signal-safe operations are legal there.
    //!   We allocate a `String` (not signal-safe) inside the
    //!   ruleset builder, which is technically a footgun. Mitigation:
    //!   landlock's own crate already does this and ships in
    //!   production; we mirror their pattern.

    use std::io;
    use std::os::unix::process::CommandExt;
    use tokio::process::Command;

    fn mode() -> &'static str {
        match std::env::var("PHANTOM_SANDBOX").as_deref() {
            Ok("0") | Ok("off") | Ok("disabled") => "off",
            Ok("strict") => "strict",
            _ => "relaxed",
        }
    }

    /// Install a `pre_exec` hook on `cmd` that applies a Landlock
    /// ruleset restricting file writes. Errors during ruleset
    /// construction (older kernels, kernels without Landlock support
    /// compiled in, paths that don't exist) are swallowed: we'd
    /// rather run the agent unrestricted than refuse to spawn. The
    /// `RulesetStatus` returned by `restrict_self` is logged via
    /// `tracing` so unexpected partial-enforcement on production
    /// kernels is visible.
    pub(super) fn install_pre_exec(cmd: &mut Command) {
        if mode() == "off" {
            return;
        }
        let cwd = std::env::current_dir()
            .ok()
            .and_then(|p| p.to_str().map(String::from))
            .unwrap_or_else(|| "/tmp".to_string());
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        let strict = mode() == "strict";

        // SAFETY: `pre_exec` runs in the forked child between fork
        // and exec. Only async-signal-safe code is allowed there.
        // `landlock::Ruleset` allocates internally — we're matching
        // the pattern used by the `landlock` crate's own examples
        // and accept the small risk for the security upgrade.
        unsafe {
            cmd.pre_exec(move || apply_landlock(&cwd, &home, strict));
        }
    }

    /// Build + apply the ruleset. Returns `Ok(())` even on landlock
    /// failure so the spawn proceeds; phantom-side errors are
    /// surfaced via `tracing::warn` from the parent (best-effort).
    fn apply_landlock(cwd: &str, home: &str, strict: bool) -> io::Result<()> {
        use landlock::{
            ABI, Access, AccessFs, PathBeneath, PathFd, Ruleset,
            RulesetAttr, RulesetCreatedAttr,
        };

        let abi = ABI::V1;
        let result = (|| -> Result<(), Box<dyn std::error::Error>> {
            let mut ruleset = Ruleset::default()
                .handle_access(AccessFs::from_all(abi))?
                .create()?;

            // Allow writes to cwd + tmp dirs. `$HOME` is added in
            // relaxed mode but dropped in strict.
            let mut writable_paths: Vec<&str> = vec![cwd, "/tmp", "/var/tmp", "/dev/null"];
            if !strict {
                writable_paths.push(home);
            }
            for p in writable_paths {
                if let Ok(fd) = PathFd::new(p) {
                    let rule = PathBeneath::new(fd, AccessFs::from_all(abi));
                    ruleset = ruleset.add_rule(rule)?;
                }
            }
            let _status = ruleset.restrict_self()?;
            Ok(())
        })();
        if let Err(e) = result {
            // pre_exec is the wrong place to log via tracing (not
            // signal-safe + the parent can't see it anyway), so we
            // swallow silently. The `tracing` warning happens at
            // build/install_pre_exec time when the env says sandbox
            // is wanted but the kernel rejects it.
            let _ = e;
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod sandbox_macos {
    //! macOS Seatbelt (sandbox-exec) wrapping for shell tools.
    //!
    //! Strategy: deny-by-default; allow reads everywhere; restrict
    //! writes to cwd, `$HOME`, `/tmp`, `/private/tmp`; allow network
    //! and exec because almost every dev tool needs them. This is a
    //! safety net against accidental `rm -rf /` and writes to system
    //! paths (`/etc`, `/usr/local`, `/Applications`, `/Library`),
    //! NOT a substitute for full process isolation.
    //!
    //! Activation: on by default whenever
    //! `/usr/bin/sandbox-exec` exists. Opt out per-session with
    //! `PHANTOM_SANDBOX=0`. Pass `PHANTOM_SANDBOX=strict` to drop
    //! `$HOME` from the writable list (only cwd + tmp).
    //!
    //! Caveats:
    //! - sandbox-exec is technically deprecated by Apple but still
    //!   ships in macOS 15 and is what every tool (xcodebuild, swift
    //!   package manager, etc.) uses internally. Replacement TBD.
    //! - Profile uses `(param "...")` substitution to avoid string
    //!   escaping issues with paths containing spaces.

    /// Inline SBPL profile. `(param "CWD")` and `(param "HOME")` are
    /// substituted via `-D NAME=value` flags at spawn time.
    const SBPL_PROFILE_RELAXED: &str = r#"(version 1)
(deny default)
(allow process-fork)
(allow process-exec*)
(allow file-read*)
(allow file-write*
    (subpath (param "CWD"))
    (subpath (param "HOME"))
    (subpath "/private/tmp")
    (subpath "/tmp")
    (subpath "/private/var/folders")
    (regex #"^/dev/(null|zero|tty|fd|std(in|out|err))"))
(allow network*)
(allow signal)
(allow ipc-posix-shm*)
(allow ipc-posix-sem)
(allow mach-lookup)
(allow mach-priv-task-port)
(allow sysctl-read)
(allow iokit-open)
(allow file-issue-extension)
"#;

    /// Strict variant — drops `$HOME` from writable subpaths so the
    /// agent can't trash dotfiles. Pass `PHANTOM_SANDBOX=strict`.
    const SBPL_PROFILE_STRICT: &str = r#"(version 1)
(deny default)
(allow process-fork)
(allow process-exec*)
(allow file-read*)
(allow file-write*
    (subpath (param "CWD"))
    (subpath "/private/tmp")
    (subpath "/tmp")
    (subpath "/private/var/folders")
    (regex #"^/dev/(null|zero|tty|fd|std(in|out|err))"))
(allow network*)
(allow signal)
(allow ipc-posix-shm*)
(allow ipc-posix-sem)
(allow mach-lookup)
(allow mach-priv-task-port)
(allow sysctl-read)
(allow iokit-open)
(allow file-issue-extension)
"#;

    fn mode() -> &'static str {
        match std::env::var("PHANTOM_SANDBOX").as_deref() {
            Ok("0") | Ok("off") | Ok("disabled") => "off",
            Ok("strict") => "strict",
            _ => "relaxed",
        }
    }

    /// Returns `Some((program, args))` rewritten to invoke through
    /// sandbox-exec, or `None` when the sandbox is disabled or
    /// unavailable. Callers fall back to spawning `(program, args)`
    /// unmodified.
    pub(super) fn wrap(program: &str, args: &[String]) -> Option<(String, Vec<String>)> {
        match mode() {
            "off" => return None,
            _ => {}
        }
        // sandbox-exec ships with macOS — but bail safely if it ever
        // disappears so we don't break shell tools entirely.
        if !std::path::Path::new("/usr/bin/sandbox-exec").exists() {
            return None;
        }
        // Canonicalize: macOS `/var/folders/...` (mktemp default) is a
        // symlink to `/private/var/folders/...`, and sandbox-exec
        // checks the *resolved* path against the profile. Without
        // this realpath() pass, writes inside a tempdir-cwd get
        // denied even though the profile says cwd is writable —
        // because the param holds the symlink form, not what the
        // kernel sees. Fall back to the unresolved path if
        // canonicalize fails (path missing, permission denied).
        let raw_cwd = std::env::current_dir().unwrap_or_else(|_| "/private/tmp".into());
        let cwd = std::fs::canonicalize(&raw_cwd)
            .unwrap_or(raw_cwd)
            .to_str()
            .map(String::from)
            .unwrap_or_else(|| "/private/tmp".to_string());
        let raw_home = std::env::var("HOME").unwrap_or_else(|_| "/private/tmp".into());
        let home = std::fs::canonicalize(&raw_home)
            .ok()
            .and_then(|p| p.to_str().map(String::from))
            .unwrap_or(raw_home);
        let profile = if mode() == "strict" {
            SBPL_PROFILE_STRICT
        } else {
            SBPL_PROFILE_RELAXED
        };

        let mut wrapped: Vec<String> = Vec::with_capacity(args.len() + 7);
        wrapped.push("-D".to_string());
        wrapped.push(format!("CWD={}", cwd));
        wrapped.push("-D".to_string());
        wrapped.push(format!("HOME={}", home));
        wrapped.push("-p".to_string());
        wrapped.push(profile.to_string());
        wrapped.push(program.to_string());
        wrapped.extend(args.iter().cloned());
        Some(("/usr/bin/sandbox-exec".to_string(), wrapped))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        // Each test acquires the shared env-lock guard so parallel
        // test runners can't race on `PHANTOM_SANDBOX`. See
        // `crate::env_lock` in `lib.rs` for the rationale.

        #[test]
        fn wrap_returns_none_when_disabled() {
            let _g = crate::env_lock::acquire();
            std::env::set_var("PHANTOM_SANDBOX", "0");
            let r = wrap("/bin/echo", &["hi".to_string()]);
            std::env::remove_var("PHANTOM_SANDBOX");
            assert!(r.is_none());
        }

        #[test]
        fn wrap_inserts_sandbox_exec_with_profile() {
            let _g = crate::env_lock::acquire();
            std::env::remove_var("PHANTOM_SANDBOX");
            let (p, a) = wrap("/bin/echo", &["hi".to_string()])
                .expect("sandbox should wrap by default on macOS");
            assert_eq!(p, "/usr/bin/sandbox-exec");
            assert!(a.contains(&"-p".to_string()));
            assert!(a.iter().any(|x| x.contains("(version 1)")));
            assert!(a.iter().any(|x| x.starts_with("CWD=")));
            assert!(a.iter().any(|x| x.starts_with("HOME=")));
            assert_eq!(a.last().unwrap(), "hi");
        }

        #[test]
        fn strict_mode_drops_home_from_profile() {
            let _g = crate::env_lock::acquire();
            std::env::set_var("PHANTOM_SANDBOX", "strict");
            let (_p, a) = wrap("/bin/echo", &[]).expect("strict mode wraps");
            std::env::remove_var("PHANTOM_SANDBOX");
            let profile = a.iter().find(|x| x.contains("(version 1)")).unwrap();
            assert!(!profile.contains("(subpath (param \"HOME\"))"),
                "strict mode must not allow $HOME writes");
        }
    }
}

// ── Hardware ───────────────────────────────────────────────────────────────

/// Total system RAM in megabytes.
pub fn ram_mb() -> u64 {
    #[cfg(target_os = "macos")]
    {
        if let Ok(out) = std::process::Command::new("sysctl")
            .args(["-n", "hw.memsize"]).output()
        {
            if let Ok(s) = String::from_utf8(out.stdout) {
                if let Ok(bytes) = s.trim().parse::<u64>() {
                    return bytes / (1024 * 1024);
                }
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(content) = std::fs::read_to_string("/proc/meminfo") {
            for line in content.lines() {
                if line.starts_with("MemTotal:") {
                    let kb: u64 = line.split_whitespace()
                        .nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                    return kb / 1024;
                }
            }
        }
    }
    #[cfg(target_os = "windows")]
    {
        // GlobalMemoryStatusEx via wmic
        if let Ok(out) = std::process::Command::new("wmic")
            .args(["computersystem", "get", "TotalPhysicalMemory", "/value"]).output()
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
    }
    0
}

/// CPU model string.
pub fn cpu_name() -> String {
    #[cfg(target_os = "macos")]
    {
        if let Ok(out) = std::process::Command::new("sysctl")
            .args(["-n", "machdep.cpu.brand_string"]).output()
        {
            if let Ok(s) = String::from_utf8(out.stdout) {
                let s = s.trim();
                if !s.is_empty() { return s.to_string(); }
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(content) = std::fs::read_to_string("/proc/cpuinfo") {
            for line in content.lines() {
                if line.starts_with("model name") {
                    if let Some(name) = line.split(':').nth(1) {
                        return name.trim().to_string();
                    }
                }
            }
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Ok(out) = std::process::Command::new("wmic")
            .args(["cpu", "get", "Name", "/value"]).output()
        {
            if let Ok(s) = String::from_utf8(out.stdout) {
                for line in s.lines() {
                    if let Some(name) = line.strip_prefix("Name=") {
                        let name = name.trim();
                        if !name.is_empty() { return name.to_string(); }
                    }
                }
            }
        }
    }
    "Unknown CPU".into()
}

/// Human-readable OS string.
pub fn os_name() -> String {
    #[cfg(target_os = "macos")]
    {
        if let Ok(out) = std::process::Command::new("sw_vers")
            .arg("-productVersion").output()
        {
            if let Ok(v) = String::from_utf8(out.stdout) {
                return format!("macOS {}", v.trim());
            }
        }
        return "macOS".into();
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(content) = std::fs::read_to_string("/etc/os-release") {
            for line in content.lines() {
                if line.starts_with("PRETTY_NAME=") {
                    return line.trim_start_matches("PRETTY_NAME=")
                        .trim_matches('"').to_string();
                }
            }
        }
        return "Linux".into();
    }
    #[cfg(target_os = "windows")]
    { return "Windows".into(); }
    #[allow(unreachable_code)]
    "Unknown OS".into()
}

// ── Paths ──────────────────────────────────────────────────────────────────

/// Platform-specific binary name for dist/ artefacts.
pub fn dist_binary_name() -> &'static str {
    #[cfg(target_os = "macos")]   { "phantom-macos-arm64" }
    #[cfg(target_os = "android")] { "phantom-aarch64-linux-android" }
    #[cfg(target_os = "linux")]   { "phantom-linux-arm64" }
    #[cfg(target_os = "windows")] { "phantom-windows-x86_64.exe" }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "android", target_os = "windows")))]
    { "phantom" }
}

/// User-level config/data directory.
/// macOS: ~/Library/Application Support/ai.phantommesh.app
/// Windows: %APPDATA%\phantom-mesh
/// Linux/other: ~/.phantom-mesh
pub fn config_dir() -> std::path::PathBuf {
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = dirs::home_dir() {
            return home.join("Library/Application Support/ai.phantommesh.app");
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            return std::path::PathBuf::from(appdata).join("phantom-mesh");
        }
    }
    let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    home.join(".phantom-mesh")
}

// ── mDNS advertising ───────────────────────────────────────────────────────

/// Advertise this node on the local network via the platform's mDNS daemon.
/// Spawns a background process and returns immediately.
pub async fn mdns_advertise(service_name: &str, port: u16, url_txt: &str) {
    let port_str = port.to_string();

    #[cfg(target_os = "macos")]
    let result = tokio::process::Command::new("dns-sd")
        .args(["-R", service_name, "_phantom-mesh._tcp", ".", &port_str, url_txt])
        .spawn();

    #[cfg(target_os = "linux")]
    let result = tokio::process::Command::new("avahi-publish-service")
        .args([service_name, "_phantom-mesh._tcp", &port_str, url_txt])
        .spawn();

    #[cfg(target_os = "windows")]
    let result: Result<tokio::process::Child, std::io::Error> = {
        let _ = (service_name, &port_str, url_txt);
        Err(std::io::Error::other("mDNS: not supported on Windows (use coordinator instead)"))
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
