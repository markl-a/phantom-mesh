//! Full-screen ratatui TUI for `phantom tui`.
//!
//! Layout:
//!   ┌─────────────── top status bar (1 line) ──────────────┐
//!   │  middle: scrollable transcript                        │
//!   │                                                       │
//!   │                                                       │
//!   ├─── bottom: multi-line input (1..=5 lines, grows) ────┤
//!   └───────────────────────────────────────────────────────┘
//!
//! Keys:
//!   Enter        → submit
//!   Shift-Enter  → newline
//!   Alt-Enter    → newline (fallback)
//!   Esc          → cancel running stream OR clear input
//!   Ctrl-C       → exit
//!   Ctrl-L       → clear input
//!   PageUp/Down  → scroll transcript
//!   Tab          → cycle agent (master → coder → reviewer → researcher)
//!
//! The TUI reuses `AgentRuntime`, `ConversationStore`, `CostTracker`, and
//! `WorkspaceContext::capture()` so existing config / sessions Just Work.

use std::io::{self, Stdout};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers, KeyboardEnhancementFlags, MouseButton, MouseEventKind,
        PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame, Terminal,
};
use tokio::sync::mpsc;

use crate::agent::{AgentEvent, AgentRuntime};
use crate::interrupt::InterruptHandle;
use crate::context::WorkspaceContext;
use crate::cost::CostTracker;
use crate::providers::traits::ChatMessage;
use crate::session::ConversationStore;

const AGENTS: &[&str] = &["master", "coder", "reviewer", "researcher"];
const TUI_HISTORY_MAX: usize = 100;
/// Per-entry byte cap when persisting. Without this, accidental paste-bombs
/// (or fuzz tests that send 15 KB of random input) ride the history file
/// and on the next Up-arrow recall the input box ballooned with multi-KB
/// garbage, visually swallowing the prompt. The cap is generous enough for
/// realistic prompts including pasted stack traces but bounds the worst case.
const TUI_HISTORY_ENTRY_MAX_BYTES: usize = 4096;

/// Path to ~/.phantom-mesh/tui-history (one prompt per line, oldest → newest).
fn tui_history_path() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|h| h.join(".phantom-mesh").join("tui-history"))
}

/// Read persisted history into the in-memory ring at TUI startup. Best-
/// effort — missing file / parse errors yield an empty Vec. The result is
/// capped at the last `TUI_HISTORY_MAX` entries so a bloated file (e.g. one
/// that grew before compaction kicked in) doesn't push the cursor to garbage
/// when the user presses Up.
///
/// Reads as bytes and lossy-decodes so a single invalid UTF-8 sequence
/// somewhere in the file (paste of binary, ANSI escapes, fuzzed bytes) does
/// not silently disable the entire history feature. `read_to_string` would
/// have rejected the whole file in that case.
fn load_tui_history() -> Vec<String> {
    let Some(path) = tui_history_path() else { return Vec::new(); };
    let Ok(bytes) = std::fs::read(&path) else { return Vec::new(); };
    let content = String::from_utf8_lossy(&bytes);
    let all: Vec<String> = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        // Decode the multi-line marker we encode on save.
        .map(|l| l.replace(" ⏎ ", "\n"))
        // Defensively drop entries that exceed the persist cap. Anything
        // beyond this size is almost certainly bot-pasted noise — surfacing
        // it as the "newest" recall entry just hides the real prompt.
        .filter(|s| s.len() <= TUI_HISTORY_ENTRY_MAX_BYTES)
        .collect();
    if all.len() <= TUI_HISTORY_MAX { return all; }
    let drop_n = all.len() - TUI_HISTORY_MAX;
    all.into_iter().skip(drop_n).collect()
}

/// Append a single prompt to ~/.phantom-mesh/tui-history. Encodes newlines
/// to " ⏎ " so each prompt stays one line. Atomic-ish (open append, write,
/// close) — no .tmp+rename because a partial line is recoverable on next read.
/// Entries exceeding `TUI_HISTORY_ENTRY_MAX_BYTES` after encoding are skipped
/// rather than truncated — a half-prompt is worse than no record.
fn append_tui_history(prompt: &str) {
    use std::io::Write;
    let Some(path) = tui_history_path() else { return; };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let line = prompt.replace('\n', " ⏎ ");
    if line.len() > TUI_HISTORY_ENTRY_MAX_BYTES { return; }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{}", line);
    }
}

/// Trim the persisted history file when it grows past TUI_HISTORY_MAX*2
/// lines (lazy compaction — avoids a write on every submit). Keeps the
/// last TUI_HISTORY_MAX entries.
fn maybe_compact_tui_history() {
    let Some(path) = tui_history_path() else { return; };
    // Read as bytes + lossy decode so a paste-bomb containing invalid UTF-8
    // doesn't permanently disable compaction. `read_to_string` rejects the
    // whole file on the first bad sequence — that exact bug let a 20 MB
    // file accumulate in the wild because compaction silently no-op'd.
    let Ok(bytes) = std::fs::read(&path) else { return; };
    let content = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = content.lines().collect();
    if lines.len() <= TUI_HISTORY_MAX * 2 {
        return;
    }
    let keep_from = lines.len() - TUI_HISTORY_MAX;
    let new_content: String = lines[keep_from..].join("\n") + "\n";
    let _ = std::fs::write(&path, new_content);
}

/// Items that appear in the scrollable transcript region.
#[derive(Debug, Clone)]
enum TranscriptItem {
    User(String),
    Assistant(String),
    AssistantPartial(String), // streaming, mutated in place
    /// Per-turn reasoning trace, accumulated as Thinking deltas arrive and
    /// rendered as a single dim-italic block above the assistant's answer.
    ThinkingPartial(String),
    Thinking(String),
    ToolCall { name: String, args: String },
    ToolResult { name: String, output: String },
    System(String),
    Error(String),
    /// Non-fatal red heads-up — currently used for "response truncated by
    /// max_tokens cap". Visually distinct from `Error` (which sets a fatal
    /// tone) so the user knows the run will continue and the partial reply
    /// they see is intentional output, just incomplete.
    Warning(String),
}

/// UI events flowing from the agent task or input loop into the render loop.
#[derive(Debug)]
enum UiEvent {
    Token(String),
    Thinking(String),
    ToolStart { name: String, args: String },
    ToolDone { name: String, output: String },
    Done { output: String, elapsed: f64 },
    Error(String),
    /// Non-fatal heads-up from the agent layer (e.g. AgentEvent::Notice).
    Notice(String),
}

struct AppState {
    transcript: Vec<TranscriptItem>,
    input: String,
    /// Cursor byte offset into `input`.
    cursor: usize,
    /// Lines scrolled from the bottom of the transcript (0 = bottom-pinned).
    scroll: u16,
    agent_idx: usize,
    /// True while an agent run is in flight.
    running: bool,
    /// Last + session cost cached for the status bar.
    last_cost: f64,
    session_cost: f64,
    chat_id: String,
    /// Recent submitted prompts (Up/Down arrow recall). Capped at 100.
    history: Vec<String>,
    /// Cursor into history when navigating with Up/Down. None = not navigating.
    /// 0 = oldest, history.len()-1 = newest.
    history_idx: Option<usize>,
    /// Saved current input when starting history navigation, restored on Down past newest.
    history_saved: String,
    /// Per-session model override set by /model — None = use agent's
    /// configured default. Same semantics as the REPL's `model_override`.
    model_override: Option<String>,
    /// /mouse on|off slash sets this to Some(target_state); the main
    /// loop calls EnableMouseCapture / DisableMouseCapture and clears
    /// the field. None = no transition pending. Tracks the live state
    /// so /mouse status can report it.
    mouse_capture_pending: Option<bool>,
    /// Mirrors the actual mouse-capture state currently in effect on
    /// the terminal (initialized at setup_terminal time).
    mouse_capture_active: bool,
    /// /freeze sets this true → main loop skips redraw so the terminal's
    /// drag-selection isn't erased on every frame. /resume clears it.
    /// Old conhost / some terminals erase mouse selection on any screen
    /// modification, even an idempotent redraw, so freezing the redraw
    /// is the only reliable way to copy text out of those terminals.
    render_frozen: bool,
    /// `/models` populates this with the `provider:model` entries it just
    /// printed, in display order. The `/pick` slash references those by
    /// row number so users don't have to retype long ids:
    ///     /models               → numbers each row 1..N
    ///     /pick master 3 17 5   → use last_models_list[2], [16], [4]
    ///                             as agent.master.providers = [...]
    last_models_list: Vec<String>,
    /// In-progress / completed mouse-drag text selection. `None` while the
    /// user is just looking at the screen. Set on `Down(Left)`, updated on
    /// every `Drag(Left)`, and on `Up(Left)` we extract the highlighted
    /// cells from the just-rendered terminal buffer, push the text to the
    /// OS clipboard, and leave the highlight visible until any keystroke
    /// or the next `Down(Left)` clears it. Lets the user select-and-copy
    /// without having to flip mouse capture off and back on.
    selection: Option<Selection>,
    /// Right-side sidebar visibility (~36 cols) listing local agents +
    /// remote peers for at-a-glance "what targets exist". Toggled via
    /// /sidebar. Also auto-hidden when terminal is too narrow (<100 cols).
    /// Default ON because discoverability of cluster peers is the main
    /// thing it adds — invisible-by-default would defeat the purpose.
    sidebar_visible: bool,
    /// Index into the combined target list [local_agents..., peers...]
    /// for the sidebar selection cursor. Shift+Down/Up cycles. Pure UI
    /// state — does NOT change the active agent. Committing a selection
    /// is via Tab (for agents) or `@<name>` prefix (for peers/agents).
    sidebar_focus: usize,
    /// Timestamp of the most recent Ctrl-C press. Used to implement
    /// aider-style double-tap-to-exit: the first press cancels (if
    /// running) or arms a 2-second exit window (if idle); a second
    /// press within that window actually exits. Cleared on any other
    /// keystroke so a stray press an hour later doesn't quit on the
    /// next Ctrl-C.
    last_ctrl_c_at: Option<Instant>,
    /// Cached peer names from peers.json read at TUI launch. Refreshed
    /// when /sidebar refresh is invoked. Empty when not logged in or
    /// peers.json doesn't exist (sidebar shows "no peers" hint).
    sidebar_peers: Vec<String>,
    /// Modal interactive picker for editing [agent.X].providers priority.
    /// Opened by `/priority` slash; swallows all key events while open.
    /// On Enter, saves back to agents.toml. On Esc, discards changes.
    priority_picker: Option<PriorityPicker>,
}

/// Modal state for the /priority popup.
#[derive(Debug, Clone)]
struct PriorityPicker {
    /// Which [agent.<name>] block we're editing.
    agent_name: String,
    /// Current order of `provider:model` entries (live, edits land here
    /// before commit).
    items: Vec<String>,
    /// Cursor row.
    focused: usize,
    /// True if reorder/remove happened since open — controls the title
    /// indicator and (eventually) confirm-on-discard behavior.
    dirty: bool,
}

/// Linear text selection in screen-cell coordinates. Anchor is where the
/// drag started; cursor is the current end. `dragging=true` while the
/// mouse button is held; goes false on `Up`. The selection persists past
/// the up event so the highlight reads as confirmation that the copy
/// happened — any other input (keystroke, new mouse-down) clears it.
#[derive(Debug, Clone, Copy)]
struct Selection {
    anchor: (u16, u16),  // (col, row) at Down(Left)
    cursor: (u16, u16),  // (col, row) latest Drag/Up
    dragging: bool,
}

impl Selection {
    /// Normalize anchor/cursor so we always iterate top-left → bottom-right.
    /// Linear (text-flow) order, NOT rectangular: the start row's columns
    /// run from `start.0` to end-of-line, middle rows are full-width, last
    /// row's columns run from 0 to `end.0`. Same as how a normal text
    /// editor selection works when you drag across multiple lines.
    fn normalized(&self) -> ((u16, u16), (u16, u16)) {
        let (a, b) = (self.anchor, self.cursor);
        if (a.1, a.0) <= (b.1, b.0) { (a, b) } else { (b, a) }
    }

    /// True when the selection covers at least one cell. A pure click with
    /// no drag (anchor == cursor) is treated as "not really a selection"
    /// so it doesn't trigger an empty-string copy.
    fn is_meaningful(&self) -> bool {
        self.anchor != self.cursor
    }
}

impl AppState {
    fn agent_name(&self) -> &'static str { AGENTS[self.agent_idx] }
    fn cycle_agent(&mut self) { self.agent_idx = (self.agent_idx + 1) % AGENTS.len(); }
}

