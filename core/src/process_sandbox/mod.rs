// core/src/sandbox/mod.rs
//
// Per-OS process sandbox abstraction. Wraps `tokio::process::Command`
// invocations so shell-tool spawns run inside a Linux Landlock LSM
// ruleset / macOS Seatbelt profile (deny-by-default; read everywhere;
// write only to cwd + $HOME + tmp dirs).
//
// PF-6 (this commit): Moves Linux Landlock + macOS Seatbelt impls out
// of `core/src/platform/{linux,macos}.rs` into per-OS files under this
// module. Adds a `Sandbox` trait so callers can program against an
// abstraction.
//
// Activation: on by default. Opt out per-session via
//   PHANTOM_SANDBOX=0       disable entirely
//   PHANTOM_SANDBOX=strict  drop $HOME from writable paths
//
// Not a substitute for full process isolation — a safety net against
// accidental `rm -rf /` and writes to system paths. Network is
// unrestricted on both OSes (Landlock doesn't cover sockets; Seatbelt
// profile explicitly allows network*).

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod macos;

/// Generic process sandbox. Implementations transform a `program` +
/// `args` invocation into something safer to spawn — either by
/// modifying the Command in place (Linux pre_exec → Landlock) or by
/// rewriting program+args to invoke a wrapping binary (macOS
/// sandbox-exec).
///
/// Implementations swallow construction errors and return an
/// unsandboxed Command as fallback — the agent should keep running
/// even if the sandbox is unavailable on this kernel/host.
pub trait Sandbox: Send + Sync {
    /// Build a `tokio::process::Command` that, when spawned, runs
    /// `program` with `args` inside this OS's sandbox (where
    /// available + enabled).
    ///
    /// `raw_cmd` is the original shell expression for Linux's
    /// pre_exec path (used to capture cwd in the child). On macOS
    /// it's currently unused; reserved for future use.
    fn wrap_command(
        &self,
        program: &str,
        args: &[String],
        raw_cmd: &str,
    ) -> tokio::process::Command;
}

/// Active sandbox for the current target OS. Returns `None` on
/// platforms without a Sandbox impl (Windows, Android, iOS).
pub fn current() -> Option<Box<dyn Sandbox>> {
    #[cfg(target_os = "linux")]
    {
        return Some(Box::new(linux::LinuxSandbox));
    }
    #[cfg(target_os = "macos")]
    {
        return Some(Box::new(macos::MacosSandbox));
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}
