//! StructuredAdapter — spawn a structured CLI, collect stdout, parse + reconcile,
//! emit CliEvents on a channel. One impl, parameterised per CLI via StructuredCfg.

use crate::cli_session::error::SessionError;
use crate::cli_session::event::{CliEvent, EventKind, Fidelity, Source};
use crate::cli_session::{normalizer, parse, CliKind, CliSession, SessionId, SessionSpec, TurnInput};
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

pub struct StructuredCfg {
    pub program: &'static str,
    pub base_args: Vec<String>,
    pub parse: fn(&[&str]) -> Vec<CliEvent>,
}

impl StructuredCfg {
    pub fn for_kind(kind: CliKind) -> Option<Self> {
        let s = |x: &str| x.to_string();
        match kind {
            CliKind::Claude => Some(StructuredCfg {
                program: "claude",
                base_args: vec![s("-p"), s("--output-format"), s("stream-json"),
                    s("--verbose"), s("--permission-mode"), s("dontAsk")],
                parse: parse::parse_claude_stream,
            }),
            CliKind::Codex => Some(StructuredCfg {
                program: "codex",
                // --dangerously-bypass-approvals-and-sandbox: codex's default sandbox
                // is READ-ONLY, so without this a driven codex cannot edit files (the
                // distributed-dev worker needs writes). Same flag ask.sh uses. Safety
                // for unattended runs is the L1 governor + the master's PR review.
                base_args: vec![
                    s("exec"),
                    s("--dangerously-bypass-approvals-and-sandbox"),
                    s("--json"),
                    s("--skip-git-repo-check"),
                ],
                parse: parse::parse_codex_jsonl,
            }),
            CliKind::Opencode => Some(StructuredCfg {
                program: "opencode",
                base_args: vec![s("run"), s("--format"), s("json")],
                parse: parse::parse_opencode_json,
            }),
            CliKind::External(spec) => Some(StructuredCfg {
                program: spec.program,
                // Registry args become the base; the spec's MUST-stay-last arg is the
                // final entry. Governed runs splice extra_args between base_args and the
                // prompt — External gateways are observed-post-action, so this is safe.
                base_args: spec.args.iter().map(|a| a.to_string()).collect(),
                parse: match spec.output_style {
                    crate::cli_session::external_gateway::ExternalOutputStyle::JsonPayload => {
                        parse::parse_external_json
                    }
                    crate::cli_session::external_gateway::ExternalOutputStyle::PlainText => {
                        parse::parse_external_plain
                    }
                },
            }),
            CliKind::Agy => None, // agy uses HybridSession
        }
    }
}

pub struct StructuredSession {
    cfg: StructuredCfg,
    spec: SessionSpec,
    id: Option<SessionId>,
}

impl StructuredSession {
    pub fn start(spec: SessionSpec) -> Result<Self, SessionError> {
        let cfg = StructuredCfg::for_kind(spec.cli)
            .ok_or_else(|| SessionError::Transport("not a structured CLI".into()))?;
        Ok(Self { cfg, spec, id: None })
    }
}