/// Public entry point invoked by `phantom tui`.
pub async fn run_tui(
    runtime: AgentRuntime,
    conversations: ConversationStore,
    cost_tracker: CostTracker,
    chat_id: String,
    initial_agent: String,
    extra_context: String,
) -> Result<()> {
    // Stdin must be a TTY for raw mode; bail with a helpful message otherwise
    // (e.g. when launched under `echo "" | phantom tui`).
    use std::io::IsTerminal;
    if !io::stdin().is_terminal() {
        eprintln!("phantom tui: stdin is not a terminal — TUI requires an interactive shell.");
        return Ok(());
    }

    let mut terminal = setup_terminal()?;
    let agent_idx = AGENTS.iter().position(|a| *a == initial_agent).unwrap_or(0);

    // Load persisted history from disk so Up/Down recall survives
    // restarts. File is one-prompt-per-line, capped at MAX_HISTORY (100)
    // most-recent. Newlines inside multi-line prompts are encoded as " ⏎ "
    // (matches the REPL's own convention) so each entry stays one line.
    // Also opportunistically compact on startup — the on-submit compaction
    // never fires if the user quits without ever pressing Enter, so a file
    // polluted by a fuzz run or accidental paste-bomb otherwise stays
    // bloated forever.
    maybe_compact_tui_history();
    let history_init = load_tui_history();

    // Mouse capture defaults ON so the wheel scrolls the transcript out of
    // the box (the standard terminal expectation). Users who need to drag-
    // select text for copy run `/mouse off` (or set `PHANTOM_MOUSE=0`) to
    // hand drag events back to the terminal. Reversed from the previous
    // opt-in default after user feedback that "wheel = scroll output" is
    // the obvious binding and dragging-to-copy is the rarer case worth a
    // single command.
    let initial_mouse = std::env::var("PHANTOM_MOUSE")
        .map(|v| !(v == "0" || v.eq_ignore_ascii_case("off") || v.eq_ignore_ascii_case("false")))
        .unwrap_or(true);
    let app = Arc::new(Mutex::new(AppState {
        transcript: Vec::new(),
        input: String::new(),
        cursor: 0,
        scroll: 0,
        agent_idx,
        running: false,
        last_cost: 0.0,
        session_cost: 0.0,
        chat_id: chat_id.clone(),
        history: history_init,
        history_idx: None,
        history_saved: String::new(),
        model_override: None,
        mouse_capture_pending: None,
        mouse_capture_active: initial_mouse,
        render_frozen: false,
        last_models_list: Vec::new(),
        selection: None,
        sidebar_visible: true,
        sidebar_focus: 0,
        sidebar_peers: crate::cli_config::read_peers_json()
            .map(|peers| peers.into_iter()
                .map(|p| p.name)
                .filter(|n| Some(n.as_str()) != crate::cli_config::resolve_self_node_name().as_deref())
                .collect())
            .unwrap_or_default(),
        priority_picker: None,
        last_ctrl_c_at: None,
    }));

    // Welcome banner
    {
        let mut s = app.lock().unwrap();
        s.transcript.push(TranscriptItem::System(format!(
            "phantom tui — agent: {} · session: {} · type a message and press Enter (Tab cycles agents, Esc cancels, Ctrl-C twice to exit)",
            AGENTS[agent_idx],
            &chat_id[..chat_id.len().min(12)]
        )));
    }

    // Live presence: register a session with the broker so other machines
    // can see "this TUI is open on Z13 in /path/to/project". No-op when
    // not logged in (e.g. running purely local). Handle dropped after the
    // run loop exits → triggers best-effort DELETE of the row.
    let _session_handle = {
        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        crate::cli_config::start_session_heartbeat(AGENTS[agent_idx].to_string(), cwd)
    };

    let result = run_loop(
        &mut terminal,
        app.clone(),
        runtime,
        conversations,
        cost_tracker,
        extra_context,
    )
    .await;

    // Best-effort: ask the heartbeat task to send DELETE before the
    // process exits. If the channel is full or the task already
    // terminated, this just no-ops.
    if let Some(h) = &_session_handle {
        let _ = h.stop.try_send(());
        // Give the delete request ~200ms to flush; not strictly needed
        // (60s stale window will clean it up anyway) but makes
        // `phantom sessions` from another machine feel snappier.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }

    restore_terminal(&mut terminal)?;
    result
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    // Mouse capture defaults ON — wheel-to-scroll is the obvious binding
    // every terminal user expects. The trade is that the terminal can't
    // see drag events, so native click-and-drag text selection is blocked
    // until the user runs `/mouse off` (or sets PHANTOM_MOUSE=0). For
    // copying without toggling, /copy <all|turn|last> still works in any
    // mode and is the recommended way to get clean text into the clipboard.
    let want_mouse = std::env::var("PHANTOM_MOUSE")
        .map(|v| !(v == "0" || v.eq_ignore_ascii_case("off") || v.eq_ignore_ascii_case("false")))
        .unwrap_or(true);
    if want_mouse {
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    } else {
        execute!(stdout, EnterAlternateScreen)?;
    }

    // Best-effort: push the kitty keyboard protocol flags so the terminal
    // sends a distinct sequence for Shift+Enter, Ctrl+Enter, etc. Without
    // this, Shift+Enter is indistinguishable from Enter on most terminals
    // and 'newline-in-input' silently submits the message instead.
    //
    // Supported by: kitty, wezterm, iTerm2 (with the right setting),
    // foot, alacritty (recent). NOT supported by Apple Terminal.app —
    // we ignore the error so phantom still starts there.
    let _ = execute!(
        io::stdout(),
        PushKeyboardEnhancementFlags(
            KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                | KeyboardEnhancementFlags::REPORT_EVENT_TYPES,
        )
    );

    let backend = CrosstermBackend::new(stdout);
    Ok(Terminal::new(backend)?)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    // Pop the kbd enhancement flags before leaving alt-screen so the user's
    // original terminal state isn't left in kitty kbd mode.
    let _ = execute!(terminal.backend_mut(), PopKeyboardEnhancementFlags);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: Arc<Mutex<AppState>>,
    runtime: AgentRuntime,
    conversations: ConversationStore,
    cost_tracker: CostTracker,
    extra_context: String,
) -> Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel::<UiEvent>();
    let mut current_task: Option<tokio::task::JoinHandle<()>> = None;
    // Pairs 1:1 with `current_task`. Lets a second Enter (or any
    // SubmitWhileRunning) call `interrupt(Some(new_prompt))` instead of
    // hard-aborting the JoinHandle — the agent loop unwinds cleanly via
    // `core::interrupt::InterruptHandle` (see `agent.rs::is_interrupted`)
    // and the queued message is picked up on the next top-of-loop pass.
    let mut current_interrupt: Option<InterruptHandle> = None;

    // Frame-rate cap for streaming output. Without this, every Token
    // event drained from `rx` triggers an immediate `redraw → continue
    // → terminal.draw()`, hammering ratatui's layout pass at 1000+ Hz
    // on hot streams (Anthropic emits ~50 tokens/s, openrouter bursts
    // higher). Capping at FRAME_BUDGET draws cleanly at ~30 fps and
    // lets the inter-event drain in step 2 batch tokens between
    // frames, so each redraw renders the cumulative AssistantPartial
    // in one shot. Codex calls this newline-gated commit; ours is the
    // simpler time-gated variant since the markdown-aware renderer
    // already coalesces appends inside `TranscriptItem::AssistantPartial`.
    const FRAME_BUDGET: Duration = Duration::from_millis(33); // ≈30 fps
    let mut last_draw = Instant::now() - FRAME_BUDGET;       // draw the very first frame
    // Set to true after a key event mutates state so typing latency
    // doesn't get held up behind FRAME_BUDGET. Cleared after the next
    // draw. Streaming events do NOT set this — they should ride the
    // frame cap to keep the redraw rate sane on hot streams.
    let mut force_redraw_next: bool = false;
    // Diagnostic: track last-seen state of the agent task so we log
    // the running→finished transition exactly once per turn instead
    // of polling-flood the events log. Also lets us verify the chain
    // check IS evaluating each iteration (the absence of this event
    // points at the loop being stuck instead of the chain logic
    // misfiring).
    let mut last_task_finished_state: Option<bool> = None;

    // Helper: spawn a single agent turn. Centralises the 60-line
    // dispatch path that runs both for keyboard Submit and for the
    // chained follow-up turn after a graceful interrupt. Returns the
    // task handle + the fresh InterruptHandle bound to that turn.
    #[allow(clippy::too_many_arguments)]
    fn spawn_agent_turn(
        prompt: String,
        agent_name: String,
        chat_id: String,
        history: Vec<ChatMessage>,
        runtime: AgentRuntime,
        cost: CostTracker,
        conv: ConversationStore,
        extra: String,
        tx: mpsc::UnboundedSender<UiEvent>,
    ) -> (tokio::task::JoinHandle<()>, InterruptHandle) {
        let interrupt = InterruptHandle::new();
        let runtime = runtime.with_interrupt(interrupt.clone());
        let prompt_for_save = prompt.clone();
        // Cloned for the spawned task so we can tell, after the run
        // returns Ok, whether it returned because of a cooperative
        // interrupt (Ctrl-C / submit-while-running redirect) vs. a
        // normal completion. The agent runtime always returns Ok on
        // interrupt — even for empty output (see agent.rs:680) — so
        // this flag is the only way to distinguish, and it's what
        // lets the next turn's LLM context see why we stopped.
        let interrupt_for_check = interrupt.clone();
        let tx_outer = tx.clone();
        let handle = tokio::spawn(async move {
            let tx_inner = tx_outer.clone();
            let handler = move |ev: AgentEvent| {
                let _ = match ev {
                    AgentEvent::Token { content } => {
                        tx_inner.send(UiEvent::Token(content))
                    }
                    AgentEvent::ToolStart { name, args_preview } => {
                        tx_inner.send(UiEvent::ToolStart { name, args: args_preview })
                    }
                    AgentEvent::ToolDone { name, output_preview } => {
                        tx_inner.send(UiEvent::ToolDone { name, output: output_preview })
                    }
                    AgentEvent::Thinking { content } => {
                        tx_inner.send(UiEvent::Thinking(content))
                    }
                    AgentEvent::Done { output, elapsed_secs, .. } => {
                        tx_inner.send(UiEvent::Done { output, elapsed: elapsed_secs })
                    }
                    AgentEvent::Notice { message } => {
                        tx_inner.send(UiEvent::Notice(message))
                    }
                };
            };
            let result = runtime
                .run_with_callbacks(
                    &agent_name,
                    &prompt,
                    &history,
                    Some(&extra),
                    &cost,
                    handler,
                )
                .await;
            match result {
                Ok(r) => {
                    let user_msg = ChatMessage {
                        role: "user".into(),
                        content: prompt_for_save,
                        tool_calls: None,
                    };
                    // If the user cancelled (Ctrl-C) or redirected
                    // (Submit-while-running), append a marker to the
                    // assistant turn so the *next* LLM call sees why
                    // the previous reply was cut short. Mirrors aider's
                    // `^C KeyboardInterrupt` history append. Without
                    // this, the model sees an unexplained truncation
                    // and often re-starts from scratch — wasting the
                    // partial work the user is trying to redirect.
                    let content = if interrupt_for_check.is_cancelled() {
                        const MARKER: &str = "[interrupted by user before completion]";
                        let trimmed = r.output.trim();
                        if trimmed.is_empty() {
                            MARKER.to_string()
                        } else {
                            format!("{}\n\n{}", trimmed, MARKER)
                        }
                    } else {
                        r.output
                    };
                    let asst_msg = ChatMessage {
                        role: "assistant".into(),
                        content,
                        tool_calls: None,
                    };
                    conv.append(&chat_id, user_msg, asst_msg).await;
                }
                Err(e) => {
                    let _ = tx_outer.send(UiEvent::Error(format!("{}", e)));
                }
            }
        });
        (handle, interrupt)
    }

    loop {
        // ── 0a. Chain a queued follow-up turn from a graceful interrupt
        // ──────────────────────────────────────────────────────────────
        // The previous turn was unwound via InterruptHandle::interrupt(Some(msg)).
        // Now that its JoinHandle reports finished, spawn a fresh turn for
        // the queued message so the user's redirected prompt isn't dropped.
        let cur_finished = current_task.as_ref().map(|h| h.is_finished());
        if last_task_finished_state != cur_finished {
            crate::diag::record(
                "tui_task_state",
                format!("{:?} → {:?}", last_task_finished_state, cur_finished),
            );
            last_task_finished_state = cur_finished;
        }
        if cur_finished == Some(true) {
            let queued = current_interrupt.as_ref().and_then(|h| h.take_message());
            crate::diag::record(
                "tui_chain_check",
                format!("task finished; queued = {:?}",
                    queued.as_ref().map(|s| s.chars().take(60).collect::<String>())),
            );
            current_task = None;
            current_interrupt = None;
            if let Some(prompt) = queued {
                crate::diag::record(
                    "tui_redirect_chain",
                    format!(
                        "previous turn finished; spawning chained turn for queued prompt: {}",
                        prompt.chars().take(60).collect::<String>()
                    ),
                );
                // Pull chat_id out first so the MutexGuard drops before the
                // .await — otherwise the lock is held across the suspend
                // point and any other task (render loop, input handler)
                // touching `app` blocks until get_history returns.
                let chat_id = app.lock().unwrap().chat_id.clone();
                let history = conversations.get_history(&chat_id).await;
                let agent_name = app.lock().unwrap().agent_name().to_string();
                crate::diag::record(
                    "tui_redirect_chain",
                    format!("chained turn history len = {}", history.len()),
                );
                {
                    let mut s = app.lock().unwrap();
                    s.transcript.push(TranscriptItem::User(prompt.clone()));
                    s.running = true;
                    s.scroll = 0;
                }
                let (handle, interrupt) = spawn_agent_turn(
                    prompt, agent_name, chat_id, history,
                    runtime.clone(), cost_tracker.clone(),
                    conversations.clone(), extra_context.clone(),
                    tx.clone(),
                );
                current_task = Some(handle);
                current_interrupt = Some(interrupt);
            }
        }

        // ── 0. Apply any pending mouse-capture toggle (from /mouse slash) ──
        // Runs BEFORE the draw so the new state is in effect before any
        // input events that frame.
        {
            let mut s = app.lock().unwrap();
            if let Some(want) = s.mouse_capture_pending.take() {
                let result = if want {
                    execute!(terminal.backend_mut(), EnableMouseCapture)
                } else {
                    execute!(terminal.backend_mut(), DisableMouseCapture)
                };
                if result.is_ok() {
                    s.mouse_capture_active = want;
                }
                // On error: silently keep current state (don't crash a TUI
                // session over a terminal-feature negotiation hiccup).
            }
        }

        // ── 1. Draw frame (skipped when /freeze is active so the terminal
        //                  drag-selection isn't erased every iteration).
        //                  Also skipped when less than FRAME_BUDGET has
        //                  elapsed since the last draw — see the
        //                  declaration above for rationale. Key input
        //                  always forces a draw via `redraw_now` below
        //                  so typing remains zero-latency even when the
        //                  cap is active.
        let elapsed = last_draw.elapsed();
        if elapsed >= FRAME_BUDGET || force_redraw_next {
            let s = app.lock().unwrap();
            if !s.render_frozen {
                terminal.draw(|f| ui(f, &s))?;
                last_draw = Instant::now();
            }
            force_redraw_next = false;
        }

        // ── 2. Drain any pending UI events from the agent task (non-blocking).
        // Multiple Token events drained here all append to the same
        // `AssistantPartial` buffer, so the next eligible draw renders
        // the full burst in one frame instead of N intermediate frames.
        while let Ok(ev) = rx.try_recv() {
            handle_ui_event(&app, &cost_tracker, ev).await;
        }
        // Don't force-redraw on every event drain. The top-of-loop
        // FRAME_BUDGET check governs cadence; events accumulate into
        // app state and ride the next eligible frame.

        // ── 3. Poll for terminal input with a short timeout so we keep
        //      streaming UI events flowing into the render loop.
        if event::poll(Duration::from_millis(50))? {
            // Any terminal event (key, mouse) feels laggy if it has to
            // wait for FRAME_BUDGET to elapse before showing — bypass
            // the cap on the next iteration.
            force_redraw_next = true;
            match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press || k.kind == KeyEventKind::Repeat => {
                    let action = handle_key(&app, k);
                    match action {
                        KeyAction::Exit => break,
                        KeyAction::Submit(prompt) => {
                            // Slash-command shortcut — handled inline, never
                            // dispatched to the agent.
                            if prompt.starts_with('/') {
                                handle_tui_slash(&app, &runtime, &conversations, &cost_tracker, &prompt).await;
                                continue;
                            }
                            // @target prefix — `@<name> <prompt>` either
                            // switches the local agent (when <name> matches
                            // an agents.toml entry) or dispatches to a remote
                            // peer via cluster RPC (when <name> matches a
                            // peer in sidebar_peers). Bare `@<name>` with no
                            // body just switches focus.
                            let mut prompt = prompt;
                            if let Some(rest) = prompt.clone().strip_prefix('@') {
                                let (token, body) = match rest.split_once(char::is_whitespace) {
                                    Some((t, b)) => (t.trim().to_string(), b.trim().to_string()),
                                    None         => (rest.trim().to_string(), String::new()),
                                };
                                if !token.is_empty() {
                                    let local_idx = AGENTS.iter().position(|a| a.eq_ignore_ascii_case(&token));
                                    let is_peer = {
                                        let s = app.lock().unwrap();
                                        s.sidebar_peers.iter().any(|p| p.eq_ignore_ascii_case(&token))
                                    };
                                    if let Some(idx) = local_idx {
                                        {
                                            let mut s = app.lock().unwrap();
                                            s.agent_idx = idx;
                                            s.sidebar_focus = idx;
                                            if body.is_empty() {
                                                s.transcript.push(TranscriptItem::System(
                                                    format!("◆ → {}", AGENTS[idx])));
                                            }
                                        }
                                        if body.is_empty() { continue; }
                                        prompt = body;
                                    } else if is_peer {
                                        if body.is_empty() {
                                            let mut s = app.lock().unwrap();
                                            s.transcript.push(TranscriptItem::Warning(
                                                format!("@{} needs a prompt — try `@{} <task>`", token, token)));
                                            continue;
                                        }
                                        {
                                            let mut s = app.lock().unwrap();
                                            s.transcript.push(TranscriptItem::User(format!("@{} {}", token, body)));
                                            s.transcript.push(TranscriptItem::System(
                                                format!("◆ dispatching to {} …", token)));
                                        }
                                        let app_clone = app.clone();
                                        let token_c = token.clone();
                                        let body_c = body.clone();
                                        tokio::spawn(async move {
                                            let result = crate::cli_config::dispatch_lines(
                                                &[], Some(&token_c), "master", &body_c, false,
                                            ).await;
                                            let mut s = app_clone.lock().unwrap();
                                            match result {
                                                Ok(lines) => {
                                                    for l in lines {
                                                        s.transcript.push(TranscriptItem::System(l));
                                                    }
                                                }
                                                Err(e) => s.transcript.push(TranscriptItem::Error(
                                                    format!("@{} dispatch failed: {}", token_c, e))),
                                            }
                                        });
                                        continue;
                                    } else {
                                        let warn = {
                                            let s = app.lock().unwrap();
                                            format!("@{} not recognized — agents: {} · peers: {}",
                                                token, AGENTS.join(","),
                                                if s.sidebar_peers.is_empty() { "(none)".into() } else { s.sidebar_peers.join(",") })
                                        };
                                        let mut s = app.lock().unwrap();
                                        s.transcript.push(TranscriptItem::Warning(warn));
                                        continue;
                                    }
                                }
                            }
                            // Submit-while-running → graceful redirect. The
                            // current turn unwinds via InterruptHandle and
                            // the queued prompt is picked up by the
                            // top-of-loop chain check (block 0a above).
                            // Strictly preferable to a hard abort: the
                            // already-streamed assistant text + tool
                            // outputs stay in conversation history.
                            if current_task.as_ref().is_some_and(|h| !h.is_finished()) {
                                if let Some(ih) = &current_interrupt {
                                    let preview: String = prompt.chars().take(60).collect();
                                    crate::diag::record(
                                        "tui_redirect",
                                        format!("interrupt fired with new prompt: {}", preview),
                                    );
                                    ih.interrupt(Some(prompt.clone()));
                                    let mut s = app.lock().unwrap();
                                    s.transcript.push(TranscriptItem::System(
                                        format!("↺ redirecting → {}", preview)
                                    ));
                                    continue;
                                } else {
                                    crate::diag::record(
                                        "tui_redirect_skipped",
                                        "task running but no interrupt handle attached",
                                    );
                                }
                            }
                            // No turn in flight: spawn a fresh one.
                            // Read chat_id first so the MutexGuard drops
                            // before the await (clippy await_holding_lock).
                            let chat_id = app.lock().unwrap().chat_id.clone();
                            let history = conversations.get_history(&chat_id).await;
                            let agent_name = app.lock().unwrap().agent_name().to_string();
                            {
                                let mut s = app.lock().unwrap();
                                s.transcript.push(TranscriptItem::User(prompt.clone()));
                                s.running = true;
                                s.scroll = 0;
                            }
                            let (handle, interrupt) = spawn_agent_turn(
                                prompt, agent_name, chat_id, history,
                                runtime.clone(), cost_tracker.clone(),
                                conversations.clone(), extra_context.clone(),
                                tx.clone(),
                            );
                            current_task = Some(handle);
                            current_interrupt = Some(interrupt);
                        }
                        KeyAction::Cancel => {
                            // Graceful cancel. Setting the interrupt flag
                            // lets the agent loop wind down at the next
                            // safe point (round boundary or SSE-chunk
                            // race). The hard JoinHandle::abort() fallback
                            // remains as a 1-second safety net for tasks
                            // that don't honour cancellation (none today).
                            if let Some(ih) = current_interrupt.as_ref() {
                                ih.interrupt(None);
                            }
                            if let Some(h) = current_task.take() {
                                // Don't await — the main loop's
                                // `is_finished()` check will reap it.
                                let _ = h;
                                let mut s = app.lock().unwrap();
                                s.running = false;
                                s.transcript.push(TranscriptItem::System("⊗ stream cancelled".into()));
                            }
                            current_interrupt = None;
                        }
                        KeyAction::CopySelection(sel) => {
                            // Same logic as Up(MouseButton::Left) below —
                            // extract from the just-rendered terminal buffer
                            // and pipe through the OS clipboard helper.
                            // Lets Ctrl+C with a visible selection copy
                            // instead of exiting (matches Win Terminal /
                            // iTerm convention).
                            let text = extract_selection_text(
                                terminal.current_buffer_mut(),
                                sel,
                            );
                            if !text.is_empty() {
                                let chars = text.chars().count();
                                match copy_to_os_clipboard(&text) {
                                    Ok(cmd) => {
                                        let mut s = app.lock().unwrap();
                                        s.transcript.push(TranscriptItem::System(
                                            format!("  ✓ Ctrl+C copied {} chars via {} (selection-priority — press Ctrl+C again with no selection to exit)", chars, cmd)
                                        ));
                                    }
                                    Err(e) => {
                                        let mut s = app.lock().unwrap();
                                        s.transcript.push(TranscriptItem::Error(
                                            format!("  ✗ clipboard copy failed: {}", e)
                                        ));
                                    }
                                }
                            }
                        }
                        KeyAction::None => {}
                    }
                }
                Event::Mouse(m) => match m.kind {
                    // Wheel: scroll the transcript regardless of cursor
                    // position. This is the primary way to navigate the
                    // LLM output; ↑/↓ stays bound to input history.
                    MouseEventKind::ScrollUp => {
                        let mut s = app.lock().unwrap();
                        s.scroll = s.scroll.saturating_add(3);
                    }
                    MouseEventKind::ScrollDown => {
                        let mut s = app.lock().unwrap();
                        s.scroll = s.scroll.saturating_sub(3);
                    }
                    // Drag-to-select: start tracking on Down(Left). Any
                    // existing selection is replaced — clicking elsewhere
                    // discards the highlight.
                    MouseEventKind::Down(MouseButton::Left) => {
                        let mut s = app.lock().unwrap();
                        s.selection = Some(Selection {
                            anchor: (m.column, m.row),
                            cursor: (m.column, m.row),
                            dragging: true,
                        });
                    }
                    // Update the live cursor as the user drags. We only
                    // update if we already have a selection in `dragging`
                    // state — guards against spurious Drag events arriving
                    // without a preceding Down (some terminals do this on
                    // touchpad gestures).
                    MouseEventKind::Drag(MouseButton::Left) => {
                        let mut s = app.lock().unwrap();
                        if let Some(sel) = s.selection.as_mut() {
                            if sel.dragging {
                                sel.cursor = (m.column, m.row);
                            }
                        }
                    }
                    // Mouse up → finalize the selection, extract the text
                    // from the just-rendered terminal buffer, and push to
                    // the OS clipboard. Highlight stays on screen so the
                    // user has visual confirmation of what was copied;
                    // any keystroke (or the next Down) clears it.
                    MouseEventKind::Up(MouseButton::Left) => {
                        let sel_snapshot = {
                            let mut s = app.lock().unwrap();
                            if let Some(sel) = s.selection.as_mut() {
                                sel.dragging = false;
                                sel.cursor = (m.column, m.row);
                                Some(*sel)
                            } else { None }
                        };
                        if let Some(sel) = sel_snapshot {
                            // Always log the up event so phantom debug shows
                            // mouse activity for remote debugging.
                            let (a, b) = sel.normalized();
                            crate::diag::record("mouse_up_select",
                                format!("anchor=({},{}) cursor=({},{}) meaningful={}",
                                    a.0, a.1, b.0, b.1, sel.is_meaningful()));
                            if sel.is_meaningful() {
                                let text = extract_selection_text(
                                    terminal.current_buffer_mut(),
                                    sel,
                                );
                                if !text.is_empty() {
                                    let chars = text.chars().count();
                                    crate::diag::record("clipboard_copy_attempt",
                                        format!("chars={}", chars));
                                    match copy_to_os_clipboard(&text) {
                                        Ok(cmd) => {
                                            crate::diag::record("clipboard_copy_ok",
                                                format!("via {}", cmd));
                                            let mut s = app.lock().unwrap();
                                            s.transcript.push(TranscriptItem::System(
                                                format!("  ✓ selected {} chars copied via {}", chars, cmd)
                                            ));
                                        }
                                        Err(e) => {
                                            crate::diag::record("clipboard_copy_fail",
                                                format!("{}", e));
                                            let mut s = app.lock().unwrap();
                                            s.transcript.push(TranscriptItem::Error(
                                                format!("  ✗ clipboard copy failed: {}", e)
                                            ));
                                        }
                                    }
                                } else {
                                    // Selection covered cells but they were
                                    // all blank/whitespace — surface that so
                                    // the user knows the gesture was seen,
                                    // we just had nothing to copy.
                                    crate::diag::record("clipboard_copy_skip",
                                        "selection extracted to empty string (cells were blank)".to_string());
                                    let mut s = app.lock().unwrap();
                                    s.transcript.push(TranscriptItem::System(
                                        "  ◇ selection was empty (only blank cells) — nothing copied".into()
                                    ));
                                }
                            } else {
                                // Pure click with no drag — discard the
                                // empty selection so it doesn't render an
                                // invisible highlight on a single cell.
                                app.lock().unwrap().selection = None;
                            }
                        }
                    }
                    _ => {}
                },
                Event::Resize(_, _) => {
                    // Ratatui handles resize on next draw automatically; just
                    // force a redraw on the next loop iteration.
                }
                _ => {}
            }
        }

        // Reap finished task
        if let Some(h) = &current_task {
            if h.is_finished() {
                current_task = None;
            }
        }
    }

    if let Some(h) = current_task.take() { h.abort(); }
    Ok(())
}

enum KeyAction {
    None,
    Exit,
    Cancel,
    Submit(String),
    /// Ctrl+C with an active text selection — copy the highlighted cells
    /// to the OS clipboard instead of exiting. Carries the selection so
    /// the run_loop can extract text from `terminal.current_buffer_mut()`
    /// (handle_key doesn't have terminal access).
    CopySelection(Selection),
}

// Keep this list in sync with the match arms in handle_tui_slash. Adding a
// command here without a handler makes Tab completion suggest it, then submit
// it, then have the handler bail with 'unknown command' — which is the worst
// of both worlds. /show and /fork live in the REPL only; they aren't here.
const SLASH_COMMANDS_TUI: &[&str] = &[
    "/help", "/exit", "/clear", "/agent", "/agents", "/sessions", "/resume",
    "/tools", "/todo", "/cost", "/perm", "/density", "/theme",
    "/tasks", "/plan",
    // Config management — /keys writes to ~/.phantom-mesh/env (set/list/remove/test),
    // /provider lists providers + /provider priority edits failover order in agents.toml.
    "/keys", "/provider", "/providers", "/models", "/cluster",
    // Interactive priority picker — modal popup for arrow-key reorder of
    // [agent.X].providers. /priority [agent_name], default = active agent.
    "/priority", "/prio",
    // Multi-machine broadcast: send one prompt to every peer in parallel.
    // Optional `--agent <name>` to override which agent runs on the remote.
    "/fanout", "/broadcast",
    // Output management — /copy gets agent reply / turn / full session into
    // the OS clipboard. /mouse runtime-toggles mouse capture so users can
    // switch between drag-select-text mode and mouse-wheel-scroll mode
    // without restarting the TUI.
    "/copy", "/mouse",
    // /sidebar toggles the right-rail target picker (local agents +
    // remote peers).
    "/sidebar",
    // /freeze pauses ratatui's redraw so the terminal's drag-selection
    // isn't erased between frames. /resume returns to live updates.
    "/freeze", "/resume",
    // /pick — set agent.X.providers by selecting from the numbered list
    // produced by /models, no need to retype long provider:model ids.
    "/pick",
];

