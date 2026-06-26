//! Live smoke: drive each REAL CLI once. #[ignore] (costs quota + needs the CLI
//! installed + authed). Run on z13:
//!   cd core && cargo test --test cli_session_live_smoke -- --ignored --nocapture
use phantom_mesh::cli_session::{self, event::EventKind, CliKind, SessionSpec, TurnInput};

fn smoke(cli: CliKind) {
    let spec = SessionSpec::new(cli, std::env::temp_dir(), 120, None);
    let mut session = match cli_session::start(spec) {
        Ok(s) => s,
        Err(e) => { eprintln!("skip {cli:?}: {e}"); return; }
    };
    let rx = match session.turn(TurnInput { prompt: "Reply with exactly one word: PONG".into() }) {
        Ok(rx) => rx,
        Err(e) => { eprintln!("skip {cli:?}: turn error: {e}"); return; }
    };
    let events: Vec<_> = rx.iter().collect();
    eprintln!("{cli:?} produced {} events: {events:?}", events.len());
    assert!(events.iter().any(|e| matches!(e.event, EventKind::AssistantText { .. }))
            || events.iter().any(|e| matches!(e.event, EventKind::TurnDone { .. })),
        "{cli:?} produced no AssistantText/TurnDone");
}

#[test] #[ignore] fn smoke_claude()   { smoke(CliKind::Claude); }
#[test] #[ignore] fn smoke_codex()    { smoke(CliKind::Codex); }
#[test] #[ignore] fn smoke_opencode() { smoke(CliKind::Opencode); }
#[test] #[ignore] fn smoke_agy()      { smoke(CliKind::Agy); }
