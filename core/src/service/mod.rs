// core/src/service/mod.rs
//
// Per-OS service-manager adapters. `phantom service <install|uninstall|status>`
// installs phantom as a user-level auto-start service:
//
//   Windows → Scheduled Task at logon (schtasks / PowerShell Register-ScheduledTask)
//   macOS   → LaunchAgent in ~/Library/LaunchAgents (launchctl)
//   Linux   → systemd --user unit
//
// Layout follows `core/src/platform/`: one file per OS, cfg-gated. Public
// entry point is `run_service_subcommand(action)` which dispatches to the
// active OS implementation.
//
// PF-2a (2026-05-18): Windows impl extracted from `core/src/bin/phantom.rs`.
// PF-2b (this commit): Linux impl extracted + `PROPAGATED_ENV_KEYS` const
// promoted here as a shared list (used by Linux systemd install AND macOS
// launchd install).
// PF-2c (follow-up): macOS impl (currently lives at phantom.rs:6834).
//
// After PF-2b: macOS dispatch in phantom.rs `main` still uses the in-bin
// function; Windows + Linux now route through here.

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "windows")]
pub mod windows;

/// Env-var names that are propagated from the install-time shell into
/// the service's environment block (Linux systemd `Environment=` lines,
/// macOS launchd `<EnvironmentVariables>` plist dict). Shared between
/// both per-OS implementations so the list grows in one place.
///
/// Values are NEVER printed to install logs — only the names of keys
/// that were found.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub const PROPAGATED_ENV_KEYS: &[&str] = &[
    // LLM provider keys
    "OPENCODE_API_KEY",
    "OPENROUTER_API_KEY",
    "GROQ_API_KEY",
    "GEMINI_API_KEY",
    "ANTHROPIC_API_KEY",
    "OPENAI_API_KEY",
    "CEREBRAS_API_KEY",
    "DEEPSEEK_API_KEY",
    // phantom runtime knobs
    "PHANTOM_MAX_TOKENS",
    "PHANTOM_NODE_NAME",
    // L1 governed-worker knobs — so `phantom service install` produces a DURABLE
    // governed worker (flight-recorder + governor) on macOS/Linux, not just an
    // ungoverned serve. (Windows scheduled tasks inherit the user env directly.)
    "PHANTOM_GOVERN_CLI",
    "PHANTOM_CLI_SESSION_REPO",
];

/// Dispatch `phantom service <action>` to the active OS's implementation.
/// `action` is one of `install`, `uninstall`, `status` (others print an
/// error and exit 1).
///
/// On macOS this is currently a stub: the binary's dispatch site
/// (phantom.rs `main`) handles macOS directly via in-bin function until
/// PF-2c lands.
#[cfg(target_os = "windows")]
pub async fn run_service_subcommand(action: &str) -> anyhow::Result<()> {
    windows::run_service_subcommand(action).await
}

#[cfg(target_os = "linux")]
pub async fn run_service_subcommand(action: &str) -> anyhow::Result<()> {
    linux::run_service_subcommand(action).await
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
pub async fn run_service_subcommand(_action: &str) -> anyhow::Result<()> {
    // PF-2a + PF-2b routed Windows + Linux; the bin's `main` dispatches
    // macOS directly to its in-bin function until PF-2c lands — so this
    // stub should never be reached on macOS.
    Err(anyhow::anyhow!(
        "service: Windows + Linux routed through phantom_mesh::service; \
         macOS service install still lives in phantom.rs (PF-2c pending)."
    ))
}