fn handle_key(app: &Arc<Mutex<AppState>>, k: KeyEvent) -> KeyAction {
    let mut s = app.lock().unwrap();

    // Defensive cursor normalisation: if some upstream code (history
    // restore, paste, mouse-position-derived cursor, etc.) set
    // `s.cursor` to a byte index that's mid-codepoint, snap it to the
    // nearest valid boundary BEFORE any handler does `&s.input[..pos]`
    // or `s.input.insert(pos, ...)`. Without this, a single corrupted
    // cursor value crashes the entire TUI mid-keystroke. Found via
    // `fuzz_handle_key_never_panics_with_random_initial_input`.
    if s.cursor > s.input.len() {
        s.cursor = s.input.len();
    } else if !s.input.is_char_boundary(s.cursor) {
        s.cursor = prev_char_boundary(&s.input, s.cursor);
    }

    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let shift = k.modifiers.contains(KeyModifiers::SHIFT);
    let alt = k.modifiers.contains(KeyModifiers::ALT);

    // ── Modal: priority picker swallows ALL keys when open ───────────────
    if s.priority_picker.is_some() {
        return handle_priority_picker_key(&mut s, k.code, shift);
    }

    // Any key other than Up/Down resets history navigation state.
    let reset_history = !matches!(k.code, KeyCode::Up | KeyCode::Down);
    if reset_history { s.history_idx = None; }

    // Snapshot the selection BEFORE we clear it below, so the Ctrl+C
    // "copy if selection exists" branch can still see it. Without this,
    // the universal clear runs first and Ctrl+C always reads as
    // "no selection → exit".
    let selection_at_keypress = s.selection;

    // Any keystroke clears the visible drag-selection highlight. This
    // makes the highlight read as a transient confirmation of the copy
    // (it appears on Up, disappears on the user's next action) rather
    // than a sticky overlay the user has to dismiss explicitly.
    s.selection = None;

    // Any keystroke that *isn't* Ctrl+C clears the double-tap-to-exit
    // arming window — otherwise a Ctrl+C an hour ago could combine with
    // a Ctrl+C today and quit on the very first press of the day.
    let is_ctrl_c = matches!(k.code, KeyCode::Char('c')) && ctrl;
    if !is_ctrl_c {
        s.last_ctrl_c_at = None;
    }

    match k.code {
        // ── Ctrl+C ───────────────────────────────────────────────────────
        // Selection priority: if there's a meaningful highlighted region,
        // Ctrl+C copies it instead of touching the agent state. Matches
        // Windows Terminal / iTerm convention so users with Windows-
        // keyboard muscle memory don't lose their phantom session every
        // time they want to copy a model response.
        //
        // Otherwise: aider-style double-tap.
        //   * 1st press while running → cancel the turn, arm 2 s window.
        //   * 1st press while idle    → push hint, arm 2 s window.
        //   * 2nd press within 2 s    → Exit (whether running or idle).
        // The window is cleared by any other keystroke (above) so a stray
        // press an hour earlier can't combine with today's first press.
        KeyCode::Char('c') if ctrl => {
            if let Some(sel) = selection_at_keypress {
                if sel.is_meaningful() {
                    return KeyAction::CopySelection(sel);
                }
            }
            const DOUBLE_TAP_WINDOW: Duration = Duration::from_secs(2);
            let armed = s.last_ctrl_c_at
                .map(|t| t.elapsed() < DOUBLE_TAP_WINDOW)
                .unwrap_or(false);
            if armed {
                return KeyAction::Exit;
            }
            s.last_ctrl_c_at = Some(Instant::now());
            if s.running {
                return KeyAction::Cancel;
            }
            s.transcript.push(TranscriptItem::System(
                "press Ctrl-C again within 2 s to exit".into()
            ));
            return KeyAction::None;
        }
        KeyCode::Char('d') if ctrl && s.input.is_empty() => return KeyAction::Exit,

        // ── Emacs-style line edit ────────────────────────────────────────
        KeyCode::Char('a') if ctrl => { s.cursor = 0; }
        KeyCode::Char('e') if ctrl => { s.cursor = s.input.len(); }
        KeyCode::Char('u') if ctrl => {
            // Delete from cursor to start of line (or whole input).
            let cur = s.cursor;
            let line_start = s.input[..cur].rfind('\n').map(|i| i + 1).unwrap_or(0);
            s.input.replace_range(line_start..cur, "");
            s.cursor = line_start;
        }
        KeyCode::Char('k') if ctrl => {
            // Delete from cursor to end of line.
            let cur = s.cursor;
            let line_end = s.input[cur..].find('\n').map(|i| cur + i).unwrap_or(s.input.len());
            s.input.replace_range(cur..line_end, "");
        }
        KeyCode::Char('w') if ctrl => {
            // Delete word before cursor (whitespace-bounded).
            let cur = s.cursor;
            let prefix = &s.input[..cur];
            // Skip trailing whitespace, then delete back to next whitespace.
            let trimmed_end = prefix.trim_end();
            let target_end = trimmed_end.len();
            let word_start = trimmed_end.rfind(|c: char| c.is_whitespace())
                .map(|i| i + 1)
                .unwrap_or(0);
            let _ = target_end;
            s.input.replace_range(word_start..cur, "");
            s.cursor = word_start;
        }
        KeyCode::Char('l') if ctrl => {
            // Clear input (Codex/Claude Code convention).
            s.input.clear();
            s.cursor = 0;
        }

        KeyCode::Esc => {
            if s.running { return KeyAction::Cancel; }
            s.input.clear();
            s.cursor = 0;
        }

        // ── Smart Tab ────────────────────────────────────────────────────
        // / prefix → cycle slash command completions
        // @ prefix → cycle file completions
        // otherwise → cycle agent (legacy behaviour)
        KeyCode::Tab => {
            let token = current_token_tui(&s.input, s.cursor);
            if token.starts_with('/') {
                let candidates: Vec<&&str> = SLASH_COMMANDS_TUI.iter()
                    .filter(|c| c.starts_with(&token))
                    .collect();
                if candidates.len() == 1 {
                    let want = candidates[0];
                    let token_start = s.cursor.saturating_sub(token.len());
                    let end = s.cursor; s.input.replace_range(token_start..end, want);
                    s.cursor = token_start + want.len();
                } else if candidates.len() > 1 {
                    // Find common prefix among candidates and extend to it.
                    let common = longest_common_prefix(&candidates.iter().map(|c| **c).collect::<Vec<_>>());
                    if common.len() > token.len() {
                        let token_start = s.cursor.saturating_sub(token.len());
                        let end = s.cursor; s.input.replace_range(token_start..end, &common);
                        s.cursor = token_start + common.len();
                    }
                    // Otherwise: do nothing (could show candidates inline in v2)
                }
            } else if token.starts_with('@') {
                // Filesystem completion — best-effort
                let path_part = &token[1..];
                let (dir_str, file_prefix) = match path_part.rsplit_once('/') {
                    Some((d, f)) => (d.to_string(), f.to_string()),
                    None => (".".to_string(), path_part.to_string()),
                };
                let dir = if dir_str.is_empty() { "/" } else { dir_str.as_str() };
                let mut matches: Vec<String> = Vec::new();
                if let Ok(entries) = std::fs::read_dir(dir) {
                    for entry in entries.flatten() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        if name.starts_with(&file_prefix) {
                            if name.starts_with('.') && !file_prefix.starts_with('.') { continue; }
                            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                            let suffix = if is_dir { "/" } else { "" };
                            let full = if dir_str == "." || dir_str.is_empty() {
                                format!("@{}{}", name, suffix)
                            } else {
                                format!("@{}/{}{}", dir_str, name, suffix)
                            };
                            matches.push(full);
                        }
                    }
                }
                if matches.len() == 1 {
                    let want = &matches[0];
                    let token_start = s.cursor.saturating_sub(token.len());
                    let end = s.cursor; s.input.replace_range(token_start..end, want);
                    s.cursor = token_start + want.len();
                } else if matches.len() > 1 {
                    let common = longest_common_prefix(&matches.iter().map(|m| m.as_str()).collect::<Vec<_>>());
                    if common.len() > token.len() {
                        let token_start = s.cursor.saturating_sub(token.len());
                        let end = s.cursor; s.input.replace_range(token_start..end, &common);
                        s.cursor = token_start + common.len();
                    }
                }
            } else {
                // Empty input or non-special token → cycle agent (legacy).
                s.cycle_agent();
            }
        }

        KeyCode::PageUp => { s.scroll = s.scroll.saturating_add(5); }
        KeyCode::PageDown => { s.scroll = s.scroll.saturating_sub(5); }

        // ── Up/Down: in-input cursor move (multi-line) OR history recall ─
        //
        // For single-line inputs, Up/Down navigates the prompt history (the
        // historical convention from the REPL).
        //
        // For multi-line inputs (s.input contains '\n'), Up/Down should move
        // the cursor between lines instead — otherwise typing a multi-line
        // prompt and pressing Up to fix a typo on the previous line wipes
        // the entire input with whatever was last in history. That is what
        // the comprehensive test caught.
        //
        // Rule: if the cursor is below row 0 (Up) / above the last row (Down),
        // move the cursor; otherwise fall through to history. Column is
        // matched by display width so CJK lines line up correctly.
        // Shift+Up/Down cycles the sidebar target focus instead — that's
        // the convention from Claude Code's agent-teams in-process mode.
        KeyCode::Up if shift => {
            let total = AGENTS.len() + s.sidebar_peers.len();
            if total > 0 {
                s.sidebar_focus = (s.sidebar_focus + total - 1) % total;
            }
        }
        KeyCode::Down if shift => {
            let total = AGENTS.len() + s.sidebar_peers.len();
            if total > 0 {
                s.sidebar_focus = (s.sidebar_focus + 1) % total;
            }
        }
        KeyCode::Up => {
            let before = &s.input[..s.cursor.min(s.input.len())];
            let cursor_row = before.matches('\n').count();
            if cursor_row > 0 {
                if let Some(new_cursor) = move_cursor_up_one_line(&s.input, s.cursor) {
                    s.cursor = new_cursor;
                }
            } else if !s.history.is_empty() {
                let next = match s.history_idx {
                    None => {
                        s.history_saved = s.input.clone();
                        s.history.len() - 1
                    }
                    Some(0) => 0,
                    Some(i) => i - 1,
                };
                s.history_idx = Some(next);
                let entry = s.history[next].clone();
                s.input = entry;
                s.cursor = s.input.len();
            }
        }
        KeyCode::Down => {
            // If cursor is on a multi-line input but not on the last line,
            // move cursor down. Otherwise fall through to history-restore.
            let total_rows = s.input.matches('\n').count();
            let before = &s.input[..s.cursor.min(s.input.len())];
            let cursor_row = before.matches('\n').count();
            if cursor_row < total_rows {
                if let Some(new_cursor) = move_cursor_down_one_line(&s.input, s.cursor) {
                    s.cursor = new_cursor;
                }
            } else if let Some(i) = s.history_idx {
                if i + 1 < s.history.len() {
                    s.history_idx = Some(i + 1);
                    let entry = s.history[i + 1].clone();
                    s.input = entry;
                    s.cursor = s.input.len();
                } else {
                    // Past newest → restore saved current
                    s.history_idx = None;
                    let saved = std::mem::take(&mut s.history_saved);
                    s.input = saved;
                    s.cursor = s.input.len();
                }
            }
        }

        // Ctrl+J also inserts a newline. In raw mode \n (Ctrl+J) and \r
        // (Enter) are distinct bytes, so this fallback works on every
        // terminal — including Apple Terminal.app where Shift+Enter is
        // indistinguishable from Enter without kitty kbd protocol support.
        KeyCode::Char('j') if ctrl => {
            let pos = s.cursor;
            s.input.insert(pos, '\n');
            s.cursor += 1;
        }

        KeyCode::Enter => {
            // Alt+Enter — commit whatever the sidebar focus is on:
            //   focused agent  → switch active agent (no prompt sent)
            //   focused peer   → prefill input with `@<peer> ` so the
            //                    user just types the prompt + Enter
            // Lets users navigate the sidebar with Shift+↑↓ then act on
            // the highlighted target without retyping its name.
            if alt {
                let total = AGENTS.len() + s.sidebar_peers.len();
                if total > 0 {
                    let focus = s.sidebar_focus % total;
                    if focus < AGENTS.len() {
                        s.agent_idx = focus;
                        s.transcript.push(TranscriptItem::System(
                            format!("◆ → {}", AGENTS[focus])));
                    } else {
                        let peer_name = s.sidebar_peers[focus - AGENTS.len()].clone();
                        let prefix = format!("@{} ", peer_name);
                        // Replace input head with @peer prefix; preserve
                        // anything the user already typed after.
                        if !s.input.starts_with(&prefix) {
                            // Strip any existing `@xxx ` prefix first
                            let body = if let Some(rest) = s.input.strip_prefix('@') {
                                rest.split_once(char::is_whitespace)
                                    .map(|(_, b)| b.to_string())
                                    .unwrap_or_default()
                            } else {
                                s.input.clone()
                            };
                            s.input = format!("{}{}", prefix, body);
                            s.cursor = s.input.len();
                        }
                    }
                }
                return KeyAction::None;
            }
            if shift {
                let pos = s.cursor;
                s.input.insert(pos, '\n');
                s.cursor += 1;
            } else {
                let prompt = s.input.trim().to_string();
                // Always allow Enter through, regardless of `s.running`.
                // The run_loop's `KeyAction::Submit` handler decides what
                // to do based on task state:
                //   * idle      → spawn a new turn
                //   * running   → call `InterruptHandle::interrupt(Some(prompt))`,
                //                 the queued message fires as the next turn
                //                 the moment the current one unwinds (top-of-loop
                //                 chain check in run_loop)
                //   * slash cmd → handled inline before either branch
                //
                // Earlier the gate read `!s.running || is_slash` so non-slash
                // Enters during a stream were silently swallowed. That broke
                // the Hermes-style mid-stream redirect — the user pressed Enter
                // and nothing happened until they hit Esc first.
                if !prompt.is_empty() {
                    // Push to in-memory ring (dedup last) + persist to disk
                    // so Up/Down recall survives phantom restart.
                    if s.history.last().map(|h| h != &prompt).unwrap_or(true) {
                        s.history.push(prompt.clone());
                        if s.history.len() > TUI_HISTORY_MAX {
                            let drop_n = s.history.len() - TUI_HISTORY_MAX;
                            s.history.drain(0..drop_n);
                        }
                        append_tui_history(&prompt);
                        // Cheap to call — only rewrites the file when it has
                        // grown past 2× the cap.
                        maybe_compact_tui_history();
                    }
                    s.input.clear();
                    s.cursor = 0;
                    s.history_idx = None;
                    return KeyAction::Submit(prompt);
                }
            }
        }
        KeyCode::Backspace => {
            if s.cursor > 0 {
                let cur = s.cursor;
                let prev = prev_char_boundary(&s.input, cur);
                s.input.replace_range(prev..cur, "");
                s.cursor = prev;
            }
        }
        KeyCode::Delete => {
            if s.cursor < s.input.len() {
                let cur = s.cursor;
                let next = next_char_boundary(&s.input, cur);
                s.input.replace_range(cur..next, "");
            }
        }
        KeyCode::Left => {
            if alt {
                // Alt-Left = previous word
                let cur = s.cursor;
                let prefix = &s.input[..cur];
                let trimmed = prefix.trim_end();
                let new = trimmed.rfind(|c: char| c.is_whitespace()).map(|i| i + 1).unwrap_or(0);
                s.cursor = new;
            } else if s.cursor > 0 {
                s.cursor = prev_char_boundary(&s.input, s.cursor);
            }
        }
        KeyCode::Right => {
            if alt {
                // Alt-Right = next word
                let cur = s.cursor;
                let suffix = &s.input[cur..];
                let trimmed_start = suffix.find(|c: char| !c.is_whitespace()).unwrap_or(0);
                let after_word = suffix[trimmed_start..]
                    .find(|c: char| c.is_whitespace())
                    .map(|i| cur + trimmed_start + i)
                    .unwrap_or(s.input.len());
                s.cursor = after_word;
            } else if s.cursor < s.input.len() {
                s.cursor = next_char_boundary(&s.input, s.cursor);
            }
        }
        KeyCode::Home => { s.cursor = 0; }
        KeyCode::End => { s.cursor = s.input.len(); }
        KeyCode::Char(c) => {
            if !ctrl {
                let mut buf = [0u8; 4];
                let s_bytes = c.encode_utf8(&mut buf);
                let len = s_bytes.len();
                let pos = s.cursor;
                s.input.insert(pos, c);
                s.cursor += len;
            }
        }
        _ => {}
    }
    KeyAction::None
}

/// Extract the whitespace-bounded token ending at `pos`.
///
/// Defensive against `pos` landing mid-codepoint: if the cursor byte
/// index is inside a multi-byte char (which can happen if input was
/// pasted with mismatched cursor state, or if upstream code lost
/// boundary tracking), round DOWN to the previous char boundary
/// before slicing. Without this, `&line[..pos]` panics with "start
/// byte index N is not a char boundary" — which kills the TUI mid-
/// keystroke. Found via `fuzz_handle_key_never_panics_with_random_initial_input`.
fn current_token_tui(line: &str, pos: usize) -> String {
    let safe_pos = if pos >= line.len() {
        line.len()
    } else if line.is_char_boundary(pos) {
        pos
    } else {
        prev_char_boundary(line, pos)
    };
    let prefix = &line[..safe_pos];
    // `rfind` returns the START byte of the whitespace char; the next
    // char's start is `idx + char.len_utf8()`, NOT `idx + 1`. The naive
    // `+1` breaks when the whitespace is multi-byte (e.g. NBSP `\u{a0}`
    // is 2 bytes; some Unicode whitespace runs 3-4). Use `char_indices`
    // so we know the matched char's byte length.
    let start = match prefix.char_indices().rev().find(|(_, c)| c.is_whitespace()) {
        Some((i, c)) => i + c.len_utf8(),
        None         => 0,
    };
    prefix[start..].to_string()
}

fn longest_common_prefix(strs: &[&str]) -> String {
    if strs.is_empty() { return String::new(); }
    let mut prefix = strs[0].to_string();
    for s in &strs[1..] {
        while !s.starts_with(&prefix) {
            prefix.pop();
            if prefix.is_empty() { return String::new(); }
        }
    }
    prefix
}

fn prev_char_boundary(s: &str, i: usize) -> usize {
    let mut idx = i.saturating_sub(1);
    while idx > 0 && !s.is_char_boundary(idx) { idx -= 1; }
    idx
}
fn next_char_boundary(s: &str, i: usize) -> usize {
    let mut idx = (i + 1).min(s.len());
    while idx < s.len() && !s.is_char_boundary(idx) { idx += 1; }
    idx
}

async fn handle_ui_event(app: &Arc<Mutex<AppState>>, cost: &CostTracker, ev: UiEvent) {
    match ev {
        UiEvent::Token(t) => {
            let mut s = app.lock().unwrap();
            // First, freeze any in-progress thinking partial as a final block.
            if let Some(TranscriptItem::ThinkingPartial(buf)) = s.transcript.last() {
                let buf = buf.clone();
                s.transcript.pop();
                s.transcript.push(TranscriptItem::Thinking(buf));
            }
            // Append to last AssistantPartial, or start a new one.
            if let Some(TranscriptItem::AssistantPartial(buf)) = s.transcript.last_mut() {
                buf.push_str(&t);
            } else {
                s.transcript.push(TranscriptItem::AssistantPartial(t));
            }
        }
        UiEvent::Thinking(t) => {
            let mut s = app.lock().unwrap();
            // Append to last ThinkingPartial, or start a new one.
            if let Some(TranscriptItem::ThinkingPartial(buf)) = s.transcript.last_mut() {
                buf.push_str(&t);
            } else {
                s.transcript.push(TranscriptItem::ThinkingPartial(t));
            }
        }
        UiEvent::ToolStart { name, args } => {
            let mut s = app.lock().unwrap();
            // If we have a streaming partial, finalize it first.
            finalize_partial(&mut s);
            s.transcript.push(TranscriptItem::ToolCall { name, args });
        }
        UiEvent::ToolDone { name, output } => {
            let mut s = app.lock().unwrap();
            s.transcript.push(TranscriptItem::ToolResult { name, output });
        }
        UiEvent::Done { output, elapsed } => {
            // Update cost from tracker and finalize the message
            let last = cost.last_request_cost().await;
            let session = cost.session_cost().await;
            let mut s = app.lock().unwrap();
            // If no streaming tokens arrived, push the final output as one assistant message.
            let had_partial = matches!(
                s.transcript.last(),
                Some(TranscriptItem::AssistantPartial(_)) | Some(TranscriptItem::ThinkingPartial(_))
            );
            if had_partial {
                finalize_partial(&mut s);
                if !output.trim().is_empty()
                    && !matches!(s.transcript.last(), Some(TranscriptItem::Assistant(_)))
                {
                    s.transcript.push(TranscriptItem::Assistant(output));
                }
            } else if !output.trim().is_empty() {
                s.transcript.push(TranscriptItem::Assistant(output));
            }
            s.last_cost = last;
            s.session_cost = session;
            s.running = false;
            s.transcript.push(TranscriptItem::System(format!(
                "↑ ${:.4}  ∑ ${:.4}  {:.1}s",
                last, session, elapsed
            )));
        }
        UiEvent::Error(msg) => {
            let mut s = app.lock().unwrap();
            finalize_partial(&mut s);
            s.transcript.push(TranscriptItem::Error(msg));
            s.running = false;
        }
        UiEvent::Notice(msg) => {
            // Surface as a Warning row WITHOUT clearing `running` — the
            // stream continues normally after a max_tokens truncation
            // signal arrives, and Done will land afterward. We also keep
            // any partial intact: the user wants to see the truncated
            // response below the warning, not have it disappear.
            let mut s = app.lock().unwrap();
            // De-duplicate: providers can emit multiple message_delta or
            // finish_reason chunks during the wind-down; we only want one
            // visible warning per turn.
            let already = matches!(s.transcript.last(), Some(TranscriptItem::Warning(_)));
            if !already {
                s.transcript.push(TranscriptItem::Warning(msg));
            }
        }
    }
}

fn finalize_partial(s: &mut AppState) {
    match s.transcript.last() {
        Some(TranscriptItem::AssistantPartial(buf)) => {
            let buf = buf.clone();
            let _ = s.transcript.pop();
            s.transcript.push(TranscriptItem::Assistant(buf));
        }
        Some(TranscriptItem::ThinkingPartial(buf)) => {
            let buf = buf.clone();
            let _ = s.transcript.pop();
            s.transcript.push(TranscriptItem::Thinking(buf));
        }
        _ => {}
    }
}

// ── Rendering ────────────────────────────────────────────────────────────────

fn ui(f: &mut Frame, s: &AppState) {
    let area = f.area();

    // Compute input height accounting for both explicit newlines AND visual
    // wrap. A 120-char ASCII line on a 100-wide terminal needs 2 visual rows;
    // 50 wide CJK chars on the same terminal also need 2. Counting only `\n`
    // (the previous logic) made wrapped second rows invisible.
    let inner_w = area.width.saturating_sub(2).max(1) as usize; // -2 for borders
    let mut visual_rows: usize = 0;
    if s.input.is_empty() {
        visual_rows = 1;  // placeholder hint line
    } else {
        for line in s.input.split('\n') {
            let dw = unicode_width::UnicodeWidthStr::width(line);
            // Each \n contributes one row; long lines wrap, taking ceil(dw/inner_w) rows.
            visual_rows += (dw / inner_w) + 1;
        }
        if s.input.ends_with('\n') { visual_rows += 1; }
    }
    let input_lines = visual_rows.clamp(1, 5) as u16;
    let input_h = input_lines + 2; // +2 for borders

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),       // status bar
            Constraint::Min(3),          // body (transcript [+ sidebar])
            Constraint::Length(input_h), // input box
        ])
        .split(area);

    render_status(f, chunks[0], s);

    // Body: transcript on left, optional sidebar on right. Sidebar
    // auto-hides only when there's literally not enough room (transcript
    // would shrink under 30 cols). Default 80-col PowerShell windows
    // get the sidebar — that was the threshold-too-tight bug where
    // /sidebar appeared to do nothing.
    let sidebar_w: u16 = 32;
    let body = chunks[1];
    let min_transcript: u16 = 30;
    if s.sidebar_visible && body.width >= sidebar_w + min_transcript {
        let body_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Min(min_transcript),
                Constraint::Length(sidebar_w),
            ])
            .split(body);
        render_transcript(f, body_chunks[0], s);
        render_sidebar(f, body_chunks[1], s);
    } else {
        render_transcript(f, body, s);
    }

    render_input(f, chunks[2], s);

    // Overlay selection highlight LAST so it sits on top of every pane's
    // own painted cells. Without this, render_transcript / render_input
    // paint cells fresh every frame and would erase any background style
    // we set earlier.
    if let Some(sel) = &s.selection {
        apply_selection_highlight(f.buffer_mut(), sel);
    }

    // Modal popup goes ABOVE the selection highlight too so it's never
    // obscured by a stale drag. Drawn last on purpose.
    if let Some(picker) = &s.priority_picker {
        render_priority_picker(f, area, picker);
    }
}

/// Paint a selection-highlight background over the cells covered by `sel`,
/// using linear (text-flow) coverage rather than rectangular: the start
/// row's cells run from `start.0` to end-of-row, middle rows are full
/// width, and the end row's cells run from `0..=end.0`. Same model as a
/// normal text-editor selection.
///
/// Style: light-blue background, fg untouched. Plain `Color::Blue` would
/// hide black foreground glyphs on a dark theme; the RGB picks a value
/// that contrasts with both the default fg and our magenta/cyan accents.
fn apply_selection_highlight(buf: &mut ratatui::buffer::Buffer, sel: &Selection) {
    let (start, end) = sel.normalized();
    let area = buf.area;
    let highlight = Style::default().bg(Color::Rgb(60, 90, 140));

    // Helper: paint cells [col_lo..=col_hi] on row `row`, clamped to area.
    let paint_row = |buf: &mut ratatui::buffer::Buffer, row: u16, col_lo: u16, col_hi: u16| {
        if row >= area.height { return; }
        let lo = col_lo.min(area.width.saturating_sub(1));
        let hi = col_hi.min(area.width.saturating_sub(1));
        for col in lo..=hi {
            if let Some(cell) = buf.cell_mut((col, row)) {
                cell.set_style(cell.style().patch(highlight));
            }
        }
    };

    if start.1 == end.1 {
        paint_row(buf, start.1, start.0, end.0);
    } else {
        // First row: from anchor col to end-of-row
        paint_row(buf, start.1, start.0, area.width.saturating_sub(1));
        // Middle rows: full width
        for row in (start.1 + 1)..end.1 {
            paint_row(buf, row, 0, area.width.saturating_sub(1));
        }
        // Last row: from 0 to cursor col
        paint_row(buf, end.1, 0, end.0);
    }
}

