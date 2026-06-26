//! L0 — interactive AI-CLI session substrate. See
//! docs/superpowers/specs/2026-06-16-cli-session-substrate-design.md
pub mod event;
pub mod error;
// `hybrid` (the agy adapter) needs portable-pty, which is EXCLUDED on mobile
// (android/ios) because it pulls `termios`, uncompilable for Android. The mobile
// app is a remote supervisor and never drives a local AI CLI via a PTY.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod hybrid;
pub mod external_gateway;
pub mod normalizer;
pub mod parse;
pub mod structured;
pub mod transcript;

use crate::cli_session::error::SessionError;
use crate::cli_session::event::CliEvent;
use std::sync::mpsc::Receiver;

pub type SessionId = String;

/// Which CLI + how to run this session.
#[derive(Clone, Debug)]
pub struct SessionSpec {
    pub cli: CliKind,
    /// Working directory for the CLI.
    pub cwd: std::path::PathBuf,
    /// Per-turn timeout in seconds (the watchdog around stdout/PTY reads).
    pub timeout_secs: u64,
    /// Optional model override.
    pub model: Option<String>,
    /// Extra CLI args spliced in AFTER the base args and BEFORE the prompt. The L1
    /// governor uses this to inject claude's PreToolUse-hook `--settings` so a
    /// governed run pauses before each tool. Empty for an ungoverned session.
    pub extra_args: Vec<String>,
    /// Extra environment variables for the spawned CLI process. The L1 governor uses
    /// this to pass `PHANTOM_HOME` + `PHANTOM_GOVERN_TASK_ID` so the hook binds to
    /// the run's identity. Empty for an ungoverned session.
    pub env: Vec<(String, String)>,
}

impl SessionSpec {
    /// An ungoverned spec (no injected hook args/env) — the common case.
    pub fn new(cli: CliKind, cwd: std::path::PathBuf, timeout_secs: u64, model: Option<String>) -> Self {
        Self { cli, cwd, timeout_secs, model, extra_args: Vec::new(), env: Vec::new() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CliKind {
    Claude,
    Codex,
    Opencode,
    Agy,
    /// External one-shot gateway CLI registered in external_gateway::REGISTRY.
    /// The spec carries the program name, fixed args, and output-parse strategy.
    External(&'static external_gateway::ExternalGatewaySpec),
}

/// One turn's input.
#[derive(Clone, Debug)]
pub struct TurnInput {
    pub prompt: String,
}

/// A live session. `turn` returns a channel of normalized events as they are
/// parsed + reconciled; the channel closes when the turn ends.
pub trait CliSession: Send {
    fn session_id(&self) -> Option<&SessionId>;
    fn turn(&mut self, input: TurnInput) -> Result<Receiver<CliEvent>, SessionError>;
    /// Resume is only meaningful for claude/codex/opencode (agy = one-shot).
    fn resumable(&self) -> bool;
}

/// Construct the right session for a spec.
pub fn start(spec: SessionSpec) -> Result<Box<dyn CliSession>, error::SessionError> {
    match spec.cli {
        // agy uses the PTY hybrid adapter on desktop; it is not built for mobile
        // (no portable-pty), where it falls through to the structured path.
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        CliKind::Agy => Ok(Box::new(hybrid::HybridSession::start(spec)?)),
        _ => Ok(Box::new(structured::StructuredSession::start(spec)?)),
    }
}
