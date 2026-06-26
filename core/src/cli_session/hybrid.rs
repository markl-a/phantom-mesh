//! HybridAdapter (agy) — capture plain-text stdout through a real PTY (so antigravity
//! #76 can't silent-drop it) via portable-pty, tail the --log-file klog, parse.
//! Native Rust port of the shipped .claude/skills/local-ai/agy_pty.py technique.

use crate::cli_session::error::SessionError;
use crate::cli_session::event::{CliEvent, EventKind};
use crate::cli_session::{parse, CliKind, CliSession, SessionId, SessionSpec, TurnInput};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::Read;
use std::sync::mpsc::{channel, Receiver};

pub struct HybridSession {
    spec: SessionSpec,
    id: Option<SessionId>,
}

impl HybridSession {
    pub fn start(spec: SessionSpec) -> Result<Self, SessionError> {
        if spec.cli != CliKind::Agy {
            return Err(SessionError::Transport("HybridSession is agy-only".into()));
        }
        Ok(Self { spec, id: None })
    }
}

impl CliSession for HybridSession {
    fn session_id(&self) -> Option<&SessionId> { self.id.as_ref() }
    fn resumable(&self) -> bool { false } // agy #7: no reliable resume

    fn turn(&mut self, input: TurnInput) -> Result<Receiver<CliEvent>, SessionError> {
        let klog = std::env::temp_dir().join(format!("agy-l0-{}.log", std::process::id()));
        let pty = native_pty_system();
        let pair = pty.openpty(PtySize { rows: 40, cols: 200, pixel_width: 0, pixel_height: 0 })
            .map_err(|e| SessionError::Transport(e.to_string()))?;
        let mut cmd = CommandBuilder::new("agy");
        cmd.arg("-p");
        cmd.arg(&input.prompt);
        cmd.arg("--log-file");
        cmd.arg(klog.to_string_lossy().to_string());
        cmd.cwd(self.spec.cwd.to_string_lossy().to_string());
        let mut child = pair.slave.spawn_command(cmd)
            .map_err(|e| SessionError::SpawnFailed(e.to_string()))?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader()
            .map_err(|e| SessionError::Transport(e.to_string()))?;
        use std::sync::mpsc::channel as chan;
        use std::time::{Duration, Instant};
        let (ctx, crx) = chan::<Vec<u8>>();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => { if ctx.send(buf[..n].to_vec()).is_err() { break; } }
                    Err(_) => break,
                }
            }
        });
        let mut raw: Vec<u8> = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(self.spec.timeout_secs.max(1));
        let mut grace_until: Option<Instant> = None;
        loop {
            if let Ok(chunk) = crx.recv_timeout(Duration::from_millis(200)) {
                raw.extend_from_slice(&chunk);
            }
            match grace_until {
                None => match child.try_wait() {
                    Ok(Some(_)) => grace_until = Some(Instant::now() + Duration::from_millis(800)),
                    Ok(None) => { if Instant::now() > deadline { let _ = child.kill(); break; } }
                    Err(_) => break,
                },
                Some(g) => { if Instant::now() > g { break; } }
            }
        }
        drop(pair.master);
        let stdout = String::from_utf8_lossy(&raw).into_owned();
        let klog_body = std::fs::read_to_string(&klog).unwrap_or_default();
        let klog_lines: Vec<&str> = klog_body.lines().collect();
        let events = parse::parse_agy(&stdout, &klog_lines);
        let _ = std::fs::remove_file(&klog);
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