/// Read the cells inside `sel` from `buf` and rebuild the selected text
/// as a single String with `\n` between rows. Trailing whitespace per row
/// is trimmed because terminal cells are space-padded out to the right
/// margin and pasting "hello                 " when the user only saw
/// "hello" is annoying.
///
/// Wide chars (CJK) take two cells in the buffer: the leading cell holds
/// the symbol, the trailing cell holds an empty placeholder. Iterating
/// `cell.symbol()` for every column naturally skips the placeholder
/// (its symbol is "") so we don't double-emit.
fn extract_selection_text(buf: &ratatui::buffer::Buffer, sel: Selection) -> String {
    let (start, end) = sel.normalized();
    let area = buf.area;
    let mut out = String::new();

    let row_text = |row: u16, col_lo: u16, col_hi: u16| -> String {
        let mut line = String::new();
        let lo = col_lo.min(area.width.saturating_sub(1));
        let hi = col_hi.min(area.width.saturating_sub(1));
        for col in lo..=hi {
            if let Some(cell) = buf.cell((col, row)) {
                line.push_str(cell.symbol());
            }
        }
        line.trim_end().to_string()
    };

    if start.1 == end.1 {
        out.push_str(&row_text(start.1, start.0, end.0));
    } else {
        out.push_str(&row_text(start.1, start.0, area.width.saturating_sub(1)));
        out.push('\n');
        for row in (start.1 + 1)..end.1 {
            out.push_str(&row_text(row, 0, area.width.saturating_sub(1)));
            out.push('\n');
        }
        out.push_str(&row_text(end.1, 0, end.0));
    }
    out
}

/// Pipe `text` to the OS clipboard via the same external command the
/// `/copy` slash uses. Returns the command name on success so the caller
/// can echo "copied via clip" / "copied via pbcopy".
///
/// Why an external command instead of a clipboard crate: phantom already
/// uses this approach for `/copy <all|turn|last>`, and adding `arboard`
/// or similar pulls in X11/Wayland deps on Linux that complicate cross-
/// compile to platforms where clipboard isn't core. The user will already
/// have `clip.exe` (Windows), `pbcopy` (macOS), or `xclip` (Linux) on
/// their box if they intend to copy at all.
fn copy_to_os_clipboard(text: &str) -> Result<&'static str, String> {
    let cmd = if cfg!(target_os = "macos") { "pbcopy" }
              else if cfg!(target_os = "linux") { "xclip" }
              else { "clip" };
    let mut child = std::process::Command::new(cmd)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn {}: {}", cmd, e))?;
    if let Some(stdin) = child.stdin.as_mut() {
        use std::io::Write;
        stdin.write_all(text.as_bytes())
            .map_err(|e| format!("write to {}: {}", cmd, e))?;
    }
    child.wait().map_err(|e| format!("wait {}: {}", cmd, e))?;
    Ok(cmd)
}

/// Wall-clock-derived "LLM is working" glyph — pulsing phantom diamond.
/// Was a Braille rotor; that's what every other CLI agent uses
/// (Codex/Claude Code/Aider all share the spinner crate's default), so
/// switched to the brand glyph fading in/out: ◇ → ◈ → ◆ → ◈ → ...
/// 4 frames × 150ms = 600ms full cycle — slow enough to read each frame,
/// fast enough to clearly signal "alive". Layout stays single-cell-wide
/// so the version text doesn't shift between idle and busy.
fn spinner_glyph() -> &'static str {
    const FRAMES: [&str; 4] = ["◇", "◈", "◆", "◈"];
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    FRAMES[((now_ms / 150) % FRAMES.len() as u128) as usize]
}

fn render_status(f: &mut Frame, area: Rect, s: &AppState) {
    f.render_widget(Clear, area);

    let version = env!("CARGO_PKG_VERSION");
    // Spinner sits to the LEFT of "phantom v0.1.0" while busy. We always
    // emit one glyph (the spinner OR a space) so the version text never
    // shifts position between idle and busy states — flicker is what makes
    // a status bar feel cheap.
    let busy_glyph = if s.running { spinner_glyph() } else { " " };
    let busy_style = if s.running {
        // Brand gold (#d6b270 — matches phantommesh.io landing). Uses BOLD
        // so even terminals that round to 8-color still light it up.
        Style::default().fg(Color::Rgb(214, 178, 112)).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let mode = if s.running { " · streaming…" } else { "" };
    let line = Line::from(vec![
        Span::styled(format!(" {} ", busy_glyph), busy_style),
        Span::styled(format!("phantom v{}", version), Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
        Span::styled(" · ", Style::default().fg(Color::DarkGray)),
        Span::styled("agent: ", Style::default().fg(Color::DarkGray)),
        Span::styled(s.agent_name(), Style::default().fg(Color::Cyan)),
        Span::styled(" · ", Style::default().fg(Color::DarkGray)),
        Span::styled(format!("cost: ${:.4} session", s.session_cost), Style::default().fg(Color::DarkGray)),
        Span::styled(mode.to_string(), Style::default().fg(Color::Yellow)),
    ]);
    let p = Paragraph::new(line).style(Style::default().bg(Color::Reset));
    f.render_widget(p, area);
}

fn render_transcript(f: &mut Frame, area: Rect, s: &AppState) {
    // Wipe stale cells first. Paragraph only paints span content; cells beyond
    // the line end keep whatever was in the previous frame's buffer, leaking
    // ghost text into the right margin when a wide line is replaced by a
    // narrower one (regression test: transcript_does_not_retain_stale_chars_*).
    f.render_widget(Clear, area);

    let mut lines: Vec<Line> = Vec::new();
    for item in &s.transcript {
        match item {
            TranscriptItem::User(t) => {
                lines.push(Line::from(vec![
                    Span::styled("◆ ", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
                    Span::styled(t.clone(), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                ]));
                lines.push(Line::from(""));
            }
            TranscriptItem::Thinking(t) | TranscriptItem::ThinkingPartial(t) => {
                if std::env::var("PHANTOM_THINKING").map(|v| v != "0").unwrap_or(true) {
                    let total = t.lines().filter(|l| !l.trim().is_empty()).count();
                    let style = Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC | Modifier::DIM);
                    lines.push(Line::from(Span::styled(
                        format!("⌖ thinking ({} line{})", total, if total == 1 { "" } else { "s" }),
                        style,
                    )));
                    for l in t.lines().take(3) {
                        let truncated: String = l.chars().take(160).collect();
                        let suffix = if l.chars().count() > 160 { "…" } else { "" };
                        lines.push(Line::from(Span::styled(
                            format!("⌖ ┊ {}{}", truncated, suffix),
                            style,
                        )));
                    }
                    if total > 3 {
                        lines.push(Line::from(Span::styled(
                            format!("⌖ … +{} more", total - 3),
                            style,
                        )));
                    }
                }
            }
            TranscriptItem::Assistant(t) | TranscriptItem::AssistantPartial(t) => {
                for (i, l) in t.lines().enumerate() {
                    if i == 0 {
                        lines.push(Line::from(vec![
                            Span::styled("● ", Style::default().fg(Color::Green)),
                            Span::raw(l.to_string()),
                        ]));
                    } else {
                        lines.push(Line::from(vec![
                            Span::raw("  "),
                            Span::raw(l.to_string()),
                        ]));
                    }
                }
                lines.push(Line::from(""));
            }
            TranscriptItem::ToolCall { name, args } => {
                let preview = truncate(args, 80);
                lines.push(Line::from(vec![
                    Span::styled("● ", Style::default().fg(Color::Cyan)),
                    Span::styled(format!("{}({})", name, preview), Style::default().fg(Color::Cyan)),
                ]));
            }
            TranscriptItem::ToolResult { name, output } => {
                let preview = truncate(output, 200);
                lines.push(Line::from(vec![
                    Span::styled("  ✓ ", Style::default().fg(Color::Green)),
                    Span::styled(format!("{}: ", name), Style::default().fg(Color::Cyan)),
                    Span::styled(preview, Style::default().fg(Color::DarkGray)),
                ]));
            }
            TranscriptItem::System(t) => {
                // Split on '\n' so each logical line becomes its own ratatui
                // Line. Otherwise an embedded newline collapses into a single
                // Span whose visual layout under Wrap{trim:false} can't be
                // distinguished from word-wrapping — multi-line slash output
                // ( /provider, /agents, /keys list, etc.) renders as one
                // wide line that wraps weirdly into garbled right-margin
                // fragments. Mirrors what TranscriptItem::Assistant already does.
                for l in t.lines() {
                    lines.push(Line::from(Span::styled(
                        l.to_string(),
                        Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
                    )));
                }
                if t.is_empty() {
                    // Preserve the slot a bare push("") would have taken,
                    // useful as a separator.
                    lines.push(Line::from(""));
                }
            }
            TranscriptItem::Error(t) => {
                lines.push(Line::from(vec![
                    Span::styled("✗ ", Style::default().fg(Color::Red)),
                    Span::styled(t.clone(), Style::default().fg(Color::Red)),
                ]));
            }
            TranscriptItem::Warning(t) => {
                // Bold red glyph + the message itself in regular red.
                // ⚠ at the start makes the row scannable when the user
                // is skimming the scrollback, even before reading the
                // message text. Run continues — this is a heads-up, not
                // a failure, but it's red because the user explicitly
                // asked for "紅色 system message".
                lines.push(Line::from(vec![
                    Span::styled(
                        "⚠ ",
                        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(t.clone(), Style::default().fg(Color::Red)),
                ]));
            }
        }
    }

    // Bottom-pinned scroll: we want the latest content always at the bottom
    // of the viewport. Paragraph::scroll((y, x)) skips y rows from the top
    // of the rendered paragraph (borders included). The total row count
    // must therefore match what ratatui's renderer actually produces under
    // Wrap { trim: false } — including word-wrap break points, CJK width
    // doubling, soft hyphens, and the block's border rows.
    //
    // Doing this estimate by hand (sum of ceil(display_width / inner_w))
    // diverges from ratatui's WordWrapper: a 985-char CJK assistant message
    // could under-count by enough rows that the bottom of the message got
    // pushed past viewport.height and the user saw the head of the message
    // pinned to the top instead of the tail pinned to the bottom (the
    // user-visible bug fixed by this change).
    //
    // Use Paragraph::line_count(width) — gated by ratatui's
    // `unstable-rendered-line-info` feature — which runs the same
    // WordWrapper the renderer uses, so the count and the rendered output
    // are guaranteed consistent across platforms and ratatui upgrades.
    let block = Block::default()
        .borders(Borders::TOP | Borders::BOTTOM)
        .border_style(Style::default().fg(Color::DarkGray));
    let p = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    // line_count includes the block's vertical borders, so the comparison
    // is against the full area.height (no manual `-2`).
    let total_visual = p.line_count(area.width) as u16;
    let offset = total_visual
        .saturating_sub(area.height)
        .saturating_sub(s.scroll);
    let p = p.scroll((offset, 0));
    f.render_widget(p, area);
}

/// Right-rail target picker. Lists local agents (configurable in
/// agents.toml [agent.*]) and remote peers (synced via `phantom config
/// pull` → ~/.phantom-mesh/peers.json). Highlights the current focus
/// (Shift+Down/Up cycles); the active agent gets `*`. The visible focus
/// is purely a hint — committing happens via Tab (cycles local agents)
/// or `@<name>` prefix in the input.
///
/// Phase 1 deliberately ships WITHOUT live state badges (idle / streaming
/// / error per peer) — adding those means a background poller against
/// /healthz + /api/me/sessions every few seconds. Static list first, see
/// if it's enough.
fn render_sidebar(f: &mut Frame, area: Rect, s: &AppState) {
    f.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(" targets ", Style::default()
            .fg(Color::Rgb(214, 178, 112))
            .add_modifier(Modifier::BOLD)));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let total = AGENTS.len() + s.sidebar_peers.len();
    let focus = if total == 0 { 0 } else { s.sidebar_focus % total };
    let mut lines: Vec<Line> = Vec::with_capacity(total + 6);

    // Section header for local agents
    lines.push(Line::from(Span::styled(" LOCAL AGENTS",
        Style::default().fg(Color::DarkGray))));
    for (i, name) in AGENTS.iter().enumerate() {
        let is_active = i == s.agent_idx;
        let is_focused = i == focus;
        let glyph = if is_active { "◆" } else if is_focused { "▸" } else { " " };
        let style = if is_focused {
            Style::default().fg(Color::Rgb(214, 178, 112)).add_modifier(Modifier::BOLD)
        } else if is_active {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::Gray)
        };
        lines.push(Line::from(vec![
            Span::raw(" "),
            Span::styled(format!("{} {}", glyph, name), style),
        ]));
    }

    // Spacer + remote section
    lines.push(Line::from(""));
    let peer_count = s.sidebar_peers.len();
    lines.push(Line::from(Span::styled(
        format!(" REMOTE ({} peer{})", peer_count, if peer_count == 1 { "" } else { "s" }),
        Style::default().fg(Color::DarkGray),
    )));
    if s.sidebar_peers.is_empty() {
        lines.push(Line::from(vec![
            Span::raw(" "),
            Span::styled("(none — phantom config pull)",
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC)),
        ]));
    } else {
        for (i, peer) in s.sidebar_peers.iter().enumerate() {
            let combined_idx = AGENTS.len() + i;
            let is_focused = combined_idx == focus;
            let glyph = if is_focused { "▸" } else { " " };
            let style = if is_focused {
                Style::default().fg(Color::Rgb(214, 178, 112)).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            };
            lines.push(Line::from(vec![
                Span::raw(" "),
                Span::styled(format!("{} {}", glyph, peer), style),
            ]));
        }
    }

    // Footer — explicit how-to so the rail isn't a mystery box.
    let muted = Style::default().fg(Color::DarkGray);
    let key   = Style::default().fg(Color::Rgb(214, 178, 112));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(" HOW TO USE", muted.add_modifier(Modifier::BOLD))));
    lines.push(Line::from(vec![
        Span::raw(" "),
        Span::styled("Tab", key),
        Span::styled("        next agent", muted),
    ]));
    lines.push(Line::from(vec![
        Span::raw(" "),
        Span::styled("@name <prompt>", key),
    ]));
    lines.push(Line::from(Span::styled("              → switch / send", muted)));
    lines.push(Line::from(vec![
        Span::raw(" "),
        Span::styled("Shift+↑↓", key),
        Span::styled("   move ▸", muted),
    ]));
    lines.push(Line::from(vec![
        Span::raw(" "),
        Span::styled("Alt+Enter", key),
        Span::styled("  commit ▸", muted),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(" /sidebar off · status",
        muted.add_modifier(Modifier::ITALIC))));

    let p = Paragraph::new(lines).wrap(Wrap { trim: false });
    f.render_widget(p, inner);
}

/// Key dispatch while the /priority modal is open. All keystrokes are
/// captured here — the regular input handler runs only when the modal
/// is closed. Returns KeyAction::None for "stay in modal" or
/// KeyAction::None after Esc/Enter (which both close the modal but
/// don't submit anything to the agent).
fn handle_priority_picker_key(s: &mut AppState, code: KeyCode, shift: bool) -> KeyAction {
    let p = match s.priority_picker.as_mut() {
        Some(p) => p,
        None    => return KeyAction::None,
    };
    let len = p.items.len();
    match code {
        KeyCode::Esc => {
            // Discard all changes
            s.priority_picker = None;
            s.transcript.push(TranscriptItem::System("  ◆ priority: cancelled (no changes saved)".into()));
        }
        KeyCode::Up if shift => {
            if len > 1 && p.focused > 0 {
                p.items.swap(p.focused - 1, p.focused);
                p.focused -= 1;
                p.dirty = true;
            }
        }
        KeyCode::Down if shift => {
            if len > 1 && p.focused + 1 < len {
                p.items.swap(p.focused, p.focused + 1);
                p.focused += 1;
                p.dirty = true;
            }
        }
        KeyCode::Up => {
            if len > 0 { p.focused = (p.focused + len - 1) % len; }
        }
        KeyCode::Down => {
            if len > 0 { p.focused = (p.focused + 1) % len; }
        }
        KeyCode::Delete | KeyCode::Char('x') | KeyCode::Char('d') => {
            if len > 0 {
                p.items.remove(p.focused);
                if p.focused >= p.items.len() && !p.items.is_empty() {
                    p.focused = p.items.len() - 1;
                }
                p.dirty = true;
            }
        }
        KeyCode::Enter => {
            // Save via providers_priority_lines, then close.
            let agent = p.agent_name.clone();
            let new_order: Vec<String> = if p.items.is_empty() {
                // Empty list — pass a single space-trimmed sentinel? Skip.
                Vec::new()
            } else {
                p.items.clone()
            };
            s.priority_picker = None;
            if new_order.is_empty() {
                s.transcript.push(TranscriptItem::Warning(
                    "  ⚠ priority: list is empty — refusing to save (use /provider priority <agent> <p1> <p2> ... to set explicitly)".into()));
                return KeyAction::None;
            }
            match crate::cli_config::providers_priority_lines(Some(&agent), &new_order) {
                Ok(lines) => {
                    s.transcript.push(TranscriptItem::System(
                        format!("  ◆ priority saved for agent.{}", agent)));
                    for l in lines {
                        s.transcript.push(TranscriptItem::System(format!("  {}", l)));
                    }
                }
                Err(e) => s.transcript.push(TranscriptItem::Error(
                    format!("  ✗ save failed: {}", e))),
            }
        }
        _ => {} // ignore everything else while modal is up
    }
    KeyAction::None
}

/// Modal popup for editing [agent.X].providers priority order.
/// Centered ~50 cols × ~auto rows. Shows numbered list of provider:model
/// entries, current focus highlighted in brand gold. Footer hints the
/// keybindings. Drawn over the entire frame — opaque background so the
/// transcript underneath is hidden.
fn render_priority_picker(f: &mut Frame, area: Rect, p: &PriorityPicker) {
    let popup_w: u16 = 56.min(area.width.saturating_sub(4));
    let popup_h: u16 = (p.items.len() as u16 + 6).min(area.height.saturating_sub(2));
    if popup_w < 20 || popup_h < 6 { return; }
    let x = area.x + (area.width.saturating_sub(popup_w)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_h)) / 2;
    let popup = Rect::new(x, y, popup_w, popup_h);

    f.render_widget(Clear, popup);
    let title = if p.dirty {
        format!(" priority — agent.{} • unsaved ", p.agent_name)
    } else {
        format!(" priority — agent.{} ", p.agent_name)
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(214, 178, 112)))
        .title(Span::styled(title, Style::default()
            .fg(Color::Rgb(214, 178, 112))
            .add_modifier(Modifier::BOLD)));
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let mut lines: Vec<Line> = Vec::with_capacity(p.items.len() + 4);
    if p.items.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("  (empty — no providers configured for this agent)",
            Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC))));
    } else {
        for (i, entry) in p.items.iter().enumerate() {
            let is_focused = i == p.focused;
            let glyph = if is_focused { "▸" } else { " " };
            let style = if is_focused {
                Style::default().fg(Color::Rgb(214, 178, 112)).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            };
            lines.push(Line::from(vec![
                Span::raw(" "),
                Span::styled(format!("{} {:>2}. {}", glyph, i + 1, entry), style),
            ]));
        }
    }

    // Footer
    lines.push(Line::from(""));
    let muted = Style::default().fg(Color::DarkGray);
    let key   = Style::default().fg(Color::Rgb(214, 178, 112));
    lines.push(Line::from(vec![
        Span::raw(" "),
        Span::styled("↑↓", key), Span::styled(" nav  ", muted),
        Span::styled("Shift+↑↓", key), Span::styled(" reorder", muted),
    ]));
    lines.push(Line::from(vec![
        Span::raw(" "),
        Span::styled("Del/x", key), Span::styled(" remove  ", muted),
        Span::styled("Enter", key), Span::styled(" save  ", muted),
        Span::styled("Esc", key), Span::styled(" cancel", muted),
    ]));

    let para = Paragraph::new(lines).wrap(Wrap { trim: false });
    f.render_widget(para, inner);
}

fn render_input(f: &mut Frame, area: Rect, s: &AppState) {
    f.render_widget(Clear, area);

    let prompt_style = if s.running {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::Magenta)
    };

    // While streaming, surface the user's last submission inside the input
    // box's top border. Without this, long agent replies wrap-pushed the
    // ◆ User transcript item past the viewport and the user lost sight of
    // what they had just typed. The title is always on screen — even when
    // the transcript has scrolled the User row out — so this becomes a
    // reliable echo of "what's the agent currently working on for me".
    let title_owned: String;
    let title: &str = if s.running {
        let last_user = s.transcript.iter().rev().find_map(|t| {
            if let TranscriptItem::User(p) = t { Some(p.as_str()) } else { None }
        });
        match last_user {
            Some(p) => {
                // Width-aware truncation so CJK glyphs don't blow past the
                // border. Reserve ~24 cells for the prefix/suffix decoration.
                let max_cells = (area.width as usize).saturating_sub(24).max(8);
                let preview = truncate_to_width(p, max_cells);
                title_owned = format!(" ◆ streaming · you: {} — Esc to cancel ", preview);
                title_owned.as_str()
            }
            None => " ◆ (streaming — Esc to cancel) ",
        }
    } else {
        " ◆ "
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(prompt_style)
        .title(title);

    // Build one Line per logical line. Smashing the whole input (including
    // embedded \n) into a single Line made ratatui render every \n as a
    // printable character, so Ctrl-J / Shift-Enter newlines collapsed into
    // one visual row even though the input box was sized to fit them.
    let lines_vec: Vec<Line> = if s.input.is_empty() {
        vec![Line::from(Span::styled(
            "Enter sends · Shift-Enter / Alt-Enter / Ctrl-J newline · Tab agent · Ctrl-C ×2 exits",
            Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
        ))]
    } else {
        // Style the typed input as bold gold (24-bit RGB) so it doesn't
        // inherit the terminal default fg, which on common dark themes
        // and Apple Terminal Profiles renders as muted gray. Same color
        // the REPL prompt uses, so both surfaces stay consistent.
        let style = Style::default()
            .fg(Color::Rgb(255, 215, 0))
            .add_modifier(Modifier::BOLD);
        s.input
            .split('\n')
            .map(|seg| Line::from(Span::styled(seg.to_string(), style)))
            .collect()
    };

    let p = Paragraph::new(lines_vec).block(block).wrap(Wrap { trim: false });
    f.render_widget(p, area);

    // Cursor positioning: compute (col, row) from byte offset using display
    // width (CJK / wide glyphs count as 2 cells each, not 1). Also handle
    // visual wrap so the cursor lands on the correct wrapped row.
    if !s.input.is_empty() {
        let before = &s.input[..s.cursor.min(s.input.len())];
        let inner_w = area.width.saturating_sub(2).max(1) as usize;

        let mut row: u16 = 0;
        let mut col: u16 = 0;
        for line in before.split('\n') {
            let dw = unicode_width::UnicodeWidthStr::width(line);
            // Wrap: each row holds `inner_w` cells.
            let line_rows = dw / inner_w;
            row += line_rows as u16;
            col = (dw % inner_w) as u16;
            // If this isn't the last line in `before`, account for the \n.
            row += 1;
        }
        // We over-counted by 1 row (the final \n that doesn't exist if `before`
        // doesn't end with one).
        if row > 0 { row -= 1; }

        f.set_cursor_position((area.x + 1 + col, area.y + 1 + row));
    } else {
        f.set_cursor_position((area.x + 1, area.y + 1));
    }
}