impl CliSession for StructuredSession {
    fn session_id(&self) -> Option<&SessionId> { self.id.as_ref() }
    fn resumable(&self) -> bool {
        // External gateways are one-shot; only claude/codex/opencode keep session state.
        matches!(self.spec.cli, CliKind::Claude | CliKind::Codex | CliKind::Opencode)
    }
    fn turn(&mut self, input: TurnInput) -> Result<Receiver<CliEvent>, SessionError> {
        let mut args = self.cfg.base_args.clone();
        // L1 governor flags (e.g. claude's PreToolUse-hook `--settings`) ride AFTER
        // the base args and BEFORE the positional prompt. Empty for ungoverned runs.
        //
        // If extra_args OVERRIDES a base value flag, drop the base copy so we do NOT
        // depend on the CLI's "last flag wins" (fragile across versions — a future
        // claude could honor the first `--permission-mode` and run in the base
        // `dontAsk` mode, SKIPPING the governor hook = a fail-open). Currently only
        // `--permission-mode` is overridden; the governor sets it to `default`.
        for flag in ["--permission-mode"] {
            if self.spec.extra_args.iter().any(|a| a == flag) {
                if let Some(i) = args.iter().position(|a| a == flag) {
                    let end = (i + 1).min(args.len() - 1);
                    args.drain(i..=end); // remove the base flag AND its value
                }
            }
        }
        args.extend(self.spec.extra_args.iter().cloned());
        args.push(input.prompt);
        #[cfg(windows)]
        let mut command = {
            // npm global bins on Windows are .cmd shims that CreateProcess can't exec
            // directly — route through cmd /c.
            let mut c = Command::new("cmd");
            c.arg("/C").arg(self.cfg.program).args(&args);
            c
        };
        #[cfg(not(windows))]
        let mut command = {
            let mut c = Command::new(self.cfg.program);
            c.args(&args);
            c
        };
        // Spawn (not `.output()`): `.output()` blocks with NO timeout, so a hung
        // claude/codex/opencode would hang the session forever. We enforce
        // `spec.timeout_secs` with a deadline + kill, the way agy's HybridSession
        // already does. stdout is collected, stderr is drained (both on threads so
        // a full pipe buffer can't deadlock the child).
        let mut child = command
            .current_dir(&self.spec.cwd)
            .envs(self.spec.env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    SessionError::CliNotFound(self.cfg.program.to_string())
                } else {
                    SessionError::SpawnFailed(e.to_string())
                }
            })?;

        let mut cout = child.stdout.take().expect("stdout was piped");
        let (otx, orx) = channel::<Vec<u8>>();
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = cout.read_to_end(&mut buf);
            let _ = otx.send(buf);
        });
        if let Some(mut cerr) = child.stderr.take() {
            std::thread::spawn(move || {
                let mut sink = Vec::new();
                let _ = cerr.read_to_end(&mut sink); // drain so stderr can't deadlock
            });
        }

        let deadline = Instant::now() + Duration::from_secs(self.spec.timeout_secs.max(1));
        // Surface a nonzero exit / timeout-kill so a failed run isn't read as success.
        let mut run_failed = false;
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    run_failed = !status.success();
                    break;
                } // exited on its own
                Ok(None) => {
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        run_failed = true;
                        // Best-effort reap, BOUNDED so a rare D-state child can't
                        // re-hang turn(). LIMITATION: kill() terminates only the
                        // spawned process (the `cmd /C` / `sh` wrapper, or the CLI
                        // itself) — a CLI that forked background helpers can be
                        // orphaned and keep the stdout pipe open, leaking the reader
                        // thread until it exits. Full process-tree termination (job
                        // objects on Windows / process groups on *nix) is a follow-up;
                        // this guarantees turn() RETURNS at the deadline either way.
                        let reap = Instant::now() + Duration::from_millis(600);
                        while Instant::now() < reap {
                            if let Ok(Some(_)) = child.try_wait() {
                                break;
                            }
                            std::thread::sleep(Duration::from_millis(30));
                        }
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(_) => {
                    run_failed = true;
                    break;
                }
            }
        }

        // Collect whatever stdout was produced (the reader thread ends on EOF,
        // which a kill triggers); brief wait so a just-exited child's tail lands.
        let body_bytes = orx.recv_timeout(Duration::from_secs(2)).unwrap_or_default();
        let body = String::from_utf8_lossy(&body_bytes).into_owned();
        let lines: Vec<&str> = body.lines().collect();
        let live = (self.cfg.parse)(&lines);
        let mut events = normalizer::reconcile(live, Vec::new());
        // A nonzero exit / timeout-kill with NO answer must surface as an error rather
        // than a silent empty turn — critical for external one-shot gateways whose
        // failures are not otherwise visible in the parsed stdout.
        if run_failed
            && !events.iter().any(|e| matches!(e.event, EventKind::AssistantText { .. }))
        {
            events.push(CliEvent::new(
                EventKind::Error {
                    error_kind: "cli_failed".into(),
                    detail: format!(
                        "{} exited with failure or was killed at the timeout and produced no answer",
                        self.cfg.program
                    ),
                },
                Fidelity::StructuredBestEffort,
                Source::LiveStream,
            ));
        }
        for e in &events {
            if let EventKind::SessionStarted { id } = &e.event {
                self.id = Some(id.clone());
                break;
            }
        }
        let (tx, rx) = channel();
        for e in events { let _ = tx.send(e); }
        Ok(rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A "CLI" that just sleeps, proving the deadline watchdog kills a hung child
    /// instead of hanging `turn()` forever (the bug: `.output()` had no timeout).
    /// *nix only (uses sh/sleep); the WSL test run exercises it. The Windows spawn
    /// path differs (`cmd /C`) but the watchdog logic is identical.
    #[test]
    #[cfg(not(windows))]
    fn turn_kills_a_hung_cli_at_the_deadline() {
        let mut session = StructuredSession {
            cfg: StructuredCfg {
                program: "sh",
                base_args: vec!["-c".into(), "sleep 8".into()],
                parse: parse::parse_codex_jsonl, // parser irrelevant; killed output is empty
            },
            spec: SessionSpec::new(CliKind::Codex, std::env::temp_dir(), 1, None),
            id: None,
        };
        let start = Instant::now();
        let _rx = session
            .turn(TurnInput { prompt: "ignored".into() })
            .expect("turn returns even when the CLI hangs");
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_secs(5),
            "a hung CLI must be killed at the ~1s deadline, not run to 8s; took {elapsed:?}"
        );
    }
}