/// Move the cursor up one logical line in `input`. Preserves the visual
/// column (display width). Returns the new byte offset, or None if there's
/// no line above.
fn move_cursor_up_one_line(input: &str, cursor: usize) -> Option<usize> {
    use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
    let cursor = cursor.min(input.len());
    let before = &input[..cursor];
    let cur_line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
    if cur_line_start == 0 {
        return None;  // already on first line
    }
    let cur_col = UnicodeWidthStr::width(&before[cur_line_start..]);
    // Previous line bounds: end is the \n at cur_line_start-1, start is the
    // \n before that (or 0).
    let prev_end = cur_line_start - 1;
    let prev_start = before[..prev_end].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let prev_line = &input[prev_start..prev_end];

    // Walk prev_line by chars, accumulating display width, until we hit cur_col.
    let mut walked: usize = 0;
    let mut new_cursor = prev_start;
    for (byte_off, ch) in prev_line.char_indices() {
        let w = UnicodeWidthChar::width(ch).unwrap_or(0);
        if walked + w > cur_col { break; }
        walked += w;
        new_cursor = prev_start + byte_off + ch.len_utf8();
    }
    Some(new_cursor)
}

/// Move the cursor down one logical line. Returns None if no next line.
fn move_cursor_down_one_line(input: &str, cursor: usize) -> Option<usize> {
    use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
    let cursor = cursor.min(input.len());
    let before = &input[..cursor];
    let cur_line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let cur_col = UnicodeWidthStr::width(&before[cur_line_start..]);

    // Find the start of the line *after* the cursor's current line.
    let after = &input[cursor..];
    let next_line_start = match after.find('\n') {
        Some(i) => cursor + i + 1,
        None => return None,
    };
    if next_line_start > input.len() { return None; }
    // Next line bounds: from next_line_start to next \n or end of input.
    let rest = &input[next_line_start..];
    let next_line_end = next_line_start + rest.find('\n').unwrap_or(rest.len());
    let next_line = &input[next_line_start..next_line_end];

    let mut walked: usize = 0;
    let mut new_cursor = next_line_start;
    for (byte_off, ch) in next_line.char_indices() {
        let w = UnicodeWidthChar::width(ch).unwrap_or(0);
        if walked + w > cur_col { break; }
        walked += w;
        new_cursor = next_line_start + byte_off + ch.len_utf8();
    }
    Some(new_cursor)
}

fn truncate(s: &str, max: usize) -> String {
    let s = s.replace('\n', " ");
    if s.chars().count() <= max { s } else {
        let truncated: String = s.chars().take(max).collect();
        format!("{}…", truncated)
    }
}

/// Truncate by *display width* (CJK = 2 cells), not codepoint count, so the
/// result fits inside `max_cells` cells in the rendered terminal.
fn truncate_to_width(s: &str, max_cells: usize) -> String {
    use unicode_width::UnicodeWidthChar;
    let single_line = s.replace('\n', " ");
    let total: usize = single_line.chars()
        .map(|c| c.width().unwrap_or(0))
        .sum();
    if total <= max_cells {
        return single_line;
    }
    // Reserve 1 cell for the trailing ellipsis.
    let budget = max_cells.saturating_sub(1).max(1);
    let mut out = String::new();
    let mut used = 0;
    for c in single_line.chars() {
        let w = c.width().unwrap_or(0);
        if used + w > budget { break; }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}

/// Convenience builder used by `phantom tui` in the binary. Mirrors the same
/// initialization the REPL does, but produces no output to stderr (because the
/// alternate screen is about to take over).
pub async fn launch_default() -> Result<()> {
    use crate::AppState;

    // Discover config — same lookup the binary uses (kept in sync but
    // simplified for the TUI launcher).
    let mut app_state = AppState::new();
    if let Some(content) = find_config_simple() {
        app_state.load_config_toml(&content);
    }

    let conversations = ConversationStore::new();
    let cost_tracker = CostTracker::new();
    let runtime = app_state.agent_runtime.clone();

    // Wire runtime + cost into the global slot used by the `task` /
    // `subagent` tool. Idempotent (OnceLock) — REPL path also does this.
    crate::tools::subagent::init_global(runtime.clone(), cost_tracker.clone());

    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let chat_id = cwd_chat_id(&cwd);
    let extra_context = WorkspaceContext::capture().to_system_context();

    // Pinned-agent override from [workspace].pinned_agent — bin/phantom.rs
    // sets PHANTOM_DEFAULT_AGENT before calling us when bare `phantom`
    // is run + workspace pin is configured. Falls back to "master" so
    // explicit `phantom tui` calls behave as before.
    let initial_agent = std::env::var("PHANTOM_DEFAULT_AGENT")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "master".to_string());

    run_tui(runtime, conversations, cost_tracker, chat_id, initial_agent, extra_context).await
}

fn find_config_simple() -> Option<String> {
    // Local agents.toml first, then ~/.phantom-mesh/agents.toml
    if let Ok(c) = std::fs::read_to_string("agents.toml") { return Some(c); }
    if let Some(home) = dirs::home_dir() {
        let p = home.join(".phantom-mesh").join("agents.toml");
        if let Ok(c) = std::fs::read_to_string(&p) { return Some(c); }
    }
    None
}

fn cwd_chat_id(cwd: &std::path::Path) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    cwd.hash(&mut h);
    format!("cwd-{:x}", h.finish())
}

// ── slash commands ────────────────────────────────────────────────────────
//
// Mirror the REPL's slash commands inside the TUI. Each command writes a
// System line into the transcript instead of stdout; agent dispatch is
// skipped (handled in the Submit branch before this is called).

async fn handle_tui_slash(
    app: &Arc<Mutex<AppState>>,
    runtime: &crate::agent::AgentRuntime,
    conversations: &crate::session::ConversationStore,
    cost_tracker: &crate::cost::CostTracker,
    line: &str,
) {
    let parts: Vec<&str> = line.splitn(2, ' ').collect();
    let cmd = parts[0];
    let arg = parts.get(1).map(|s| s.trim()).filter(|s| !s.is_empty());

    let push = |text: String| {
        let mut s = app.lock().unwrap();
        s.transcript.push(TranscriptItem::System(text));
    };
    let push_err = |text: String| {
        let mut s = app.lock().unwrap();
        s.transcript.push(TranscriptItem::Error(text));
    };

    match cmd {
        "/help" | "/?" => {
            push(
                "  TUI slash commands:\n\
                 \n\
                 \x20 help & exit\n\
                 \x20   /help, /?                     this list\n\
                 \x20   /exit, /quit                  exit (or press Ctrl-C)\n\
                 \x20\n\
                 \x20 session\n\
                 \x20   /clear                        clear transcript & evict session history\n\
                 \x20   /compact                      LLM-summarize older turns, keep last 6\n\
                 \x20   /sessions                     list saved sessions\n\
                 \x20   /resume <prefix>              switch to another session\n\
                 \x20   /copy [all|turn]              copy: last assistant / full session / last turn\n\
                 \x20   /export [path]                save session as markdown\n\
                 \x20   /show <id>                    expand a captured tool-call output\n\
                 \x20\n\
                 \x20 agents & models & keys\n\
                 \x20   /agent [name]                 show or switch active agent\n\
                 \x20   /agents                       list configured agents\n\
                 \x20   /model                        show models / switch / fast|smart|cheap / fetch\n\
                 \x20   /keys [list|test|remove]      manage provider api keys\n\
                 \x20   /provider                     list providers + key state\n\
                 \x20   /tools                        list registered tools\n\
                 \x20   /todo                         show todos (use the todo_add tool to create)\n\
                 \x20\n\
                 \x20 context & auth\n\
                 \x20   /init                         generate PHANTOM.md in cwd\n\
                 \x20   /whoami                       show broker login state\n\
                 \x20   /cost                         show cost summary\n\
                 \x20\n\
                 \x20 environment\n\
                 \x20   /perm ask|allow|deny|list     permission mode\n\
                 \x20   /density compact|full         tool result density\n\
                 \x20   /theme <name>                 color theme\n\
                 \x20   /plan                         toggle plan-then-execute mode\n\
                 \x20\n\
                 \x20 REPL-only (interactive prompts)\n\
                 \x20   /login /logout /add /undo /keys add /model pick\n\
                 \x20   For these, run `phantom --repl` instead."
                    .to_string(),
            );
        }

        "/exit" | "/quit" => {
            // pseudo-handled — actual exit is via Ctrl-C; tell user.
            push("  press Ctrl-C to exit the TUI.".to_string());
        }

        "/clear" => {
            let chat_id = app.lock().unwrap().chat_id.clone();
            conversations.evict(&chat_id).await;
            let mut s = app.lock().unwrap();
            s.transcript.clear();
            s.transcript.push(TranscriptItem::System(format!(
                "  ◆ cleared transcript and session history for {}", chat_id
            )));
        }

        "/plan" => {
            let cur = std::env::var("PHANTOM_PLAN_MODE").unwrap_or_default();
            if cur == "1" {
                std::env::remove_var("PHANTOM_PLAN_MODE");
                push("  ✓ plan mode OFF — agents execute tools immediately.".to_string());
            } else {
                std::env::set_var("PHANTOM_PLAN_MODE", "1");
                push("  ✓ plan mode ON — agents will preview their plan, then wait for 'go' before any tool call.".to_string());
            }
        }

        "/agent" => {
            let cfg = runtime.config();
            if let Some(name) = arg {
                if cfg.agent.contains_key(name) {
                    let mut s = app.lock().unwrap();
                    if let Some(idx) = AGENTS.iter().position(|&a| a == name) {
                        s.agent_idx = idx;
                    }
                    push(format!("  ◆ active agent: {}", name));
                } else {
                    let names: Vec<_> = cfg.agent.keys().cloned().collect();
                    push_err(format!("  unknown agent: {} (available: {})", name, names.join(", ")));
                }
            } else {
                let cur = app.lock().unwrap().agent_name().to_string();
                push(format!("  ◆ active agent: {}", cur));
            }
        }

        "/agents" => {
            let cfg = runtime.config();
            let cur = app.lock().unwrap().agent_name().to_string();
            let mut text = format!("  ◆ {} agents configured:\n", cfg.agent.len());
            for (name, ag) in cfg.agent.iter() {
                let marker = if name == &cur { "*" } else { " " };
                let provider = if ag.provider.is_empty() { "<inherit>" } else { ag.provider.as_str() };
                text.push_str(&format!("    {} {:<14} provider={}\n", marker, name, provider));
            }
            push(text);
        }

        "/tools" => {
            let cfg = runtime.config();
            let cur_agent = app.lock().unwrap().agent_name().to_string();
            let tools = cfg.agent.get(&cur_agent)
                .map(|a| a.tools.clone())
                .unwrap_or_default();
            if tools.is_empty() {
                push(format!("  ◆ agent '{}' has no tools configured", cur_agent));
            } else {
                let text = format!("  ◆ {} tools enabled for {}:\n    {}",
                    tools.len(), cur_agent, tools.join(", "));
                push(text);
            }
        }

        "/sessions" => {
            let ids = conversations.list().await;
            let cur = app.lock().unwrap().chat_id.clone();
            if ids.is_empty() {
                push("  no saved sessions.".into());
            } else {
                let mut text = format!("  ◆ {} sessions:", ids.len());
                for id in ids.iter().take(15) {
                    let m = if id == &cur { "*" } else { " " };
                    text.push_str(&format!("\n    {} {}", m, id));
                }
                push(text);
            }
        }

        "/resume" => {
            let ids = conversations.list().await;
            let target = match arg {
                Some(a) if ids.iter().any(|id| id == a) => Some(a.to_string()),
                Some(a) => {
                    let m: Vec<&String> = ids.iter().filter(|id| id.starts_with(a)).collect();
                    match m.len() {
                        0 => None,
                        1 => Some(m[0].clone()),
                        _ => { push_err(format!("  '{}' is ambiguous ({} matches)", a, m.len())); return; }
                    }
                }
                None => {
                    // pick most-recent by mtime
                    let home = dirs::home_dir().unwrap_or_default()
                        .join(".phantom-mesh").join("conversations");
                    ids.into_iter()
                        .filter_map(|id| {
                            let p = home.join(format!("{}.jsonl", id));
                            std::fs::metadata(&p).ok()
                                .and_then(|m| m.modified().ok())
                                .map(|t| (t, id))
                        })
                        .max_by_key(|(t, _)| *t)
                        .map(|(_, id)| id)
                }
            };
            match target {
                Some(id) => {
                    let mut s = app.lock().unwrap();
                    s.chat_id = id.clone();
                    s.transcript.clear();
                    s.transcript.push(TranscriptItem::System(format!("  ◆ resumed session {}", id)));
                }
                None => push_err(format!("  no session matched: {:?}", arg)),
            }
        }

        "/todo" => {
            // Subcommand handling. Bare `/todo` lists; `/todo add <text>`
            // / `/todo done <text>` are explicitly redirected to the
            // todo_add / todo_update tools — the slash command UI is
            // read-only on purpose so the agent's view of the todo
            // store stays the source of truth (an arbitrary edit from
            // the prompt would bypass the agent's plan tracking).
            // Before this, `/todo add foo` was silently dropped on the
            // floor and rendered "no todos." which made users think
            // the add succeeded — see Bug #7 in the 2026-05-01 sweep.
            if let Some(sub) = arg {
                let head = sub.split_whitespace().next().unwrap_or("");
                if matches!(head, "add" | "done" | "update" | "remove" | "rm" | "delete") {
                    push_err(format!(
                        "  /todo is read-only here. To {} a todo, ask the agent — it has \
                         the `todo_add` / `todo_update` tools and they update the same store.",
                        head,
                    ));
                    return;
                }
                push_err(format!("  unknown /todo subcommand: {sub}"));
                return;
            }

            let path = dirs::home_dir().map(|h| h.join(".phantom-mesh").join("todos.json"));
            let raw = path.as_ref()
                .and_then(|p| std::fs::read_to_string(p).ok())
                .unwrap_or_else(|| "[]".to_string());
            let parsed: serde_json::Value = serde_json::from_str(&raw)
                .unwrap_or(serde_json::Value::Array(vec![]));
            let items = parsed.get("todos").and_then(|v| v.as_array())
                .or_else(|| parsed.as_array())
                .cloned().unwrap_or_default();
            if items.is_empty() {
                push("  no todos.".into());
            } else {
                let mut text = format!("  ◆ {} todo{}:", items.len(), if items.len()==1 {""} else {"s"});
                for it in items.iter() {
                    let st   = it.get("status").and_then(|v| v.as_str()).unwrap_or("?");
                    let txt  = it.get("text").and_then(|v| v.as_str()).unwrap_or("?");
                    let dot  = match st { "done" => "●", "in_progress" => "◐", _ => "○" };
                    text.push_str(&format!("\n    {} {}", dot, txt));
                }
                push(text);
            }
        }

        "/cost" => {
            let summary = cost_tracker.summary().await;
            let total   = summary["total_usd"].as_f64().unwrap_or(0.0);
            let session = summary["session_usd"].as_f64().unwrap_or(0.0);
            let reqs    = summary["requests"].as_u64().unwrap_or(0);
            push(format!("  ◆ cost: ${:.4} session  ${:.4} total  ·  {} requests", session, total, reqs));
        }

        "/perm" => {
            match arg {
                Some(m @ ("ask" | "allow" | "deny")) => {
                    std::env::set_var("PHANTOM_PERM", m);
                    push(format!("  ◆ permission mode: {}", m));
                }
                Some(other) => push_err(format!("  unknown: {}. usage: /perm ask|allow|deny", other)),
                None => {
                    let cur = std::env::var("PHANTOM_PERM").unwrap_or_else(|_| "allow".into());
                    push(format!("  ◆ permission mode: {}  (usage: /perm ask|allow|deny)", cur));
                }
            }
        }

        "/density" => {
            match arg {
                Some("compact") => {
                    std::env::set_var("PHANTOM_DENSITY", "compact");
                    push("  ◆ density: compact (1-line tool results)".into());
                }
                Some("full") | Some("normal") => {
                    std::env::remove_var("PHANTOM_DENSITY");
                    push("  ◆ density: full (multi-line)".into());
                }
                Some(other) => push_err(format!("  unknown: {}", other)),
                None => {
                    let cur = std::env::var("PHANTOM_DENSITY").unwrap_or_else(|_| "full".into());
                    push(format!("  ◆ density: {}", cur));
                }
            }
        }

        "/theme" => {
            match arg {
                Some(n @ ("dark" | "light" | "claude" | "codex" | "gemini" | "mono")) => {
                    std::env::set_var("PHANTOM_THEME", n);
                    push(format!("  ◆ theme: {} (restart TUI for full effect)", n));
                }
                Some(other) => push_err(format!("  unknown theme: {}", other)),
                None => {
                    let cur = std::env::var("PHANTOM_THEME").unwrap_or_else(|_| "dark".into());
                    push(format!("  ◆ theme: {}", cur));
                }
            }
        }

        // ── /freeze · /resume — pause/resume the redraw loop so the
        //   terminal's drag-selection isn't erased between frames.
        //   Useful in old conhost / Apple Terminal where any screen
        //   write clears the mouse selection. Workflow:
        //       /freeze       (TUI stops drawing — your last frame stays)
        //       drag-select with mouse, right-click → copy
        //       /resume       (TUI resumes drawing)
        //   Slash commands themselves still work — they push to transcript
        //   but the change won't be visible until you /resume.
        // ── /cluster — show / ping mesh peers from inside TUI ─────────
        // Without this, the user has to drop to a separate PowerShell
        // tab + run `phantom cluster status` to verify peers are alive.
        // Same call surface as the CLI subcommand:
        //   /cluster                  → status (default)
        //   /cluster status           → status (explicit)
        //   /cluster ping             → status alias
        "/cluster" | "/peers" | "/mesh" => {
            let action = arg.unwrap_or("status").trim();
            match action {
                "" | "status" | "ping" | "ls" | "list" => {
                    push("  ◆ pinging cluster peers …".into());
                    match crate::cli_config::cluster_status_lines().await {
                        Ok(lines) => {
                            let mut text = String::new();
                            for l in lines { text.push_str(&format!("  {}\n", l)); }
                            push(text.trim_end().to_string());
                        }
                        Err(e) => push_err(format!("  ✗ {}", e)),
                    }
                }
                // Live TUIs across the user's mesh — answers "who else is
                // running phantom right now?" without leaving the TUI.
                // Same data as the CLI `phantom sessions` command.
                "who" | "sessions" | "online" => {
                    push("  ◆ fetching live sessions …".into());
                    match crate::cli_config::sessions_lines().await {
                        Ok(lines) => {
                            let mut text = String::new();
                            for l in lines { text.push_str(&format!("  {}\n", l)); }
                            push(text.trim_end().to_string());
                        }
                        Err(e) => push_err(format!("  ✗ {}", e)),
                    }
                }
                "leave" => {
                    match crate::cli_config::cluster_leave_lines() {
                        Ok(lines) => for l in lines { push(format!("  {}", l)); }
                        Err(e) => push_err(format!("  ✗ {}", e)),
                    }
                }
                "help" | "?" => {
                    push("  /cluster              → status of all peers (alias: ping/list)\n  \
                          /cluster status       → same\n  \
                          /cluster who          → live TUIs on other machines (alias: sessions/online)\n  \
                          /cluster leave        → remove [cluster] block from agents.toml\n  \
                          \n  \
                          (To join: run `phantom cluster join <name>` from PowerShell —\n  \
                          requires CLUSTER_SECRET in env which is why it's not in TUI)".into());
                }
                other => push_err(format!("  unknown /cluster sub: {} — try /cluster help", other)),
            }
        }

        // ── /fanout — broadcast a single prompt to every peer in parallel ─
        // Hybrid-mode shortcut so users can fan-out work without composing
        // N separate `@<peer>` lines. Skips self automatically.
        // Usage:
        //   /fanout cargo build
        //   /fanout --agent coder lint
        "/fanout" | "/broadcast" => {
            let arg_str = arg.unwrap_or("").trim().to_string();
            if arg_str.is_empty() {
                push_err("  /fanout <prompt>   broadcast prompt to all peers in parallel.\n  \
                          /fanout --agent coder <prompt>   override remote agent.".into());
                return;
            }
            // Parse optional --agent flag
            let (remote_agent, prompt_body) = if let Some(rest) = arg_str.strip_prefix("--agent ") {
                let mut p = rest.splitn(2, ' ');
                let a = p.next().unwrap_or("master").trim().to_string();
                let b = p.next().unwrap_or("").trim().to_string();
                (a, b)
            } else {
                ("master".to_string(), arg_str.clone())
            };
            if prompt_body.is_empty() {
                push_err("  /fanout: prompt is empty after parsing --agent".into());
                return;
            }
            let peers: Vec<String> = {
                let s = app.lock().unwrap();
                s.sidebar_peers.clone()
            };
            if peers.is_empty() {
                push_err("  /fanout: no peers in sidebar — run `phantom config pull` first".into());
                return;
            }
            push(format!("  ◆ fanout → {} peer(s): {}", peers.len(), peers.join(", ")));
            for peer in peers {
                let peer_c = peer.clone();
                let prompt_c = prompt_body.clone();
                let agent_c = remote_agent.clone();
                let app_clone = app.clone();
                tokio::spawn(async move {
                    let result = crate::cli_config::dispatch_lines(
                        &[], Some(&peer_c), &agent_c, &prompt_c, false,
                    ).await;
                    let mut s = app_clone.lock().unwrap();
                    match result {
                        Ok(lines) => {
                            s.transcript.push(TranscriptItem::System(
                                format!("  ─ {} ─", peer_c)));
                            for l in lines {
                                s.transcript.push(TranscriptItem::System(l));
                            }
                        }
                        Err(e) => s.transcript.push(TranscriptItem::Error(
                            format!("  ✗ {} dispatch failed: {}", peer_c, e))),
                    }
                });
            }
        }

        "/freeze" | "/pause" => {
            let mut s = app.lock().unwrap();
            if s.render_frozen {
                push("  ◆ already frozen — /resume to continue".into());
            } else {
                s.render_frozen = true;
                push("  ◆ render frozen — drag-select text with mouse, then /resume".into());
            }
        }
        // `/unfreeze` and `/play` un-freeze the render after `/freeze`.
        // We deliberately do NOT include `/resume` here even though
        // earlier code did — `/resume` already routes to "resume a
        // conversation by id" higher in this match (line ~2666), so
        // listing it again would be unreachable. Use `/unfreeze` or
        // `/play` to thaw the render; use `/resume <id>` for sessions.
        "/unfreeze" | "/play" => {
            let mut s = app.lock().unwrap();
            if !s.render_frozen {
                push("  ◆ not frozen".into());
            } else {
                s.render_frozen = false;
                push("  ◆ render resumed".into());
            }
        }


        // ── /mouse on|off|status — runtime toggle of mouse capture ────
        // With capture ON, ratatui consumes mouse events for scroll-wheel
        // support but the terminal can't see drag → can't select text.
        // OFF = native text-select works; PgUp/PgDn for scroll instead.
        // Setting `mouse_capture_pending` triggers the main loop to call
        // EnableMouseCapture / DisableMouseCapture on the terminal backend
        // before the next draw, no restart needed.
        "/mouse" => {
            let mut s = app.lock().unwrap();
            match arg {
                Some("on") | Some("enable") => {
                    if s.mouse_capture_active {
                        push("  ◆ mouse capture: already ON".into());
                    } else {
                        s.mouse_capture_pending = Some(true);
                        push("  ◆ mouse capture → ON (scroll-wheel works, text-select blocked)".into());
                    }
                }
                Some("off") | Some("disable") => {
                    if !s.mouse_capture_active {
                        push("  ◆ mouse capture: already OFF".into());
                    } else {
                        s.mouse_capture_pending = Some(false);
                        push("  ◆ mouse capture → OFF (drag to select text; PgUp/PgDn to scroll)".into());
                    }
                }
                Some("status") | None => {
                    let state = if s.mouse_capture_active { "ON" } else { "OFF" };
                    push(format!("  ◆ mouse capture: {}    /mouse on | off  to toggle", state));
                }
                Some(other) => push_err(format!("  unknown: {}.  usage: /mouse on | off | status", other)),
            }
        }

        // ── /model ─────────────────────────────────────────────────────
        // Mirrors REPL semantics. Picker (/model pick) needs a blocking
        // readline which is awkward in TUI's single-input panel — point
        // those users at `phantom --repl` instead.
        "/model" => {
            let cfg = runtime.config();
            let arg_str = arg.unwrap_or("");
            let (action, target) = arg_str.split_once(' ')
                .map(|(a, t)| (a.trim(), t.trim()))
                .unwrap_or((arg_str, ""));

            if arg_str.is_empty() {
                let mut text = String::from("  ◆ model — current + provider defaults:\n");
                let cur_agent = app.lock().unwrap().agent_name().to_string();
                let cur_model = cfg.agent.get(&cur_agent)
                    .map(|a| a.model.as_str())
                    .unwrap_or("<unset>");
                text.push_str(&format!("    active: {}/{}\n", cur_agent, cur_model));
                if !cfg.providers.is_empty() {
                    text.push_str("\n    providers:\n");
                    for (pname, pent) in cfg.providers.iter() {
                        let model = pent.default_model.as_deref().unwrap_or("<no default_model>");
                        text.push_str(&format!("      • {:<14} {}\n", pname, model));
                    }
                }
                text.push_str("\n    switch:  /model <name>            (model only)\n");
                text.push_str("             /model <provider>:<name>   (provider + model)\n");
                text.push_str("             /model fast | smart | cheap\n");
                text.push_str("             /model fetch <provider>    (live model list)\n");
                text.push_str("             /model pick  <provider>    (REPL only — phantom --repl)");
                push(text);
            } else if action == "fetch" {
                if target.is_empty() {
                    push_err("  usage: /model fetch <provider>".into());
                } else {
                    let ent = cfg.providers.get(target).cloned();
                    match ent {
                        None => push_err(format!("  unknown provider '{}'.", target)),
                        Some(ent) => {
                            let key = ent.api_key.clone().filter(|s| !s.is_empty())
                                .or_else(|| ent.api_key_env.as_ref()
                                    .and_then(|v| std::env::var(v).ok())
                                    .filter(|s| !s.is_empty()));
                            let url = ent.url.clone()
                                .or_else(|| crate::keys::default_provider_meta(target).map(|(_, u)| u.to_string()))
                                .unwrap_or_default();
                            match (key, url.is_empty()) {
                                (None, _) => push_err(format!("  no key for {} — /keys add {} first", target, target)),
                                (_, true) => push_err(format!("  no base url for {}", target)),
                                (Some(k), false) => {
                                    push(format!("  ◆ fetching models from {} → {} …", target, url));
                                    // Annotated form: each row carries a free/paid heuristic
                                    // (see keys::is_likely_free_model). Free models float to
                                    // the top so users picking from a long list see the no-cost
                                    // options first.
                                    match crate::keys::fetch_models_annotated(&ent.provider_type, &url, &k).await {
                                        Err(e) => push_err(format!("  ✗ {}", e)),
                                        Ok(rows) if rows.is_empty() => push_err("  ⚠ empty model list".into()),
                                        Ok(mut rows) => {
                                            // Free-first ordering, alphabetical within each tier.
                                            rows.sort_by(|a, b| b.is_free.cmp(&a.is_free).then(a.id.cmp(&b.id)));
                                            let n_free = rows.iter().filter(|r| r.is_free).count();
                                            let mut text = format!("  ✓ {} models from {}  ({} free · {} paid):\n",
                                                rows.len(), target, n_free, rows.len() - n_free);
                                            // Each row prints `provider:model` so the user can copy
                                            // the line and paste it directly into `/model <X>:<Y>`
                                            // or into `[agent.X].providers = [...]`.
                                            for r in &rows {
                                                let tag = if r.is_free { "[FREE]" } else { "[paid]" };
                                                text.push_str(&format!("    {} {:<6}  {}:{}\n",
                                                    "•", tag, target, r.id));
                                            }
                                            text.push_str(&format!("\n  switch:  /model {}:<name>", target));
                                            text.push_str("\n  free/paid is a heuristic — confirm with the provider's billing dashboard before relying on it.");
                                            push(text);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            } else if matches!(action, "fast" | "smart" | "cheap") {
                let preset = match action {
                    "fast"  => &[
                        ("groq",       "llama-3.3-70b-versatile"),
                        ("gemini",     "gemini-2.0-flash-exp"),
                        ("openrouter", "google/gemini-2.0-flash-exp"),
                        ("openai",     "gpt-4o-mini"),
                    ][..],
                    "smart" => &[
                        ("anthropic",  "claude-sonnet-4-20250514"),
                        ("openai",     "gpt-4o"),
                        ("openrouter", "anthropic/claude-sonnet-4"),
                        ("groq",       "llama-3.3-70b-versatile"),
                    ][..],
                    "cheap" => &[
                        ("groq",       "llama-3.1-8b-instant"),
                        ("gemini",     "gemini-2.0-flash-lite"),
                        ("openrouter", "google/gemini-2.0-flash-lite"),
                        ("opencode",   "claude-haiku-4-5-free"),
                    ][..],
                    _ => &[][..],
                };
                let pick = preset.iter().find(|(p, _)| cfg.providers.contains_key(*p)).copied();
                match pick {
                    None => push_err(format!(
                        "  ✗ no '{}' preset — none of [{}] are configured. /keys add <provider>",
                        action,
                        preset.iter().map(|(p, _)| *p).collect::<Vec<_>>().join(", "),
                    )),
                    Some((pname, mname)) => {
                        let value = format!("{}:{}", pname, mname);
                        std::env::set_var("PHANTOM_RUNTIME_OVERRIDE", &value);
                        std::env::set_var("PHANTOM_PROVIDER_OVERRIDE", pname);
                        let _ = crate::cli_config::write_runtime_override(Some(&value));
                        let mut s = app.lock().unwrap();
                        s.model_override = Some(mname.to_string());
                        push(format!("  ✓ {} preset → {}/{} (saved to ~/.phantom-mesh/runtime-override)", action, pname, mname));
                    }
                }
            } else if let Some((pname, mname)) = arg_str.split_once(':') {
                if !cfg.providers.contains_key(pname) {
                    push_err(format!("  unknown provider '{}'", pname));
                } else {
                    // Set the override TWO ways so all dispatch paths see it:
                    //   1. PHANTOM_RUNTIME_OVERRIDE env (this process only)
                    //   2. ~/.phantom-mesh/runtime-override file (shared with
                    //      the separately-spawned `phantom serve` daemon and
                    //      any other phantom process the user runs)
                    let value = format!("{}:{}", pname, mname);
                    std::env::set_var("PHANTOM_RUNTIME_OVERRIDE", &value);
                    std::env::set_var("PHANTOM_PROVIDER_OVERRIDE", pname);
                    let _ = crate::cli_config::write_runtime_override(Some(&value));
                    let mut s = app.lock().unwrap();
                    s.model_override = Some(mname.to_string());
                    push(format!("  ✓ switched to {}/{} for this session", pname, mname));
                    push("    (effective on next message; persists in ~/.phantom-mesh/runtime-override so `phantom serve` sees it too)".into());
                }
            } else if action == "pick" {
                push_err("  /model pick is REPL-only (interactive readline) — start `phantom --repl` for it.".into());
            } else {
                let mut s = app.lock().unwrap();
                s.model_override = Some(arg_str.to_string());
                push(format!("  ✓ model override → {}", arg_str));
            }
        }

        // ── /models — fetch ALL providers' catalogs in parallel + show
        //              free/paid grouped per provider, alphabetical inside.
        // Convenience plural form of `/model fetch <X>` — runs the fetch
        // for every configured provider that has a key in env, surfaces
        // the result in one combined panel so the user can pick.
        "/models" => {
            // Optional `--refresh` flag (or `--fresh`) bypasses the cache.
            let arg_str = arg.unwrap_or("");
            let force_refresh = arg_str.contains("--refresh") || arg_str.contains("--fresh");
            let cfg = runtime.config();
            // Collect (name, type, base_url, key) tuples for providers with a key.
            let mut targets: Vec<(String, String, String, String)> = Vec::new();
            let mut skipped: Vec<String> = Vec::new();
            for (name, ent) in cfg.providers.iter() {
                let key = ent.api_key.clone().filter(|s| !s.is_empty())
                    .or_else(|| ent.api_key_env.as_ref()
                        .and_then(|v| std::env::var(v).ok())
                        .filter(|s| !s.is_empty()));
                let url = ent.url.clone()
                    .or_else(|| crate::keys::default_provider_meta(name).map(|(_, u)| u.to_string()))
                    .unwrap_or_default();
                match (key, url.is_empty()) {
                    (Some(k), false) => targets.push((name.clone(), ent.provider_type.clone(), url, k)),
                    (None, _)        => skipped.push(format!("{} (no key — /keys set {} <key>)", name, name)),
                    (_, true)        => skipped.push(format!("{} (no base url)", name)),
                }
            }
            if targets.is_empty() {
                push_err("  no providers with usable keys — /keys set <provider> <key> first".into());
            } else {
                // Split providers into "served from cache" (fresh enough) and
                // "needs live fetch". Cached entries return instantly with the
                // same ModelInfo shape; live ones go through the wire and update
                // the cache for next time.
                let mut cached_results: Vec<(String, Vec<crate::keys::ModelInfo>)> = Vec::new();
                let mut to_fetch: Vec<(String, String, String, String)> = Vec::new();
                if !force_refresh {
                    for (name, ptype, url, key) in &targets {
                        if let Some(rows) = crate::models_cache::get_fresh(name, crate::models_cache::DEFAULT_TTL_MS) {
                            cached_results.push((name.clone(), rows));
                        } else {
                            to_fetch.push((name.clone(), ptype.clone(), url.clone(), key.clone()));
                        }
                    }
                } else {
                    to_fetch = targets.iter().cloned().collect();
                }

                if !cached_results.is_empty() && to_fetch.is_empty() {
                    push(format!("  ◆ all {} provider(s) served from cache (use /models --refresh to force live fetch)",
                        cached_results.len()));
                } else if !to_fetch.is_empty() {
                    push(format!("  ◆ fetching {} provider(s) live, {} from cache (TTL {}m) …",
                        to_fetch.len(), cached_results.len(),
                        crate::models_cache::DEFAULT_TTL_MS / 60_000));
                }

                // Parallel fetch via futures::join_all (mirrors agent.rs:5).
                let futs = to_fetch.iter().map(|(name, ptype, url, key)| {
                    let name = name.clone();
                    let ptype = ptype.clone();
                    let url = url.clone();
                    let key = key.clone();
                    async move {
                        let result = crate::keys::fetch_models_annotated(&ptype, &url, &key).await;
                        // Update cache on success.
                        if let Ok(rows) = &result {
                            crate::models_cache::put(&name, rows);
                        }
                        (name, result)
                    }
                });
                let mut results: Vec<(String, anyhow::Result<Vec<crate::keys::ModelInfo>>)> =
                    futures::future::join_all(futs).await;
                // Merge cached into results so the per-provider render loop
                // below is unified.
                for (name, rows) in cached_results {
                    results.push((name, Ok(rows)));
                }
                results.sort_by(|a, b| a.0.cmp(&b.0));

                let mut total_free  = 0usize;
                let mut total_paid  = 0usize;
                let mut text = String::new();
                // Build a flat ordered list (provider:model entries) AS we
                // render. /pick references it by 1-based row number.
                let mut flat: Vec<String> = Vec::new();
                for (name, res) in results {
                    text.push_str(&format!("\n  ── {} ──\n", name));
                    match res {
                        Err(e) => {
                            text.push_str(&format!("    ✗ {}\n", e));
                        }
                        Ok(mut rows) => {
                            // Free first, alphabetical inside each tier.
                            rows.sort_by(|a, b| b.is_free.cmp(&a.is_free).then(a.id.cmp(&b.id)));
                            let n_free = rows.iter().filter(|r| r.is_free).count();
                            total_free += n_free;
                            total_paid += rows.len() - n_free;
                            text.push_str(&format!("    {} models  ({} free · {} paid)\n",
                                rows.len(), n_free, rows.len() - n_free));
                            // Print with a global 1-based row number so the
                            // user can /pick by number instead of retyping
                            // long ids. Each row is also still copy-pasteable
                            // (`provider:model` form) for direct /model use.
                            for r in &rows {
                                let tag = if r.is_free { "[FREE]" } else { "[paid]" };
                                let entry = format!("{}:{}", name, r.id);
                                flat.push(entry.clone());
                                text.push_str(&format!("      {:>3}. {} {}\n",
                                    flat.len(), tag, entry));
                            }
                        }
                    }
                }
                if !skipped.is_empty() {
                    text.push_str("\n  skipped:\n");
                    for s in &skipped { text.push_str(&format!("    • {}\n", s)); }
                }
                let header = format!(
                    "  ◆ {} providers · {} free · {} paid total\n  switch by id:    /model <provider>:<name>\n  set priority by row#:  /pick <agent> <n1> <n2> ...   (uses the numbers below)",
                    targets.len(), total_free, total_paid);
                push(header);
                push(text);
                push("  free/paid is a heuristic — confirm with the provider's billing dashboard.".into());
                // Save the rendered ordering for /pick to reference by number.
                {
                    let mut s = app.lock().unwrap();
                    s.last_models_list = flat;
                }
            }
        }

        // ── /pick <agent> <n1> <n2> ... — set agent.X.providers from
        //   the numbered rows of the most recent /models output. Saves
        //   the user from typing long provider:model strings; just point
        //   at the row numbers.
        //   Example workflow:
        //       /models                         (lists 46 models, numbered)
        //       /pick master 12 4 1 39          (master.providers becomes
        //                                        [row#12, row#4, row#1, row#39])
        //   Numbers out of range are skipped with a warning.
        "/pick" => {
            let arg_str = arg.unwrap_or("");
            let mut parts = arg_str.split_whitespace();
            let agent = parts.next();
            let nums: Vec<&str> = parts.collect();
            if agent.is_none() || nums.is_empty() {
                push_err("  usage: /pick <agent> <n1> <n2> ...     (run /models first to get the row numbers)".into());
            } else {
                let last = {
                    let s = app.lock().unwrap();
                    s.last_models_list.clone()
                };
                if last.is_empty() {
                    push_err("  no model list cached — run /models first".into());
                } else {
                    let mut picked: Vec<String> = Vec::new();
                    let mut bad: Vec<String> = Vec::new();
                    for n in &nums {
                        match n.parse::<usize>() {
                            Ok(i) if i >= 1 && i <= last.len() => picked.push(last[i - 1].clone()),
                            Ok(i) => bad.push(format!("{} (out of range 1..{})", i, last.len())),
                            Err(_) => bad.push(format!("{} (not a number)", n)),
                        }
                    }
                    for b in &bad { push_err(format!("  ⚠ skipped {}", b)); }
                    if picked.is_empty() {
                        push_err("  no valid numbers provided".into());
                    } else {
                        match crate::cli_config::providers_priority_lines(agent, &picked) {
                            Ok(lines) => for line in lines { push(format!("  {}", line)); },
                            Err(e) => push_err(format!("  ✗ {}", e)),
                        }
                    }
                }
            }
        }

        // ── /keys ──────────────────────────────────────────────────────
        // list / remove / test work in TUI; add is REPL-only (interactive
        // readline + tcflush would tangle with TUI's input event loop).
        "/keys" => {
            let arg_str = arg.unwrap_or("");
            let mut sub_parts = arg_str.splitn(2, ' ');
            let action = sub_parts.next().unwrap_or("").trim();
            let target = sub_parts.next().unwrap_or("").trim();

            match action {
                "" | "list" => {
                    let cfg = runtime.config();
                    let states = crate::keys::snapshot_states(&cfg);
                    if states.is_empty() {
                        push("  ◆ no providers configured. /keys add <name> in REPL mode.".into());
                    } else {
                        let mut text = format!("  ◆ {} providers:\n", states.len());
                        for (name, state) in &states {
                            let badge = match state {
                                crate::keys::KeyState::Inline                  => "✓ inline".to_string(),
                                crate::keys::KeyState::EnvResolved { var }     => format!("✓ env (${})", var),
                                crate::keys::KeyState::EnvMissing  { var }     => format!("⚠ env-unset (${})", var),
                                crate::keys::KeyState::NotConfigured           => "✗ no key".to_string(),
                            };
                            text.push_str(&format!("    • {:<14} {}\n", name, badge));
                        }
                        text.push_str("\n  /keys test <provider>    probe the key\n");
                        text.push_str("  /keys remove <provider>  drop api_key from agents.toml\n");
                        text.push_str("  /keys add — REPL only (interactive paste). `phantom --repl` then /keys add <name>");
                        push(text);
                    }
                }
                "remove" | "rm" => {
                    if target.is_empty() {
                        push_err("  usage: /keys remove <provider>".into());
                    } else {
                        let path = crate::keys::agents_toml_path();
                        match crate::keys::remove_api_key(&path, target) {
                            Ok(()) => push(format!("  ✓ dropped api_key for {} (restart phantom to apply)", target)),
                            Err(e) => push_err(format!("  ✗ {}", e)),
                        }
                    }
                }
                "test" => {
                    if target.is_empty() {
                        push_err("  usage: /keys test <provider>".into());
                    } else {
                        let cfg = runtime.config();
                        let ent = cfg.providers.get(target).cloned();
                        match ent {
                            None => push_err(format!("  unknown provider '{}'", target)),
                            Some(ent) => {
                                let key = ent.api_key.clone().filter(|s| !s.is_empty())
                                    .or_else(|| ent.api_key_env.as_ref()
                                        .and_then(|v| std::env::var(v).ok())
                                        .filter(|s| !s.is_empty()));
                                let url = ent.url.clone()
                                    .or_else(|| crate::keys::default_provider_meta(target).map(|(_, u)| u.to_string()))
                                    .unwrap_or_default();
                                match (key, url.is_empty()) {
                                    (None, _) => push_err(format!("  no key for {}", target)),
                                    (_, true) => push_err(format!("  no base url for {}", target)),
                                    (Some(k), false) => {
                                        push(format!("  ◆ probing {} → {} (5s timeout) …", target, url));
                                        match crate::keys::probe_provider(target, &url, &k).await {
                                            Ok(r) => {
                                                let mark = if r.ok { "✓" } else { "✗" };
                                                push(format!("  {} {} ({} ms)", mark, r.message, r.elapsed_ms));
                                            }
                                            Err(e) => push_err(format!("  ✗ transport: {}", e)),
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                "add" => {
                    push_err("  /keys add is REPL only (interactive paste). Run `phantom --repl` then /keys add <provider>.".into());
                }
                "set" => {
                    // Persist to ~/.phantom-mesh/env (the auto-loaded env file).
                    // Different from /keys remove which scrubs inline api_key from
                    // agents.toml — /keys set updates the env-var-based path so
                    // the existing api_key_env = "..." resolution sees it.
                    let mut tparts = target.splitn(2, ' ');
                    let provider = tparts.next().unwrap_or("").trim();
                    let key = tparts.next().unwrap_or("").trim();
                    if provider.is_empty() || key.is_empty() {
                        push_err("  usage: /keys set <provider> <key>".into());
                    } else {
                        match crate::cli_config::keys_set_lines(provider, key) {
                            Ok(lines) => {
                                for line in lines { push(format!("  {}", line)); }
                            }
                            Err(e) => push_err(format!("  ✗ {}", e)),
                        }
                    }
                }
                other => push_err(format!("  unknown /keys subcommand: {}", other)),
            }
        }

        // ── /provider — list providers OR adjust failover priority ──────
        "/provider" | "/providers" => {
            let arg_str = arg.unwrap_or("");
            let mut sub_parts = arg_str.splitn(2, ' ');
            let action = sub_parts.next().unwrap_or("").trim();
            let rest   = sub_parts.next().unwrap_or("").trim();
            match action {
                "" | "list" => {
                    let cfg = runtime.config();
                    if cfg.providers.is_empty() {
                        push_err("  no [providers.*] block in agents.toml — /keys add <name> in REPL".into());
                    } else {
                        let mut text = format!("  ◆ {} configured providers:\n", cfg.providers.len());
                        for (name, ent) in cfg.providers.iter() {
                            let key_state = if ent.api_key.as_ref().filter(|s| !s.is_empty()).is_some() {
                                "✓ key"
                            } else if ent.api_key_env.is_some() {
                                "✓ env"
                            } else {
                                "✗ no key"
                            };
                            let model = ent.default_model.as_deref().unwrap_or("<none>");
                            text.push_str(&format!("    • {:<14} {} · {}\n", name, key_state, model));
                        }
                        text.push_str("\n  /provider priority <agent>                 show current failover order\n");
                        text.push_str("  /provider priority <agent> <p1> <p2> ...   set the failover order");
                        push(text);
                    }
                }
                "priority" => {
                    // Parse "<agent> <p1> <p2> ..." into agent + name list.
                    let mut tparts = rest.split_whitespace();
                    let agent = tparts.next();
                    let names: Vec<String> = tparts.map(String::from).collect();
                    if agent.is_none() {
                        push_err("  usage: /provider priority <agent> [<p1> <p2> ...]".into());
                    } else {
                        match crate::cli_config::providers_priority_lines(agent, &names) {
                            Ok(lines) => for line in lines { push(format!("  {}", line)); },
                            Err(e) => push_err(format!("  ✗ {}", e)),
                        }
                    }
                }
                other => push_err(format!("  unknown /provider subcommand: {} — try list / priority", other)),
            }
        }

        // ── /priority — open interactive priority picker ──────────────
        // Modal popup that lists [agent.X].providers and lets the user
        // arrow-key reorder, delete, and save. Acts on the currently
        // active agent unless a name is given as arg.
        "/priority" | "/prio" => {
            let target_agent = arg.map(String::from)
                .unwrap_or_else(|| {
                    let s = app.lock().unwrap();
                    s.agent_name().to_string()
                });
            let items = crate::cli_config::read_agent_priority(&target_agent);
            if items.is_empty() {
                push_err(format!(
                    "  no [agent.{}].providers list set. Use /provider priority {} <p1> <p2> ... to seed one first.",
                    target_agent, target_agent));
                return;
            }
            let mut s = app.lock().unwrap();
            s.priority_picker = Some(PriorityPicker {
                agent_name: target_agent,
                items,
                focused: 0,
                dirty: false,
            });
        }

        // ── /sidebar ──────────────────────────────────────────────────
        // Toggle / configure the right-side targets rail. Auto-hides on
        // narrow terminals (<100 cols) regardless of this setting.
        //
        // Lock-discipline note: the `push` / `push_err` closures defined
        // at the top of this function each take their OWN lock on `app`.
        // Calling them while we hold a lock here would deadlock the
        // render loop — that was the literal "screen freezes after
        // /sidebar Enter" bug. So we acquire, mutate, release, then push
        // (or push directly via the held guard, but never both).
        "/sidebar" => {
            let action = arg.unwrap_or("toggle").trim();
            match action {
                "" | "toggle" => {
                    let now = {
                        let mut s = app.lock().unwrap();
                        s.sidebar_visible = !s.sidebar_visible;
                        s.sidebar_visible
                    };
                    push(format!("  ◆ sidebar {}", if now { "on" } else { "off" }));
                }
                "on" | "show" => {
                    { app.lock().unwrap().sidebar_visible = true; }
                    push("  ◆ sidebar on".into());
                }
                "off" | "hide" => {
                    { app.lock().unwrap().sidebar_visible = false; }
                    push("  ◆ sidebar off".into());
                }
                "refresh" | "sync" => {
                    let me = crate::cli_config::resolve_self_node_name();
                    let new_peers: Vec<String> = crate::cli_config::read_peers_json()
                        .map(|peers| peers.into_iter()
                            .map(|p| p.name)
                            .filter(|n| Some(n.as_str()) != me.as_deref())
                            .collect())
                        .unwrap_or_default();
                    let n = new_peers.len();
                    { app.lock().unwrap().sidebar_peers = new_peers; }
                    push(format!("  ◆ sidebar peers refreshed: {} peer(s)", n));
                }
                "status" => {
                    // Read terminal size directly — handler doesn't have
                    // frame access. Useful when /sidebar appears to do
                    // nothing (gate vs flag mismatch).
                    let (cols, rows) = crossterm::terminal::size()
                        .unwrap_or((0, 0));
                    let (visible, peer_count) = {
                        let s = app.lock().unwrap();
                        (s.sidebar_visible, s.sidebar_peers.len())
                    };
                    let needs = 32 + 30; // sidebar + min transcript
                    let actually_shown = visible && cols >= needs;
                    push(format!(
                        "  ◆ sidebar status:\n  \
                           flag: {}  ({} cols × {} rows; needs ≥{} cols to display)\n  \
                           actually shown right now: {}\n  \
                           peers cached: {}",
                        if visible { "on" } else { "off" },
                        cols, rows, needs,
                        if actually_shown { "YES" } else { "no (auto-hidden — window too narrow OR flag off)" },
                        peer_count,
                    ));
                }
                "help" | "?" => {
                    push("  /sidebar              → toggle on/off\n  \
                          /sidebar on|off       → set explicitly\n  \
                          /sidebar status       → show flag + actual visibility + window size\n  \
                          /sidebar refresh      → reload peers from peers.json".into());
                }
                other => push_err(format!("  unknown /sidebar sub: {} — try /sidebar help", other)),
            }
        }

        // ── /copy ─────────────────────────────────────────────────────
        "/copy" => {
            let mode = arg.unwrap_or("");
            let chat_id = app.lock().unwrap().chat_id.clone();
            let history = conversations.get_history(&chat_id).await;
            let payload: String = match mode {
                // /copy transcript — entire on-screen TUI scrollback INCLUDING
                // System items (slash command output like /models, /provider,
                // /keys list, etc.). Use this when the conversation history
                // alone (/copy all) won't capture what you want, e.g. you ran
                // /models and want the model list in your clipboard.
                "transcript" | "screen" | "tui" => {
                    let s = app.lock().unwrap();
                    let mut buf = String::new();
                    for item in &s.transcript {
                        match item {
                            TranscriptItem::User(t)         => buf.push_str(&format!("◆ {}\n", t)),
                            TranscriptItem::Assistant(t) | TranscriptItem::AssistantPartial(t)
                                                            => buf.push_str(&format!("● {}\n", t)),
                            TranscriptItem::ToolCall { name, args } => buf.push_str(&format!("● {}({})\n", name, args)),
                            TranscriptItem::ToolResult { name, output } => buf.push_str(&format!("  ✓ {}: {}\n", name, output)),
                            TranscriptItem::System(t)       => buf.push_str(&format!("{}\n", t)),
                            TranscriptItem::Error(t)        => buf.push_str(&format!("✗ {}\n", t)),
                            TranscriptItem::Warning(t)      => buf.push_str(&format!("⚠ {}\n", t)),
                            TranscriptItem::Thinking(_) | TranscriptItem::ThinkingPartial(_) => {}
                        }
                    }
                    buf
                }
                "all" => conversations.export_markdown(&chat_id).await,
                "turn" => {
                    let mut last_user: Option<&crate::providers::traits::ChatMessage> = None;
                    let mut last_asst: Option<&crate::providers::traits::ChatMessage> = None;
                    for m in history.iter().rev() {
                        if last_asst.is_none() && m.role == "assistant" {
                            last_asst = Some(m);
                        } else if last_asst.is_some() && m.role == "user" {
                            last_user = Some(m);
                            break;
                        }
                    }
                    let mut s = String::new();
                    if let Some(u) = last_user { s.push_str(&format!("**You:** {}\n\n", u.content.trim())); }
                    if let Some(a) = last_asst { s.push_str(&format!("**Assistant:** {}\n", a.content.trim())); }
                    s
                }
                _ => history.iter().rev()
                    .find(|m| m.role == "assistant")
                    .map(|m| m.content.clone())
                    .unwrap_or_default(),
            };
            if payload.is_empty() {
                push("  ◆ nothing to copy yet.".into());
            } else {
                let cmd = if cfg!(target_os = "macos") { "pbcopy" }
                    else if cfg!(target_os = "linux") { "xclip" }
                    else { "clip" };
                let mut child = std::process::Command::new(cmd)
                    .stdin(std::process::Stdio::piped())
                    .spawn();
                match &mut child {
                    Ok(c) => {
                        if let Some(stdin) = c.stdin.as_mut() {
                            use std::io::Write;
                            let _ = stdin.write_all(payload.as_bytes());
                        }
                        let _ = c.wait();
                        let label = match mode {
                            "all"        => "entire session",
                            "turn"       => "last turn",
                            "transcript" | "screen" | "tui" => "TUI transcript",
                            _            => "last assistant message",
                        };
                        push(format!("  ✓ copied {} ({} chars) via {}", label, payload.len(), cmd));
                    }
                    Err(e) => push_err(format!("  ✗ {}: {}", cmd, e)),
                }
            }
        }

        // ── /export ───────────────────────────────────────────────────
        "/export" => {
            let chat_id = app.lock().unwrap().chat_id.clone();
            let md = conversations.export_markdown(&chat_id).await;
            if md.is_empty() {
                push("  ◆ nothing to export yet.".into());
            } else {
                let path = match arg {
                    Some(p) => std::path::PathBuf::from(p),
                    None => {
                        let ts = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs()).unwrap_or(0);
                        let dir = dirs::home_dir()
                            .unwrap_or_else(|| std::path::PathBuf::from("."))
                            .join(".phantom-mesh/exports");
                        std::fs::create_dir_all(&dir).ok();
                        let safe_id: String = chat_id.chars()
                            .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
                            .collect();
                        dir.join(format!("{}-{}.md", safe_id, ts))
                    }
                };
                match std::fs::write(&path, &md) {
                    Ok(()) => push(format!("  ✓ exported {} chars → {}\n  open it: open \"{}\"",
                        md.len(), path.display(), path.display())),
                    Err(e) => push_err(format!("  ✗ write failed: {}", e)),
                }
            }
        }

        // ── /compact ──────────────────────────────────────────────────
        "/compact" => {
            let chat_id = app.lock().unwrap().chat_id.clone();
            let agent_name_str = app.lock().unwrap().agent_name().to_string();
            let history = conversations.get_history(&chat_id).await;
            if history.len() < 4 {
                push(format!("  ◆ nothing to compact ({} messages). Use /clear to start fresh.", history.len()));
            } else {
                push(format!("  ◆ summarizing {} messages via {}…", history.len(), agent_name_str));
                match crate::session::compact_via_llm(
                    runtime,
                    &agent_name_str,
                    cost_tracker,
                    conversations,
                    &chat_id,
                    &history,
                    6,
                ).await {
                    Ok((dropped, summary_chars)) => {
                        push(format!("  ✓ compacted {} old messages → 1 summary ({} chars), kept last 6.",
                            dropped, summary_chars));
                    }
                    Err(e) => push_err(format!("  ✗ compact failed: {}", e)),
                }
            }
        }

        // ── /whoami ───────────────────────────────────────────────────
        "/whoami" => {
            match crate::auth::load() {
                Some(s) => {
                    let provider = s.provider.clone();
                    let email = s.email.clone();
                    push(format!("  ◆ logged in as {} via {}", email, provider));
                }
                None => push("  ◆ not logged in. /login in REPL mode.".into()),
            }
        }

        // ── /init ─────────────────────────────────────────────────────
        "/init" => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            let content = crate::scaffold::generate_phantom_md_async(&cwd).await;
            let out_path = cwd.join("PHANTOM.md");
            match std::fs::write(&out_path, &content) {
                Ok(()) => push(format!("  ✓ created {}", out_path.display())),
                Err(e) => push_err(format!("  ✗ write failed: {}", e)),
            }
        }

        // ── interactive-only commands — point at REPL ────────────────
        "/login" | "/logout" | "/add" | "/undo" =>
            push_err(format!("  {} is REPL only (interactive prompt). Run `phantom --repl`.", cmd)),

        other => push_err(format!("  unknown command: {}.  type /help for the list", other)),
    }
}

// ── tests ──────────────────────────────────────────────────────────────
//
// Headless TUI render tests via ratatui's TestBackend. Same `ui()`
// function the live TUI uses, rendered into an in-memory Buffer instead
// of a real terminal. Deterministic, fast, CI-able. The companion smoke
// suite at scripts/smoke-mac.sh covers the same surfaces black-box via
// tmux; these run-time tests cover the render-path subset where a
// pseudo-terminal would only add flakiness.

#[cfg(test)]
mod tui_render_tests {
    use super::*;
    use ratatui::backend::TestBackend;

    fn fresh_state() -> AppState {
        AppState {
            transcript: vec![],
            input: String::new(),
            cursor: 0,
            scroll: 0,
            agent_idx: 0,
            running: false,
            last_cost: 0.0,
            session_cost: 0.0,
            chat_id: "test-chat-id".to_string(),
            history: vec![],
            history_idx: None,
            history_saved: String::new(),
            model_override: None,
            mouse_capture_pending: None,
            mouse_capture_active: false,
            render_frozen: false,
            last_models_list: Vec::new(),
            selection: None,
            sidebar_visible: false,
            sidebar_focus: 0,
            sidebar_peers: Vec::new(),
            priority_picker: None,
            last_ctrl_c_at: None,
        }
    }

    fn render(state: &AppState, w: u16, h: u16) -> ratatui::buffer::Buffer {
        let backend = TestBackend::new(w, h);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| ui(f, state)).unwrap();
        term.backend().buffer().clone()
    }

    fn buffer_text(buf: &ratatui::buffer::Buffer) -> String {
        let mut out = String::new();
        for y in 0..buf.area().height {
            for x in 0..buf.area().width {
                out.push_str(buf.cell((x, y)).map(|c| c.symbol()).unwrap_or(""));
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn renders_empty_state_with_box_drawing() {
        let state = fresh_state();
        let buf = render(&state, 80, 24);
        let text = buffer_text(&buf);
        // ratatui's Block draws unicode box chars somewhere in the layout
        let has_box = text.contains('│') || text.contains('─')
                   || text.contains('┌') || text.contains('└');
        assert!(has_box, "expected box-drawing chars; got:\n{}", text);
    }

    #[test]
    fn renders_typed_input() {
        let mut state = fresh_state();
        state.input = "hello-render-marker-91".into();
        state.cursor = state.input.len();
        let buf = render(&state, 80, 24);
        let text = buffer_text(&buf);
        assert!(
            text.contains("hello-render-marker-91"),
            "expected typed text in buffer; got:\n{}", text,
        );
    }

    #[test]
    fn slash_completions_filter_by_prefix() {
        let matches: Vec<&&str> = SLASH_COMMANDS_TUI.iter()
            .filter(|c| c.starts_with("/co"))
            .collect();
        assert!(matches.iter().any(|c| **c == "/cost"), "/co prefix should include /cost");
        // Sanity: unrelated commands are excluded
        assert!(!matches.iter().any(|c| **c == "/help"), "/co should not include /help");
    }

    #[test]
    fn history_up_arrow_recalls_newest() {
        let mut state = fresh_state();
        state.history.push("oldest prompt".into());
        state.history.push("newest-recall-marker".into());
        // Simulate the Up-arrow handler effect: save current input,
        // jump to newest history entry.
        state.history_saved = state.input.clone();
        state.history_idx = Some(state.history.len() - 1);
        state.input = state.history[state.history.len() - 1].clone();
        state.cursor = state.input.len();

        let buf = render(&state, 80, 24);
        let text = buffer_text(&buf);
        assert!(
            text.contains("newest-recall-marker"),
            "Up arrow should bring the newest history entry into the input; got:\n{}", text,
        );
    }

    /// Drives the real `handle_key(Up)` path end-to-end so a regression
    /// in cursor/history wiring fails here instead of only being seen in
    /// the tmux selftest. Mirrors the selftest case: two history entries,
    /// press Up twice, expect input == oldest of the two.
    #[test]
    fn handle_key_up_arrow_recalls_history_end_to_end() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut state = fresh_state();
        state.history.push("older-entry".into());
        state.history.push("newer-entry".into());
        let app = wrap(state);

        let up = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        let _ = handle_key(&app, up);
        assert_eq!(app.lock().unwrap().input, "newer-entry",
            "first Up should pull the newest history entry into the input");
        let _ = handle_key(&app, up);
        assert_eq!(app.lock().unwrap().input, "older-entry",
            "second Up should walk one entry further back");
    }

    #[test]
    fn renders_at_smaller_terminal_size() {
        // Catch panics in layout when the terminal is unusually small.
        // We don't assert specific content — just that draw() doesn't panic.
        let state = fresh_state();
        let _ = render(&state, 40, 12);
    }

    #[test]
    fn renders_cjk_input_without_clipping() {
        // 4 CJK characters = 8 display cells. ratatui's TestBackend places
        // the wide char in cell N and a continuation space in cell N+1, so
        // the buffer dump reads "你 好 世 界" rather than "你好世界". Check
        // each glyph individually.
        let mut state = fresh_state();
        state.input = "你好世界".into();
        state.cursor = state.input.len();

        let buf = render(&state, 80, 24);
        let text = buffer_text(&buf);
        for ch in "你好世界".chars() {
            assert!(
                text.contains(ch),
                "CJK glyph '{}' should appear in render; got:\n{}", ch, text,
            );
        }
    }

    #[test]
    fn move_cursor_up_one_line_preserves_column() {
        // input is "abc\nxyz", cursor at the end (byte 7).
        // Cursor row=1, col=3. Up → row=0, col=3 → byte 3.
        let new_cur = move_cursor_up_one_line("abc\nxyz", 7);
        assert_eq!(new_cur, Some(3));
    }

    #[test]
    fn move_cursor_up_clamps_to_short_prev_line() {
        // input is "ab\nlonger", cursor at end (byte 9, col=6 on line 2).
        // Up → cursor on line 1 which is only "ab" (2 cells), so cursor
        // clamps to end of "ab" → byte 2.
        let new_cur = move_cursor_up_one_line("ab\nlonger", 9);
        assert_eq!(new_cur, Some(2));
    }

    #[test]
    fn move_cursor_up_returns_none_on_first_line() {
        assert_eq!(move_cursor_up_one_line("only one line", 5), None);
    }

    #[test]
    fn move_cursor_up_with_cjk_uses_display_width() {
        // "你好\nabcd" — cursor at end (byte 8 = 6 of CJK + 1 \n + 4 ASCII).
        // Wait: "你好" is 6 bytes, "\n" is 1, "abcd" is 4. Total bytes = 11.
        // Cursor at byte 11 → row=1, col=4 (display width of "abcd").
        // Up → row=0, col=4 cells. "你好" has display width 4 too, so cursor
        // lands at end of "你好" = byte 6.
        let new_cur = move_cursor_up_one_line("你好\nabcd", 11);
        assert_eq!(new_cur, Some(6));
    }

    #[test]
    fn move_cursor_down_returns_none_on_last_line() {
        assert_eq!(move_cursor_down_one_line("only", 4), None);
    }

    #[test]
    fn move_cursor_down_basic() {
        // "abc\nxyz", cursor at byte 1 (col=1 on line 0).
        // Down → line 1, col=1 → byte 4 + 1 = 5.
        let new_cur = move_cursor_down_one_line("abc\nxyz", 1);
        assert_eq!(new_cur, Some(5));
    }

    #[test]
    fn renders_multiline_input_with_newlines() {
        // Regression: previously Ctrl-J / Shift-Enter inserted '\n' into
        // the input string, the box height grew correctly, but the render
        // packed the whole text into a single Line so '\n' rendered as a
        // printable char and all lines collapsed onto one row. Fix splits
        // the input into Vec<Line>.
        let mut state = fresh_state();
        state.input = "line one\nline two\nline three".to_string();
        state.cursor = state.input.len();

        let buf = render(&state, 80, 24);
        let text = buffer_text(&buf);

        // All three line bodies should appear in the buffer
        for needle in ["line one", "line two", "line three"] {
            assert!(text.contains(needle), "{} missing from render:\n{}", needle, text);
        }
        // And they shouldn't all collapse onto one row — count rows that
        // contain a 'line ' prefix
        let line_rows = text.lines()
            .filter(|l| l.contains("line one") || l.contains("line two") || l.contains("line three"))
            .count();
        assert!(
            line_rows >= 3,
            "expected each input line on its own row; got {} rows. Text:\n{}",
            line_rows, text,
        );
    }

    #[test]
    fn renders_long_line_wraps_within_box() {
        // 120 'x' chars on a 60-wide terminal should wrap to 3 lines (58 + 58
        // + 4 with 2-cell border accounting). Earlier the input box was a
        // fixed 1 content row tall, which hid wrapped rows entirely.
        let mut state = fresh_state();
        state.input = "x".repeat(120);
        state.cursor = state.input.len();

        let buf = render(&state, 60, 24);
        let text = buffer_text(&buf);
        // Count rows in the buffer that consist mostly of x's (the wrapped
        // input). Need at least 2 such rows to confirm the box grew.
        let rows_with_xs = text.lines()
            .filter(|line| line.chars().filter(|&c| c == 'x').count() > 30)
            .count();
        assert!(
            rows_with_xs >= 2,
            "Expected long input to wrap to >=2 visible rows; got {} rows of x's. Text:\n{}",
            rows_with_xs, text,
        );
    }

    #[test]
    fn renders_with_a_transcript_item() {
        let mut state = fresh_state();
        state.transcript.push(TranscriptItem::User("hi from test".into()));
        let buf = render(&state, 80, 24);
        let text = buffer_text(&buf);
        // We can't assume exact rendering of a User item without committing
        // to the format, but the literal substring should make it through.
        assert!(
            text.contains("hi from test"),
            "User transcript item should render somewhere; got:\n{}", text,
        );
    }

    #[test]
    fn spinner_animates_while_running_and_clears_when_idle() {
        // Acceptance for EVOLVE-GOALS.md "TUI: spinner left of phantom v0.1.0".
        // The status bar's busy cell must be a phantom diamond glyph while
        // running and a plain space when idle. (Was a Braille rotor; switched
        // to ◇/◈/◆ to differentiate from Codex/Claude Code which all use the
        // same default Braille spinner.)
        fn row0_has_spinner(buf: &ratatui::buffer::Buffer) -> bool {
            (0..buf.area().width).any(|x| {
                let s = buf.cell((x, 0)).map(|c| c.symbol()).unwrap_or("");
                matches!(s, "◇" | "◈" | "◆")
            })
        }

        let mut state = fresh_state();

        state.running = false;
        let buf_idle = render(&state, 80, 5);
        assert!(
            !row0_has_spinner(&buf_idle),
            "idle status bar must not contain a spinner glyph",
        );

        state.running = true;
        let buf_run = render(&state, 80, 5);
        assert!(
            row0_has_spinner(&buf_run),
            "running status bar must contain a phantom diamond glyph",
        );

        // Bonus: ensure the spinner is to the LEFT of the "phantom v" text,
        // not anywhere else on the row.
        let mut spinner_col: Option<u16> = None;
        for x in 0..buf_run.area().width {
            let s = buf_run.cell((x, 0)).map(|c| c.symbol()).unwrap_or("");
            if matches!(s, "◇" | "◈" | "◆") {
                spinner_col = Some(x);
                break;
            }
        }
        let spinner_col = spinner_col.unwrap();
        // Find first column where 'p' from "phantom" lands.
        let mut phantom_col: Option<u16> = None;
        for x in 0..buf_run.area().width {
            let s = buf_run.cell((x, 0)).map(|c| c.symbol()).unwrap_or("");
            if s == "p" {
                let s2 = buf_run.cell((x + 1, 0)).map(|c| c.symbol()).unwrap_or("");
                if s2 == "h" {
                    phantom_col = Some(x);
                    break;
                }
            }
        }
        let phantom_col = phantom_col.expect("'phantom' should appear in status row");
        assert!(
            spinner_col < phantom_col,
            "spinner (col {}) must appear LEFT of 'phantom' (col {})",
            spinner_col, phantom_col,
        );
    }

    #[test]
    fn warning_transcript_item_renders_with_glyph_and_message() {
        // Acceptance for the max_tokens-truncation visibility goal: when
        // the agent emits AgentEvent::Notice (e.g. response truncated at
        // the cap), the user must see a "⚠" glyph and the explanatory
        // text in the transcript scrollback.
        let mut state = fresh_state();
        state.transcript.push(TranscriptItem::Warning(
            "Response truncated: provider hit max_tokens cap (8192). \
             Set PHANTOM_MAX_TOKENS=16384 and re-run.".into(),
        ));

        let buf = render(&state, 100, 24);
        let text = buffer_text(&buf);

        assert!(
            text.contains("⚠"),
            "warning glyph should appear in transcript; got:\n{}", text,
        );
        assert!(
            text.contains("Response truncated"),
            "warning message should appear in transcript; got:\n{}", text,
        );
        assert!(
            text.contains("PHANTOM_MAX_TOKENS"),
            "env var hint should be visible to the user; got:\n{}", text,
        );
    }

    #[test]
    fn last_transcript_line_visible_with_multiline_input() {
        // Acceptance for EVOLVE-GOALS.md "TUI: text output area must NEVER
        // overlap with the input box". We grow the input to 5 logical lines
        // (the cap), push enough transcript content that wrapping forces the
        // overflow path, and assert the latest content marker is still in
        // the rendered buffer.
        let mut state = fresh_state();
        // Big transcript so total_visual >> viewport
        for i in 0..10 {
            state.transcript.push(TranscriptItem::Assistant(format!(
                "earlier line {} {}", i, "z".repeat(150),
            )));
        }
        state.transcript.push(TranscriptItem::Assistant(
            "BOTTOM-MARKER-must-stay-visible".into(),
        ));
        // 5-line input — hits the clamp(1, 5) ceiling.
        state.input = "a\nb\nc\nd\ne".to_string();
        state.cursor = state.input.len();

        let buf = render(&state, 80, 24);
        let text = buffer_text(&buf);

        assert!(
            text.contains("BOTTOM-MARKER-must-stay-visible"),
            "the latest transcript line must remain visible above the multi-line \
             input frame; got:\n{}", text,
        );
        // And the input lines must also render — confirms layout split correctly.
        for needle in ["a", "b", "c", "d", "e"] {
            assert!(
                text.lines().any(|line| line.trim() == needle || line.contains(needle)),
                "input line {:?} should render; got:\n{}", needle, text,
            );
        }
    }

    #[test]
    fn latest_user_message_stays_visible_after_wrap() {
        // Regression test for the disappearing-text bug: when wrapped
        // transcript content overflows the viewport, the *bottom* content
        // (the user's just-typed message + agent reply) must remain
        // visible. The previous version computed scroll offset from logical
        // line count, missing the wrap, and pushed the bottom out of view.
        let mut state = fresh_state();
        // Several earlier messages that wrap heavily
        for i in 0..5 {
            state.transcript.push(TranscriptItem::User(format!(
                "earlier message {} {}", i, "x".repeat(200)
            )));
            state.transcript.push(TranscriptItem::Assistant(format!(
                "earlier reply  {} {}", i, "y".repeat(200)
            )));
        }
        // The latest message — the one whose text user said "disappears"
        state.transcript.push(TranscriptItem::User(
            "MARKER-just-submitted-this-needs-to-show".into(),
        ));

        // Small viewport (40x20 = 18 content rows after borders) forces the
        // overflow scenario.
        let buf = render(&state, 40, 20);
        let text = buffer_text(&buf);
        assert!(
            text.contains("MARKER-just-submitted-this-needs-to-show"),
            "latest User message must remain visible after wrap-induced \
             overflow; got:\n{}", text,
        );
    }

    /// Render two consecutive frames on the SAME backend. The live TUI's
    /// alt-screen buffer persists between draws, so cells not painted by
    /// the second frame still show the first frame's content. Reproduces
    /// the bleed-through bug; the fresh-backend `render()` helper above
    /// would mask it.
    fn render_two_frames(
        s1: &AppState, s2: &AppState, w: u16, h: u16,
    ) -> ratatui::buffer::Buffer {
        let backend = TestBackend::new(w, h);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| ui(f, s1)).unwrap();
        term.draw(|f| ui(f, s2)).unwrap();
        term.backend().buffer().clone()
    }

    #[test]
    fn transcript_does_not_retain_stale_chars_after_replace_with_shorter() {
        // Reproduces the right-margin ghost-text seen on Windows Terminal
        // when a wide transcript line is replaced by a narrower one.
        // Paragraph only paints cells covered by span content; cells
        // beyond the line end keep whatever the previous frame wrote.
        // Fix: render `Clear` over the transcript area before drawing.
        let mut s1 = fresh_state();
        s1.transcript.push(TranscriptItem::Assistant(
            "BEGIN_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA_END".into()
        ));

        let mut s2 = fresh_state();
        s2.transcript.push(TranscriptItem::Assistant("S".into()));

        let buf = render_two_frames(&s1, &s2, 80, 24);
        let text = buffer_text(&buf);

        assert!(
            !text.contains("AAAAAAAA"),
            "stale wide content from frame 1 leaked into frame 2; got:\n{}", text,
        );
    }

    /// Regression test for the CJK-tail-pushed-off-screen bug. The
    /// previous manual visual-rows estimator (sum of ceil(display_width
    /// / inner_w) per Line) under-counted on real markdown bodies that
    /// mix CJK + ASCII + emoji + indent — the bottom-pin offset came
    /// out too small and the tail of a long Assistant message ended
    /// up below the viewport. Switching to Paragraph::line_count(width)
    /// (gated by ratatui's `unstable-rendered-line-info` feature) makes
    /// the count match what the renderer actually wraps to.
    ///
    /// The body intentionally interleaves CJK with markdown headings,
    /// fenced code, emoji, indented bullets, and trailing text. On the
    /// 80×24 viewport a hand-rolled estimator is essentially guaranteed
    /// to disagree with WordWrapper somewhere in this content.
    #[test]
    fn long_cjk_assistant_message_keeps_tail_visible_under_bottom_pin() {
        let body = "## ✅ 完成！GPU 加速圖像卷積程式\n\
            \n\
            我已經為您完成了一個實用的 GPU 調用 Python 程式：\n\
            \n\
            ### 📋 程式主題\n\
            **GPU 加速的圖像卷積濾波器** - 這是電腦視覺和深度學習中的基礎應用\n\
            \n\
            ### 🎯 程式功能\n\
            1. **自動檢查 GPU**：支援 NVIDIA CUDA、Apple MPS,並有 CPU 回退機制\n\
            2. **5 種卷積濾波器**：\n\
               - 模糊 (Gaussian Blur)\n\
               - 銳化 (Sharpen)\n\
               - 邊緣檢測 (Edge Detection)\n\
               - 浮雕 (Emboss)\n\
               - 拉普拉斯 (Laplacian)\n\
            3. **效能測試**:測量每種濾波器的處理時間和 FPS\n\
            \n\
            ### 📂 輸出檔案位置\n\
            ```\n\
            C:\\Users\\m4932\\\n\
            ├── gpu_convolution.py\n\
            ├── gpu_output_input.png\n\
            └── gpu_output\\\n\
                ├── 模糊.png\n\
                ├── 銳化.png\n\
                └── 拉普拉斯.png\n\
            ```\n\
            \n\
            END-OF-MARKDOWN-XYZQ-7732".to_string();

        let mut state = fresh_state();
        state.transcript.push(TranscriptItem::Assistant(body));

        // 80×24 is small enough that the body cannot fit and must be
        // bottom-pinned. The fix's whole point is that the LAST line
        // is what the user sees.
        let buf = render(&state, 80, 24);
        let text = buffer_text(&buf);

        assert!(
            text.contains("XYZQ-7732"),
            "tail marker must remain visible after bottom-pin scroll; got:\n{}",
            text,
        );
    }

    /// Sister test for ASCII-only long content — proves the fix doesn't
    /// regress the simpler case the old hand-rolled estimator handled
    /// reasonably well. If this one starts failing it means the new
    /// line_count-based offset broke a path that used to work.
    #[test]
    fn long_ascii_assistant_message_keeps_tail_visible_under_bottom_pin() {
        let mut body = String::new();
        for i in 0..40 {
            body.push_str(&format!(
                "Line {:02} aaaaaaaa bbbbbbbb cccccccc dddddddd eeeeeeee ffffffff\n",
                i,
            ));
        }
        body.push_str("END-OF-ASCII-MARKER-9911");

        let mut state = fresh_state();
        state.transcript.push(TranscriptItem::Assistant(body));

        let buf = render(&state, 80, 24);
        let text = buffer_text(&buf);

        assert!(
            text.contains("END-OF-ASCII-MARKER-9911"),
            "tail marker on ASCII-only body must remain visible; got:\n{}",
            text,
        );
    }

    // ── Selection: extraction + highlight ─────────────────────────────────
    //
    // The big risk for the user-facing copy feature is that the extracted
    // text doesn't match what they highlighted on screen. These tests
    // build a synthetic ratatui Buffer, set known content, run a known
    // selection through extract_selection_text, and assert the output
    // matches the visible characters.

    fn make_buf(width: u16, lines: &[&str]) -> ratatui::buffer::Buffer {
        let mut b = ratatui::buffer::Buffer::empty(
            ratatui::layout::Rect::new(0, 0, width, lines.len() as u16),
        );
        for (row, line) in lines.iter().enumerate() {
            // Buffer::set_string lays out display-width-aware (CJK = 2).
            b.set_string(0, row as u16, *line, Style::default());
        }
        b
    }

    fn sel(anchor: (u16, u16), cursor: (u16, u16)) -> Selection {
        Selection { anchor, cursor, dragging: false }
    }

    #[test]
    fn extract_single_row_selection_returns_substring() {
        let buf = make_buf(20, &["hello world here"]);
        let s = sel((6, 0), (10, 0));  // "world"
        assert_eq!(extract_selection_text(&buf, s), "world");
    }

    #[test]
    fn extract_multi_row_selection_joins_with_newlines_and_trims_pad() {
        let buf = make_buf(20, &[
            "first row aaaaaaaa",
            "middle row bbb",
            "last row cccccc",
        ]);
        let s = sel((6, 0), (8, 2));  // from col 6 of row 0 to col 8 of row 2
        let got = extract_selection_text(&buf, s);
        // Row 0: "row aaaaaaaa" (col 6..end, trimmed of trailing pad)
        // Row 1: "middle row bbb" (full, trimmed)
        // Row 2: "last row " (col 0..=8, trimmed of trailing space → "last row")
        let expected = "row aaaaaaaa\nmiddle row bbb\nlast row";
        assert_eq!(got, expected,
            "multi-row selection text doesn't match. got:\n{}\nexpected:\n{}", got, expected);
    }

    #[test]
    fn extract_handles_anchor_below_cursor_normalizing_order() {
        // User dragged from bottom-right UP to top-left. Selection.normalized
        // should swap them so extraction still goes top→bottom.
        let buf = make_buf(20, &[
            "row zero",
            "row one",
        ]);
        let s = sel((4, 1), (0, 0));  // anchor=lower, cursor=upper
        let got = extract_selection_text(&buf, s);
        assert_eq!(got, "row zero\nrow o");
    }

    #[test]
    fn pure_click_selection_is_not_meaningful() {
        // Click without drag (anchor == cursor) — caller uses this to skip
        // pushing an empty string to the clipboard.
        let s = sel((5, 3), (5, 3));
        assert!(!s.is_meaningful(),
            "anchor == cursor must read as not-meaningful, otherwise a stray click triggers an empty clipboard write");
    }

    #[test]
    fn meaningful_selection_when_anchor_differs_from_cursor() {
        let s1 = sel((5, 3), (6, 3));     // 1 cell right
        let s2 = sel((5, 3), (5, 4));     // 1 row down
        assert!(s1.is_meaningful());
        assert!(s2.is_meaningful());
    }

    // ── Ctrl-C double-tap-to-exit ────────────────────────────────────────
    //
    // The first press shouldn't quit — historically it did, and we lost
    // sessions to fat-fingered exits. These tests pin the new contract:
    // first press warns or cancels; second press within 2 s exits; an
    // unrelated keystroke disarms the window.
    fn ctrl_c_event() -> KeyEvent {
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)
    }

    fn wrap(state: AppState) -> Arc<Mutex<AppState>> {
        Arc::new(Mutex::new(state))
    }

    #[test]
    fn first_ctrl_c_idle_warns_does_not_exit() {
        let app = wrap(fresh_state());
        let action = handle_key(&app, ctrl_c_event());
        assert!(matches!(action, KeyAction::None),
            "first idle Ctrl-C must NOT exit; got {:?}", std::mem::discriminant(&action));
        let s = app.lock().unwrap();
        assert!(s.last_ctrl_c_at.is_some(), "first press must arm the window");
        let last_msg = s.transcript.last();
        assert!(matches!(last_msg, Some(TranscriptItem::System(m)) if m.contains("again")),
            "expected hint transcript entry, got {:?}", last_msg);
    }

    #[test]
    fn second_ctrl_c_within_window_exits() {
        let app = wrap(fresh_state());
        let _ = handle_key(&app, ctrl_c_event());
        let action = handle_key(&app, ctrl_c_event());
        assert!(matches!(action, KeyAction::Exit),
            "second Ctrl-C inside the window must Exit");
    }

    #[test]
    fn first_ctrl_c_running_cancels() {
        let mut s = fresh_state();
        s.running = true;
        let app = wrap(s);
        let action = handle_key(&app, ctrl_c_event());
        assert!(matches!(action, KeyAction::Cancel),
            "first Ctrl-C while running must Cancel, not Exit");
        assert!(app.lock().unwrap().last_ctrl_c_at.is_some());
    }

    #[test]
    fn unrelated_key_disarms_double_tap() {
        let app = wrap(fresh_state());
        let _ = handle_key(&app, ctrl_c_event());
        // Type a single char — should disarm.
        let _ = handle_key(&app, KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        assert!(app.lock().unwrap().last_ctrl_c_at.is_none(),
            "an unrelated keystroke must clear the arming window");
        let action = handle_key(&app, ctrl_c_event());
        assert!(matches!(action, KeyAction::None),
            "after disarm, next Ctrl-C is treated as a fresh first press");
    }

    #[test]
    fn ctrl_c_with_meaningful_selection_copies_not_arms() {
        let mut s = fresh_state();
        s.selection = Some(sel((1, 1), (5, 1)));
        let app = wrap(s);
        let action = handle_key(&app, ctrl_c_event());
        assert!(matches!(action, KeyAction::CopySelection(_)),
            "selection-priority Ctrl-C must copy");
        // Selection-copy path must NOT arm the exit window — otherwise a
        // user copying twice could quit.
        assert!(app.lock().unwrap().last_ctrl_c_at.is_none(),
            "copy path must not arm the exit window");
    }

    // ── Crash-resistance fuzz tests ──────────────────────────────────────
    //
    // The TUI is a state machine with a non-trivial input alphabet (every
    // KeyCode × KeyModifier combination + mouse events + resize). A bug
    // anywhere — UTF-8 boundary issues, off-by-one in cursor math, panic
    // in selection extraction — surfaces only when a user happens to
    // produce that exact input. These tests don't assert *behavior*; they
    // assert **the process never panics** under arbitrarily-chosen input.
    //
    // Why this is the right layer: panics in handle_key / ui() bubble all
    // the way up through ratatui's draw loop and crash the user's session
    // mid-conversation. A test that throws thousands of random events at
    // the state machine catches drift far cheaper than waiting for a real
    // user to hit the case in production.
    //
    // Reproducible: every test seeds rng with a fixed constant. A future
    // failure can be replayed by reusing the same seed.

    use rand::{Rng, SeedableRng};
    use rand::rngs::StdRng;

    /// Non-character KeyCodes worth randomly throwing at handle_key.
    /// Excluding KeyCode::Char (which we generate separately) and the
    /// less-common variants (Modifier, Media, etc.) that aren't in the
    /// realistic input distribution.
    fn fuzz_keycodes() -> &'static [KeyCode] {
        &[
            KeyCode::Backspace, KeyCode::Enter, KeyCode::Tab, KeyCode::BackTab,
            KeyCode::Up, KeyCode::Down, KeyCode::Left, KeyCode::Right,
            KeyCode::Home, KeyCode::End, KeyCode::PageUp, KeyCode::PageDown,
            KeyCode::Insert, KeyCode::Delete, KeyCode::Esc,
        ]
    }

    fn fuzz_modifiers() -> Vec<KeyModifiers> {
        vec![
            KeyModifiers::NONE,
            KeyModifiers::CONTROL,
            KeyModifiers::SHIFT,
            KeyModifiers::ALT,
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            KeyModifiers::CONTROL | KeyModifiers::ALT,
            KeyModifiers::SHIFT   | KeyModifiers::ALT,
        ]
    }

    fn fuzz_random_key(rng: &mut StdRng) -> KeyEvent {
        let mods_pool = fuzz_modifiers();
        let m = mods_pool[rng.gen_range(0..mods_pool.len())];
        let r: f32 = rng.gen();
        let code = if r < 0.55 {
            // ASCII printable — covers most real input.
            let c = rng.gen_range(0x20u32..0x7Fu32);
            KeyCode::Char(char::from_u32(c).unwrap())
        } else if r < 0.70 {
            // Random Unicode — exercises UTF-8 width + multi-byte cursor math.
            // Bias toward CJK + emoji ranges since those break naive byte
            // arithmetic.
            let c = match rng.gen_range(0..3) {
                0 => rng.gen_range(0x4E00u32..0x9FFFu32),     // CJK Unified
                1 => rng.gen_range(0x1F600u32..0x1F64Fu32),   // emoticons
                _ => rng.gen_range(0x0080u32..0x07FFu32),     // Latin-ext + Greek + Cyrillic
            };
            KeyCode::Char(char::from_u32(c).unwrap_or('?'))
        } else if r < 0.85 {
            // F-keys + control codes
            let codes = fuzz_keycodes();
            codes[rng.gen_range(0..codes.len())]
        } else {
            KeyCode::F(rng.gen_range(1..=12))
        };
        KeyEvent::new(code, m)
    }

    #[test]
    fn fuzz_handle_key_never_panics_on_clean_state() {
        // 10 K events on a fresh state. None should panic; we don't care
        // what they return. The state at the end may be wild — that's OK.
        let mut rng = StdRng::seed_from_u64(0x7044704E_5E_u64);
        let app = wrap(fresh_state());
        for _ in 0..10_000 {
            let _ = handle_key(&app, fuzz_random_key(&mut rng));
        }
    }

    #[test]
    fn fuzz_handle_key_never_panics_on_running_state() {
        // Same fuzz but with `running=true` — exercises the cancel /
        // submit-while-running paths which take different branches.
        let mut rng = StdRng::seed_from_u64(0xC0FFEEDEAD_u64);
        let mut s = fresh_state();
        s.running = true;
        let app = wrap(s);
        for _ in 0..10_000 {
            let _ = handle_key(&app, fuzz_random_key(&mut rng));
        }
    }

    #[test]
    fn fuzz_handle_key_never_panics_with_random_initial_input() {
        // Cursor positioned at every byte boundary inside increasingly-
        // weird input strings. The classic UTF-8 panic is "char_indices
        // gave me boundary X, but I wrote a `..end` slice with `end-1`".
        let mut rng = StdRng::seed_from_u64(0x123_456_789u64);
        let weird_inputs = vec![
            String::from(""),
            String::from("plain ascii"),
            String::from("中文混合 mixed text"),
            String::from("🎉 emoji 👨‍👩‍👧‍👦 zwj-family"),
            String::from("\u{200B}zero-width\u{FEFF}joiner"),
            String::from("a".repeat(10_000)),    // very long
            String::from("中".repeat(5_000)),    // long CJK
            String::from("a\nb\nc\nd\ne"),       // multi-line
        ];
        for input in weird_inputs {
            for cursor_offset in 0..=input.len() {
                // Skip cursor positions mid-codepoint — handle_key trusts
                // the cursor invariant. We only verify no panic at valid
                // boundaries.
                if !input.is_char_boundary(cursor_offset) { continue; }
                let mut s = fresh_state();
                s.input = input.clone();
                s.cursor = cursor_offset;
                let app = wrap(s);
                // 50 random events from this state.
                for _ in 0..50 {
                    let _ = handle_key(&app, fuzz_random_key(&mut rng));
                }
            }
        }
    }

    #[test]
    fn fuzz_render_at_extreme_sizes() {
        // ratatui's TestBackend constructs a buffer of the requested
        // dimensions. The TUI's layout code must not panic on tiny or
        // pathologically tall/wide screens.
        let state = fresh_state();
        for (w, h) in [
            (1u16, 1u16),     // 1×1 — degenerate
            (1, 100),         // 1 wide × 100 tall
            (200, 1),         // 200 wide × 1 tall
            (40, 12),         // smallest practical
            (200, 500),       // huge
            (300, 80),        // ultrawide
            (10, 200),        // narrow phone-ish
        ] {
            let _ = render(&state, w, h);
        }
    }

    #[test]
    fn fuzz_render_with_long_transcript() {
        // 1000 transcript items at varied widths shouldn't blow up either
        // memory or layout. Exercises the bottom-pin scroll math.
        let mut state = fresh_state();
        for i in 0..1000 {
            state.transcript.push(match i % 4 {
                0 => TranscriptItem::User(format!("user line {}", i)),
                1 => TranscriptItem::System(format!("system line {} with some longer content to exercise wrapping", i)),
                2 => TranscriptItem::Assistant("a".repeat(200)), // very wide
                _ => TranscriptItem::Warning(format!("warn {}", i)),
            });
        }
        let _ = render(&state, 80, 24);
        let _ = render(&state, 200, 50);
    }

    #[test]
    fn fuzz_render_with_unicode_stress_content() {
        // Lines that historically break naive width calculations.
        let mut state = fresh_state();
        let huge_line = "x".repeat(10_000);
        let stressors: Vec<&str> = vec![
            "中文很長一行測試unicode width處理是否正確",
            "🎉🎊🎈🎁 emoji 🚀🌟💎🎯 row",
            "👨‍👩‍👧‍👦 ZWJ family one cell or four?",
            "RTL: مرحبا עברית",
            "combining: a\u{0301}e\u{0301}i\u{0301}o\u{0301}u\u{0301}",
            "\u{200B}zero\u{200B}width\u{200B}joiners\u{200B}",
            "control chars: \x07\x08\x0c (ignored)",
            &huge_line,  // single very-long line
        ];
        for s in &stressors {
            state.transcript.push(TranscriptItem::Assistant((*s).to_string()));
        }
        let _ = render(&state, 80, 24);
        let _ = render(&state, 40, 12);
        let _ = render(&state, 200, 50);
    }

    #[test]
    fn fuzz_render_with_extreme_input_field() {
        // The input box has its own wrap math separate from transcript.
        for (label, input) in [
            ("empty", String::new()),
            ("10K ascii", "a".repeat(10_000)),
            ("1K CJK", "中".repeat(1_000)),
            ("single emoji ZWJ", String::from("👨‍👩‍👧‍👦")),
            ("multi-line", "line\n".repeat(500)),
        ] {
            let mut state = fresh_state();
            state.input = input.clone();
            state.cursor = state.input.len();
            // Walk a few cursor positions just in case.
            for cur in [0usize, state.input.len() / 2, state.input.len()] {
                if state.input.is_char_boundary(cur) {
                    state.cursor = cur;
                    let _ = render(&state, 80, 24);
                }
            }
            let _ = label;
        }
    }

    #[test]
    fn fuzz_selection_at_arbitrary_terminal_positions() {
        // Selection extracts text from a rendered buffer using terminal
        // (col, row) coordinates. Off-screen / out-of-bounds selections
        // must not panic; they should just yield empty / clamped output.
        let mut state = fresh_state();
        state.transcript.push(TranscriptItem::Assistant("test row 0".into()));
        state.transcript.push(TranscriptItem::Assistant("中文 row 1".into()));
        let mut rng = StdRng::seed_from_u64(0xB0BAFE77_u64);
        for _ in 0..200 {
            let ax = rng.gen_range(0..100);
            let ay = rng.gen_range(0..30);
            let cx = rng.gen_range(0..100);
            let cy = rng.gen_range(0..30);
            state.selection = Some(sel((ax, ay), (cx, cy)));
            let _ = render(&state, 80, 24);
        }
    }
}
