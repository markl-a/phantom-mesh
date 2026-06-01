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
use crate::context::WorkspaceContext;
use crate::cost::CostTracker;
use crate::interrupt::InterruptHandle;
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

/// Hard cap on the on-disk TUI history file. If `~/.phantom-mesh/tui-history`
/// somehow grows past this (e.g. a bug bypassed `maybe_compact_tui_history`,
/// the user manually concatenated logs into it, or a malicious actor wrote
/// a multi-GB file to the user's home dir), reading it into memory at TUI
/// startup would OOM-panic the process. Per issue #71, we stat the file
/// first and skip the load entirely (with a `tracing::warn`) if it exceeds
/// this limit — better to lose history recall than to crash the TUI on
/// launch.
///
/// 5 MiB is multiple orders of magnitude beyond any realistic accumulation:
/// 100 entries * 4096 B/entry caps the *intended* size at ~400 KiB, and
/// compaction triggers at 2x that. 5 MiB leaves headroom for slow growth
/// between compactions while still bounding the worst case.
const MAX_TUI_HISTORY_BYTES: u64 = 5 * 1024 * 1024;

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
    let Some(path) = tui_history_path() else {
        return Vec::new();
    };
    load_tui_history_from(&path)
}

/// Path-parameterised core of [`load_tui_history`], so the size guard can
/// be unit-tested against a temp file without poking at `$HOME`.
fn load_tui_history_from(path: &std::path::Path) -> Vec<String> {
    // Size guard (issue #71): stat first; if the file is too big, skip the
    // load entirely rather than risk OOM-panicking the TUI on launch.
    // `metadata` failure (e.g. file does not exist) falls through to the
    // existing `fs::read` path, which returns the same empty-Vec fallback.
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.len() > MAX_TUI_HISTORY_BYTES {
            tracing::warn!(
                target: "phantom_mesh::tui",
                path = %path.display(),
                size_bytes = meta.len(),
                limit_bytes = MAX_TUI_HISTORY_BYTES,
                "tui-history file exceeds size cap; skipping load to avoid OOM (issue #71)"
            );
            return Vec::new();
        }
    }

    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
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
    if all.len() <= TUI_HISTORY_MAX {
        return all;
    }
    let drop_n = all.len() - TUI_HISTORY_MAX;
    all.into_iter().skip(drop_n).collect()
}

/// Tighten `path` to `0o600` (owner read/write only) on Unix. No-op on
/// Windows where POSIX mode bits don't apply. Best-effort — a permission
/// failure here is logged-and-swallowed rather than propagated, because
/// the TUI's history write path is fire-and-forget already and we'd
/// rather take a (rare) wider-than-ideal file than crash the TUI on a
/// chmod hiccup. Mirrors `core/src/auth.rs` which does the same for
/// `auth.json`.
#[cfg(unix)]
fn restrict_tui_history_perms(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let perm = std::fs::Permissions::from_mode(0o600);
    let _ = std::fs::set_permissions(path, perm);
}
#[cfg(not(unix))]
fn restrict_tui_history_perms(_path: &std::path::Path) {}

/// Append a single prompt to ~/.phantom-mesh/tui-history. Encodes newlines
/// to " ⏎ " so each prompt stays one line. Atomic-ish (open append, write,
/// close) — no .tmp+rename because a partial line is recoverable on next read.
/// Entries exceeding `TUI_HISTORY_ENTRY_MAX_BYTES` after encoding are skipped
/// rather than truncated — a half-prompt is worse than no record.
fn append_tui_history(prompt: &str) {
    let Some(path) = tui_history_path() else {
        return;
    };
    append_tui_history_to(&path, prompt);
}

/// Path-parameterised core of [`append_tui_history`], so the permission
/// tightening can be unit-tested against a temp file without poking at
/// `$HOME`.
///
/// V12 HIGH (TUI-2): the on-disk history may contain copy-pasted
/// credentials, internal URLs, or other reasonably-sensitive text. On a
/// stock Linux umask of `0o022`, `OpenOptions::create(true)` lands the
/// file at `0o644`, which is world-readable. Mirror `auth.rs`'s
/// `0o600` chmod immediately after the open so other accounts on the
/// same host can't slurp the user's TUI history.
fn append_tui_history_to(path: &std::path::Path, prompt: &str) {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let line = prompt.replace('\n', " ⏎ ");
    if line.len() > TUI_HISTORY_ENTRY_MAX_BYTES {
        return;
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        // Tighten perms BEFORE writing — if the chmod fails, the line is
        // still written, but we'd rather a tiny window of correct perms
        // than no chmod at all. set_permissions(&Path, …) works on an
        // already-open file on every Unix we ship to.
        restrict_tui_history_perms(path);
        let _ = writeln!(f, "{}", line);
    }
}

/// Trim the persisted history file when it grows past TUI_HISTORY_MAX*2
/// lines (lazy compaction — avoids a write on every submit). Keeps the
/// last TUI_HISTORY_MAX entries.
fn maybe_compact_tui_history() {
    let Some(path) = tui_history_path() else {
        return;
    };
    maybe_compact_tui_history_at(&path);
}

/// Path-parameterised core of [`maybe_compact_tui_history`], factored out
/// so the V12 HIGH (TUI-2) perm-tightening can be unit-tested against a
/// temp file without poking at `$HOME`.
fn maybe_compact_tui_history_at(path: &std::path::Path) {
    // Issue #71: if the file is past the OOM cap, truncate it outright
    // instead of slurping it into memory just to decide to compact. This
    // is the *only* operation that can recover from a runaway history
    // file — the read path skips load entirely, leaving the file in place.
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.len() > MAX_TUI_HISTORY_BYTES {
            tracing::warn!(
                target: "phantom_mesh::tui",
                path = %path.display(),
                size_bytes = meta.len(),
                limit_bytes = MAX_TUI_HISTORY_BYTES,
                "tui-history file exceeds size cap; truncating during compaction"
            );
            let _ = std::fs::write(path, "");
            // V12 HIGH (TUI-2): `fs::write` on a pre-existing file usually
            // preserves perms on Unix, but if the file was created out-of-
            // band (e.g. by an older phantom build before the 0o600 fix
            // landed) it may still be 0o644. Re-tighten after every write
            // so compaction is also a passive migration path.
            restrict_tui_history_perms(path);
            return;
        }
    }
    // Read as bytes + lossy decode so a paste-bomb containing invalid UTF-8
    // doesn't permanently disable compaction. `read_to_string` rejects the
    // whole file on the first bad sequence — that exact bug let a 20 MB
    // file accumulate in the wild because compaction silently no-op'd.
    let Ok(bytes) = std::fs::read(path) else {
        return;
    };
    let content = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = content.lines().collect();
    if lines.len() <= TUI_HISTORY_MAX * 2 {
        return;
    }
    let keep_from = lines.len() - TUI_HISTORY_MAX;
    let new_content: String = lines[keep_from..].join("\n") + "\n";
    let _ = std::fs::write(path, new_content);
    // V12 HIGH (TUI-2): same rationale as above — migrate stale 0o644
    // history files to 0o600 on every successful compaction.
    restrict_tui_history_perms(path);
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
    ToolCall {
        name: String,
        args: String,
    },
    ToolResult {
        name: String,
        output: String,
    },
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
    ToolStart {
        name: String,
        args: String,
    },
    ToolDone {
        name: String,
        output: String,
    },
    Done {
        output: String,
        elapsed: f64,
    },
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
    /// Cluster Status pane (P1): when true, the body renders the cluster pane
    /// instead of the chat transcript. Toggled via `/cluster`. See
    /// docs/superpowers/design/tui-cluster-pane.md.
    cluster_view: bool,
    /// Rows shown in the cluster pane. Built from peers.json config on
    /// `/cluster` (pinged=false); live-ping refresh is a follow-on.
    cluster_rows: Vec<ClusterRow>,
    /// Evolve Goals pane (P3): when true, the body renders the goals pane.
    /// Toggled via `/goals`. See docs/superpowers/design/tui-evolve-goals.md.
    goals_view: bool,
    /// Rows for the goals pane, built from EVOLVE-GOALS.md on `/goals`.
    goals_rows: Vec<GoalRow>,
    /// Selected row in the goals pane (↑/↓), for in-pane space-to-mark-done.
    goals_selected: usize,
    /// Identity & Vault pane (P4): when true, the body renders it. `/identity`.
    /// See docs/superpowers/design/tui-identity-vault.md.
    identity_view: bool,
    /// Identity data for the pane, built from auth + identity on `/identity`.
    identity_data: Option<IdentityView>,
    /// Daily Review pane (P2 / Life Track): when true, the body renders it.
    /// `/review [<date>|reload|off]`. See docs/superpowers/design/tui-daily-review.md.
    review_view: bool,
    /// Review data for the pane (holds the resolved date), built on `/review`.
    review_data: Option<ReviewView>,
    /// Cost pane (Work Track): when true, the body renders it. `/cost`.
    /// See docs/superpowers/design/tui-cost-pane.md.
    cost_view: bool,
    /// Cost data for the pane, built from cost_tracker.summary() on `/cost`.
    cost_data: Option<CostView>,
    /// Focus pane (P2 / Life Track): when true, the body renders it. `/focus`.
    /// See docs/superpowers/design/terminal-focus.md.
    focus_view: bool,
    /// Focus session snapshot, built from life_node::focus_session::status().
    /// None = no active session (the pane shows the empty state).
    focus_data: Option<FocusView>,
    /// Evolve Runs pane (P3): when true, the body renders it. `/evolve`.
    /// See docs/superpowers/design/tui-evolve-runs.md.
    evolve_view: bool,
    /// Recent autoevolve runs (newest-first), parsed from autoevolve.log on
    /// `/evolve`. Empty = no runs (the pane shows the empty state).
    evolve_runs: Vec<EvolveRun>,
    /// Habits pane (P2 / Life Track): when true, the body renders it. `/habits`.
    /// See docs/superpowers/design/tui-habits.md.
    habits_view: bool,
    /// Habit rows (active-first), built from capture_habit_wire::list_habits()
    /// on `/habits`. Empty = no habits (the pane shows the empty state).
    habits_rows: Vec<HabitRow>,
    /// Modal interactive picker for editing [agent.X].providers priority.
    /// Opened by `/priority` slash; swallows all key events while open.
    /// On Enter, saves back to agents.toml. On Esc, discards changes.
    priority_picker: Option<PriorityPicker>,
    /// Signalling flag for slash commands that must cancel the in-flight
    /// turn before mutating shared state (currently `/clear` and `/resume`).
    /// The slash handler can't see `current_task` / `current_interrupt`
    /// (they live in `run_loop`'s stack), so it flips this and the loop's
    /// top-of-iteration check fires `interrupt(None)` on the current
    /// `InterruptHandle` and abort()s the JoinHandle. Without this, tokens
    /// from session A streamed into session B after `/resume`, and a
    /// `/clear` mid-stream left a zombie task that kept writing tokens
    /// into the freshly-cleared transcript (TUI-1, V12 HIGH).
    pending_interrupt: Option<()>,
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
    anchor: (u16, u16), // (col, row) at Down(Left)
    cursor: (u16, u16), // (col, row) latest Drag/Up
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
        if (a.1, a.0) <= (b.1, b.0) {
            (a, b)
        } else {
            (b, a)
        }
    }

    /// True when the selection covers at least one cell. A pure click with
    /// no drag (anchor == cursor) is treated as "not really a selection"
    /// so it doesn't trigger an empty-string copy.
    fn is_meaningful(&self) -> bool {
        self.anchor != self.cursor
    }
}

impl AppState {
    fn agent_name(&self) -> &'static str {
        AGENTS[self.agent_idx]
    }
    fn cycle_agent(&mut self) {
        self.agent_idx = (self.agent_idx + 1) % AGENTS.len();
    }
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
    // (e.g. when launched under `echo "" | phantom tui`). Return an error (not
    // Ok) so the process exits non-zero — a refusal-to-run must not look like
    // success to a script/CI that invoked bare `phantom` in a non-interactive
    // context.
    use std::io::IsTerminal;
    if !io::stdin().is_terminal() {
        anyhow::bail!(
            "phantom tui: stdin is not a terminal — TUI requires an interactive shell. \
             Use `phantom repl` for line mode, or `phantom exec` for a headless one-shot."
        );
    }

    let mut terminal = setup_terminal()?;

    // Mark the TUI as drawing the screen so background paths (LLM provider
    // fallback chain `eprintln!`s in `agent.rs`) can suppress stderr writes
    // that would otherwise collide with ratatui's draw buffer. RAII guard
    // — restores the flag on every exit path (normal return, `?`, panic),
    // so a TUI session that fails mid-stream still hands stderr back to
    // subsequent CLI commands cleanly.
    struct TuiActiveGuard;
    impl Drop for TuiActiveGuard {
        fn drop(&mut self) {
            crate::diag::TUI_ACTIVE
                .store(false, std::sync::atomic::Ordering::Relaxed);
        }
    }
    crate::diag::TUI_ACTIVE
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let _tui_active_guard = TuiActiveGuard;

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
        cluster_view: false,
        cluster_rows: Vec::new(),
        goals_view: false,
        goals_rows: Vec::new(),
        goals_selected: 0,
        identity_view: false,
        identity_data: None,
        review_view: false,
        review_data: None,
        cost_view: false,
        cost_data: None,
        focus_view: false,
        focus_data: None,
        evolve_view: false,
        evolve_runs: Vec::new(),
        habits_view: false,
        habits_rows: Vec::new(),
        sidebar_peers: crate::cli_config::read_peers_json()
            .map(|peers| {
                peers
                    .into_iter()
                    .map(|p| p.name)
                    .filter(|n| {
                        Some(n.as_str()) != crate::cli_config::resolve_self_node_name().as_deref()
                    })
                    .collect()
            })
            .unwrap_or_default(),
        priority_picker: None,
        last_ctrl_c_at: None,
        pending_interrupt: None,
    }));

    // Welcome banner
    {
        let mut s = app.lock().unwrap();
        s.transcript.push(TranscriptItem::System(crate::i18n::tr_owned(
            format!(
                "phantom tui — agent: {} · session: {} · type a message and press Enter (Tab cycles agents, Esc cancels, Ctrl-C twice to exit)",
                AGENTS[agent_idx],
                &chat_id[..chat_id.len().min(12)]
            ),
            format!(
                "phantom tui — 代理人：{} · session：{} · 輸入訊息後按 Enter（Tab 切換代理人，Esc 取消，Ctrl-C 按兩次離開）",
                AGENTS[agent_idx],
                &chat_id[..chat_id.len().min(12)]
            ),
        )));
    }

    // First-run guidance: with no usable provider key, a fresh user would
    // otherwise just hit "all providers failed" on their first message. Surface
    // a one-time hint pointing at /login + /keys (only when genuinely keyless).
    if !runtime.config().has_usable_provider_key() {
        let mut s = app.lock().unwrap();
        s.transcript.push(TranscriptItem::System(
            crate::i18n::tr(
                "  ⚠ no AI provider key found — run /login, or /keys set <provider> <key>. /help for more.",
                "  ⚠ 找不到 AI 供應商金鑰 — 執行 /login，或 /keys set <provider> <key>。詳見 /help。",
            )
            .to_string(),
        ));
    }

    // Live presence: register a session with the broker so other machines
    // can see "this TUI is open on node-a in /path/to/project". No-op when
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
    let mut last_draw = Instant::now() - FRAME_BUDGET; // draw the very first frame
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
                    AgentEvent::Token { content } => tx_inner.send(UiEvent::Token(content)),
                    AgentEvent::ToolStart { name, args_preview } => {
                        tx_inner.send(UiEvent::ToolStart {
                            name,
                            args: args_preview,
                        })
                    }
                    AgentEvent::ToolDone {
                        name,
                        output_preview,
                    } => tx_inner.send(UiEvent::ToolDone {
                        name,
                        output: output_preview,
                    }),
                    AgentEvent::Thinking { content } => tx_inner.send(UiEvent::Thinking(content)),
                    AgentEvent::Done {
                        output,
                        elapsed_secs,
                        ..
                    } => tx_inner.send(UiEvent::Done {
                        output,
                        elapsed: elapsed_secs,
                    }),
                    AgentEvent::Notice { message } => tx_inner.send(UiEvent::Notice(message)),
                    #[cfg(feature = "experimental-anti-hallucination")]
                    AgentEvent::ConsistencyWarning { unbacked_claims } => {
                        tx_inner.send(UiEvent::Notice(format!(
                            "anti-hallucination: {} unbacked claim(s) — {}",
                            unbacked_claims.len(),
                            unbacked_claims.join(" | "),
                        )))
                    }
                };
            };
            let result = runtime
                .run_with_callbacks(&agent_name, &prompt, &history, Some(&extra), &cost, handler)
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
        // ── 0z. Honour pending_interrupt from slash handlers (/clear, /resume).
        // ──────────────────────────────────────────────────────────────
        // Those handlers can't see `current_task` / `current_interrupt`
        // because both live in this stack frame, so they signal via the
        // AppState flag and we cancel here. Without this, a /clear or
        // /resume mid-stream left the previous turn alive and tokens
        // from session A kept appending to the cleared / replaced
        // transcript of session B (TUI-1, V12 HIGH). We fire the
        // interrupt token (cooperative) AND drop the JoinHandle's
        // running task via abort() so any streaming `tx.send(...)` calls
        // already buffered are dropped on the next drain instead of
        // posting tokens into the new session.
        let take_pending = {
            let mut s = app.lock().unwrap();
            s.pending_interrupt.take()
        };
        if take_pending.is_some() {
            if let Some(ih) = current_interrupt.as_ref() {
                ih.interrupt(None);
            }
            if let Some(h) = current_task.take() {
                h.abort();
            }
            current_interrupt = None;
            // Drain any UI events queued before the abort so they don't
            // leak into the freshly-cleared/replaced transcript on the
            // next drain pass. Errors / tokens from the killed task are
            // intentionally dropped — the user just cleared or switched
            // sessions, they don't want to see the prior turn's tail.
            while rx.try_recv().is_ok() {}
            let mut s = app.lock().unwrap();
            s.running = false;
        }

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
                format!(
                    "task finished; queued = {:?}",
                    queued
                        .as_ref()
                        .map(|s| s.chars().take(60).collect::<String>())
                ),
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
                    prompt,
                    agent_name,
                    chat_id,
                    history,
                    runtime.clone(),
                    cost_tracker.clone(),
                    conversations.clone(),
                    extra_context.clone(),
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
                Event::Key(k)
                    if k.kind == KeyEventKind::Press || k.kind == KeyEventKind::Repeat =>
                {
                    let action = handle_key(&app, k);
                    match action {
                        KeyAction::Exit => break,
                        KeyAction::Submit(prompt) => {
                            // Slash-command shortcut — handled inline, never
                            // dispatched to the agent.
                            if prompt.starts_with('/') {
                                handle_tui_slash(
                                    &app,
                                    &runtime,
                                    &conversations,
                                    &cost_tracker,
                                    &prompt,
                                )
                                .await;
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
                                    None => (rest.trim().to_string(), String::new()),
                                };
                                if !token.is_empty() {
                                    let local_idx =
                                        AGENTS.iter().position(|a| a.eq_ignore_ascii_case(&token));
                                    let is_peer = {
                                        let s = app.lock().unwrap();
                                        s.sidebar_peers
                                            .iter()
                                            .any(|p| p.eq_ignore_ascii_case(&token))
                                    };
                                    if let Some(idx) = local_idx {
                                        {
                                            let mut s = app.lock().unwrap();
                                            s.agent_idx = idx;
                                            s.sidebar_focus = idx;
                                            if body.is_empty() {
                                                s.transcript.push(TranscriptItem::System(format!(
                                                    "◆ → {}",
                                                    AGENTS[idx]
                                                )));
                                            }
                                        }
                                        if body.is_empty() {
                                            continue;
                                        }
                                        prompt = body;
                                    } else if is_peer {
                                        if body.is_empty() {
                                            let mut s = app.lock().unwrap();
                                            s.transcript.push(TranscriptItem::Warning(format!(
                                                "@{} needs a prompt — try `@{} <task>`",
                                                token, token
                                            )));
                                            continue;
                                        }
                                        {
                                            let mut s = app.lock().unwrap();
                                            s.transcript.push(TranscriptItem::User(format!(
                                                "@{} {}",
                                                token, body
                                            )));
                                            s.transcript.push(TranscriptItem::System(format!(
                                                "◆ dispatching to {} …",
                                                token
                                            )));
                                        }
                                        let app_clone = app.clone();
                                        let token_c = token.clone();
                                        let body_c = body.clone();
                                        tokio::spawn(async move {
                                            let result = crate::cli_config::dispatch_lines(
                                                &[],
                                                Some(&token_c),
                                                "master",
                                                &body_c,
                                                false,
                                            )
                                            .await;
                                            let mut s = app_clone.lock().unwrap();
                                            match result {
                                                Ok(lines) => {
                                                    for l in lines {
                                                        s.transcript
                                                            .push(TranscriptItem::System(l));
                                                    }
                                                }
                                                Err(e) => s.transcript.push(TranscriptItem::Error(
                                                    format!("@{} dispatch failed: {}", token_c, e),
                                                )),
                                            }
                                        });
                                        continue;
                                    } else {
                                        let warn = {
                                            let s = app.lock().unwrap();
                                            format!(
                                                "@{} not recognized — agents: {} · peers: {}",
                                                token,
                                                AGENTS.join(","),
                                                if s.sidebar_peers.is_empty() {
                                                    "(none)".into()
                                                } else {
                                                    s.sidebar_peers.join(",")
                                                }
                                            )
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
                                    s.transcript.push(TranscriptItem::System(format!(
                                        "↺ redirecting → {}",
                                        preview
                                    )));
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
                                prompt,
                                agent_name,
                                chat_id,
                                history,
                                runtime.clone(),
                                cost_tracker.clone(),
                                conversations.clone(),
                                extra_context.clone(),
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
                                s.transcript
                                    .push(TranscriptItem::System("⊗ stream cancelled".into()));
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
                            let text = extract_selection_text(terminal.current_buffer_mut(), sel);
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
                                        s.transcript.push(TranscriptItem::Error(format!(
                                            "  ✗ clipboard copy failed: {}",
                                            e
                                        )));
                                    }
                                }
                            }
                        }
                        KeyAction::None => {}
                    }
                }
                Event::Mouse(m) => {
                    match m.kind {
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
                                } else {
                                    None
                                }
                            };
                            if let Some(sel) = sel_snapshot {
                                // Always log the up event so phantom debug shows
                                // mouse activity for remote debugging.
                                let (a, b) = sel.normalized();
                                crate::diag::record(
                                    "mouse_up_select",
                                    format!(
                                        "anchor=({},{}) cursor=({},{}) meaningful={}",
                                        a.0,
                                        a.1,
                                        b.0,
                                        b.1,
                                        sel.is_meaningful()
                                    ),
                                );
                                if sel.is_meaningful() {
                                    let text =
                                        extract_selection_text(terminal.current_buffer_mut(), sel);
                                    if !text.is_empty() {
                                        let chars = text.chars().count();
                                        crate::diag::record(
                                            "clipboard_copy_attempt",
                                            format!("chars={}", chars),
                                        );
                                        match copy_to_os_clipboard(&text) {
                                            Ok(cmd) => {
                                                crate::diag::record(
                                                    "clipboard_copy_ok",
                                                    format!("via {}", cmd),
                                                );
                                                let mut s = app.lock().unwrap();
                                                s.transcript.push(TranscriptItem::System(format!(
                                                    "  ✓ selected {} chars copied via {}",
                                                    chars, cmd
                                                )));
                                            }
                                            Err(e) => {
                                                crate::diag::record(
                                                    "clipboard_copy_fail",
                                                    format!("{}", e),
                                                );
                                                let mut s = app.lock().unwrap();
                                                s.transcript.push(TranscriptItem::Error(format!(
                                                    "  ✗ clipboard copy failed: {}",
                                                    e
                                                )));
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
                    }
                }
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

    if let Some(h) = current_task.take() {
        h.abort();
    }
    Ok(())
}

#[cfg_attr(test, derive(Debug))]
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
    "/help",
    "/exit",
    "/clear",
    "/agent",
    "/agents",
    "/sessions",
    "/resume",
    "/tools",
    "/todo",
    "/cost",
    "/focus",
    "/evolve",
    "/habits",
    "/note",
    "/recall",
    "/event",
    "/stats",
    "/diff",
    "/log",
    "/branch",
    "/perm",
    "/density",
    "/theme",
    "/tasks",
    "/plan",
    // Config management — /keys writes to ~/.phantom-mesh/env (set/list/remove/test),
    // /provider lists providers + /provider priority edits failover order in agents.toml.
    "/keys",
    "/provider",
    "/providers",
    "/models",
    "/cluster",
    "/goals",
    "/identity",
    "/review",
    // Interactive priority picker — modal popup for arrow-key reorder of
    // [agent.X].providers. /priority [agent_name], default = active agent.
    "/priority",
    "/prio",
    // Multi-machine broadcast: send one prompt to every peer in parallel.
    // Optional `--agent <name>` to override which agent runs on the remote.
    "/fanout",
    "/broadcast",
    // Output management — /copy gets agent reply / turn / full session into
    // the OS clipboard. /mouse runtime-toggles mouse capture so users can
    // switch between drag-select-text mode and mouse-wheel-scroll mode
    // without restarting the TUI.
    "/copy",
    "/mouse",
    // /sidebar toggles the right-rail target picker (local agents +
    // remote peers).
    "/sidebar",
    // /freeze pauses ratatui's redraw so the terminal's drag-selection
    // isn't erased between frames. /resume returns to live updates.
    "/freeze",
    "/resume",
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

    // ── Goals pane: ↑/↓ select, space marks the selected row done, r reloads.
    // Only these keys are pane-local; everything else (incl. Esc → close) falls
    // through to the normal handler below.
    if s.goals_view && !s.goals_rows.is_empty() {
        match k.code {
            KeyCode::Up => {
                s.goals_selected = s.goals_selected.saturating_sub(1);
                return KeyAction::None;
            }
            KeyCode::Down => {
                let max = s.goals_rows.len().saturating_sub(1);
                s.goals_selected = (s.goals_selected + 1).min(max);
                return KeyAction::None;
            }
            KeyCode::Char(' ') => {
                mark_selected_goal_done(&mut s);
                return KeyAction::None;
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                s.goals_rows = goal_rows_from_file();
                let max = s.goals_rows.len().saturating_sub(1);
                s.goals_selected = s.goals_selected.min(max);
                return KeyAction::None;
            }
            _ => {}
        }
    }

    // ── Review pane: ←/→ step the date by a day, r reloads. Pane-local;
    // everything else (incl. Esc → close) falls through.
    if s.review_view {
        match k.code {
            KeyCode::Left | KeyCode::Right => {
                let delta = if k.code == KeyCode::Left { -1 } else { 1 };
                if let Some(d) = s.review_data.as_ref().map(|v| shift_date(&v.date, delta)) {
                    s.review_data = Some(review_view_from_state(&d));
                }
                return KeyAction::None;
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                if let Some(d) = s.review_data.as_ref().map(|v| v.date.clone()) {
                    s.review_data = Some(review_view_from_state(&d));
                }
                return KeyAction::None;
            }
            _ => {}
        }
    }

    // ── Focus pane: i logs an interruption, s stops the session, r reloads.
    // i/s act only with an active session; all are swallowed (return) so they
    // don't leak to the input. Everything else (incl. Esc → close) falls through.
    if s.focus_view {
        let active = s.focus_data.is_some();
        match k.code {
            KeyCode::Char('i') | KeyCode::Char('I') => {
                if active {
                    log_focus_interruption(&mut s);
                }
                return KeyAction::None;
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                if active {
                    stop_focus_session(&mut s);
                }
                return KeyAction::None;
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                s.focus_data = focus_view_from_state();
                return KeyAction::None;
            }
            _ => {}
        }
    }

    // Any key other than Up/Down resets history navigation state.
    let reset_history = !matches!(k.code, KeyCode::Up | KeyCode::Down);
    if reset_history {
        s.history_idx = None;
    }

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
            let armed = s
                .last_ctrl_c_at
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
                "press Ctrl-C again within 2 s to exit".into(),
            ));
            return KeyAction::None;
        }
        KeyCode::Char('d') if ctrl && s.input.is_empty() => return KeyAction::Exit,

        // ── Emacs-style line edit ────────────────────────────────────────
        KeyCode::Char('a') if ctrl => {
            s.cursor = 0;
        }
        KeyCode::Char('e') if ctrl => {
            s.cursor = s.input.len();
        }
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
            let line_end = s.input[cur..]
                .find('\n')
                .map(|i| cur + i)
                .unwrap_or(s.input.len());
            s.input.replace_range(cur..line_end, "");
        }
        KeyCode::Char('w') if ctrl => {
            // Delete word before cursor (whitespace-bounded).
            let cur = s.cursor;
            let prefix = &s.input[..cur];
            // Skip trailing whitespace, then delete back to next whitespace.
            let trimmed_end = prefix.trim_end();
            let target_end = trimmed_end.len();
            let word_start = trimmed_end
                .rfind(|c: char| c.is_whitespace())
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
            if s.running {
                return KeyAction::Cancel;
            }
            // A full-body pane is open → Esc closes it ("esc: back to chat", as
            // every pane footer promises). Only with no pane up does Esc fall
            // through to clearing the input line.
            if s.cluster_view
                || s.goals_view
                || s.identity_view
                || s.review_view
                || s.cost_view
                || s.focus_view
                || s.evolve_view
                || s.habits_view
            {
                s.cluster_view = false;
                s.goals_view = false;
                s.identity_view = false;
                s.review_view = false;
                s.cost_view = false;
                s.focus_view = false;
                s.evolve_view = false;
                s.habits_view = false;
                return KeyAction::None;
            }
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
                let candidates: Vec<&&str> = SLASH_COMMANDS_TUI
                    .iter()
                    .filter(|c| c.starts_with(&token))
                    .collect();
                if candidates.len() == 1 {
                    let want = candidates[0];
                    let token_start = s.cursor.saturating_sub(token.len());
                    let end = s.cursor;
                    s.input.replace_range(token_start..end, want);
                    s.cursor = token_start + want.len();
                } else if candidates.len() > 1 {
                    // Find common prefix among candidates and extend to it.
                    let common =
                        longest_common_prefix(&candidates.iter().map(|c| **c).collect::<Vec<_>>());
                    if common.len() > token.len() {
                        let token_start = s.cursor.saturating_sub(token.len());
                        let end = s.cursor;
                        s.input.replace_range(token_start..end, &common);
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
                let dir = if dir_str.is_empty() {
                    "/"
                } else {
                    dir_str.as_str()
                };
                let mut matches: Vec<String> = Vec::new();
                if let Ok(entries) = std::fs::read_dir(dir) {
                    for entry in entries.flatten() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        if name.starts_with(&file_prefix) {
                            if name.starts_with('.') && !file_prefix.starts_with('.') {
                                continue;
                            }
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
                    let end = s.cursor;
                    s.input.replace_range(token_start..end, want);
                    s.cursor = token_start + want.len();
                } else if matches.len() > 1 {
                    let common = longest_common_prefix(
                        &matches.iter().map(|m| m.as_str()).collect::<Vec<_>>(),
                    );
                    if common.len() > token.len() {
                        let token_start = s.cursor.saturating_sub(token.len());
                        let end = s.cursor;
                        s.input.replace_range(token_start..end, &common);
                        s.cursor = token_start + common.len();
                    }
                }
            } else {
                // Empty input or non-special token → cycle agent (legacy).
                s.cycle_agent();
            }
        }

        KeyCode::PageUp => {
            s.scroll = s.scroll.saturating_add(5);
        }
        KeyCode::PageDown => {
            s.scroll = s.scroll.saturating_sub(5);
        }

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
                        s.transcript
                            .push(TranscriptItem::System(format!("◆ → {}", AGENTS[focus])));
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
                let new = trimmed
                    .rfind(|c: char| c.is_whitespace())
                    .map(|i| i + 1)
                    .unwrap_or(0);
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
        KeyCode::Home => {
            s.cursor = 0;
        }
        KeyCode::End => {
            s.cursor = s.input.len();
        }
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
        None => 0,
    };
    prefix[start..].to_string()
}

fn longest_common_prefix(strs: &[&str]) -> String {
    if strs.is_empty() {
        return String::new();
    }
    let mut prefix = strs[0].to_string();
    for s in &strs[1..] {
        while !s.starts_with(&prefix) {
            prefix.pop();
            if prefix.is_empty() {
                return String::new();
            }
        }
    }
    prefix
}

fn prev_char_boundary(s: &str, i: usize) -> usize {
    let mut idx = i.saturating_sub(1);
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}
fn next_char_boundary(s: &str, i: usize) -> usize {
    let mut idx = (i + 1).min(s.len());
    while idx < s.len() && !s.is_char_boundary(idx) {
        idx += 1;
    }
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
            s.transcript
                .push(TranscriptItem::ToolResult { name, output });
        }
        UiEvent::Done { output, elapsed } => {
            // Update cost from tracker and finalize the message
            let last = cost.last_request_cost().await;
            let session = cost.session_cost().await;
            let mut s = app.lock().unwrap();
            // If no streaming tokens arrived, push the final output as one assistant message.
            let had_partial = matches!(
                s.transcript.last(),
                Some(TranscriptItem::AssistantPartial(_))
                    | Some(TranscriptItem::ThinkingPartial(_))
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
        visual_rows = 1; // placeholder hint line
    } else {
        for line in s.input.split('\n') {
            let dw = unicode_width::UnicodeWidthStr::width(line);
            // Each \n contributes one row; long lines wrap, taking ceil(dw/inner_w) rows.
            visual_rows += (dw / inner_w) + 1;
        }
        if s.input.ends_with('\n') {
            visual_rows += 1;
        }
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
    if s.identity_view {
        // Identity & Vault pane (P4) takes over the whole body. `/identity`.
        let fallback = IdentityView {
            identity_line: None,
            fingerprint: "—".into(),
            created_at: "—".into(),
            keystore: "—".into(),
            key_present: true,
        };
        render_identity_pane(f, body, s.identity_data.as_ref().unwrap_or(&fallback));
    } else if s.goals_view {
        // Evolve Goals pane (P3) takes over the whole body. `/goals` toggle.
        render_goals_pane(f, body, &s.goals_rows, s.goals_selected);
    } else if s.cluster_view {
        // Cluster Status pane (P1) takes over the whole body. `/cluster` toggle.
        render_cluster_pane(f, body, &s.cluster_rows, 0);
    } else if s.review_view {
        // Daily Review pane (P2 / Life Track) takes over the body. `/review`.
        let fallback = ReviewView {
            date: "—".into(),
            state: ReviewState::Empty,
            event_count: 0,
            rows: Vec::new(),
            flagged: false,
        };
        render_daily_review_pane(f, body, s.review_data.as_ref().unwrap_or(&fallback));
    } else if s.cost_view {
        // Cost pane (Work Track) takes over the whole body. `/cost` toggle.
        let fallback = CostView {
            session_usd: 0.0,
            total_usd: 0.0,
            requests: 0,
            prompt_tokens: 0,
            completion_tokens: 0,
            budget_limit_usd: 0.0,
            over_budget: false,
            models: Vec::new(),
        };
        render_cost_pane(f, body, s.cost_data.as_ref().unwrap_or(&fallback));
    } else if s.focus_view {
        // Focus pane (P2 / Life Track) takes over the body. `/focus` toggle.
        // `None` → empty state (no active session).
        render_focus_pane(f, body, s.focus_data.as_ref());
    } else if s.evolve_view {
        // Evolve Runs pane (P3) takes over the body. `/evolve` toggle.
        // Empty Vec → empty state.
        render_evolve_pane(f, body, &s.evolve_runs);
    } else if s.habits_view {
        // Habits pane (P2 / Life Track) takes over the body. `/habits` toggle.
        // Empty Vec → empty state.
        render_habits_pane(f, body, &s.habits_rows);
    } else if s.sidebar_visible && body.width >= sidebar_w + min_transcript {
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
        if row >= area.height {
            return;
        }
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
    let cmd = if cfg!(target_os = "macos") {
        "pbcopy"
    } else if cfg!(target_os = "linux") {
        "xclip"
    } else {
        "clip"
    };
    let mut child = std::process::Command::new(cmd)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn {}: {}", cmd, e))?;
    if let Some(stdin) = child.stdin.as_mut() {
        use std::io::Write;
        stdin
            .write_all(text.as_bytes())
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
        Style::default()
            .fg(Color::Rgb(214, 178, 112))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let mode = if s.running { " · streaming…" } else { "" };
    let line = Line::from(vec![
        Span::styled(format!(" {} ", busy_glyph), busy_style),
        Span::styled(
            format!("phantom v{}", version),
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" · ", Style::default().fg(Color::DarkGray)),
        Span::styled("agent: ", Style::default().fg(Color::DarkGray)),
        Span::styled(s.agent_name(), Style::default().fg(Color::Cyan)),
        Span::styled(" · ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("cost: ${:.4} session", s.session_cost),
            Style::default().fg(Color::DarkGray),
        ),
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
                    Span::styled(
                        "◆ ",
                        Style::default()
                            .fg(Color::Magenta)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        t.clone(),
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]));
                lines.push(Line::from(""));
            }
            TranscriptItem::Thinking(t) | TranscriptItem::ThinkingPartial(t) => {
                if std::env::var("PHANTOM_THINKING")
                    .map(|v| v != "0")
                    .unwrap_or(true)
                {
                    let total = t.lines().filter(|l| !l.trim().is_empty()).count();
                    let style = Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::ITALIC | Modifier::DIM);
                    lines.push(Line::from(Span::styled(
                        format!(
                            "⌖ thinking ({} line{})",
                            total,
                            if total == 1 { "" } else { "s" }
                        ),
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
                        lines.push(Line::from(vec![Span::raw("  "), Span::raw(l.to_string())]));
                    }
                }
                lines.push(Line::from(""));
            }
            TranscriptItem::ToolCall { name, args } => {
                let preview = truncate(args, 80);
                lines.push(Line::from(vec![
                    Span::styled("● ", Style::default().fg(Color::Cyan)),
                    Span::styled(
                        format!("{}({})", name, preview),
                        Style::default().fg(Color::Cyan),
                    ),
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
                        Style::default()
                            .fg(Color::DarkGray)
                            .add_modifier(Modifier::ITALIC),
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
        .title(Span::styled(
            " targets ",
            Style::default()
                .fg(Color::Rgb(214, 178, 112))
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let total = AGENTS.len() + s.sidebar_peers.len();
    let focus = if total == 0 {
        0
    } else {
        s.sidebar_focus % total
    };
    let mut lines: Vec<Line> = Vec::with_capacity(total + 6);

    // Section header for local agents
    lines.push(Line::from(Span::styled(
        crate::i18n::tr(" LOCAL AGENTS", " 本機代理人"),
        Style::default().fg(Color::DarkGray),
    )));
    for (i, name) in AGENTS.iter().enumerate() {
        let is_active = i == s.agent_idx;
        let is_focused = i == focus;
        let glyph = if is_active {
            "◆"
        } else if is_focused {
            "▸"
        } else {
            " "
        };
        let style = if is_focused {
            Style::default()
                .fg(Color::Rgb(214, 178, 112))
                .add_modifier(Modifier::BOLD)
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
        crate::i18n::tr_owned(
            format!(
                " REMOTE ({} peer{})",
                peer_count,
                if peer_count == 1 { "" } else { "s" }
            ),
            format!(" 遠端（{} 個節點）", peer_count),
        ),
        Style::default().fg(Color::DarkGray),
    )));
    if s.sidebar_peers.is_empty() {
        lines.push(Line::from(vec![
            Span::raw(" "),
            Span::styled(
                crate::i18n::tr(
                    "(none — phantom config pull)",
                    "（無 — 執行 phantom config pull）",
                ),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            ),
        ]));
    } else {
        for (i, peer) in s.sidebar_peers.iter().enumerate() {
            let combined_idx = AGENTS.len() + i;
            let is_focused = combined_idx == focus;
            let glyph = if is_focused { "▸" } else { " " };
            let style = if is_focused {
                Style::default()
                    .fg(Color::Rgb(214, 178, 112))
                    .add_modifier(Modifier::BOLD)
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
    let key = Style::default().fg(Color::Rgb(214, 178, 112));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        crate::i18n::tr(" HOW TO USE", " 使用方式"),
        muted.add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(vec![
        Span::raw(" "),
        Span::styled("Tab", key),
        Span::styled(crate::i18n::tr("        next agent", "        下一個代理人"), muted),
    ]));
    lines.push(Line::from(vec![
        Span::raw(" "),
        Span::styled("@name <prompt>", key),
    ]));
    lines.push(Line::from(Span::styled(
        crate::i18n::tr("              → switch / send", "              → 切換／傳送"),
        muted,
    )));
    lines.push(Line::from(vec![
        Span::raw(" "),
        Span::styled("Shift+↑↓", key),
        Span::styled(crate::i18n::tr("   move ▸", "   移動 ▸"), muted),
    ]));
    lines.push(Line::from(vec![
        Span::raw(" "),
        Span::styled("Alt+Enter", key),
        Span::styled(crate::i18n::tr("  commit ▸", "  送出 ▸"), muted),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        " /sidebar off · status",
        muted.add_modifier(Modifier::ITALIC),
    )));

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
        None => return KeyAction::None,
    };
    let len = p.items.len();
    match code {
        KeyCode::Esc => {
            // Discard all changes
            s.priority_picker = None;
            s.transcript.push(TranscriptItem::System(
                "  ◆ priority: cancelled (no changes saved)".into(),
            ));
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
            if len > 0 {
                p.focused = (p.focused + len - 1) % len;
            }
        }
        KeyCode::Down => {
            if len > 0 {
                p.focused = (p.focused + 1) % len;
            }
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
                    s.transcript.push(TranscriptItem::System(format!(
                        "  ◆ priority saved for agent.{}",
                        agent
                    )));
                    for l in lines {
                        s.transcript
                            .push(TranscriptItem::System(format!("  {}", l)));
                    }
                }
                Err(e) => s
                    .transcript
                    .push(TranscriptItem::Error(format!("  ✗ save failed: {}", e))),
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
    if popup_w < 20 || popup_h < 6 {
        return;
    }
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
        .title(Span::styled(
            title,
            Style::default()
                .fg(Color::Rgb(214, 178, 112))
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let mut lines: Vec<Line> = Vec::with_capacity(p.items.len() + 4);
    if p.items.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  (empty — no providers configured for this agent)",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )));
    } else {
        for (i, entry) in p.items.iter().enumerate() {
            let is_focused = i == p.focused;
            let glyph = if is_focused { "▸" } else { " " };
            let style = if is_focused {
                Style::default()
                    .fg(Color::Rgb(214, 178, 112))
                    .add_modifier(Modifier::BOLD)
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
    let key = Style::default().fg(Color::Rgb(214, 178, 112));
    lines.push(Line::from(vec![
        Span::raw(" "),
        Span::styled("↑↓", key),
        Span::styled(" nav  ", muted),
        Span::styled("Shift+↑↓", key),
        Span::styled(" reorder", muted),
    ]));
    lines.push(Line::from(vec![
        Span::raw(" "),
        Span::styled("Del/x", key),
        Span::styled(" remove  ", muted),
        Span::styled("Enter", key),
        Span::styled(" save  ", muted),
        Span::styled("Esc", key),
        Span::styled(" cancel", muted),
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
            if let TranscriptItem::User(p) = t {
                Some(p.as_str())
            } else {
                None
            }
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
    let lines_vec: Vec<Line> =
        if s.input.is_empty() {
            vec![Line::from(Span::styled(
            crate::i18n::tr(
                "Enter sends · Shift-Enter / Alt-Enter / Ctrl-J newline · Tab agent · Ctrl-C ×2 exits",
                "Enter 送出 · Shift-Enter／Alt-Enter／Ctrl-J 換行 · Tab 切代理人 · Ctrl-C ×2 離開",
            ),
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

    let p = Paragraph::new(lines_vec)
        .block(block)
        .wrap(Wrap { trim: false });
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
        if row > 0 {
            row -= 1;
        }

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
        return None; // already on first line
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
        if walked + w > cur_col {
            break;
        }
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
    if next_line_start > input.len() {
        return None;
    }
    // Next line bounds: from next_line_start to next \n or end of input.
    let rest = &input[next_line_start..];
    let next_line_end = next_line_start + rest.find('\n').unwrap_or(rest.len());
    let next_line = &input[next_line_start..next_line_end];

    let mut walked: usize = 0;
    let mut new_cursor = next_line_start;
    for (byte_off, ch) in next_line.char_indices() {
        let w = UnicodeWidthChar::width(ch).unwrap_or(0);
        if walked + w > cur_col {
            break;
        }
        walked += w;
        new_cursor = next_line_start + byte_off + ch.len_utf8();
    }
    Some(new_cursor)
}

fn truncate(s: &str, max: usize) -> String {
    let s = s.replace('\n', " ");
    if s.chars().count() <= max {
        s
    } else {
        let truncated: String = s.chars().take(max).collect();
        format!("{}…", truncated)
    }
}

/// Truncate by *display width* (CJK = 2 cells), not codepoint count, so the
/// result fits inside `max_cells` cells in the rendered terminal.
fn truncate_to_width(s: &str, max_cells: usize) -> String {
    use unicode_width::UnicodeWidthChar;
    let single_line = s.replace('\n', " ");
    let total: usize = single_line.chars().map(|c| c.width().unwrap_or(0)).sum();
    if total <= max_cells {
        return single_line;
    }
    // Reserve 1 cell for the trailing ellipsis.
    let budget = max_cells.saturating_sub(1).max(1);
    let mut out = String::new();
    let mut used = 0;
    for c in single_line.chars() {
        let w = c.width().unwrap_or(0);
        if used + w > budget {
            break;
        }
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

    run_tui(
        runtime,
        conversations,
        cost_tracker,
        chat_id,
        initial_agent,
        extra_context,
    )
    .await
}

fn find_config_simple() -> Option<String> {
    // Local agents.toml first, then ~/.phantom-mesh/agents.toml
    if let Ok(c) = std::fs::read_to_string("agents.toml") {
        return Some(c);
    }
    if let Some(home) = dirs::home_dir() {
        let p = home.join(".phantom-mesh").join("agents.toml");
        if let Ok(c) = std::fs::read_to_string(&p) {
            return Some(c);
        }
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
                 \x20   /sessions                     list saved sessions (title · msgs · time)\n\
                 \x20   /resume <prefix>              switch to another session\n\
                 \x20   /copy [all|turn]              copy: last assistant / full session / last turn\n\
                 \x20   /export [path]                save session as markdown\n\
                 \x20   /show <id>                    expand a captured tool-call output\n\
                 \x20\n\
                 \x20 views / panes\n\
                 \x20   /sidebar [on|off]             toggle the targets sidebar\n\
                 \x20   /cluster [refresh|off]        mesh peers + capabilities (P1)\n\
                 \x20   /goals [reload|off]           EVOLVE-GOALS.md milestones (P3)\n\
                 \x20   /identity [reload|off]        identity + what's encrypted at rest (P4)\n\
                 \x20   /review [<date>|reload|off]    Life Node daily review (P2 / Life Track)\n\
                 \x20   /cost [reload|off|line]       session + lifetime spend, per-model (Work Track)\n\
                 \x20   /focus [reload|off]           live focus-session status (P2 / Life Track)\n\
                 \x20   /habits [reload|checkin <slug>|off]  habit streaks; checkin logs one (P2 / Life Track)\n\
                 \x20   /note <text>                  capture a quick Life Node note (P2; see /review)\n\
                 \x20   /recall <text> [--kind|--since]  search past Life Node events (P2)\n\
                 \x20   /event <id>                   show one Life Node event in full (id from /recall)\n\
                 \x20   /stats                        life-log rollup: total · span · by-kind (P2)\n\
                 \x20   /evolve [reload|off]          recent autoevolve runs (P3 Evolve Mesh)\n\
                 \x20\n\
                 \x20 agents & models & keys\n\
                 \x20   /agent [name]                 show or switch active agent\n\
                 \x20   /agents                       list configured agents\n\
                 \x20   /model                        show models / switch / fast|smart|cheap / fetch\n\
                 \x20   /keys [list|test|remove]      manage provider api keys\n\
                 \x20   /provider                     list providers + key state\n\
                 \x20   /tools                        list registered tools\n\
                 \x20   /todo                         show todos (use the todo_add tool to create)\n\
                 \x20   /diff [--cached] [--full] [<file>]  git status + diff (--stat, or --full patch)\n\
                 \x20   /log [n]                      recent git commits (oneline; default 10)\n\
                 \x20   /branch                       list git branches (* = current)\n\
                 \x20\n\
                 \x20 context & auth\n\
                 \x20   /init                         generate PHANTOM.md in cwd\n\
                 \x20   /whoami                       show broker login state\n\
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
            // Signal run_loop to cancel any in-flight agent task before
            // it streams more tokens into the freshly-cleared transcript.
            // See AppState::pending_interrupt for the full rationale
            // (TUI-1, V12 HIGH).
            s.pending_interrupt = Some(());
            s.transcript.push(TranscriptItem::System(format!(
                "  ◆ cleared transcript and session history for {}",
                chat_id
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
                    push_err(format!(
                        "  unknown agent: {} (available: {})",
                        name,
                        names.join(", ")
                    ));
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
                let provider = if ag.provider.is_empty() {
                    "<inherit>"
                } else {
                    ag.provider.as_str()
                };
                text.push_str(&format!(
                    "    {} {:<14} provider={}\n",
                    marker, name, provider
                ));
            }
            push(text);
        }

        "/tools" => {
            let cfg = runtime.config();
            let cur_agent = app.lock().unwrap().agent_name().to_string();
            let tools = cfg
                .agent
                .get(&cur_agent)
                .map(|a| a.tools.clone())
                .unwrap_or_default();
            if tools.is_empty() {
                push(format!("  ◆ agent '{}' has no tools configured", cur_agent));
            } else {
                let text = format!(
                    "  ◆ {} tools enabled for {}:\n    {}",
                    tools.len(),
                    cur_agent,
                    tools.join(", ")
                );
                push(text);
            }
        }

        "/sessions" => {
            let mut infos = conversations.list_with_info().await;
            let cur = app.lock().unwrap().chat_id.clone();
            if infos.is_empty() {
                push(crate::i18n::tr("  no saved sessions.", "  尚無已儲存的工作階段。").into());
            } else {
                // newest first — last_modified is zero-padded ISO, so a
                // descending string sort is chronological.
                infos.sort_by(|a, b| b.last_modified.cmp(&a.last_modified));
                let total = infos.len();
                let mut text = crate::i18n::tr_owned(
                    format!("  ◆ {} sessions (newest first):", total),
                    format!("  ◆ {} 個工作階段（最新在前）：", total),
                );
                for info in infos.iter().take(15) {
                    let marker = if info.id == cur { "*" } else { " " };
                    let short: String = info.id.chars().take(8).collect();
                    let title = conversations.get_title(&info.id).await;
                    let title_disp = match title {
                        Some(t) => format!("\"{}\"", t),
                        None => crate::i18n::tr("(untitled)", "（未命名）").to_string(),
                    };
                    let count = crate::i18n::tr_owned(
                        format!("{} msg", info.message_count),
                        format!("{} 則訊息", info.message_count),
                    );
                    text.push_str(&format!(
                        "\n    {} {}  {} · {} · {}",
                        marker, short, count, info.last_modified, title_disp
                    ));
                }
                if total > 15 {
                    text.push_str(&crate::i18n::tr_owned(
                        format!("\n    … and {} more", total - 15),
                        format!("\n    …還有 {} 個", total - 15),
                    ));
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
                        _ => {
                            push_err(format!("  '{}' is ambiguous ({} matches)", a, m.len()));
                            return;
                        }
                    }
                }
                None => {
                    // pick most-recent by mtime
                    let home = dirs::home_dir()
                        .unwrap_or_default()
                        .join(".phantom-mesh")
                        .join("conversations");
                    ids.into_iter()
                        .filter_map(|id| {
                            let p = home.join(format!("{}.jsonl", id));
                            std::fs::metadata(&p)
                                .ok()
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
                    // Signal run_loop to cancel any in-flight agent task
                    // from the *previous* session — otherwise its tokens
                    // bleed into the resumed session's transcript and the
                    // conversation store (TUI-1, V12 HIGH).
                    s.pending_interrupt = Some(());
                    s.transcript.push(TranscriptItem::System(format!(
                        "  ◆ resumed session {}",
                        id
                    )));
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
            let raw = path
                .as_ref()
                .and_then(|p| std::fs::read_to_string(p).ok())
                .unwrap_or_else(|| "[]".to_string());
            let parsed: serde_json::Value =
                serde_json::from_str(&raw).unwrap_or(serde_json::Value::Array(vec![]));
            let items = parsed
                .get("todos")
                .and_then(|v| v.as_array())
                .or_else(|| parsed.as_array())
                .cloned()
                .unwrap_or_default();
            if items.is_empty() {
                push("  no todos.".into());
            } else {
                let mut text = format!(
                    "  ◆ {} todo{}:",
                    items.len(),
                    if items.len() == 1 { "" } else { "s" }
                );
                for it in items.iter() {
                    let st = it.get("status").and_then(|v| v.as_str()).unwrap_or("?");
                    let txt = it.get("text").and_then(|v| v.as_str()).unwrap_or("?");
                    let dot = match st {
                        "done" => "●",
                        "in_progress" => "◐",
                        _ => "○",
                    };
                    text.push_str(&format!("\n    {} {}", dot, txt));
                }
                push(text);
            }
        }

        // ── /cost — toggle the Cost pane (Work Track, read-only) ──────────
        "/cost" => {
            let action = arg.unwrap_or("toggle").trim();
            match action {
                "" | "toggle" => {
                    let view = cost_view_from_summary(&cost_tracker.summary().await);
                    let now_on = {
                        let mut s = app.lock().unwrap();
                        s.cost_view = !s.cost_view;
                        if s.cost_view {
                            s.cost_data = Some(view);
                        }
                        s.cost_view
                    };
                    push(if now_on {
                        crate::i18n::tr(
                            "  ◆ cost pane on (session + lifetime spend · per-model)",
                            "  ◆ 成本面板已開啟（本次 + 累計花費 · 各模型）",
                        ).to_string()
                    } else {
                        crate::i18n::tr("  ◆ cost pane off", "  ◆ 成本面板已關閉").to_string()
                    });
                }
                "on" | "show" | "reload" | "refresh" => {
                    let view = cost_view_from_summary(&cost_tracker.summary().await);
                    {
                        let mut s = app.lock().unwrap();
                        s.cost_data = Some(view);
                        s.cost_view = true;
                    }
                    push(crate::i18n::tr("  ◆ cost pane (reloaded)", "  ◆ 成本面板（已重新整理）").into());
                }
                "off" | "hide" => {
                    app.lock().unwrap().cost_view = false;
                    push(crate::i18n::tr("  ◆ cost pane off", "  ◆ 成本面板已關閉").into());
                }
                "line" | "summary" => {
                    // Quick one-liner (the pre-pane behavior, kept for muscle memory).
                    let summary = cost_tracker.summary().await;
                    let total = summary["total_usd"].as_f64().unwrap_or(0.0);
                    let session = summary["session_usd"].as_f64().unwrap_or(0.0);
                    let reqs = summary["requests"].as_u64().unwrap_or(0);
                    push(format!(
                        "  ◆ cost: ${:.4} session  ${:.4} total  ·  {} requests (lifetime)",
                        session, total, reqs
                    ));
                }
                "help" | "?" => {
                    push(crate::i18n::tr(
                        "  /cost [toggle|reload|off|line]  — session + lifetime spend + per-model breakdown",
                        "  /cost [toggle|reload|off|line]  — 本次 + 累計花費 + 各模型明細",
                    ).into());
                }
                other => push_err(crate::i18n::tr_owned(
                    format!("  unknown /cost action: {} (try toggle|reload|off|line)", other),
                    format!("  未知的 /cost 動作：{}（可用 toggle|reload|off|line）", other),
                )),
            }
        }

        // ── /focus — toggle the Focus session pane (P2 / Life Track) ──────
        // Read-only live status of the on-disk focus session; start/stop via
        // the `phantom focus` CLI. Snapshot at toggle/reload (not a live tick).
        "/focus" => {
            let action = arg.unwrap_or("toggle").trim();
            match action {
                "" | "toggle" => {
                    let view = focus_view_from_state();
                    let now_on = {
                        let mut s = app.lock().unwrap();
                        s.focus_view = !s.focus_view;
                        if s.focus_view {
                            s.focus_data = view;
                        }
                        s.focus_view
                    };
                    push(if now_on {
                        crate::i18n::tr(
                            "  ◆ focus pane on (active session timer · `phantom focus start` to begin)",
                            "  ◆ 專注面板已開啟（進行中時段計時 · 用 `phantom focus start` 開始）",
                        ).to_string()
                    } else {
                        crate::i18n::tr("  ◆ focus pane off", "  ◆ 專注面板已關閉").to_string()
                    });
                }
                "on" | "show" | "reload" | "refresh" => {
                    let view = focus_view_from_state();
                    {
                        let mut s = app.lock().unwrap();
                        s.focus_data = view;
                        s.focus_view = true;
                    }
                    push(crate::i18n::tr("  ◆ focus pane (reloaded)", "  ◆ 專注面板（已重新整理）").into());
                }
                "off" | "hide" => {
                    app.lock().unwrap().focus_view = false;
                    push(crate::i18n::tr("  ◆ focus pane off", "  ◆ 專注面板已關閉").into());
                }
                "help" | "?" => {
                    push(crate::i18n::tr(
                        "  /focus [toggle|reload|off]  — live focus-session status (start/stop via `phantom focus`)",
                        "  /focus [toggle|reload|off]  — 即時專注時段狀態（用 `phantom focus` 開始／停止）",
                    ).into());
                }
                other => push_err(crate::i18n::tr_owned(
                    format!("  unknown /focus action: {} (try toggle|reload|off)", other),
                    format!("  未知的 /focus 動作：{}（可用 toggle|reload|off）", other),
                )),
            }
        }

        // ── /evolve — toggle the Evolve Runs pane (P3 Evolve Mesh) ────────
        // Read-only: recent autoevolve runs from ~/.phantom-mesh/autoevolve.log.
        "/evolve" => {
            let action = arg.unwrap_or("toggle").trim();
            match action {
                "" | "toggle" => {
                    let runs = evolve_runs_from_log();
                    let n = runs.len();
                    let now_on = {
                        let mut s = app.lock().unwrap();
                        s.evolve_view = !s.evolve_view;
                        if s.evolve_view {
                            s.evolve_runs = runs;
                        }
                        s.evolve_view
                    };
                    if now_on {
                        push(crate::i18n::tr_owned(
                            format!("  ◆ evolve pane on — {} autoevolve run(s)", n),
                            format!("  ◆ 演化面板已開啟 — {} 筆 autoevolve 執行", n),
                        ));
                    } else {
                        push(crate::i18n::tr("  ◆ evolve pane off", "  ◆ 演化面板已關閉").into());
                    }
                }
                "on" | "show" | "reload" | "refresh" => {
                    let runs = evolve_runs_from_log();
                    let n = runs.len();
                    {
                        let mut s = app.lock().unwrap();
                        s.evolve_runs = runs;
                        s.evolve_view = true;
                    }
                    push(crate::i18n::tr_owned(
                        format!("  ◆ evolve pane: {} run(s) (reloaded)", n),
                        format!("  ◆ 演化面板：{} 筆執行（已重新整理）", n),
                    ));
                }
                "off" | "hide" => {
                    app.lock().unwrap().evolve_view = false;
                    push(crate::i18n::tr("  ◆ evolve pane off", "  ◆ 演化面板已關閉").into());
                }
                "help" | "?" => {
                    push(crate::i18n::tr(
                        "  /evolve [toggle|reload|off]  — recent autoevolve self-improvement runs",
                        "  /evolve [toggle|reload|off]  — 近期 autoevolve 自我改進執行紀錄",
                    ).into());
                }
                other => push_err(crate::i18n::tr_owned(
                    format!("  unknown /evolve action: {} (try toggle|reload|off)", other),
                    format!("  未知的 /evolve 動作：{}（可用 toggle|reload|off）", other),
                )),
            }
        }

        // ── /habits — toggle the Habits pane (P2 / Life Track) ────────────
        // Read-only: recurring-habit streaks from ~/.phantom-mesh/habits.sqlite.
        // Create / check in via the `phantom habit` CLI.
        "/habits" => {
            // First token is the action; the remainder (if any) carries
            // `checkin`'s `<slug> [note]`.
            let full = arg.unwrap_or("toggle").trim();
            let mut split = full.splitn(2, ' ');
            let action = split.next().unwrap_or("toggle").trim();
            let rest = split.next().map(|s| s.trim()).filter(|s| !s.is_empty());
            match action {
                "" | "toggle" => {
                    let rows = habit_rows_load();
                    let n = rows.len();
                    let now_on = {
                        let mut s = app.lock().unwrap();
                        s.habits_view = !s.habits_view;
                        if s.habits_view {
                            s.habits_rows = rows;
                        }
                        s.habits_view
                    };
                    if now_on {
                        push(crate::i18n::tr_owned(
                            format!("  ◆ habits pane on — {} habit(s) tracked", n),
                            format!("  ◆ 習慣面板已開啟 — 追蹤 {} 個習慣", n),
                        ));
                    } else {
                        push(crate::i18n::tr("  ◆ habits pane off", "  ◆ 習慣面板已關閉").into());
                    }
                }
                "on" | "show" | "reload" | "refresh" => {
                    let rows = habit_rows_load();
                    let n = rows.len();
                    {
                        let mut s = app.lock().unwrap();
                        s.habits_rows = rows;
                        s.habits_view = true;
                    }
                    push(crate::i18n::tr_owned(
                        format!("  ◆ habits pane: {} habit(s) (reloaded)", n),
                        format!("  ◆ 習慣面板：{} 個習慣（已重新整理）", n),
                    ));
                }
                // Actionable capture (P2 / Life Track): log a check-in against an
                // existing habit, then re-paint the pane with the fresh streak.
                // Mirrors `phantom habit checkin` (same shared habits.sqlite).
                "checkin" | "log" => match rest {
                    None => push_err(crate::i18n::tr(
                        "  usage: /habits checkin <slug> [note]",
                        "  用法：/habits checkin <slug> [備註]",
                    ).into()),
                    Some(r) => {
                        let mut sr = r.splitn(2, ' ');
                        let slug = sr.next().unwrap_or("").to_string();
                        let note = sr.next().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
                        let checkin = crate::capture_habit_wire::HabitCheckin {
                            habit_slug: slug.clone(),
                            timestamp_ms: chrono::Utc::now().timestamp_millis(),
                            note,
                            source: crate::capture_habit_wire::HabitCheckinSource::Manual,
                        };
                        match crate::capture_habit_wire::record_checkin(&checkin) {
                            Ok(streak) => {
                                // Refresh rows + open the pane so the new streak shows.
                                let rows = habit_rows_load();
                                {
                                    let mut s = app.lock().unwrap();
                                    s.habits_rows = rows;
                                    s.habits_view = true;
                                }
                                push(crate::i18n::tr_owned(
                                    format!(
                                        "  ◆ checked in: {} — streak {} (longest {})",
                                        slug, streak.current_streak, streak.longest_streak
                                    ),
                                    format!(
                                        "  ◆ 已打卡：{} — 連續 {} 天（最長 {}）",
                                        slug, streak.current_streak, streak.longest_streak
                                    ),
                                ));
                            }
                            Err(crate::capture_habit_wire::HabitCaptureError::ChipNotFound { slug }) => {
                                push_err(crate::i18n::tr_owned(
                                    format!("  no such habit: {slug} — create it with `phantom habit create {slug}`"),
                                    format!("  找不到習慣：{slug} — 用 `phantom habit create {slug}` 建立"),
                                ));
                            }
                            Err(e) => push_err(crate::i18n::tr_owned(
                                format!("  habit checkin failed: {e}"),
                                format!("  打卡失敗：{e}"),
                            )),
                        }
                    }
                },
                "off" | "hide" => {
                    app.lock().unwrap().habits_view = false;
                    push(crate::i18n::tr("  ◆ habits pane off", "  ◆ 習慣面板已關閉").into());
                }
                "help" | "?" => {
                    push(crate::i18n::tr(
                        "  /habits [toggle|reload|checkin <slug> [note]|off]  — habit streaks; checkin logs one (same as `phantom habit checkin`)",
                        "  /habits [toggle|reload|checkin <slug> [備註]|off]  — 習慣連續打卡；checkin 記錄一次（等同 `phantom habit checkin`）",
                    ).into());
                }
                other => push_err(crate::i18n::tr_owned(
                    format!("  unknown /habits action: {} (try toggle|reload|checkin|off)", other),
                    format!("  未知的 /habits 動作：{}（可用 toggle|reload|checkin|off）", other),
                )),
            }
        }

        // ── /note — capture a free-text Life Node event (P2 / Life Track) ──
        // Direct synchronous write to ~/.phantom-mesh/events (no daemon, no
        // LLM); age-encrypts at rest when identity.key is present, else
        // plaintext (the confirmation says which). Surfaces in /review today.
        "/note" => match arg {
            None => push_err(crate::i18n::tr("  usage: /note <text>", "  用法：/note <文字>").into()),
            Some(text) => match dirs::home_dir() {
                None => push_err(crate::i18n::tr(
                    "  could not resolve home dir",
                    "  無法解析家目錄",
                ).into()),
                Some(home) => {
                    let phantom = home.join(".phantom-mesh");
                    match crate::life_node::note_capture::capture_note(&phantom, text, &["note".to_string()]) {
                        Ok(out) => {
                            let short: String = out.event_id.chars().take(8).collect();
                            let enc = if out.encrypted {
                                crate::i18n::tr("encrypted", "已加密")
                            } else {
                                crate::i18n::tr("plaintext — no identity.key", "明文 — 無 identity.key")
                            };
                            push(crate::i18n::tr_owned(
                                format!("  ◆ note captured ({}… · {}) — see /review", short, enc),
                                format!("  ◆ 已記錄筆記（{}… · {}）— 用 /review 查看", short, enc),
                            ));
                        }
                        Err(e) => push_err(crate::i18n::tr_owned(
                            format!("  note capture failed: {e}"),
                            format!("  記錄筆記失敗：{e}"),
                        )),
                    }
                }
            },
        },

        // ── /recall — text-search past Life Node events (P2 / Life Track) ──
        // Content-scoped counterpart to /review's date scope. Read-only; reads
        // the same file store /note + /review use; decrypts when identity.key
        // is present (encrypted events skipped otherwise).
        "/recall" | "/find" => match arg {
            None => push_err(crate::i18n::tr(
                "  usage: /recall <text> [--kind food|focus|habit|text] [--since YYYY-MM-DD]",
                "  用法：/recall <文字> [--kind food|focus|habit|text] [--since YYYY-MM-DD]",
            ).into()),
            Some(raw) => match dirs::home_dir() {
                None => push_err(crate::i18n::tr(
                    "  could not resolve home dir",
                    "  無法解析家目錄",
                ).into()),
                Some(home) => {
                    // Parse `--kind`/`--since` flags out of the arg; the rest is
                    // the free-text query (may be empty when only filters given).
                    let mut kind: Option<String> = None;
                    let mut since: Option<String> = None;
                    let mut qwords: Vec<&str> = Vec::new();
                    let toks: Vec<&str> = raw.split_whitespace().collect();
                    let mut i = 0;
                    while i < toks.len() {
                        match toks[i] {
                            "--kind" | "-k" => { kind = toks.get(i + 1).map(|s| s.to_string()); i += 2; }
                            "--since" | "-s" => { since = toks.get(i + 1).map(|s| s.to_string()); i += 2; }
                            w => { qwords.push(w); i += 1; }
                        }
                    }
                    let qtext = qwords.join(" ");
                    // Human label for the header — text + any active filters.
                    let label = {
                        let mut parts: Vec<String> = Vec::new();
                        if !qtext.is_empty() { parts.push(format!("\"{}\"", qtext)); }
                        if let Some(k) = &kind { parts.push(format!("kind={}", k)); }
                        if let Some(s) = &since { parts.push(format!("since={}", s)); }
                        if parts.is_empty() {
                            crate::i18n::tr("recent events", "近期事件").to_string()
                        } else {
                            parts.join(" · ")
                        }
                    };
                    let phantom = home.join(".phantom-mesh");
                    let key = crate::life_node::key_derivation::load_event_key(
                        &phantom.join("identity.key"),
                    )
                    .ok();
                    let filter = crate::life_node::recall::RecallFilter {
                        query: &qtext,
                        kind: kind.as_deref(),
                        since: since.as_deref(),
                    };
                    match crate::life_node::recall::search_events(&phantom.join("events"), key, &filter, 15) {
                        Ok(hits) if hits.is_empty() => push(crate::i18n::tr_owned(
                            format!("  ◆ no events match {label}"),
                            format!("  ◆ 沒有符合 {label} 的事件"),
                        )),
                        Ok(hits) => {
                            let mut text = crate::i18n::tr_owned(
                                format!("  ◆ {} match(es) for {}:", hits.len(), label),
                                format!("  ◆ {} — 找到 {} 筆：", label, hits.len()),
                            );
                            for h in &hits {
                                let when = h.timestamp.get(0..10).unwrap_or("—");
                                let id8: String = h.event_id.chars().take(8).collect();
                                let snippet: String = {
                                    let s: String = h.summary.chars().take(64).collect();
                                    if h.summary.chars().count() > 64 {
                                        format!("{s}…")
                                    } else {
                                        s
                                    }
                                };
                                // id8 → `/event <id>` shows it in full.
                                text.push_str(&format!("\n    {}  {} [{}]   {}", id8, when, h.kind, snippet));
                            }
                            push(text);
                        }
                        Err(e) => push_err(crate::i18n::tr_owned(
                            format!("  recall failed: {e}"),
                            format!("  搜尋失敗：{e}"),
                        )),
                    }
                }
            },
        },

        // ── /event <id> — show one Life Node event in full (P2 / Life Track) ─
        // TUI twin of `phantom event show`: resolve an id (from /recall) → full
        // detail in the transcript. Read-only; decrypts when identity.key present.
        "/event" => match arg {
            None => push_err(crate::i18n::tr(
                "  usage: /event <id>   (id from /recall)",
                "  用法：/event <id>（id 來自 /recall）",
            ).into()),
            Some(id_arg) => match dirs::home_dir() {
                None => push_err(crate::i18n::tr("  could not resolve home dir", "  無法解析家目錄").into()),
                Some(home) => {
                    let phantom = home.join(".phantom-mesh");
                    let events_dir = phantom.join("events");
                    match crate::life_node::data_cli::resolve_event_id(&events_dir, id_arg) {
                        Err(e) => push_err(crate::i18n::tr_owned(
                            format!("  /event: {e}"),
                            format!("  /event：{e}"),
                        )),
                        Ok(id) => {
                            let store = crate::life_node::storage::EventStore::with_identity_file(
                                &events_dir,
                                &phantom.join("identity.key"),
                            );
                            match store.read_meta(&id) {
                                Err(e) => push_err(crate::i18n::tr_owned(
                                    format!("  /event: can't read {id}: {e}"),
                                    format!("  /event：無法讀取 {id}：{e}"),
                                )),
                                Ok(meta) => {
                                    let kind = match meta.kind {
                                        crate::rpc_wire::EventKind::Food => "food",
                                        crate::rpc_wire::EventKind::Focus => "focus",
                                        crate::rpc_wire::EventKind::Habit => "habit",
                                        crate::rpc_wire::EventKind::Text => "text",
                                    };
                                    let mut text = format!("  ◆ event {}", id);
                                    text.push_str(&format!("\n    kind     {}", kind));
                                    text.push_str(&format!("\n    when     {}", meta.timestamp));
                                    if !meta.tags.is_empty() {
                                        text.push_str(&format!("\n    tags     {}", meta.tags.join(", ")));
                                    }
                                    if let Ok(a) = store.read_analysis(&id) {
                                        text.push_str(&format!("\n    summary  {}", a.summary));
                                        if let Some(s) = a.goal_impact.as_deref().filter(|s| !s.is_empty()) {
                                            text.push_str(&format!("\n    impact   {}", s));
                                        }
                                        if let Some(s) = a.suggestion.as_deref().filter(|s| !s.is_empty()) {
                                            text.push_str(&format!("\n    suggest  {}", s));
                                        }
                                    }
                                    push(text);
                                }
                            }
                        }
                    }
                }
            },
        },

        // ── /stats — life-log rollup (P2 / Life Track) ────────────────────
        // Aggregate over all captured events: total · date span · last-7d ·
        // by-kind. Distinct from /review (one day) + /recall (one query).
        "/stats" => match dirs::home_dir() {
            None => push_err(crate::i18n::tr("  could not resolve home dir", "  無法解析家目錄").into()),
            Some(home) => match crate::life_node::data_cli::compute_stats(&home) {
                Ok(s) if s.total == 0 => push(crate::i18n::tr(
                    "  ◆ life log is empty — capture one with /note <text>",
                    "  ◆ 生活紀錄是空的 — 用 /note <文字> 記錄一筆",
                ).into()),
                Ok(s) => {
                    let span = match (&s.earliest, &s.latest) {
                        (Some(e), Some(l)) => format!("{} → {}", e, l),
                        _ => "—".to_string(),
                    };
                    let mut text = crate::i18n::tr_owned(
                        format!("  ◆ life log — {} events · {} · {} in last 7d", s.total, span, s.last_7d),
                        format!("  ◆ 生活紀錄 — {} 筆事件 · {} · 近 7 天 {} 筆", s.total, span, s.last_7d),
                    );
                    for (kind, n) in &s.by_kind {
                        text.push_str(&format!("\n    {:<7} {}", kind, n));
                    }
                    push(text);
                }
                Err(e) => push_err(crate::i18n::tr_owned(
                    format!("  stats failed: {e}"),
                    format!("  統計失敗：{e}"),
                )),
            },
        },

        // ── /diff — working-tree git status + diff --stat (Work Track) ────
        // Read-only; reuses the agent's own git tools. See what changed without
        // leaving the full-screen TUI. `/diff [--cached] [<file>]`.
        "/diff" => {
            let a = arg.unwrap_or("").trim();
            let cached = a.split_whitespace().any(|w| w == "--cached" || w == "--staged");
            let full = a.split_whitespace().any(|w| w == "--full" || w == "-p");
            let file = a.split_whitespace().find(|w| !w.starts_with('-'));
            let status = crate::tools::git::status(&serde_json::json!({ "path": "." })).await;
            let mut dargs = serde_json::json!({ "path": ".", "cached": cached, "full": full });
            if let Some(f) = file {
                dargs["file"] = serde_json::json!(f);
            }
            let diff = crate::tools::git::diff(&dargs).await;
            let scope = if cached {
                crate::i18n::tr("staged", "已暫存")
            } else {
                crate::i18n::tr("working tree", "工作區")
            };
            // Full patches can be long — cap so the transcript stays usable.
            let diff_body = if diff.trim().is_empty() {
                crate::i18n::tr("  (no changes)", "  （無變更）").to_string()
            } else if full {
                let lines: Vec<&str> = diff.trim_end().lines().collect();
                if lines.len() > 150 {
                    let head = lines[..150].join("\n");
                    crate::i18n::tr_owned(
                        format!("{head}\n  … ({} more lines — narrow with `/diff --full <file>`)", lines.len() - 150),
                        format!("{head}\n  …（還有 {} 行 — 用 `/diff --full <file>` 縮小範圍）", lines.len() - 150),
                    )
                } else {
                    diff.trim_end().to_string()
                }
            } else {
                diff.trim_end().to_string()
            };
            let kind = if full {
                crate::i18n::tr("diff", "diff")
            } else {
                crate::i18n::tr("diff --stat", "diff --stat")
            };
            push(crate::i18n::tr_owned(
                format!("  ◆ git status:\n{}\n  ◆ {} ({}):\n{}", status.trim_end(), kind, scope, diff_body),
                format!("  ◆ git 狀態：\n{}\n  ◆ {}（{}）：\n{}", status.trim_end(), kind, scope, diff_body),
            ));
        }

        // ── /log [n] — recent git history (Work Track) ────────────────────
        // Companion to /diff; reuses the agent's read-only git log tool.
        "/log" => {
            let n = arg
                .and_then(|a| a.trim().parse::<u64>().ok())
                .unwrap_or(10)
                .clamp(1, 50);
            let out = crate::tools::git::log(&serde_json::json!({ "path": ".", "n": n })).await;
            push(crate::i18n::tr_owned(
                format!("  ◆ git log (last {}):\n{}", n, out.trim_end()),
                format!("  ◆ git log（最近 {} 筆）：\n{}", n, out.trim_end()),
            ));
        }

        // ── /branch — current branch + list (Work Track) ─────────────────
        // Completes the git triad (/diff, /log); reuses the read-only git tool.
        "/branch" | "/branches" => {
            let out = crate::tools::git::branch(&serde_json::json!({ "path": "." })).await;
            push(crate::i18n::tr_owned(
                format!("  ◆ git branches (* = current):\n{}", out.trim_end()),
                format!("  ◆ git 分支（* = 目前）：\n{}", out.trim_end()),
            ));
        }

        "/perm" => match arg {
            Some(m @ ("ask" | "allow" | "deny")) => {
                std::env::set_var("PHANTOM_PERM", m);
                push(format!("  ◆ permission mode: {}", m));
            }
            Some(other) => push_err(format!("  unknown: {}. usage: /perm ask|allow|deny", other)),
            None => {
                let cur = std::env::var("PHANTOM_PERM").unwrap_or_else(|_| "allow".into());
                push(format!(
                    "  ◆ permission mode: {}  (usage: /perm ask|allow|deny)",
                    cur
                ));
            }
        },

        "/density" => match arg {
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
        },

        "/theme" => match arg {
            Some(n @ ("dark" | "light" | "claude" | "codex" | "gemini" | "mono")) => {
                std::env::set_var("PHANTOM_THEME", n);
                push(format!("  ◆ theme: {} (restart TUI for full effect)", n));
            }
            Some(other) => push_err(format!("  unknown theme: {}", other)),
            None => {
                let cur = std::env::var("PHANTOM_THEME").unwrap_or_else(|_| "dark".into());
                push(format!("  ◆ theme: {}", cur));
            }
        },

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
        "/peers" | "/mesh" => {
            let action = arg.unwrap_or("status").trim();
            match action {
                "" | "status" | "ping" | "ls" | "list" => {
                    push("  ◆ pinging cluster peers …".into());
                    match crate::cli_config::cluster_status_lines().await {
                        Ok(lines) => {
                            let mut text = String::new();
                            for l in lines {
                                text.push_str(&format!("  {}\n", l));
                            }
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
                            for l in lines {
                                text.push_str(&format!("  {}\n", l));
                            }
                            push(text.trim_end().to_string());
                        }
                        Err(e) => push_err(format!("  ✗ {}", e)),
                    }
                }
                "leave" => match crate::cli_config::cluster_leave_lines() {
                    Ok(lines) => {
                        for l in lines {
                            push(format!("  {}", l));
                        }
                    }
                    Err(e) => push_err(format!("  ✗ {}", e)),
                },
                "help" | "?" => {
                    push("  /cluster              → status of all peers (alias: ping/list)\n  \
                          /cluster status       → same\n  \
                          /cluster who          → live TUIs on other machines (alias: sessions/online)\n  \
                          /cluster leave        → remove [cluster] block from agents.toml\n  \
                          \n  \
                          (To join: run `phantom cluster join <name>` from PowerShell —\n  \
                          requires CLUSTER_SECRET in env which is why it's not in TUI)".into());
                }
                other => push_err(format!(
                    "  unknown /cluster sub: {} — try /cluster help",
                    other
                )),
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
                push_err(
                    "  /fanout <prompt>   broadcast prompt to all peers in parallel.\n  \
                          /fanout --agent coder <prompt>   override remote agent."
                        .into(),
                );
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
            push(format!(
                "  ◆ fanout → {} peer(s): {}",
                peers.len(),
                peers.join(", ")
            ));
            for peer in peers {
                let peer_c = peer.clone();
                let prompt_c = prompt_body.clone();
                let agent_c = remote_agent.clone();
                let app_clone = app.clone();
                tokio::spawn(async move {
                    let result = crate::cli_config::dispatch_lines(
                        &[],
                        Some(&peer_c),
                        &agent_c,
                        &prompt_c,
                        false,
                    )
                    .await;
                    let mut s = app_clone.lock().unwrap();
                    match result {
                        Ok(lines) => {
                            s.transcript
                                .push(TranscriptItem::System(format!("  ─ {} ─", peer_c)));
                            for l in lines {
                                s.transcript.push(TranscriptItem::System(l));
                            }
                        }
                        Err(e) => s.transcript.push(TranscriptItem::Error(format!(
                            "  ✗ {} dispatch failed: {}",
                            peer_c, e
                        ))),
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
            // BUG FIX (V11 P0): the previous implementation held
            // `app.lock()` for the entire match block, then called the
            // `push` / `push_err` closures which try to re-acquire the
            // same `std::sync::Mutex` → deadlock. We now compute the
            // mutation + message *first* with the lock held only across
            // the field read/write, then drop the guard before pushing
            // the transcript line. Pinned by
            // `mouse_capture_toggle_round_trip`.
            let msg: Result<String, String> = {
                let mut s = app.lock().unwrap();
                match arg {
                    Some("on") | Some("enable") => {
                        if s.mouse_capture_active {
                            Ok("  ◆ mouse capture: already ON".into())
                        } else {
                            s.mouse_capture_pending = Some(true);
                            Ok(
                                "  ◆ mouse capture → ON (scroll-wheel works, text-select blocked)"
                                    .into(),
                            )
                        }
                    }
                    Some("off") | Some("disable") => {
                        if !s.mouse_capture_active {
                            Ok("  ◆ mouse capture: already OFF".into())
                        } else {
                            s.mouse_capture_pending = Some(false);
                            Ok("  ◆ mouse capture → OFF (drag to select text; PgUp/PgDn to scroll)".into())
                        }
                    }
                    Some("status") | None => {
                        let state = if s.mouse_capture_active { "ON" } else { "OFF" };
                        Ok(format!(
                            "  ◆ mouse capture: {}    /mouse on | off  to toggle",
                            state
                        ))
                    }
                    Some(other) => Err(format!(
                        "  unknown: {}.  usage: /mouse on | off | status",
                        other
                    )),
                }
                // guard `s` dropped here
            };
            match msg {
                Ok(text) => push(text),
                Err(text) => push_err(text),
            }
        }

        // ── /model ─────────────────────────────────────────────────────
        // Mirrors REPL semantics. Picker (/model pick) needs a blocking
        // readline which is awkward in TUI's single-input panel — point
        // those users at `phantom --repl` instead.
        "/model" => {
            let cfg = runtime.config();
            let arg_str = arg.unwrap_or("");
            let (action, target) = arg_str
                .split_once(' ')
                .map(|(a, t)| (a.trim(), t.trim()))
                .unwrap_or((arg_str, ""));

            if arg_str.is_empty() {
                let mut text = String::from("  ◆ model — current + provider defaults:\n");
                let cur_agent = app.lock().unwrap().agent_name().to_string();
                let cur_model = cfg
                    .agent
                    .get(&cur_agent)
                    .map(|a| a.model.as_str())
                    .unwrap_or("<unset>");
                text.push_str(&format!("    active: {}/{}\n", cur_agent, cur_model));
                if !cfg.providers.is_empty() {
                    text.push_str("\n    providers:\n");
                    for (pname, pent) in cfg.providers.iter() {
                        let model = pent
                            .default_model
                            .as_deref()
                            .unwrap_or("<no default_model>");
                        text.push_str(&format!("      • {:<14} {}\n", pname, model));
                    }
                }
                text.push_str("\n    switch:  /model <name>            (model only)\n");
                text.push_str("             /model <provider>:<name>   (provider + model)\n");
                text.push_str("             /model fast | smart | cheap\n");
                text.push_str("             /model fetch <provider>    (live model list)\n");
                text.push_str(
                    "             /model pick  <provider>    (REPL only — phantom --repl)",
                );
                push(text);
            } else if action == "fetch" {
                if target.is_empty() {
                    push_err("  usage: /model fetch <provider>".into());
                } else {
                    let ent = cfg.providers.get(target).cloned();
                    match ent {
                        None => push_err(format!("  unknown provider '{}'.", target)),
                        Some(ent) => {
                            let key = ent.api_key.clone().filter(|s| !s.is_empty()).or_else(|| {
                                ent.api_key_env
                                    .as_ref()
                                    .and_then(|v| std::env::var(v).ok())
                                    .filter(|s| !s.is_empty())
                            });
                            let url = ent
                                .url
                                .clone()
                                .or_else(|| {
                                    crate::keys::default_provider_meta(target)
                                        .map(|(_, u)| u.to_string())
                                })
                                .unwrap_or_default();
                            match (key, url.is_empty()) {
                                (None, _) => push_err(format!(
                                    "  no key for {} — /keys add {} first",
                                    target, target
                                )),
                                (_, true) => push_err(format!("  no base url for {}", target)),
                                (Some(k), false) => {
                                    push(format!(
                                        "  ◆ fetching models from {} → {} …",
                                        target, url
                                    ));
                                    // Annotated form: each row carries a free/paid heuristic
                                    // (see keys::is_likely_free_model). Free models float to
                                    // the top so users picking from a long list see the no-cost
                                    // options first.
                                    match crate::keys::fetch_models_annotated(
                                        &ent.provider_type,
                                        &url,
                                        &k,
                                    )
                                    .await
                                    {
                                        Err(e) => push_err(format!("  ✗ {}", e)),
                                        Ok(rows) if rows.is_empty() => {
                                            push_err("  ⚠ empty model list".into())
                                        }
                                        Ok(mut rows) => {
                                            // Free-first ordering, alphabetical within each tier.
                                            rows.sort_by(|a, b| {
                                                b.is_free.cmp(&a.is_free).then(a.id.cmp(&b.id))
                                            });
                                            let n_free = rows.iter().filter(|r| r.is_free).count();
                                            let mut text = format!(
                                                "  ✓ {} models from {}  ({} free · {} paid):\n",
                                                rows.len(),
                                                target,
                                                n_free,
                                                rows.len() - n_free
                                            );
                                            // Each row prints `provider:model` so the user can copy
                                            // the line and paste it directly into `/model <X>:<Y>`
                                            // or into `[agent.X].providers = [...]`.
                                            for r in &rows {
                                                let tag =
                                                    if r.is_free { "[FREE]" } else { "[paid]" };
                                                text.push_str(&format!(
                                                    "    {} {:<6}  {}:{}\n",
                                                    "•", tag, target, r.id
                                                ));
                                            }
                                            text.push_str(&format!(
                                                "\n  switch:  /model {}:<name>",
                                                target
                                            ));
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
                    "fast" => &[
                        ("groq", "llama-3.3-70b-versatile"),
                        ("gemini", "gemini-2.0-flash-exp"),
                        ("openrouter", "google/gemini-2.0-flash-exp"),
                        ("openai", "gpt-4o-mini"),
                    ][..],
                    "smart" => &[
                        ("anthropic", "claude-sonnet-4-20250514"),
                        ("openai", "gpt-4o"),
                        ("openrouter", "anthropic/claude-sonnet-4"),
                        ("groq", "llama-3.3-70b-versatile"),
                    ][..],
                    "cheap" => &[
                        ("groq", "llama-3.1-8b-instant"),
                        ("gemini", "gemini-2.0-flash-lite"),
                        ("openrouter", "google/gemini-2.0-flash-lite"),
                        ("opencode", "claude-haiku-4-5-free"),
                    ][..],
                    _ => &[][..],
                };
                let pick = preset
                    .iter()
                    .find(|(p, _)| cfg.providers.contains_key(*p))
                    .copied();
                match pick {
                    None => {
                        push_err(format!(
                        "  ✗ no '{}' preset — none of [{}] are configured. /keys add <provider>",
                        action,
                        preset.iter().map(|(p, _)| *p).collect::<Vec<_>>().join(", "),
                    ))
                    }
                    Some((pname, mname)) => {
                        let value = format!("{}:{}", pname, mname);
                        std::env::set_var("PHANTOM_RUNTIME_OVERRIDE", &value);
                        std::env::set_var("PHANTOM_PROVIDER_OVERRIDE", pname);
                        let _ = crate::cli_config::write_runtime_override(Some(&value));
                        let mut s = app.lock().unwrap();
                        s.model_override = Some(mname.to_string());
                        push(format!(
                            "  ✓ {} preset → {}/{} (saved to ~/.phantom-mesh/runtime-override)",
                            action, pname, mname
                        ));
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
                    push(format!(
                        "  ✓ switched to {}/{} for this session",
                        pname, mname
                    ));
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
                let key = ent.api_key.clone().filter(|s| !s.is_empty()).or_else(|| {
                    ent.api_key_env
                        .as_ref()
                        .and_then(|v| std::env::var(v).ok())
                        .filter(|s| !s.is_empty())
                });
                let url = ent
                    .url
                    .clone()
                    .or_else(|| {
                        crate::keys::default_provider_meta(name).map(|(_, u)| u.to_string())
                    })
                    .unwrap_or_default();
                match (key, url.is_empty()) {
                    (Some(k), false) => {
                        targets.push((name.clone(), ent.provider_type.clone(), url, k))
                    }
                    (None, _) => {
                        skipped.push(format!("{} (no key — /keys set {} <key>)", name, name))
                    }
                    (_, true) => skipped.push(format!("{} (no base url)", name)),
                }
            }
            if targets.is_empty() {
                push_err(
                    "  no providers with usable keys — /keys set <provider> <key> first".into(),
                );
            } else {
                // Split providers into "served from cache" (fresh enough) and
                // "needs live fetch". Cached entries return instantly with the
                // same ModelInfo shape; live ones go through the wire and update
                // the cache for next time.
                let mut cached_results: Vec<(String, Vec<crate::keys::ModelInfo>)> = Vec::new();
                let mut to_fetch: Vec<(String, String, String, String)> = Vec::new();
                if !force_refresh {
                    for (name, ptype, url, key) in &targets {
                        if let Some(rows) = crate::models_cache::get_fresh(
                            name,
                            crate::models_cache::DEFAULT_TTL_MS,
                        ) {
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
                    push(format!(
                        "  ◆ fetching {} provider(s) live, {} from cache (TTL {}m) …",
                        to_fetch.len(),
                        cached_results.len(),
                        crate::models_cache::DEFAULT_TTL_MS / 60_000
                    ));
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

                let mut total_free = 0usize;
                let mut total_paid = 0usize;
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
                            text.push_str(&format!(
                                "    {} models  ({} free · {} paid)\n",
                                rows.len(),
                                n_free,
                                rows.len() - n_free
                            ));
                            // Print with a global 1-based row number so the
                            // user can /pick by number instead of retyping
                            // long ids. Each row is also still copy-pasteable
                            // (`provider:model` form) for direct /model use.
                            for r in &rows {
                                let tag = if r.is_free { "[FREE]" } else { "[paid]" };
                                let entry = format!("{}:{}", name, r.id);
                                flat.push(entry.clone());
                                text.push_str(&format!(
                                    "      {:>3}. {} {}\n",
                                    flat.len(),
                                    tag,
                                    entry
                                ));
                            }
                        }
                    }
                }
                if !skipped.is_empty() {
                    text.push_str("\n  skipped:\n");
                    for s in &skipped {
                        text.push_str(&format!("    • {}\n", s));
                    }
                }
                let header = format!(
                    "  ◆ {} providers · {} free · {} paid total\n  switch by id:    /model <provider>:<name>\n  set priority by row#:  /pick <agent> <n1> <n2> ...   (uses the numbers below)",
                    targets.len(), total_free, total_paid);
                push(header);
                push(text);
                push(
                    "  free/paid is a heuristic — confirm with the provider's billing dashboard."
                        .into(),
                );
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
                    for b in &bad {
                        push_err(format!("  ⚠ skipped {}", b));
                    }
                    if picked.is_empty() {
                        push_err("  no valid numbers provided".into());
                    } else {
                        match crate::cli_config::providers_priority_lines(agent, &picked) {
                            Ok(lines) => {
                                for line in lines {
                                    push(format!("  {}", line));
                                }
                            }
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
                                crate::keys::KeyState::Inline => "✓ inline".to_string(),
                                crate::keys::KeyState::EnvResolved { var } => {
                                    format!("✓ env (${})", var)
                                }
                                crate::keys::KeyState::EnvMissing { var } => {
                                    format!("⚠ env-unset (${})", var)
                                }
                                crate::keys::KeyState::NotConfigured => "✗ no key".to_string(),
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
                            Ok(()) => push(format!(
                                "  ✓ dropped api_key for {} (restart phantom to apply)",
                                target
                            )),
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
                                let key =
                                    ent.api_key.clone().filter(|s| !s.is_empty()).or_else(|| {
                                        ent.api_key_env
                                            .as_ref()
                                            .and_then(|v| std::env::var(v).ok())
                                            .filter(|s| !s.is_empty())
                                    });
                                let url = ent
                                    .url
                                    .clone()
                                    .or_else(|| {
                                        crate::keys::default_provider_meta(target)
                                            .map(|(_, u)| u.to_string())
                                    })
                                    .unwrap_or_default();
                                match (key, url.is_empty()) {
                                    (None, _) => push_err(format!("  no key for {}", target)),
                                    (_, true) => push_err(format!("  no base url for {}", target)),
                                    (Some(k), false) => {
                                        push(format!(
                                            "  ◆ probing {} → {} (5s timeout) …",
                                            target, url
                                        ));
                                        match crate::keys::probe_provider(target, &url, &k).await {
                                            Ok(r) => {
                                                let mark = if r.ok { "✓" } else { "✗" };
                                                push(format!(
                                                    "  {} {} ({} ms)",
                                                    mark, r.message, r.elapsed_ms
                                                ));
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
                                for line in lines {
                                    push(format!("  {}", line));
                                }
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
            let rest = sub_parts.next().unwrap_or("").trim();
            match action {
                "" | "list" => {
                    let cfg = runtime.config();
                    if cfg.providers.is_empty() {
                        push_err(
                            "  no [providers.*] block in agents.toml — /keys add <name> in REPL"
                                .into(),
                        );
                    } else {
                        let mut text =
                            format!("  ◆ {} configured providers:\n", cfg.providers.len());
                        for (name, ent) in cfg.providers.iter() {
                            let key_state =
                                if ent.api_key.as_ref().filter(|s| !s.is_empty()).is_some() {
                                    "✓ key"
                                } else if ent.api_key_env.is_some() {
                                    "✓ env"
                                } else {
                                    "✗ no key"
                                };
                            let model = ent.default_model.as_deref().unwrap_or("<none>");
                            text.push_str(&format!(
                                "    • {:<14} {} · {}\n",
                                name, key_state, model
                            ));
                        }
                        text.push_str("\n  /provider priority <agent>                 show current failover order\n");
                        text.push_str(
                            "  /provider priority <agent> <p1> <p2> ...   set the failover order",
                        );
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
                            Ok(lines) => {
                                for line in lines {
                                    push(format!("  {}", line));
                                }
                            }
                            Err(e) => push_err(format!("  ✗ {}", e)),
                        }
                    }
                }
                other => push_err(format!(
                    "  unknown /provider subcommand: {} — try list / priority",
                    other
                )),
            }
        }

        // ── /priority — open interactive priority picker ──────────────
        // Modal popup that lists [agent.X].providers and lets the user
        // arrow-key reorder, delete, and save. Acts on the currently
        // active agent unless a name is given as arg.
        "/priority" | "/prio" => {
            let target_agent = arg.map(String::from).unwrap_or_else(|| {
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
                    {
                        app.lock().unwrap().sidebar_visible = true;
                    }
                    push("  ◆ sidebar on".into());
                }
                "off" | "hide" => {
                    {
                        app.lock().unwrap().sidebar_visible = false;
                    }
                    push("  ◆ sidebar off".into());
                }
                "refresh" | "sync" => {
                    let me = crate::cli_config::resolve_self_node_name();
                    let new_peers: Vec<String> = crate::cli_config::read_peers_json()
                        .map(|peers| {
                            peers
                                .into_iter()
                                .map(|p| p.name)
                                .filter(|n| Some(n.as_str()) != me.as_deref())
                                .collect()
                        })
                        .unwrap_or_default();
                    let n = new_peers.len();
                    {
                        app.lock().unwrap().sidebar_peers = new_peers;
                    }
                    push(format!("  ◆ sidebar peers refreshed: {} peer(s)", n));
                }
                "status" => {
                    // Read terminal size directly — handler doesn't have
                    // frame access. Useful when /sidebar appears to do
                    // nothing (gate vs flag mismatch).
                    let (cols, rows) = crossterm::terminal::size().unwrap_or((0, 0));
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
                        cols,
                        rows,
                        needs,
                        if actually_shown {
                            "YES"
                        } else {
                            "no (auto-hidden — window too narrow OR flag off)"
                        },
                        peer_count,
                    ));
                }
                "help" | "?" => {
                    push(
                        "  /sidebar              → toggle on/off\n  \
                          /sidebar on|off       → set explicitly\n  \
                          /sidebar status       → show flag + actual visibility + window size\n  \
                          /sidebar refresh      → reload peers from peers.json"
                            .into(),
                    );
                }
                other => push_err(format!(
                    "  unknown /sidebar sub: {} — try /sidebar help",
                    other
                )),
            }
        }

        // ── /cluster — toggle the Cluster Status pane (P1) ────────────
        "/cluster" => {
            let action = arg.unwrap_or("toggle").trim();
            match action {
                "" | "toggle" => {
                    // Flip; rebuild rows from peers.json config when turning on
                    // (so a peers.json edit is picked up). Live ping is a follow-on.
                    let rows = cluster_rows_from_config();
                    let n = rows.len();
                    let now_on = {
                        let mut s = app.lock().unwrap();
                        s.cluster_view = !s.cluster_view;
                        if s.cluster_view {
                            s.cluster_rows = rows;
                        }
                        s.cluster_view
                    };
                    if now_on {
                        push(crate::i18n::tr_owned(
                            format!("  ◆ cluster pane on — {} node(s) (configured; live ping TBD)", n),
                            format!("  ◆ 叢集面板已開啟 — {} 個節點（已設定；即時 ping 待補）", n),
                        ));
                    } else {
                        push(crate::i18n::tr("  ◆ cluster pane off", "  ◆ 叢集面板已關閉").into());
                    }
                }
                "on" | "show" => {
                    let rows = cluster_rows_from_config();
                    let n = rows.len();
                    {
                        let mut s = app.lock().unwrap();
                        s.cluster_rows = rows;
                        s.cluster_view = true;
                    }
                    push(crate::i18n::tr_owned(
                        format!("  ◆ cluster pane on — {} node(s) (configured; /cluster refresh to ping)", n),
                        format!("  ◆ 叢集面板已開啟 — {} 個節點（已設定；用 /cluster refresh 來 ping）", n),
                    ));
                }
                "refresh" => {
                    // Live ping: build a config-backed AppState, refresh_all()
                    // pings every peer in parallel. Done BEFORE taking the app
                    // lock so the render loop never blocks on the network.
                    push(crate::i18n::tr("  ◆ pinging cluster peers…", "  ◆ 正在 ping 叢集節點…").into());
                    let mut cfg_state = crate::AppState::new();
                    if let Some(content) = find_config_simple() {
                        cfg_state.load_config_toml(&content);
                    }
                    let self_name = crate::cli_config::resolve_self_node_name();
                    let statuses = cfg_state.cluster_manager.refresh_all().await;
                    let rows = cluster_rows_from_statuses(&statuses, self_name.as_deref());
                    let n = rows.len();
                    let online = rows.iter().filter(|r| r.online).count();
                    {
                        let mut s = app.lock().unwrap();
                        s.cluster_rows = rows;
                        s.cluster_view = true;
                    }
                    push(crate::i18n::tr_owned(
                        format!("  ◆ cluster pane: {} node(s) · {} online (live)", n, online),
                        format!("  ◆ 叢集面板：{} 個節點 · {} 個上線（即時）", n, online),
                    ));
                }
                "off" | "hide" => {
                    app.lock().unwrap().cluster_view = false;
                    push(crate::i18n::tr("  ◆ cluster pane off", "  ◆ 叢集面板已關閉").into());
                }
                "help" | "?" => {
                    push(crate::i18n::tr(
                        "  /cluster [toggle|on|off|refresh]  — show configured mesh peers + caps",
                        "  /cluster [toggle|on|off|refresh]  — 顯示已設定的 mesh 節點 + 能力",
                    ).into());
                }
                other => push_err(crate::i18n::tr_owned(
                    format!("  unknown /cluster action: {} (try toggle|on|off|refresh)", other),
                    format!("  未知的 /cluster 動作：{}（可用 toggle|on|off|refresh）", other),
                )),
            }
        }

        // ── /goals — toggle the Evolve Goals pane (P3, read-only) ─────
        "/goals" => {
            let action = arg.unwrap_or("toggle").trim();
            match action {
                "" | "toggle" => {
                    let rows = goal_rows_from_file();
                    let n = rows.len();
                    let now_on = {
                        let mut s = app.lock().unwrap();
                        s.goals_view = !s.goals_view;
                        if s.goals_view {
                            s.goals_rows = rows;
                            s.goals_selected = 0;
                        }
                        s.goals_view
                    };
                    if now_on {
                        push(crate::i18n::tr_owned(
                            format!("  ◆ goals pane on — {} goal(s) from EVOLVE-GOALS.md", n),
                            format!("  ◆ 目標面板已開啟 — 來自 EVOLVE-GOALS.md 的 {} 個目標", n),
                        ));
                    } else {
                        push(crate::i18n::tr("  ◆ goals pane off", "  ◆ 目標面板已關閉").into());
                    }
                }
                "on" | "show" | "reload" | "refresh" => {
                    let rows = goal_rows_from_file();
                    let n = rows.len();
                    {
                        let mut s = app.lock().unwrap();
                        s.goals_rows = rows;
                        s.goals_view = true;
                    }
                    push(crate::i18n::tr_owned(
                        format!("  ◆ goals pane: {} goal(s) (reloaded)", n),
                        format!("  ◆ 目標面板：{} 個目標（已重新整理）", n),
                    ));
                }
                "off" | "hide" => {
                    app.lock().unwrap().goals_view = false;
                    push(crate::i18n::tr("  ◆ goals pane off", "  ◆ 目標面板已關閉").into());
                }
                "help" | "?" => {
                    push(crate::i18n::tr(
                        "  /goals [toggle|reload|off]  — view EVOLVE-GOALS.md (mark done via `phantom evolve goals mark-done <#>`)",
                        "  /goals [toggle|reload|off]  — 檢視 EVOLVE-GOALS.md（用 `phantom evolve goals mark-done <#>` 標記完成）",
                    ).into());
                }
                other => push_err(crate::i18n::tr_owned(
                    format!("  unknown /goals action: {} (try toggle|reload|off)", other),
                    format!("  未知的 /goals 動作：{}（可用 toggle|reload|off）", other),
                )),
            }
        }

        // ── /identity — toggle the Identity & Vault pane (P4, read-only) ──
        "/identity" => {
            let action = arg.unwrap_or("toggle").trim();
            match action {
                "" | "toggle" => {
                    let view = identity_view_from_state();
                    let now_on = {
                        let mut s = app.lock().unwrap();
                        s.identity_view = !s.identity_view;
                        if s.identity_view {
                            s.identity_data = Some(view);
                        }
                        s.identity_view
                    };
                    push(if now_on {
                        crate::i18n::tr(
                            "  ◆ identity pane on (P4 — identity + encryption-at-rest)",
                            "  ◆ 身分面板已開啟（P4 — 身分 + 靜態加密）",
                        ).to_string()
                    } else {
                        crate::i18n::tr("  ◆ identity pane off", "  ◆ 身分面板已關閉").to_string()
                    });
                }
                "on" | "show" | "reload" | "refresh" => {
                    let view = identity_view_from_state();
                    {
                        let mut s = app.lock().unwrap();
                        s.identity_data = Some(view);
                        s.identity_view = true;
                    }
                    push(crate::i18n::tr("  ◆ identity pane (reloaded)", "  ◆ 身分面板（已重新整理）").into());
                }
                "off" | "hide" => {
                    app.lock().unwrap().identity_view = false;
                    push(crate::i18n::tr("  ◆ identity pane off", "  ◆ 身分面板已關閉").into());
                }
                "help" | "?" => {
                    push(crate::i18n::tr(
                        "  /identity [toggle|reload|off]  — your identity + what's encrypted at rest (P4)",
                        "  /identity [toggle|reload|off]  — 你的身分 + 靜態加密狀態（P4）",
                    ).into());
                }
                other => push_err(crate::i18n::tr_owned(
                    format!("  unknown /identity action: {} (try toggle|reload|off)", other),
                    format!("  未知的 /identity 動作：{}（可用 toggle|reload|off）", other),
                )),
            }
        }

        // ── /review — daily-review pane (P2 / Life Track, read-only) ──────
        // /review                → toggle (today's date)
        // /review YYYY-MM-DD      → open that date
        // /review reload          → re-read the current date
        // /review off             → close
        "/review" => {
            let action = arg.unwrap_or("toggle").trim();
            let today = chrono::Local::now().format("%Y-%m-%d").to_string();
            match action {
                "" | "toggle" => {
                    let turning_on = {
                        let mut s = app.lock().unwrap();
                        s.review_view = !s.review_view;
                        s.review_view
                    };
                    if turning_on {
                        let view = review_view_from_state(&today);
                        let n = view.event_count;
                        app.lock().unwrap().review_data = Some(view);
                        push(crate::i18n::tr_owned(
                            format!("  ◆ review pane on — {} ({} event(s))", today, n),
                            format!("  ◆ 回顧面板已開啟 — {}（{} 筆事件）", today, n),
                        ));
                    } else {
                        push(crate::i18n::tr("  ◆ review pane off", "  ◆ 回顧面板已關閉").into());
                    }
                }
                "reload" | "refresh" => {
                    let date = {
                        let s = app.lock().unwrap();
                        s.review_data
                            .as_ref()
                            .map(|v| v.date.clone())
                            .unwrap_or_else(|| today.clone())
                    };
                    let view = review_view_from_state(&date);
                    let n = view.event_count;
                    {
                        let mut s = app.lock().unwrap();
                        s.review_data = Some(view);
                        s.review_view = true;
                    }
                    push(crate::i18n::tr_owned(
                        format!("  ◆ review pane: {} ({} event(s), reloaded)", date, n),
                        format!("  ◆ 回顧面板：{}（{} 筆事件，已重新整理）", date, n),
                    ));
                }
                "off" | "hide" => {
                    app.lock().unwrap().review_view = false;
                    push(crate::i18n::tr("  ◆ review pane off", "  ◆ 回顧面板已關閉").into());
                }
                "help" | "?" => {
                    push(crate::i18n::tr(
                        "  /review [<date>|reload|off]  — your Life Node daily review (default today; date = YYYY-MM-DD)",
                        "  /review [<date>|reload|off]  — 你的 Life Node 每日回顧（預設今天；日期 = YYYY-MM-DD）",
                    ).into());
                }
                other => {
                    if chrono::NaiveDate::parse_from_str(other, "%Y-%m-%d").is_ok() {
                        let date = other.to_string();
                        let view = review_view_from_state(&date);
                        let n = view.event_count;
                        {
                            let mut s = app.lock().unwrap();
                            s.review_data = Some(view);
                            s.review_view = true;
                        }
                        push(crate::i18n::tr_owned(
                            format!("  ◆ review pane: {} ({} event(s))", date, n),
                            format!("  ◆ 回顧面板：{}（{} 筆事件）", date, n),
                        ));
                    } else {
                        push_err(crate::i18n::tr_owned(
                            format!("  unknown /review action: {} (try <YYYY-MM-DD>|reload|off)", other),
                            format!("  未知的 /review 動作：{}（可用 <YYYY-MM-DD>|reload|off）", other),
                        ));
                    }
                }
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
                            TranscriptItem::User(t) => buf.push_str(&format!("◆ {}\n", t)),
                            TranscriptItem::Assistant(t) | TranscriptItem::AssistantPartial(t) => {
                                buf.push_str(&format!("● {}\n", t))
                            }
                            TranscriptItem::ToolCall { name, args } => {
                                buf.push_str(&format!("● {}({})\n", name, args))
                            }
                            TranscriptItem::ToolResult { name, output } => {
                                buf.push_str(&format!("  ✓ {}: {}\n", name, output))
                            }
                            TranscriptItem::System(t) => buf.push_str(&format!("{}\n", t)),
                            TranscriptItem::Error(t) => buf.push_str(&format!("✗ {}\n", t)),
                            TranscriptItem::Warning(t) => buf.push_str(&format!("⚠ {}\n", t)),
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
                    if let Some(u) = last_user {
                        s.push_str(&format!("**You:** {}\n\n", u.content.trim()));
                    }
                    if let Some(a) = last_asst {
                        s.push_str(&format!("**Assistant:** {}\n", a.content.trim()));
                    }
                    s
                }
                _ => history
                    .iter()
                    .rev()
                    .find(|m| m.role == "assistant")
                    .map(|m| m.content.clone())
                    .unwrap_or_default(),
            };
            if payload.is_empty() {
                push("  ◆ nothing to copy yet.".into());
            } else {
                let cmd = if cfg!(target_os = "macos") {
                    "pbcopy"
                } else if cfg!(target_os = "linux") {
                    "xclip"
                } else {
                    "clip"
                };
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
                            "all" => "entire session",
                            "turn" => "last turn",
                            "transcript" | "screen" | "tui" => "TUI transcript",
                            _ => "last assistant message",
                        };
                        push(format!(
                            "  ✓ copied {} ({} chars) via {}",
                            label,
                            payload.len(),
                            cmd
                        ));
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
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        let dir = dirs::home_dir()
                            .unwrap_or_else(|| std::path::PathBuf::from("."))
                            .join(".phantom-mesh/exports");
                        std::fs::create_dir_all(&dir).ok();
                        let safe_id: String = chat_id
                            .chars()
                            .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
                            .collect();
                        dir.join(format!("{}-{}.md", safe_id, ts))
                    }
                };
                match std::fs::write(&path, &md) {
                    Ok(()) => push(format!(
                        "  ✓ exported {} chars → {}\n  open it: open \"{}\"",
                        md.len(),
                        path.display(),
                        path.display()
                    )),
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
                push(format!(
                    "  ◆ nothing to compact ({} messages). Use /clear to start fresh.",
                    history.len()
                ));
            } else {
                push(format!(
                    "  ◆ summarizing {} messages via {}…",
                    history.len(),
                    agent_name_str
                ));
                match crate::session::compact_via_llm(
                    runtime,
                    &agent_name_str,
                    cost_tracker,
                    conversations,
                    &chat_id,
                    &history,
                    6,
                )
                .await
                {
                    Ok((dropped, summary_chars)) => {
                        push(format!(
                            "  ✓ compacted {} old messages → 1 summary ({} chars), kept last 6.",
                            dropped, summary_chars
                        ));
                    }
                    Err(e) => push_err(format!("  ✗ compact failed: {}", e)),
                }
            }
        }

        // ── /whoami ───────────────────────────────────────────────────
        "/whoami" => match crate::auth::load() {
            Some(s) => {
                let provider = s.provider.clone();
                let email = s.email.clone();
                push(format!("  ◆ logged in as {} via {}", email, provider));
            }
            None => push("  ◆ not logged in. /login in REPL mode.".into()),
        },

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
        "/login" | "/logout" | "/add" | "/undo" => push_err(format!(
            "  {} is REPL only (interactive prompt). Run `phantom --repl`.",
            cmd
        )),

        other => push_err(format!(
            "  unknown command: {}.  type /help for the list",
            other
        )),
    }
}

// ── Cluster status pane (P1) — prototype ─────────────────────────────────
//
// Design: docs/superpowers/design/tui-cluster-pane.md (BIG-GOAL P1 跨裝置 Mesh).
// Renders the mesh's peers as a table inside the TUI ("see your mesh from the
// terminal"). This is the prototype stage: a pure render fn over a view-model
// (`ClusterRow`) so it is decoupled from `mesh::PeerInfo` internals and unit
// testable via TestBackend. Wiring (Ctrl-P toggle + ClusterManager → rows +
// refresh loop) is the follow-on implementation step.

/// View-model for one row of the cluster pane. Built from `mesh::PeerInfo` at
/// wire time (later step); kept separate so the renderer has no dependency on
/// mesh internals (`Instant`, health enum, …).
#[derive(Debug, Clone)]
pub struct ClusterRow {
    pub name: String,
    pub is_local: bool,
    /// Whether this row reflects a live ping. `false` = built from static
    /// peers.json topology (name/caps known, live status unknown) → rendered
    /// as "configured" rather than asserting online/offline.
    pub pinged: bool,
    pub online: bool,
    /// online but with recent consecutive failures (PeerHealth::Unhealthy).
    pub degraded: bool,
    pub version: String,
    pub tasks: u32,
    /// pre-formatted relative time, e.g. "3s ago" / "now" / "4m ago".
    pub last_seen: String,
    pub caps: Vec<String>,
}

/// Status glyph + colour + screen-reader/`NO_COLOR` text for a row.
/// Glyph and text are paired so the pane reads correctly without colour.
fn cluster_status(row: &ClusterRow) -> (&'static str, &'static str, Color) {
    if !row.pinged {
        ("·", "configured", Color::DarkGray)
    } else if !row.online {
        ("○", "offline", Color::DarkGray)
    } else if row.degraded {
        ("◐", "degraded", Color::Yellow)
    } else {
        ("●", "online", Color::Green)
    }
}

/// Render the cluster pane into `area`. `selected` is the highlighted row index
/// (clamped). Renders the empty-state hint when `rows` is empty.
pub fn render_cluster_pane(f: &mut Frame, area: Rect, rows: &[ClusterRow], selected: usize) {
    f.render_widget(Clear, area);
    let any_pinged = rows.iter().any(|r| r.pinged);
    let online = rows.iter().filter(|r| r.online).count();
    let title = if rows.is_empty() {
        " phantom · cluster — 0 peers ".to_string()
    } else if any_pinged {
        format!(" phantom · cluster — {} peers · {} online ", rows.len(), online)
    } else {
        format!(" phantom · cluster — {} peers · configured (press r to ping) ", rows.len())
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(
            title,
            Style::default()
                .fg(Color::Rgb(214, 178, 112))
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::with_capacity(rows.len() + 4);

    if rows.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(crate::i18n::tr("  No peers configured.", "  尚未設定任何 peer。")));
        lines.push(Line::from(Span::styled(
            crate::i18n::tr(
                "  Add one with `phantom cluster` or edit [cluster] in agents.toml,",
                "  用 `phantom cluster` 新增，或編輯 agents.toml 的 [cluster]，",
            ),
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(Span::styled(
            crate::i18n::tr("  then press r to refresh.", "  然後按 r 重新整理。"),
            Style::default().fg(Color::DarkGray),
        )));
        let p = Paragraph::new(lines).wrap(Wrap { trim: false });
        f.render_widget(p, inner);
        return;
    }

    // Header row.
    lines.push(Line::from(Span::styled(
        format!("  {:<18}{:<13}{:<11}{:>5}  {:<11}{}", "NODE", "STATUS", "VER", "TASKS", "LAST SEEN", "CAPS"),
        Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD),
    )));

    let sel = if rows.is_empty() { 0 } else { selected.min(rows.len() - 1) };
    for (i, row) in rows.iter().enumerate() {
        let (glyph, status_txt, color) = cluster_status(row);
        let marker = if row.is_local { "★" } else { " " };
        let ver = if row.online { row.version.as_str() } else { "—" };
        let tasks = if row.online { row.tasks.to_string() } else { "—".to_string() };
        let caps = if row.caps.is_empty() { "—".to_string() } else { row.caps.join(" ") };
        let name_col = format!("{marker} {}", pad_to_width(&row.name, 16));
        let body = format!(
            "{glyph} {status_txt:<11}{ver:<11}{tasks:>5}  {:<11}{caps}",
            row.last_seen,
        );
        let row_style = if i == sel {
            Style::default().fg(color).add_modifier(Modifier::REVERSED)
        } else {
            Style::default().fg(color)
        };
        lines.push(Line::from(vec![
            Span::raw(format!("  {name_col}")),
            Span::styled(body, row_style),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        crate::i18n::tr(
            "  ↑/↓ select   enter: peer detail   r: refresh   Ctrl-P: back to chat",
            "  ↑/↓選擇   enter：節點詳情   r：重新整理   Ctrl-P：回聊天",
        ),
        Style::default().fg(Color::DarkGray),
    )));

    let p = Paragraph::new(lines).wrap(Wrap { trim: false });
    f.render_widget(p, inner);
}

/// Truncate `s` to `max` display chars, appending `…` when cut.
fn trunc(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let keep: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{keep}…")
    }
}

/// Truncate to `cells` DISPLAY columns then right-pad with spaces to exactly
/// `cells` columns. CJK/wide-glyph aware — unlike `format!("{:<n}")` which pads
/// by char count and so misaligns table columns when a cell holds wide chars.
fn pad_to_width(s: &str, cells: usize) -> String {
    use unicode_width::UnicodeWidthStr;
    let mut out = truncate_to_width(s, cells);
    let w = UnicodeWidthStr::width(out.as_str());
    for _ in w..cells {
        out.push(' ');
    }
    out
}

/// Build cluster-pane rows from static `peers.json` topology — local node first
/// (★), then configured peers. All rows are `pinged: false` (live status
/// unknown until a ping refresh lands). Caps come from the peer's config.
fn cluster_rows_from_config() -> Vec<ClusterRow> {
    let me = crate::cli_config::resolve_self_node_name();
    let mut rows = Vec::new();
    if let Some(name) = me.clone() {
        rows.push(ClusterRow {
            name,
            is_local: true,
            pinged: false,
            online: false,
            degraded: false,
            version: "—".into(),
            tasks: 0,
            last_seen: "—".into(),
            caps: Vec::new(),
        });
    }
    if let Some(peers) = crate::cli_config::read_peers_json() {
        for p in peers {
            if Some(p.name.as_str()) == me.as_deref() {
                continue;
            }
            rows.push(ClusterRow {
                name: p.name,
                is_local: false,
                pinged: false,
                online: false,
                degraded: false,
                version: "—".into(),
                tasks: 0,
                last_seen: "—".into(),
                caps: p.capabilities,
            });
        }
    }
    rows
}

/// Format a unix timestamp as a coarse relative time for the LAST SEEN column.
fn relative_unix(then: u64) -> String {
    if then == 0 {
        return "—".into();
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let d = now.saturating_sub(then);
    if d < 2 {
        "now".into()
    } else if d < 60 {
        format!("{d}s ago")
    } else if d < 3600 {
        format!("{}m ago", d / 60)
    } else {
        format!("{}h ago", d / 3600)
    }
}

/// Build cluster-pane rows from a live `refresh_all()` result (`pinged: true`).
/// Local node first (★, assumed online), then each pinged peer.
fn cluster_rows_from_statuses(
    statuses: &[crate::mesh::PeerStatus],
    self_name: Option<&str>,
) -> Vec<ClusterRow> {
    let mut rows = Vec::new();
    if let Some(name) = self_name {
        rows.push(ClusterRow {
            name: name.to_string(),
            is_local: true,
            pinged: true,
            online: true,
            degraded: false,
            version: env!("CARGO_PKG_VERSION").to_string(),
            tasks: 0,
            last_seen: "now".into(),
            caps: Vec::new(),
        });
    }
    for st in statuses {
        if Some(st.name.as_str()) == self_name {
            continue;
        }
        rows.push(ClusterRow {
            name: st.name.clone(),
            is_local: false,
            pinged: true,
            online: st.online,
            degraded: false,
            version: st.version.clone(),
            tasks: st.active_tasks,
            last_seen: relative_unix(st.last_seen),
            caps: st.capabilities.clone(),
        });
    }
    rows
}

// ── Focus session pane (P2 Life Track) — prototype render ────────────────
//
// Design: docs/superpowers/design/terminal-focus.md. Renders the active focus
// timer ("see your focus session from the terminal"). Pure render over a
// view-model (`FocusView`), decoupled from capture_focus_wire (whose backend is
// a Stage-4 stub) so it is unit-testable now. `/focus` toggle + live wiring
// land once the backend ships.

/// View-model for the focus timer pane. Built from the active session later;
/// `None` (passed to the renderer) means no session running.
#[derive(Debug, Clone)]
pub struct FocusView {
    pub task: String,
    pub planned_min: u32,
    pub remaining_secs: u32,
    pub interruptions: u32,
    /// pre-formatted clock, e.g. "14:07".
    pub started_at: String,
    pub recording: bool,
}

/// Format whole seconds as `MM:SS` (minutes uncapped, e.g. "104:09").
fn fmt_mmss(secs: u32) -> String {
    format!("{:02}:{:02}", secs / 60, secs % 60)
}

/// Render the focus pane. `view = None` → no-session hint; `Some` → live timer.
pub fn render_focus_pane(f: &mut Frame, area: Rect, view: Option<&FocusView>) {
    f.render_widget(Clear, area);
    let title = match view {
        Some(v) => format!(" phantom · focus — {} min timer ", v.planned_min),
        None => " phantom · focus ".to_string(),
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(
            title,
            Style::default()
                .fg(Color::Rgb(214, 178, 112))
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    match view {
        None => {
            lines.push(Line::from(""));
            lines.push(Line::from(crate::i18n::tr("  No active focus session.", "  目前沒有進行中的 focus 時段。")));
            lines.push(Line::from(Span::styled(
                crate::i18n::tr(
                    "  Start one with `phantom focus start --minutes 25`.",
                    "  用 `phantom focus start --minutes 25` 開始一個。",
                ),
                Style::default().fg(Color::DarkGray),
            )));
        }
        Some(v) => {
            lines.push(Line::from(""));
            let rec = if v.recording { "  ● REC" } else { "" };
            lines.push(Line::from(Span::styled(
                format!("      ⏱  {}  left{}", fmt_mmss(v.remaining_secs), rec),
                Style::default()
                    .fg(if v.recording { Color::Red } else { Color::Green })
                    .add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(format!("    task: {}", trunc(&v.task, 60))));
            lines.push(Line::from(Span::styled(
                format!("    interruptions: {}     started {}", v.interruptions, v.started_at),
                Style::default().fg(Color::DarkGray),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                crate::i18n::tr(
                    "    i: log interruption · s: stop · r: reload · esc: back to chat",
                    "    i：記錄中斷 · s：停止 · r：重新載入 · esc：回聊天",
                ),
                Style::default().fg(Color::DarkGray),
            )));
        }
    }
    let p = Paragraph::new(lines).wrap(Wrap { trim: false });
    f.render_widget(p, inner);
}

/// `/focus` `i`: log an interruption on the active session + refresh the pane.
/// Mirrors `phantom focus interrupt`, with a default note (no in-pane prompt).
fn log_focus_interruption(s: &mut AppState) {
    let Some(home) = dirs::home_dir() else {
        return;
    };
    match crate::life_node::focus_session::interrupt(&home, "(logged from /focus)") {
        Ok(n) => {
            s.transcript.push(TranscriptItem::System(format!(
                "  ◆ interruption logged ({} total)",
                n
            )));
            s.focus_data = focus_view_from_state();
        }
        Err(e) => s
            .transcript
            .push(TranscriptItem::Error(format!("  ✗ interrupt: {}", e))),
    }
}

/// `/focus` `s`: stop the active session (writes a Life Node event) + clear the
/// pane to the empty state. Mirrors `phantom focus stop`.
fn stop_focus_session(s: &mut AppState) {
    let Some(home) = dirs::home_dir() else {
        return;
    };
    match crate::life_node::focus_session::stop(&home) {
        Ok(r) => {
            s.transcript.push(TranscriptItem::System(format!(
                "  ■ focus complete — {} min planned · {:.0}% done · {} interruption(s)",
                r.planned_duration_ms / 60_000,
                r.completion_pct,
                r.interruptions
            )));
            s.focus_data = focus_view_from_state(); // None now → empty state
        }
        Err(e) => s
            .transcript
            .push(TranscriptItem::Error(format!("  ✗ stop: {}", e))),
    }
}

/// Build the focus pane view from the on-disk active session (read-only,
/// mirrors `phantom focus status`). `None` → no active session (empty state).
/// The countdown is a snapshot at toggle/reload time (not a live tick).
fn focus_view_from_state() -> Option<FocusView> {
    let home = dirs::home_dir()?;
    let s = crate::life_node::focus_session::status(&home)?;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let end_ms = s.started_at_ms.saturating_add(s.planned_duration_ms);
    let remaining_secs = (end_ms.saturating_sub(now_ms) / 1000) as u32;
    let started_at = {
        let t = std::time::UNIX_EPOCH + std::time::Duration::from_millis(s.started_at_ms);
        let dt: chrono::DateTime<chrono::Local> = t.into();
        dt.format("%H:%M").to_string()
    };
    Some(FocusView {
        task: s.task.unwrap_or_default(),
        planned_min: (s.planned_duration_ms / 60_000) as u32,
        remaining_secs,
        interruptions: s.interruptions.len() as u32,
        started_at,
        // DiskFocusSession doesn't persist a recording flag (timer-only).
        recording: false,
    })
}

// ── Evolve Goals pane (P3 Evolve Mesh) — prototype render ────────────────
//
// Design: docs/superpowers/design/tui-evolve-goals.md. Renders the
// EVOLVE-GOALS.md milestone queue. Pure render over a view-model (`GoalRow`),
// decoupled from `evolve_goals::GoalsFile` so it is unit-testable. Unlike the
// focus pane this feature's backend is REAL, so live wiring (`/goals` toggle +
// GoalsFile load + `space` mark-done) is fully feasible as a follow-on.

/// View-model for one goal row. Built from `evolve_goals::GoalLine` later.
#[derive(Debug, Clone)]
pub struct GoalRow {
    /// short source-line label, e.g. "L13".
    pub label: String,
    pub text: String,
    pub done: bool,
}

/// Render the evolve-goals pane. Renders `rows` in the order given (the caller
/// — `goal_rows_from_file`, not yet written — sorts pending-first then done to
/// match the CLI). `selected` is clamped; empty → hint.
pub fn render_goals_pane(f: &mut Frame, area: Rect, rows: &[GoalRow], selected: usize) {
    f.render_widget(Clear, area);
    let pending = rows.iter().filter(|r| !r.done).count();
    let done = rows.len() - pending;
    let title = if rows.is_empty() {
        " phantom · evolve goals — 0 goals ".to_string()
    } else {
        format!(" phantom · evolve goals — {pending} pending · {done} done ")
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(
            title,
            Style::default()
                .fg(Color::Rgb(214, 178, 112))
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    if rows.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(crate::i18n::tr("  No goals yet.", "  尚無目標。")));
        lines.push(Line::from(Span::styled(
            crate::i18n::tr(
                "  Add one with `phantom evolve goals add \"ship X\"`, then press r to reload.",
                "  用 `phantom evolve goals add \"ship X\"` 新增，然後按 r 重新載入。",
            ),
            Style::default().fg(Color::DarkGray),
        )));
        let p = Paragraph::new(lines).wrap(Wrap { trim: false });
        f.render_widget(p, inner);
        return;
    }

    let sel = selected.min(rows.len() - 1);
    // text column = inner width minus the fixed prefix, which is exactly 10
    // display cells: "  " (2) + glyph (1) + " " (1) + 4-cell label + "  " (2).
    // Subtracting 10 (not 9) keeps the longest line within inner.width so it
    // never overflows + wraps under `Wrap { trim:false }`.
    let text_cells = (inner.width as usize).saturating_sub(10).max(8);
    for (i, row) in rows.iter().enumerate() {
        let (glyph, color) = if row.done {
            ("✓", Color::Green)
        } else {
            ("○", Color::DarkGray)
        };
        let body = format!(
            "{glyph} {}  {}",
            pad_to_width(&row.label, 4),
            truncate_to_width(&row.text, text_cells),
        );
        let style = if i == sel {
            Style::default().fg(color).add_modifier(Modifier::REVERSED)
        } else if row.done {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default().fg(color)
        };
        lines.push(Line::from(vec![Span::raw("  "), Span::styled(body, style)]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        crate::i18n::tr(
            "  ↑/↓ select · space: mark done · r: reload · esc: back to chat",
            "  ↑/↓ 選擇 · space：標記完成 · r：重新載入 · esc：回聊天",
        ),
        Style::default().fg(Color::DarkGray),
    )));
    let p = Paragraph::new(lines).wrap(Wrap { trim: false });
    f.render_widget(p, inner);
}

/// Build goal rows from `EVOLVE-GOALS.md` (or `$PHANTOM_EVOLVE_GOALS`), matching
/// the `phantom evolve goals list` view: checkbox lines in file order (Pending
/// section then Done), label `L{idx+1}` (1-based, like the CLI). Read-only.
fn goal_rows_from_file() -> Vec<GoalRow> {
    // D26: shared resolver ($PHANTOM_EVOLVE_GOALS > existing ./EVOLVE-GOALS.md >
    // ~/.phantom-mesh) so the TUI and CLI always read the same file.
    let path = crate::evolve_goals::resolve_goals_path(None);
    match crate::evolve_goals::GoalsFile::load(&path) {
        Ok(g) => g
            .lines
            .iter()
            .filter_map(|l| {
                l.checkbox.as_ref().map(|cb| GoalRow {
                    label: format!("L{}", l.idx + 1),
                    text: cb.text.clone(),
                    done: cb.checked,
                })
            })
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Mark the currently-selected goals-pane row done (space in `/goals`): parse
/// its `L{idx+1}` label → 0-based file idx, run `GoalsFile::mark_done` + `save`,
/// then reload the rows. No-op (with a transcript note) on a done row or a parse
/// failure. Mirrors `phantom evolve goals mark-done`, but from inside the pane.
fn mark_selected_goal_done(s: &mut AppState) {
    let Some(row) = s.goals_rows.get(s.goals_selected) else {
        return;
    };
    if row.done {
        s.transcript
            .push(TranscriptItem::System("  ◇ already done".into()));
        return;
    }
    let Some(idx) = row
        .label
        .strip_prefix('L')
        .and_then(|n| n.parse::<usize>().ok())
        .map(|n| n.saturating_sub(1))
    else {
        return;
    };
    let path = crate::evolve_goals::resolve_goals_path(None);
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    match crate::evolve_goals::GoalsFile::load(&path) {
        Ok(mut g) => match g.mark_done(idx, &date, "tui") {
            Ok(text) => {
                if let Err(e) = g.save() {
                    s.transcript
                        .push(TranscriptItem::Error(format!("  ✗ save goals: {}", e)));
                    return;
                }
                s.transcript
                    .push(TranscriptItem::System(format!("  ✓ marked done: {}", text)));
                s.goals_rows = goal_rows_from_file();
                let max = s.goals_rows.len().saturating_sub(1);
                s.goals_selected = s.goals_selected.min(max);
            }
            Err(e) => s
                .transcript
                .push(TranscriptItem::Error(format!("  ✗ mark done: {}", e))),
        },
        Err(e) => s
            .transcript
            .push(TranscriptItem::Error(format!("  ✗ load goals: {}", e))),
    }
}

// ── Identity & Vault pane (P4 加密為先) — prototype render ────────────────
//
// Design: docs/superpowers/design/tui-identity-vault.md. Read-only pane: who
// you are + key fingerprint + keystore backend + the BIG-GOAL P4 v0.6.0
// encryption-at-rest scope table. Pure render over a view-model; no secrets
// rendered (fingerprint only). `/identity` toggle + live `auth::load` wiring is
// the follow-on.

/// View-model for the identity pane. `identity_line = None` → not logged in.
#[derive(Debug, Clone)]
pub struct IdentityView {
    /// provider · email · device id (from auth::human_summary), or None.
    pub identity_line: Option<String>,
    pub fingerprint: String,
    pub created_at: String,
    pub keystore: String,
    /// Whether `~/.phantom-mesh/identity.key` exists. When false, Life Node
    /// events are written PLAINTEXT (no key to derive the age key from) — the
    /// pane surfaces this so the P4 scope table isn't read as a false promise.
    pub key_present: bool,
}

/// BIG-GOAL P4 v0.6.0 encryption-at-rest scope — ALL 8 paths, kept verbatim in
/// sync with BIG-GOAL.md §"P4 enforcement scope". `(encrypted, label, note)`.
/// Honesty rail: every plaintext path is listed; never imply more than shipped.
const P4_SCOPE: &[(bool, &str, &str)] = &[
    (true, "Life Node events", "~/.phantom-mesh/events/   age v1"),
    (true, "identity key", "~/.phantom-mesh/identity.key"),
    (false, "agents.toml", "plaintext → v0.7.0+"),
    (false, "conversations/", "plaintext → v0.7.0+"),
    (false, "memory.db", "plaintext → v0.7.0+"),
    (false, "auth / broker tokens", "plaintext → v0.7.0+"),
    (false, "telemetry (costs/log/crashes)", "plaintext → v0.7.0+"),
    (false, "captures/*.png", "plaintext → v0.7.0+"),
];

/// Render the identity & vault pane.
pub fn render_identity_pane(f: &mut Frame, area: Rect, view: &IdentityView) {
    f.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(
            " phantom · identity & vault — 加密為先 (P4) ",
            Style::default()
                .fg(Color::Rgb(214, 178, 112))
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let dim = Style::default().fg(Color::DarkGray);
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(crate::i18n::tr("  IDENTITY", "  身分"), dim)));
    match &view.identity_line {
        Some(s) => {
            lines.push(Line::from(format!("    {}", s)));
            lines.push(Line::from(format!(
                "    fingerprint {}   (created {})",
                view.fingerprint, view.created_at
            )));
            lines.push(Line::from(format!("    keystore    {}", view.keystore)));
        }
        None => {
            lines.push(Line::from(Span::styled(
                crate::i18n::tr(
                    "    not logged in — run `phantom login`",
                    "    尚未登入 — 執行 `phantom login`",
                ),
                dim,
            )));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        crate::i18n::tr("  ENCRYPTED AT REST (v0.6.0)", "  靜態加密狀態 (v0.6.0)"),
        dim,
    )));
    for (enc, label, note) in P4_SCOPE {
        let (glyph, color) = if *enc {
            ("✓", Color::Green)
        } else {
            ("○", Color::DarkGray)
        };
        lines.push(Line::from(vec![
            Span::raw("    "),
            Span::styled(format!("{glyph} "), Style::default().fg(color)),
            Span::raw(format!("{}  ", pad_to_width(label, 24))),
            Span::styled((*note).to_string(), dim),
        ]));
    }
    // Honesty rail: the scope table above shows the DESIGN (events → age v1),
    // but without an identity.key there's no key to encrypt with, so events are
    // actually plaintext. Say so plainly rather than imply more than delivered.
    if !view.key_present {
        lines.push(Line::from(Span::styled(
            crate::i18n::tr(
                "    ⚠ no identity.key — events are written PLAINTEXT, not encrypted yet",
                "    ⚠ 沒有 identity.key — 事件目前以明文寫入，尚未加密",
            ),
            Style::default().fg(Color::Yellow),
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        crate::i18n::tr(
            "  /identity reload · esc: back to chat",
            "  /identity reload 重新整理 · esc：回聊天",
        ),
        dim,
    )));
    let p = Paragraph::new(lines).wrap(Wrap { trim: false });
    f.render_widget(p, inner);
}

/// Build the identity view from on-disk state (all read-only, no secrets):
/// identity summary (auth::load), key fingerprint (load_pub_hex →
/// fingerprint_short), keystore backend (platform default), identity.key mtime.
fn identity_view_from_state() -> IdentityView {
    let identity_line = crate::auth::load().map(|s| crate::auth::human_summary(&s));
    let fingerprint = crate::identity::load_pub_hex()
        .ok()
        .and_then(|h| hex::decode(h.trim()).ok())
        .map(|bytes| crate::identity_wire::fingerprint_short(&bytes))
        .unwrap_or_else(|| "—".into());
    // "created" = mtime of the per-device root key `~/.phantom-mesh/identity.key`
    // (matches IdentityPublic.created_at semantics) — NOT keys/ed25519.priv.
    let created_at = dirs::home_dir()
        .map(|h| h.join(".phantom-mesh").join("identity.key"))
        .and_then(|p| std::fs::metadata(p).ok())
        .and_then(|m| m.modified().ok())
        .map(|t| {
            let dt: chrono::DateTime<chrono::Local> = t.into();
            dt.format("%Y-%m-%d").to_string()
        })
        .unwrap_or_else(|| "—".into());
    // SPEC-12 backend for this platform (native arms beyond Linux are Stage-4
    // stubs; this names the intended backend, not a live probe).
    let keystore = if cfg!(target_os = "linux") {
        "linux-secret-service (or file-chmod-0600 fallback)"
    } else if cfg!(target_os = "macos") {
        "macos-keychain"
    } else if cfg!(target_os = "windows") {
        "windows-dpapi"
    } else {
        "file-chmod-0600"
    }
    .to_string();
    // Use load_event_key (not just .exists()) so a present-but-corrupt/empty
    // key — which would still leave events plaintext — correctly shows the
    // warning. This is the exact gate Life Node capture uses to pick its store.
    let key_present = dirs::home_dir()
        .map(|h| {
            crate::life_node::key_derivation::load_event_key(
                &h.join(".phantom-mesh").join("identity.key"),
            )
            .is_ok()
        })
        .unwrap_or(false);
    IdentityView {
        identity_line,
        fingerprint,
        created_at,
        keystore,
        key_present,
    }
}

// ── Daily Review pane (P2 多模態理解 / Life Track) — prototype render ──────
//
// Design: docs/superpowers/design/tui-daily-review.md. Read-only pane: a date's
// Life Node daily review — events grouped by goal-tag — reusing
// life_node::daily_review (load_events_for_date + aggregate). Offline-first; the
// network "tomorrow's action" pass stays on the CLI. `/review` toggle + live
// wiring is the follow-on. No secrets rendered (decrypted summaries only).

/// One rendered row, parsed from the `daily_review::aggregate` Markdown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewRow {
    /// A goal-tag group header, e.g. `## fat_loss (2)`.
    Group { tag: String, n: usize },
    /// A captured-event bullet, e.g. `- **food** (08:14): summary`.
    Bullet {
        kind: String,
        time: String,
        summary: String,
    },
}

/// The three pane states (design §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewState {
    /// Events present for the date.
    Events,
    /// Events dir readable, 0 events for the date (shame-free: not a failure).
    Empty,
    /// No identity key → the age-encrypted events can't be decrypted.
    Locked,
}

/// View-model for the daily-review pane.
#[derive(Debug, Clone)]
pub struct ReviewView {
    pub date: String,
    pub state: ReviewState,
    pub event_count: usize,
    pub rows: Vec<ReviewRow>,
    /// `coach_prompts::lint::check` failed on the aggregate → show a neutral
    /// banner instead of assuming shame-free (design §1 shame-free rail).
    pub flagged: bool,
}

/// `"2026-05-28T08:14:23+08:00"` → `"08:14"`. Falls back to the raw string when
/// there is no ISO `T`-separated time (so a non-ISO timestamp still renders).
fn short_time(ts: &str) -> String {
    ts.split('T')
        .nth(1)
        .map(|t| t.chars().take(5).collect::<String>())
        .filter(|s| s.len() == 5 && s.contains(':'))
        .unwrap_or_else(|| ts.to_string())
}

/// Parse `"fat_loss (2)"` → `("fat_loss", 2)`. Tolerates trailing spaces.
fn parse_group_header(s: &str) -> Option<(String, usize)> {
    let open = s.rfind('(')?;
    let close = s.rfind(')')?;
    if close < open {
        return None;
    }
    let tag = s[..open].trim().to_string();
    let n = s[open + 1..close].trim().parse::<usize>().ok()?;
    if tag.is_empty() {
        return None;
    }
    Some((tag, n))
}

/// Parse a bullet body (already past the `- **` prefix), shape
/// `"{kind}** ({timestamp}): {summary}"`. The timestamp never contains `"): "`,
/// so the first occurrence is the true delimiter even if the summary does.
fn parse_bullet(body: &str) -> Option<ReviewRow> {
    let (kind, rest) = body.split_once("** (")?;
    let (ts, summary) = rest.split_once("): ")?;
    let kind = kind.trim();
    if kind.is_empty() {
        return None;
    }
    Some(ReviewRow::Bullet {
        kind: kind.to_string(),
        time: short_time(ts.trim()),
        summary: summary.trim().to_string(),
    })
}

/// Parse the `daily_review::aggregate` Markdown into structured rows. Couples
/// only to aggregate's own output (the single source of truth) — it does NOT
/// re-derive from raw events or re-sort. Unknown / prose lines are skipped.
pub fn parse_review_rows(md: &str) -> Vec<ReviewRow> {
    let mut rows = Vec::new();
    for line in md.lines() {
        let line = line.trim_end();
        if let Some(rest) = line.strip_prefix("## ") {
            if let Some((tag, n)) = parse_group_header(rest) {
                rows.push(ReviewRow::Group { tag, n });
            }
        } else if let Some(rest) = line.strip_prefix("- **") {
            if let Some(b) = parse_bullet(rest) {
                rows.push(b);
            }
        }
    }
    rows
}

/// Render the daily-review pane. Pure over the view-model.
pub fn render_daily_review_pane(f: &mut Frame, area: Rect, view: &ReviewView) {
    f.render_widget(Clear, area);
    let title = match view.state {
        ReviewState::Locked => format!(
            " phantom · daily review — {} · {} ",
            view.date,
            crate::i18n::tr("encrypted", "已加密")
        ),
        _ => format!(
            " phantom · daily review — {} · {} ",
            view.date,
            crate::i18n::tr_owned(
                format!("{} events", view.event_count),
                format!("{} 筆事件", view.event_count),
            )
        ),
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(
            title,
            Style::default()
                .fg(Color::Rgb(214, 178, 112))
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let dim = Style::default().fg(Color::DarkGray);
    let accent = Style::default()
        .fg(Color::Rgb(214, 178, 112))
        .add_modifier(Modifier::BOLD);
    let mut lines: Vec<Line> = Vec::new();

    match view.state {
        ReviewState::Locked => {
            lines.push(Line::from(""));
            lines.push(Line::from(crate::i18n::tr(
                "  Events are encrypted at rest (age v1).",
                "  事件以靜態加密儲存（age v1）。",
            )));
            lines.push(Line::from(Span::styled(
                crate::i18n::tr(
                    "  Identity key not loaded — run `phantom onboarding`, then press r.",
                    "  尚未載入身分金鑰 — 執行 `phantom init`，然後按 r。",
                ),
                dim,
            )));
        }
        ReviewState::Empty => {
            lines.push(Line::from(""));
            lines.push(Line::from(crate::i18n::tr_owned(
                format!("  No Life Node events for {}.", view.date),
                format!("  {} 沒有任何生活節點事件。", view.date),
            )));
            lines.push(Line::from(Span::styled(
                crate::i18n::tr(
                    "  Capture one with `phantom` (Life Node), then press r.",
                    "  用 `phantom`（生活節點）擷取一筆，然後按 r。",
                ),
                dim,
            )));
            lines.push(Line::from(Span::styled(
                crate::i18n::tr(
                    "  (An empty day is fine — this is a log, not a scorecard.)",
                    "  （空白的一天沒關係 — 這是記錄，不是計分板。）",
                ),
                dim,
            )));
        }
        ReviewState::Events => {
            if view.flagged {
                lines.push(Line::from(Span::styled(
                    crate::i18n::tr(
                        "  ⚠ some entries were flagged — showing raw log",
                        "  ⚠ 部分項目被標記 — 顯示原始記錄",
                    ),
                    Style::default().fg(Color::Yellow),
                )));
            }
            // prefix before summary = "    " (4) + "· " (2) + kind(6) + " " (1)
            // + time(5) + "  " (2) = 20 display cells.
            let text_cells = (inner.width as usize).saturating_sub(20).max(8);
            for row in &view.rows {
                match row {
                    ReviewRow::Group { tag, n } => {
                        lines.push(Line::from(""));
                        lines.push(Line::from(Span::styled(
                            format!("  {} ({})", tag, n),
                            accent,
                        )));
                    }
                    ReviewRow::Bullet {
                        kind,
                        time,
                        summary,
                    } => {
                        let body = format!(
                            "· {} {}  {}",
                            pad_to_width(kind, 6),
                            pad_to_width(time, 5),
                            truncate_to_width(summary, text_cells),
                        );
                        lines.push(Line::from(vec![Span::raw("    "), Span::raw(body)]));
                    }
                }
            }
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        crate::i18n::tr(
            "  ←/→ day · r: reload · /review <date> · esc: back to chat",
            "  ←/→ 換日 · r：重新載入 · /review <date> · esc：回聊天",
        ),
        dim,
    )));
    let p = Paragraph::new(lines).wrap(Wrap { trim: false });
    f.render_widget(p, inner);
}

/// Build the daily-review view for `date` from on-disk Life Node events
/// Shift a `YYYY-MM-DD` date by `days` (±, for `/review` ←/→ nav). Falls back to
/// the input on parse error. Pure + testable.
fn shift_date(date: &str, days: i64) -> String {
    chrono::NaiveDate::parse_from_str(date.trim(), "%Y-%m-%d")
        .ok()
        .and_then(|d| {
            if days >= 0 {
                d.checked_add_days(chrono::Days::new(days as u64))
            } else {
                // `unsigned_abs` avoids the `(-days)` overflow at i64::MIN.
                d.checked_sub_days(chrono::Days::new(days.unsigned_abs()))
            }
        })
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| date.to_string())
}

/// (read-only, offline). Resolves the 3 states (design §1):
/// - events present for `date` → Events (works for plaintext OR key-decrypted)
/// - empty + an event is genuinely age-encrypted but we have no key → Locked
/// - else → Empty.
fn review_view_from_state(date: &str) -> ReviewView {
    let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    review_view_for(&home.join(".phantom-mesh"), date)
}

/// True if any event under `events_dir` has an age-encrypted `meta.json` — i.e.
/// it was written WITH a key and can't be read without one. Lets us tell a
/// genuine Locked state (encrypted, no key) apart from a plaintext Empty.
fn events_dir_has_encrypted(events_dir: &std::path::Path) -> bool {
    let Ok(rd) = std::fs::read_dir(events_dir) else {
        return false;
    };
    for entry in rd.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        if let Ok(bytes) = std::fs::read(entry.path().join("meta.json")) {
            if crate::life_node::crypto::looks_like_age(&bytes) {
                return true;
            }
        }
    }
    false
}

/// Core of `review_view_from_state`, parameterized on the phantom dir for tests.
fn review_view_for(phantom_dir: &std::path::Path, date: &str) -> ReviewView {
    let events_dir = phantom_dir.join("events");
    let event_key =
        crate::life_node::key_derivation::load_event_key(&phantom_dir.join("identity.key")).ok();
    let has_key = event_key.is_some();

    // Always try to load: EventStore reads PLAINTEXT events even with no key,
    // and decrypts age-encrypted ones when a key is present. (This used to
    // short-circuit to Locked whenever there was no key — which wrongly hid
    // plaintext events written when no identity.key existed.)
    let pairs = crate::life_node::daily_review::load_events_for_date(&events_dir, date, event_key)
        .unwrap_or_default();
    if !pairs.is_empty() {
        let event_count = pairs.len();
        let md = crate::life_node::daily_review::aggregate(date, &pairs);
        let flagged = crate::life_node::coach_prompts::lint::check(&md).is_err();
        return ReviewView {
            date: date.to_string(),
            state: ReviewState::Events,
            event_count,
            rows: parse_review_rows(&md),
            flagged,
        };
    }
    // Empty load → Locked only if events exist that we genuinely can't decrypt.
    let state = if !has_key && events_dir_has_encrypted(&events_dir) {
        ReviewState::Locked
    } else {
        ReviewState::Empty
    };
    ReviewView {
        date: date.to_string(),
        state,
        event_count: 0,
        rows: Vec::new(),
        flagged: false,
    }
}

// ── Cost pane (/cost, Work Track cost visibility) — prototype render ──────
//
// Design: docs/superpowers/design/tui-cost-pane.md. Read-only pane: session +
// lifetime spend + a per-model breakdown, reusing cost::CostTracker::summary().
// Pure render over a view-model; the `/cost` toggle wiring is the follow-on.

/// One per-model row. NOTE: `ModelCost` tracks only tokens + cost — there is NO
/// per-model request count, so the pane never shows one (design §1 caveat).
#[derive(Debug, Clone, PartialEq)]
pub struct CostModelRow {
    pub name: String,
    pub in_tokens: u64,
    pub out_tokens: u64,
    pub cost_usd: f64,
}

/// View-model for the cost pane.
#[derive(Debug, Clone)]
pub struct CostView {
    pub session_usd: f64,
    pub total_usd: f64,
    pub requests: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// 0.0 → no budget set (the budget row is hidden).
    pub budget_limit_usd: f64,
    pub over_budget: bool,
    /// Per-model rows, sorted by cost descending (biggest spend first).
    pub models: Vec<CostModelRow>,
}

/// Humanize a token count: 950 → "950", 12_300 → "12.3k", 3_400_000 → "3.4M".
fn fmt_tokens(n: u64) -> String {
    let f = n as f64;
    // Promote to M at 999_950 (not 1_000_000) so the k-form never rounds to the
    // ugly "1000.0k" seam in [999_950, 999_999].
    if f >= 999_950.0 {
        format!("{:.1}M", f / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", f / 1_000.0)
    } else {
        n.to_string()
    }
}

/// Build a `CostView` from `CostTracker::summary()` JSON. Pure + testable: the
/// caller passes the already-awaited summary; models are sorted by cost desc
/// (tie-broken by name for determinism). Missing fields default to 0/empty.
pub fn cost_view_from_summary(summary: &serde_json::Value) -> CostView {
    let f = |k: &str| summary.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0);
    let u = |k: &str| summary.get(k).and_then(|v| v.as_u64()).unwrap_or(0);
    let mut models: Vec<CostModelRow> = summary
        .get("by_model")
        .and_then(|v| v.as_object())
        .map(|map| {
            map.iter()
                .map(|(name, mc)| CostModelRow {
                    name: name.clone(),
                    in_tokens: mc.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                    out_tokens: mc.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                    cost_usd: mc.get("cost_usd").and_then(|v| v.as_f64()).unwrap_or(0.0),
                })
                .collect()
        })
        .unwrap_or_default();
    models.sort_by(|a, b| {
        b.cost_usd
            .partial_cmp(&a.cost_usd)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.name.cmp(&b.name))
    });
    CostView {
        session_usd: f("session_usd"),
        total_usd: f("total_usd"),
        requests: u("requests"),
        prompt_tokens: u("prompt_tokens"),
        completion_tokens: u("completion_tokens"),
        budget_limit_usd: f("budget_limit_usd"),
        over_budget: summary
            .get("over_budget")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        models,
    }
}

/// Render the cost pane. Pure over the view-model.
pub fn render_cost_pane(f: &mut Frame, area: Rect, view: &CostView) {
    f.render_widget(Clear, area);
    let fc = crate::cost::CostTracker::format_cost;
    // Title shows SESSION spend only — `requests` is a LIFETIME total, so it
    // must not ride next to "session" in the title (would misread as session).
    let title = format!(" phantom · cost — session {} ", fc(view.session_usd));
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(
            title,
            Style::default()
                .fg(Color::Rgb(214, 178, 112))
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let dim = Style::default().fg(Color::DarkGray);
    let mut lines: Vec<Line> = Vec::new();

    if view.requests == 0 {
        lines.push(Line::from(""));
        lines.push(Line::from(crate::i18n::tr(
            "  No spend yet this session.",
            "  本次工作階段尚無花費。",
        )));
        lines.push(Line::from(Span::styled(
            crate::i18n::tr(
                "  Costs appear here as agents make LLM calls.",
                "  代理人呼叫 LLM 時，花費會顯示在這裡。",
            ),
            dim,
        )));
    } else {
        // session_usd + by_model are SESSION-scoped; total_usd + requests +
        // prompt/completion tokens are LIFETIME totals (all summary() exposes),
        // so they're labelled distinctly — the numbers must not be misread.
        lines.push(Line::from(format!(
            "  {}   {}        {}   {}",
            crate::i18n::tr("SESSION", "本次"),
            fc(view.session_usd),
            crate::i18n::tr("LIFETIME", "累計"),
            fc(view.total_usd),
        )));
        lines.push(Line::from(crate::i18n::tr_owned(
            format!(
                "  lifetime: {} requests · {} in / {} out",
                view.requests,
                fmt_tokens(view.prompt_tokens),
                fmt_tokens(view.completion_tokens),
            ),
            format!(
                "  累計：{} 次請求 · {} 輸入 / {} 輸出",
                view.requests,
                fmt_tokens(view.prompt_tokens),
                fmt_tokens(view.completion_tokens),
            ),
        )));
        if view.budget_limit_usd > 0.0 {
            // The budget is a LIFETIME cap (`set_budget_limit` checks total_usd),
            // so the bar measures total_usd against it — NOT session_usd.
            let frac = (view.total_usd / view.budget_limit_usd).clamp(0.0, 1.0);
            let filled = (frac * 10.0).round() as usize;
            let bar: String = "■".repeat(filled) + &"□".repeat(10usize.saturating_sub(filled));
            let pct = (frac * 100.0).round() as u64;
            let bar_style = if view.over_budget {
                Style::default().fg(Color::Red)
            } else {
                dim
            };
            lines.push(Line::from(vec![
                Span::raw(format!(
                    "  {}    {} limit   ",
                    crate::i18n::tr("budget", "預算"),
                    fc(view.budget_limit_usd)
                )),
                Span::styled(format!("[{}]", bar), bar_style),
                Span::raw(format!("  {}%", pct)),
            ]));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            crate::i18n::tr("  BY MODEL (session)", "  各模型（本次）"),
            dim,
        )));
        // prefix "    " (4) + io col 14 + "  " (2) + cost ~9 → reserve ~30.
        let name_cells = (inner.width as usize).saturating_sub(30).clamp(8, 40);
        for m in &view.models {
            let io = format!("{} / {}", fmt_tokens(m.in_tokens), fmt_tokens(m.out_tokens));
            lines.push(Line::from(format!(
                "    {}  {}  {}",
                pad_to_width(&m.name, name_cells),
                pad_to_width(&io, 14),
                fc(m.cost_usd),
            )));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        crate::i18n::tr(
            "  /cost reload · esc: back to chat",
            "  /cost reload 重新整理 · esc：回聊天",
        ),
        dim,
    )));
    let p = Paragraph::new(lines).wrap(Wrap { trim: false });
    f.render_widget(p, inner);
}

// ── Evolve Runs pane (/evolve, P3 Evolve Mesh) — prototype render ─────────
//
// Design: docs/superpowers/design/tui-evolve-runs.md. Read-only pane: recent
// `autoevolve` runs parsed from ~/.phantom-mesh/autoevolve.log (JSONL). Lib-local
// mirror of bin's AutoEvolveLogEntry (which is binary-private). Pure render;
// `/evolve` toggle wiring is the follow-on (7th pane → joins the Esc-close set).

/// One autoevolve run, parsed from a JSONL line of `~/.phantom-mesh/autoevolve.log`.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct EvolveRun {
    pub started_at_ms: i64,
    pub target: String,
    pub status: String,
    #[serde(default)]
    pub rounds: usize,
    #[serde(default)]
    pub elapsed_secs: f64,
    #[serde(default)]
    pub commit: Option<String>,
    #[serde(default)]
    pub summary: String,
}

/// Parse the autoevolve JSONL log into runs, **newest first** (the file appends
/// newest last). Blank + unparseable lines are skipped (forward-compat with
/// field additions). Pure + testable.
pub fn parse_evolve_log(text: &str) -> Vec<EvolveRun> {
    let mut runs: Vec<EvolveRun> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<EvolveRun>(l).ok())
        .collect();
    runs.reverse();
    runs
}

/// Read + parse `~/.phantom-mesh/autoevolve.log` into runs (newest-first).
/// Empty vec if the log is missing/unreadable (→ the pane's empty state).
fn evolve_runs_from_log() -> Vec<EvolveRun> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    match std::fs::read_to_string(home.join(".phantom-mesh").join("autoevolve.log")) {
        Ok(text) => parse_evolve_log(&text),
        Err(_) => Vec::new(),
    }
}

/// `(glyph, ansi_color)` for an autoevolve status. Unknown → neutral dot.
fn evolve_status_glyph(status: &str) -> (&'static str, Color) {
    match status {
        "green" => ("✓", Color::Green),
        "fixed" => ("⟳", Color::Cyan),
        "failed" => ("✗", Color::Red),
        "skip" => ("○", Color::DarkGray),
        _ => ("·", Color::DarkGray),
    }
}

/// `started_at_ms` (epoch ms) → local `MM-DD HH:MM`.
fn evolve_when(started_at_ms: i64) -> String {
    if started_at_ms <= 0 {
        return "??-?? ??:??".to_string();
    }
    let t = std::time::UNIX_EPOCH + std::time::Duration::from_millis(started_at_ms as u64);
    let dt: chrono::DateTime<chrono::Local> = t.into();
    dt.format("%m-%d %H:%M").to_string()
}

/// Render the evolve-runs pane. Pure over the parsed rows (newest-first).
pub fn render_evolve_pane(f: &mut Frame, area: Rect, runs: &[EvolveRun]) {
    f.render_widget(Clear, area);
    let g = runs.iter().filter(|r| r.status == "green").count();
    let fx = runs.iter().filter(|r| r.status == "fixed").count();
    let fl = runs.iter().filter(|r| r.status == "failed").count();
    let title = if runs.is_empty() {
        " phantom · evolve — 0 runs ".to_string()
    } else {
        format!(
            " phantom · evolve — {} runs · {} green · {} fixed · {} failed ",
            runs.len(),
            g,
            fx,
            fl
        )
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(
            title,
            Style::default()
                .fg(Color::Rgb(214, 178, 112))
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let dim = Style::default().fg(Color::DarkGray);
    let mut lines: Vec<Line> = Vec::new();

    if runs.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(crate::i18n::tr(
            "  No autoevolve runs yet.",
            "  尚未有任何 autoevolve 執行紀錄。",
        )));
        lines.push(Line::from(Span::styled(
            crate::i18n::tr(
                "  Try `phantom autoevolve --once`, then press r.",
                "  試試 `phantom autoevolve --once`，然後按 r。",
            ),
            dim,
        )));
    } else {
        // prefix before SUMMARY: "  " (2) + glyph(1) + " " (1) + when(11) + "  "
        // (2) + tgt(6) + rnd(4) + elapsed(9) + commit(8) ≈ 44 cells reserved.
        let sum_cells = (inner.width as usize).saturating_sub(44).max(8);
        for r in runs {
            let (glyph, color) = evolve_status_glyph(&r.status);
            let commit = r
                .commit
                .as_deref()
                .map(|c| c.chars().take(7).collect::<String>())
                .unwrap_or_else(|| "—".to_string());
            let body = format!(
                "{}  {}  {} {} {}  {}",
                evolve_when(r.started_at_ms),
                pad_to_width(&r.target, 6),
                pad_to_width(&r.rounds.to_string(), 3),
                pad_to_width(&format!("{:.1}s", r.elapsed_secs), 8),
                pad_to_width(&commit, 7),
                truncate_to_width(&r.summary, sum_cells),
            );
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(glyph, Style::default().fg(color).add_modifier(Modifier::BOLD)),
                Span::raw(" "),
                Span::raw(body),
            ]));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        crate::i18n::tr(
            "  /evolve reload · esc: back to chat · newest first",
            "  /evolve reload 重新整理 · esc：回聊天 · 由新到舊",
        ),
        dim,
    )));
    let p = Paragraph::new(lines).wrap(Wrap { trim: false });
    f.render_widget(p, inner);
}

// ── Habits pane (/habits, P2 Life Track) — read-only render ──────────────
//
// Design: docs/superpowers/design/tui-habits.md. Read-only pane mirroring
// `phantom habit list`: recurring-habit streaks from the shared
// ~/.phantom-mesh/habits.sqlite via capture_habit_wire::list_habits()
// (SPEC-22). Pure render over the view-model; `/habits` toggle wiring lives
// in the slash handler. Display key is the slug (the wire summary carries no
// label — same as the CLI).

/// One habit row in the pane, projected from `capture_habit_wire::HabitSummary`.
#[derive(Debug, Clone, PartialEq)]
pub struct HabitRow {
    pub slug: String,
    pub current_streak: u16,
    pub longest_streak: u16,
    pub last_7d: u32,
    pub last_30d: u32,
    /// ISO-8601 UTC of the most recent check-in; `None` if never logged.
    pub last_checkin_at: Option<String>,
}

/// Project `HabitSummary` rows into `HabitRow`, sorted active-first:
/// `current_streak` desc, then `last_7d` desc, then `slug` asc (deterministic).
/// Pure + testable.
pub fn habit_rows_from_summaries(
    summaries: &[crate::capture_habit_wire::HabitSummary],
) -> Vec<HabitRow> {
    let mut rows: Vec<HabitRow> = summaries
        .iter()
        .map(|s| HabitRow {
            slug: s.habit_slug.clone(),
            current_streak: s.streak.current_streak,
            longest_streak: s.streak.longest_streak,
            last_7d: s.last_7d_count,
            last_30d: s.last_30d_count,
            last_checkin_at: s.last_checkin_at.clone(),
        })
        .collect();
    rows.sort_by(|a, b| {
        b.current_streak
            .cmp(&a.current_streak)
            .then(b.last_7d.cmp(&a.last_7d))
            .then(a.slug.cmp(&b.slug))
    });
    rows
}

/// Read + project the habits palette into rows (active-first). Empty vec on an
/// empty palette OR a db error (→ the pane's empty state) — a read-only pane
/// has nothing actionable to do with a load error beyond showing "none yet".
fn habit_rows_load() -> Vec<HabitRow> {
    match crate::capture_habit_wire::list_habits() {
        Ok(summaries) => habit_rows_from_summaries(&summaries),
        Err(_) => Vec::new(),
    }
}

/// `last_checkin_at` ISO-8601 → a compact `MM-DD` for the row, or a localized
/// "never" when the chip has never been logged. Pure + testable.
fn habit_last_when(last_checkin_at: Option<&str>) -> String {
    match last_checkin_at {
        // ISO-8601 is `YYYY-MM-DDT...`; the MM-DD slice is bytes 5..10.
        Some(iso) if iso.len() >= 10 => iso[5..10].to_string(),
        Some(_) => crate::i18n::tr("never", "從未").to_string(),
        None => crate::i18n::tr("never", "從未").to_string(),
    }
}

/// Render the habits pane. Pure over the projected rows (active-first).
pub fn render_habits_pane(f: &mut Frame, area: Rect, rows: &[HabitRow]) {
    f.render_widget(Clear, area);
    let active = rows.iter().filter(|r| r.current_streak > 0).count();
    let title = crate::i18n::tr_owned(
        format!(" phantom · habits — {} tracked · {} active ", rows.len(), active),
        format!(" phantom · 習慣 — 追蹤 {} 個 · {} 個進行中 ", rows.len(), active),
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(
            title,
            Style::default()
                .fg(Color::Rgb(214, 178, 112))
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let dim = Style::default().fg(Color::DarkGray);
    let mut lines: Vec<Line> = Vec::new();

    if rows.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(crate::i18n::tr(
            "  No habits tracked yet.",
            "  尚未追蹤任何習慣。",
        )));
        lines.push(Line::from(Span::styled(
            crate::i18n::tr(
                "  Add one with `phantom habit create <slug>`, then press r.",
                "  用 `phantom habit create <slug>` 新增一個，然後按 r。",
            ),
            dim,
        )));
    } else {
        // prefix "  " (2) + streak/7d/30d/last columns ≈ 48 cells reserved.
        let name_cells = (inner.width as usize).saturating_sub(48).clamp(8, 32);
        let streak_lbl = crate::i18n::tr("streak", "連續");
        let best_lbl = crate::i18n::tr("best", "最長");
        let last_lbl = crate::i18n::tr("last", "最後");
        for r in rows {
            // Active streaks read in a warmer color; dormant (0) stay dim.
            let streak_color = if r.current_streak > 0 {
                Color::Rgb(214, 178, 112)
            } else {
                Color::DarkGray
            };
            lines.push(Line::from(vec![
                Span::raw(format!("  {}  ", pad_to_width(&r.slug, name_cells))),
                Span::styled(
                    format!("{} {}", streak_lbl, r.current_streak),
                    Style::default().fg(streak_color),
                ),
                Span::styled(format!(" ({} {})", best_lbl, r.longest_streak), dim),
                Span::raw(format!(
                    "   7d {}   30d {}   {} {}",
                    r.last_7d,
                    r.last_30d,
                    last_lbl,
                    habit_last_when(r.last_checkin_at.as_deref()),
                )),
            ]));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        crate::i18n::tr(
            "  /habits reload · esc: back to chat · log via `phantom habit checkin`",
            "  /habits reload 重新整理 · esc：回聊天 · 用 `phantom habit checkin` 打卡",
        ),
        dim,
    )));
    let p = Paragraph::new(lines).wrap(Wrap { trim: false });
    f.render_widget(p, inner);
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
            cluster_view: false,
            cluster_rows: Vec::new(),
            goals_view: false,
            goals_rows: Vec::new(),
            goals_selected: 0,
            identity_view: false,
            identity_data: None,
            review_view: false,
            review_data: None,
            cost_view: false,
            cost_data: None,
            focus_view: false,
            focus_data: None,
            evolve_view: false,
            evolve_runs: Vec::new(),
            habits_view: false,
            habits_rows: Vec::new(),
            sidebar_peers: Vec::new(),
            priority_picker: None,
            last_ctrl_c_at: None,
            pending_interrupt: None,
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

    fn render_cluster(rows: &[super::ClusterRow], sel: usize, w: u16, h: u16) -> String {
        let backend = TestBackend::new(w, h);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| super::render_cluster_pane(f, f.area(), rows, sel))
            .unwrap();
        buffer_text(&term.backend().buffer().clone())
    }

    fn sample_rows() -> Vec<super::ClusterRow> {
        vec![
            super::ClusterRow {
                name: "this-node".into(), is_local: true, pinged: true, online: true, degraded: false,
                version: "0.6.0-rc1".into(), tasks: 2, last_seen: "now".into(),
                caps: vec!["shell".into(), "gpu".into()],
            },
            super::ClusterRow {
                name: "peer-bravo".into(), is_local: false, pinged: true, online: true, degraded: true,
                version: "0.5.0".into(), tasks: 1, last_seen: "12s ago".into(),
                caps: vec!["shell".into(), "camera".into()],
            },
            super::ClusterRow {
                name: "peer-delta".into(), is_local: false, pinged: true, online: false, degraded: false,
                version: "0.6.0-rc1".into(), tasks: 0, last_seen: "4m ago".into(),
                caps: vec![],
            },
        ]
    }

    #[test]
    fn cluster_pane_renders_configured_state_when_unpinged() {
        // Config-topology view (built from peers.json, not yet pinged): rows
        // show "configured", not online/offline, and the title says so.
        let rows = vec![super::ClusterRow {
            name: "peer-cfg".into(), is_local: false, pinged: false, online: false,
            degraded: false, version: "—".into(), tasks: 0, last_seen: "—".into(),
            caps: vec!["shell".into()],
        }];
        let txt = render_cluster(&rows, 0, 80, 8);
        assert!(txt.contains("configured"), "configured status/title missing:\n{txt}");
        assert!(txt.contains("peer-cfg"), "peer name missing");
        // must NOT claim online/offline for an unpinged peer
        assert!(!txt.contains("online") && !txt.contains("offline"), "unpinged must not assert status:\n{txt}");
    }

    #[test]
    fn cluster_pane_renders_peer_table() {
        let txt = render_cluster(&sample_rows(), 0, 80, 12);
        assert!(txt.contains("phantom · cluster"), "title missing:\n{txt}");
        assert!(txt.contains("3 peers · 2 online"), "count line wrong:\n{txt}");
        assert!(txt.contains("NODE") && txt.contains("STATUS") && txt.contains("CAPS"));
        assert!(txt.contains("this-node") && txt.contains("peer-bravo") && txt.contains("peer-delta"));
        assert!(txt.contains('★'), "local marker missing");
        // status glyphs: online ●, degraded ◐, offline ○
        assert!(txt.contains('●') && txt.contains('◐') && txt.contains('○'), "status glyphs:\n{txt}");
        // offline peer shows dashes for ver/tasks
        assert!(txt.contains('—'), "offline placeholder missing");
    }

    #[test]
    fn cluster_pane_renders_empty_state() {
        let txt = render_cluster(&[], 0, 70, 8);
        assert!(txt.contains("0 peers"), "empty title:\n{txt}");
        // Locale-robust: hint is i18n-translated; strip whitespace (CJK cell-padding).
        let compact: String = txt.split_whitespace().collect();
        assert!(
            compact.contains("Nopeersconfigured") || compact.contains("尚未設定任何peer"),
            "empty hint:\n{txt}"
        );
        assert!(txt.contains("phantom cluster"), "empty hint cmd:\n{txt}");
    }

    fn render_goals(rows: &[super::GoalRow], sel: usize, w: u16, h: u16) -> String {
        let backend = TestBackend::new(w, h);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| super::render_goals_pane(f, f.area(), rows, sel)).unwrap();
        buffer_text(&term.backend().buffer().clone())
    }

    #[test]
    fn goals_pane_renders_pending_and_done() {
        let rows = vec![
            super::GoalRow { label: "L13".into(), text: "CI release matrix".into(), done: false },
            super::GoalRow { label: "L14".into(), text: "phantom doctor --mesh".into(), done: false },
            super::GoalRow { label: "L05".into(), text: "cross-host dispatch e2e".into(), done: true },
        ];
        let txt = render_goals(&rows, 0, 70, 10);
        assert!(txt.contains("evolve goals"), "title:\n{txt}");
        assert!(txt.contains("2 pending · 1 done"), "counts:\n{txt}");
        assert!(txt.contains("CI release matrix") && txt.contains("cross-host dispatch"), "rows:\n{txt}");
        assert!(txt.contains('○') && txt.contains('✓'), "glyphs:\n{txt}");
        assert!(txt.contains("L13") && txt.contains("L05"), "labels");
    }

    fn render_identity(view: &super::IdentityView, w: u16, h: u16) -> String {
        let backend = TestBackend::new(w, h);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| super::render_identity_pane(f, f.area(), view)).unwrap();
        buffer_text(&term.backend().buffer().clone())
    }

    #[test]
    fn identity_pane_logged_in_shows_identity_and_full_scope() {
        let v = super::IdentityView {
            identity_line: Some("provider  google · you@example.com".into()),
            fingerprint: "a1b2c3d4e5f6".into(),
            created_at: "2026-05-18".into(),
            keystore: "linux-secret-service".into(),
            key_present: true,
        };
        // Tall enough to show all rows (honesty rail must not clip plaintext rows).
        let txt = render_identity(&v, 70, 20);
        assert!(txt.contains("identity & vault") && txt.contains("P4"), "title:\n{txt}");
        assert!(txt.contains("you@example.com") && txt.contains("a1b2c3d4e5f6"), "identity:\n{txt}");
        assert!(txt.contains("linux-secret-service"), "keystore");
        // Honesty rail at the RENDER level: every scope row must actually
        // appear — exactly 2 encrypted (✓) + 6 plaintext (○). Each row emits
        // exactly one glyph, so the counts prove all 8 rows rendered (none
        // dropped/clipped) — the const-only guard can't catch a render drop.
        assert_eq!(txt.matches('✓').count(), 2, "expected 2 ✓ (encrypted) rows:\n{txt}");
        assert_eq!(txt.matches('○').count(), 6, "expected 6 ○ (plaintext) rows:\n{txt}");
        // Spot-check labels that fit the 24-cell column un-truncated. (Long
        // labels like "telemetry (costs/log/crashes)" are legitimately ellipsized
        // by pad_to_width, so we check a stable prefix for those.)
        assert!(txt.contains("Life Node events") && txt.contains("agents.toml"));
        assert!(txt.contains("captures/*.png") && txt.contains("telemetry"));
    }

    #[test]
    fn identity_pane_not_logged_in_still_shows_scope() {
        let v = super::IdentityView {
            identity_line: None,
            fingerprint: "—".into(),
            created_at: "—".into(),
            keystore: "—".into(),
            key_present: true,
        };
        let txt = render_identity(&v, 70, 18);
        // Locale-robust: hint is i18n-translated; strip whitespace (CJK cell-padding).
        let compact: String = txt.split_whitespace().collect();
        assert!(
            compact.contains("notloggedin") || compact.contains("尚未登入"),
            "login hint:\n{txt}"
        );
        // scope table is identity-independent → still rendered.
        assert!(txt.contains("agents.toml") && txt.contains("memory.db"), "scope still shown:\n{txt}");
    }

    #[test]
    fn identity_pane_warns_when_no_identity_key() {
        // key_present:false → honest "events are PLAINTEXT" warning (P4 rail).
        let v = super::IdentityView {
            identity_line: None,
            fingerprint: "—".into(),
            created_at: "—".into(),
            keystore: "—".into(),
            key_present: false,
        };
        let txt = render_identity(&v, 72, 22);
        let compact: String = txt.split_whitespace().collect();
        assert!(
            compact.contains("PLAINTEXT") || compact.contains("明文"),
            "no-key plaintext warning shown:\n{txt}"
        );
        // And NOT shown when the key is present.
        let v2 = super::IdentityView { key_present: true, ..v };
        let txt2 = render_identity(&v2, 72, 22);
        assert!(
            !txt2.contains("PLAINTEXT") && !txt2.contains("明文"),
            "no warning when key present:\n{txt2}"
        );
    }

    #[test]
    fn p4_scope_lists_all_eight_paths_two_encrypted() {
        // Honesty-rail guard: the scope table must stay complete (8 rows) and
        // only events + identity.key encrypted. If BIG-GOAL P4 changes, update
        // both BIG-GOAL.md and P4_SCOPE — this test pins the v0.6.0 truth.
        assert_eq!(super::P4_SCOPE.len(), 8, "P4 scope must list all 8 paths");
        let encrypted = super::P4_SCOPE.iter().filter(|(e, _, _)| *e).count();
        assert_eq!(encrypted, 2, "only events + identity.key are encrypted in v0.6.0");
    }

    #[test]
    fn goal_rows_from_file_parses_evolve_goals() {
        let _g = crate::sandbox::test_lock();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("EVOLVE-GOALS.md");
        std::fs::write(
            &path,
            "## Pending\n- [ ] ship A\n- [ ] ship B\n\n## Done\n- [x] shipped C\n",
        )
        .unwrap();
        std::env::set_var("PHANTOM_EVOLVE_GOALS", &path);
        let rows = super::goal_rows_from_file();
        std::env::remove_var("PHANTOM_EVOLVE_GOALS");
        assert_eq!(rows.len(), 3, "expected 3 checkbox rows, got {}", rows.len());
        assert!(!rows[0].done && rows[0].text == "ship A");
        assert!(!rows[1].done && rows[1].text == "ship B");
        assert!(rows[2].done && rows[2].text == "shipped C");
    }

    #[test]
    fn goal_rows_from_file_missing_is_empty() {
        let _g = crate::sandbox::test_lock();
        std::env::set_var("PHANTOM_EVOLVE_GOALS", "/nonexistent/EVOLVE-GOALS.md");
        let rows = super::goal_rows_from_file();
        std::env::remove_var("PHANTOM_EVOLVE_GOALS");
        // GoalsFile::load returns an empty doc for a missing file → no rows.
        assert!(rows.is_empty(), "missing file should yield no rows, got {}", rows.len());
    }

    #[test]
    fn goals_pane_renders_empty_state() {
        let txt = render_goals(&[], 0, 70, 8);
        assert!(txt.contains("0 goals"), "empty title:\n{txt}");
        // Locale-robust: hint is i18n-translated (en "No goals yet" / zh-TW "尚無目標").
        // current_lang() is a process-global OnceLock resolved from LANG, so a zh-TW
        // dev machine renders the zh string while CI (en) renders English. Strip all
        // whitespace first because the buffer pads double-width CJK cells (尚 無 目 標).
        let compact: String = txt.split_whitespace().collect();
        assert!(
            compact.contains("Nogoalsyet") || compact.contains("尚無目標"),
            "empty hint:\n{txt}"
        );
        assert!(txt.contains("phantom evolve goals add"), "empty cmd hint");
    }

    #[test]
    fn goals_pane_long_text_never_overflows_width() {
        // A goal longer than the pane → must be truncated so no rendered line
        // exceeds the pane width (else `Wrap` re-flows it, corrupting the list).
        // This is the test that would have caught the prefix off-by-one.
        let rows = vec![super::GoalRow {
            label: "L99".into(),
            text: "x".repeat(200),
            done: false,
        }];
        let w: u16 = 50;
        let buf = {
            let backend = TestBackend::new(w, 6);
            let mut term = Terminal::new(backend).unwrap();
            term.draw(|f| super::render_goals_pane(f, f.area(), &rows, 0)).unwrap();
            term.backend().buffer().clone()
        };
        // Every cell column beyond the right border must be blank — i.e. the
        // content fit inside the bordered inner area, no wrap-induced 2nd line.
        let txt = buffer_text(&buf);
        for (n, line) in txt.lines().enumerate() {
            assert!(
                line.chars().count() <= w as usize,
                "line {n} exceeds width {w}: {:?}",
                line
            );
        }
        // The single goal must occupy exactly one content row (border + 1 row +
        // blank + footer), proving no wrap. Count rows containing the label.
        assert_eq!(txt.matches("L99").count(), 1, "goal row wrapped:\n{txt}");
    }

    // ── daily-review pane (/review) ─────────────────────────────────────
    fn render_review(view: &super::ReviewView, w: u16, h: u16) -> String {
        let backend = TestBackend::new(w, h);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| super::render_daily_review_pane(f, f.area(), view)).unwrap();
        buffer_text(&term.backend().buffer().clone())
    }

    #[test]
    fn parse_review_rows_parses_aggregate_shape() {
        // Mirrors daily_review::aggregate output: title + count + `## tag (n)`
        // groups + `- **kind** (ts): summary` bullets. Asserts the parser
        // extracts groups + bullets (kind, HH:MM from ISO ts, summary) and skips
        // the title / count / blank prose lines.
        let md = "# Daily review — 2026-05-28\n\n\
                  **Events captured:** 3\n\n\
                  ## fat_loss (2)\n\
                  - **food** (2026-05-28T08:14:23+08:00): oatmeal + coffee\n\
                  - **food** (2026-05-28T12:40:00+08:00): rice bowl\n\n\
                  ## untagged (1)\n\
                  - **text** (2026-05-28T21:10:00+08:00): shipped the pane\n";
        let rows = super::parse_review_rows(md);
        assert_eq!(rows.len(), 5, "2 groups + 3 bullets: {rows:?}");
        assert_eq!(
            rows[0],
            super::ReviewRow::Group { tag: "fat_loss".into(), n: 2 }
        );
        match &rows[1] {
            super::ReviewRow::Bullet { kind, time, summary } => {
                assert_eq!(kind, "food");
                assert_eq!(time, "08:14", "HH:MM extracted from ISO ts");
                assert_eq!(summary, "oatmeal + coffee");
            }
            other => panic!("expected bullet, got {other:?}"),
        }
        assert_eq!(
            rows[3],
            super::ReviewRow::Group { tag: "untagged".into(), n: 1 }
        );
    }

    #[test]
    fn parse_review_rows_first_colon_after_ts_is_delimiter() {
        // A summary containing "): " must not break parsing — the ISO timestamp
        // never contains "): ", so the FIRST occurrence is the true delimiter.
        let md = "- **focus** (2026-05-28T10:00:00Z): note (with): a colon\n";
        let rows = super::parse_review_rows(md);
        assert_eq!(rows.len(), 1);
        match &rows[0] {
            super::ReviewRow::Bullet { kind, time, summary } => {
                assert_eq!(kind, "focus");
                assert_eq!(time, "10:00");
                assert_eq!(summary, "note (with): a colon");
            }
            other => panic!("expected bullet, got {other:?}"),
        }
    }

    #[test]
    fn daily_review_pane_renders_events() {
        let view = super::ReviewView {
            date: "2026-05-28".into(),
            state: super::ReviewState::Events,
            event_count: 2,
            rows: vec![
                super::ReviewRow::Group { tag: "fat_loss".into(), n: 1 },
                super::ReviewRow::Bullet {
                    kind: "food".into(),
                    time: "08:14".into(),
                    summary: "oatmeal + black coffee".into(),
                },
                super::ReviewRow::Group { tag: "focus".into(), n: 1 },
                super::ReviewRow::Bullet {
                    kind: "focus".into(),
                    time: "10:00".into(),
                    summary: "50-min deep work".into(),
                },
            ],
            flagged: false,
        };
        let txt = render_review(&view, 70, 14);
        // "daily review" + the date are NOT translated (data/brand) → stable.
        assert!(txt.contains("daily review"), "title:\n{txt}");
        assert!(txt.contains("2026-05-28"), "date in title:\n{txt}");
        assert!(txt.contains("fat_loss") && txt.contains("focus"), "groups:\n{txt}");
        assert!(
            txt.contains("oatmeal") && txt.contains("deep work"),
            "summaries:\n{txt}"
        );
        assert!(txt.contains("08:14") && txt.contains("10:00"), "times:\n{txt}");
        assert!(!txt.contains('⚠'), "no flag banner when not flagged:\n{txt}");
    }

    #[test]
    fn daily_review_pane_flagged_shows_banner() {
        let view = super::ReviewView {
            date: "2026-05-28".into(),
            state: super::ReviewState::Events,
            event_count: 1,
            rows: vec![super::ReviewRow::Bullet {
                kind: "text".into(),
                time: "09:00".into(),
                summary: "a note".into(),
            }],
            flagged: true,
        };
        let txt = render_review(&view, 70, 8);
        assert!(txt.contains('⚠'), "flagged banner must show:\n{txt}");
    }

    #[test]
    fn daily_review_pane_empty_state() {
        let view = super::ReviewView {
            date: "2026-05-28".into(),
            state: super::ReviewState::Empty,
            event_count: 0,
            rows: vec![],
            flagged: false,
        };
        let txt = render_review(&view, 70, 8);
        let compact: String = txt.split_whitespace().collect();
        assert!(
            compact.contains("0events") || compact.contains("0筆事件"),
            "count in title:\n{txt}"
        );
        // Locale-robust: empty hint is i18n'd (en / zh-TW).
        assert!(
            compact.contains("NoLifeNodeevents") || compact.contains("沒有任何生活節點事件"),
            "empty hint:\n{txt}"
        );
    }

    #[test]
    fn daily_review_pane_locked_state() {
        let view = super::ReviewView {
            date: "2026-05-28".into(),
            state: super::ReviewState::Locked,
            event_count: 0,
            rows: vec![],
            flagged: false,
        };
        let txt = render_review(&view, 70, 8);
        let compact: String = txt.split_whitespace().collect();
        assert!(
            compact.contains("encryptedatrest") || compact.contains("以靜態加密儲存"),
            "locked body:\n{txt}"
        );
        assert!(
            compact.contains("Identitykeynotloaded") || compact.contains("尚未載入身分金鑰"),
            "locked hint:\n{txt}"
        );
    }

    #[test]
    fn parse_review_rows_matches_real_aggregate_output() {
        // Contract test (not a hand-built string): feed the REAL
        // `daily_review::aggregate` output through the parser. If aggregate ever
        // changes its `format!` shape, this catches the parser drift that a
        // hardcoded-fixture test would silently miss.
        use crate::event_storage_wire::{EventKind, EventMeta};
        use crate::life_node::multimodal::AnalysisResult;
        let meta = EventMeta {
            event_id: "evt-1".into(),
            kind: EventKind::Food,
            timestamp: "2026-05-28T08:14:23+08:00".into(),
            tags: vec!["fat_loss".into()],
        };
        let analysis = AnalysisResult {
            summary: "oatmeal + coffee".into(),
            goal_impact: None,
            suggestion: None,
            confidence: Some(0.8),
            raw_response: serde_json::json!({}),
            model_id: "test".into(),
            latency_ms: 0,
            cost_usd: None,
        };
        let md = crate::life_node::daily_review::aggregate("2026-05-28", &[(meta, analysis)]);
        let rows = super::parse_review_rows(&md);
        assert_eq!(rows.len(), 2, "real aggregate → 1 group + 1 bullet:\n{md}");
        assert_eq!(
            rows[0],
            super::ReviewRow::Group { tag: "fat_loss".into(), n: 1 }
        );
        match &rows[1] {
            super::ReviewRow::Bullet { kind, time, summary } => {
                assert_eq!(kind, "food", "EventKind::Food serializes to `food`");
                assert_eq!(time, "08:14");
                assert_eq!(summary, "oatmeal + coffee");
            }
            other => panic!("expected bullet from real aggregate, got {other:?}"),
        }
    }

    #[test]
    fn daily_review_pane_narrow_width_no_overflow() {
        // The 20-cell prefix math + `.max(8)` floor must not let a long summary
        // wrap/overflow at a narrow width (else `Wrap` re-flows + corrupts rows).
        let view = super::ReviewView {
            date: "2026-05-28".into(),
            state: super::ReviewState::Events,
            event_count: 1,
            rows: vec![super::ReviewRow::Bullet {
                kind: "food".into(),
                time: "08:14".into(),
                summary: "x".repeat(120),
            }],
            flagged: false,
        };
        let w: u16 = 30;
        let txt = render_review(&view, w, 8);
        for (n, line) in txt.lines().enumerate() {
            assert!(
                line.chars().count() <= w as usize,
                "line {n} exceeds width {w}: {line:?}"
            );
        }
    }

    // ── cost pane (/cost) ───────────────────────────────────────────────
    fn render_cost(view: &super::CostView, w: u16, h: u16) -> String {
        let backend = TestBackend::new(w, h);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| super::render_cost_pane(f, f.area(), view)).unwrap();
        buffer_text(&term.backend().buffer().clone())
    }

    #[test]
    fn fmt_tokens_humanizes() {
        assert_eq!(super::fmt_tokens(0), "0");
        assert_eq!(super::fmt_tokens(950), "950");
        assert_eq!(super::fmt_tokens(12_300), "12.3k");
        assert_eq!(super::fmt_tokens(3_400_000), "3.4M");
    }

    #[test]
    fn cost_view_from_summary_parses_and_sorts_by_cost() {
        // Mirrors the CostTracker::summary() JSON shape.
        let summary = serde_json::json!({
            "session_usd": 0.0231,
            "total_usd": 1.4820,
            "requests": 14,
            "prompt_tokens": 12300,
            "completion_tokens": 4100,
            "budget_limit_usd": 0.05,
            "over_budget": false,
            "by_model": {
                "gemini-2.5-flash": { "input_tokens": 3400, "output_tokens": 1200, "cost_usd": 0.0071 },
                "llama-3.3-70b": { "input_tokens": 8100, "output_tokens": 2700, "cost_usd": 0.0142 },
                "claude-opus-4-7": { "input_tokens": 800, "output_tokens": 200, "cost_usd": 0.0018 },
            }
        });
        let v = super::cost_view_from_summary(&summary);
        assert_eq!(v.requests, 14);
        assert!((v.session_usd - 0.0231).abs() < 1e-9);
        assert!((v.budget_limit_usd - 0.05).abs() < 1e-9);
        assert_eq!(v.models.len(), 3);
        // sorted by cost desc: llama 0.0142 > gemini 0.0071 > claude 0.0018
        assert_eq!(v.models[0].name, "llama-3.3-70b");
        assert_eq!(v.models[1].name, "gemini-2.5-flash");
        assert_eq!(v.models[2].name, "claude-opus-4-7");
        assert_eq!(v.models[0].in_tokens, 8100);
    }

    #[test]
    fn cost_view_from_summary_empty_is_zero() {
        let v = super::cost_view_from_summary(&serde_json::json!({}));
        assert_eq!(v.requests, 0);
        assert_eq!(v.session_usd, 0.0);
        assert!(v.models.is_empty());
    }

    #[test]
    fn cost_pane_renders_breakdown() {
        let view = super::CostView {
            session_usd: 0.0231,
            total_usd: 1.4820,
            requests: 14,
            prompt_tokens: 12300,
            completion_tokens: 4100,
            budget_limit_usd: 0.0,
            over_budget: false,
            models: vec![
                super::CostModelRow { name: "llama-3.3-70b".into(), in_tokens: 8100, out_tokens: 2700, cost_usd: 0.0142 },
                super::CostModelRow { name: "gemini-2.5-flash".into(), in_tokens: 3400, out_tokens: 1200, cost_usd: 0.0071 },
            ],
        };
        let txt = render_cost(&view, 70, 14);
        assert!(txt.contains("cost"), "title:\n{txt}");
        assert!(
            txt.contains("llama-3.3-70b") && txt.contains("gemini-2.5-flash"),
            "models:\n{txt}"
        );
        assert!(txt.contains("$0.0142") && txt.contains("$0.0071"), "costs:\n{txt}");
        assert!(txt.contains("8.1k") && txt.contains("2.7k"), "tokens humanized:\n{txt}");
    }

    #[test]
    fn cost_pane_zero_spend_state() {
        let view = super::CostView {
            session_usd: 0.0,
            total_usd: 0.0,
            requests: 0,
            prompt_tokens: 0,
            completion_tokens: 0,
            budget_limit_usd: 0.0,
            over_budget: false,
            models: vec![],
        };
        let txt = render_cost(&view, 70, 8);
        let compact: String = txt.split_whitespace().collect();
        assert!(
            compact.contains("Nospendyet") || compact.contains("尚無花費"),
            "zero-spend hint:\n{txt}"
        );
    }

    #[test]
    fn cost_pane_budget_bar_uses_lifetime_total() {
        // session_usd (0.01) != total_usd (0.023); the budget is a LIFETIME cap,
        // so the bar must measure total/limit = 46%, NOT session/limit (20%).
        let view = super::CostView {
            session_usd: 0.01,
            total_usd: 0.023,
            requests: 5,
            prompt_tokens: 1000,
            completion_tokens: 500,
            budget_limit_usd: 0.05,
            over_budget: false,
            models: vec![super::CostModelRow {
                name: "m".into(),
                in_tokens: 1000,
                out_tokens: 500,
                cost_usd: 0.01,
            }],
        };
        let txt = render_cost(&view, 70, 12);
        assert!(txt.contains('■') && txt.contains('□'), "budget bar glyphs:\n{txt}");
        assert!(txt.contains("46%"), "bar must use total (46%), not session (20%):\n{txt}");
        assert!(!txt.contains("20%"), "must not use session for the bar:\n{txt}");
    }

    #[test]
    fn cost_pane_over_budget_renders_without_panic() {
        let view = super::CostView {
            session_usd: 0.06,
            total_usd: 0.08,
            requests: 30,
            prompt_tokens: 50_000,
            completion_tokens: 20_000,
            budget_limit_usd: 0.05,
            over_budget: true,
            models: vec![super::CostModelRow {
                name: "m".into(),
                in_tokens: 50_000,
                out_tokens: 20_000,
                cost_usd: 0.08,
            }],
        };
        // total 0.08 / limit 0.05 clamps to 100% → full bar, no overflow/panic.
        let txt = render_cost(&view, 70, 12);
        assert!(txt.contains("100%"), "over-budget clamps to 100%:\n{txt}");
        assert_eq!(txt.matches('■').count(), 10, "full bar = 10 filled cells:\n{txt}");
    }

    #[test]
    fn cost_view_from_summary_tolerates_missing_model_fields() {
        // A by_model entry missing cost_usd / tokens must default to 0, not panic.
        let summary = serde_json::json!({
            "requests": 2,
            "by_model": { "m": { "input_tokens": 100 } }
        });
        let v = super::cost_view_from_summary(&summary);
        assert_eq!(v.models.len(), 1);
        assert_eq!(v.models[0].in_tokens, 100);
        assert_eq!(v.models[0].out_tokens, 0);
        assert_eq!(v.models[0].cost_usd, 0.0);
    }

    // ── evolve runs pane (/evolve) ──────────────────────────────────────
    fn render_evolve(runs: &[super::EvolveRun], w: u16, h: u16) -> String {
        let backend = TestBackend::new(w, h);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| super::render_evolve_pane(f, f.area(), runs)).unwrap();
        buffer_text(&term.backend().buffer().clone())
    }

    #[test]
    fn parse_evolve_log_reverses_and_skips_bad_lines() {
        // Mirrors the real ~/.phantom-mesh/autoevolve.log JSONL (oldest first in
        // file). A blank line + an unparseable line are skipped; output is
        // newest-first.
        let log = "{\"started_at_ms\":1000,\"target\":\"check\",\"status\":\"green\",\"rounds\":0,\"elapsed_secs\":27.5,\"commit\":null,\"summary\":\"no-op\"}\n\
                   \n\
                   not json at all\n\
                   {\"started_at_ms\":2000,\"target\":\"test\",\"status\":\"fixed\",\"rounds\":3,\"elapsed_secs\":120.4,\"commit\":\"a1b2c3d4e5\",\"summary\":\"fixed flaky env\"}\n";
        let runs = super::parse_evolve_log(log);
        assert_eq!(runs.len(), 2, "2 valid lines (blank + garbage skipped): {runs:?}");
        // newest-first: started_at_ms 2000 before 1000.
        assert_eq!(runs[0].started_at_ms, 2000);
        assert_eq!(runs[0].status, "fixed");
        assert_eq!(runs[0].rounds, 3);
        assert_eq!(runs[1].started_at_ms, 1000);
        assert_eq!(runs[1].commit, None);
    }

    #[test]
    fn evolve_pane_renders_tally_and_rows() {
        let runs = vec![
            super::EvolveRun {
                started_at_ms: 1_779_900_000_000,
                target: "test".into(),
                status: "fixed".into(),
                rounds: 3,
                elapsed_secs: 120.4,
                commit: Some("a1b2c3d4e5".into()),
                summary: "fixed flaky env".into(),
            },
            super::EvolveRun {
                started_at_ms: 1_779_800_000_000,
                target: "check".into(),
                status: "green".into(),
                rounds: 0,
                elapsed_secs: 27.5,
                commit: None,
                summary: "no-op (green)".into(),
            },
        ];
        let txt = render_evolve(&runs, 90, 12);
        assert!(txt.contains("evolve") && txt.contains("2 runs"), "title:\n{txt}");
        assert!(txt.contains("1 green") && txt.contains("1 fixed"), "tally:\n{txt}");
        assert!(txt.contains("fixed flaky env") && txt.contains("no-op"), "summaries:\n{txt}");
        assert!(txt.contains("120.4s") && txt.contains("27.5s"), "elapsed:\n{txt}");
        assert!(txt.contains("a1b2c3d"), "short commit (7 chars):\n{txt}");
        assert!(!txt.contains("a1b2c3d4e5"), "commit truncated to 7:\n{txt}");
    }

    #[test]
    fn evolve_pane_null_commit_shows_dash() {
        let runs = vec![super::EvolveRun {
            started_at_ms: 1_779_800_000_000,
            target: "check".into(),
            status: "green".into(),
            rounds: 0,
            elapsed_secs: 1.0,
            commit: None,
            summary: "x".into(),
        }];
        let txt = render_evolve(&runs, 90, 8);
        assert!(txt.contains('—'), "null commit must render as em-dash:\n{txt}");
    }

    #[test]
    fn evolve_pane_empty_state() {
        let txt = render_evolve(&[], 90, 8);
        assert!(txt.contains("0 runs"), "empty title:\n{txt}");
        let compact: String = txt.split_whitespace().collect();
        assert!(
            compact.contains("Noautoevolverunsyet") || compact.contains("尚未有任何"),
            "empty hint:\n{txt}"
        );
    }

    #[test]
    fn parse_evolve_log_tolerates_partial_line_and_renders_unknown_status() {
        // Only the 3 required fields present → #[serde(default)] fills the rest.
        // And an unknown status must render via the glyph fallback, not panic.
        let log = "{\"started_at_ms\":1779800000000,\"target\":\"check\",\"status\":\"queued\"}\n";
        let runs = super::parse_evolve_log(log);
        assert_eq!(runs.len(), 1, "partial line still parses: {runs:?}");
        assert_eq!(runs[0].rounds, 0, "missing rounds defaults to 0");
        assert_eq!(runs[0].commit, None, "missing commit defaults to None");
        assert_eq!(runs[0].summary, "", "missing summary defaults to empty");
        // Unknown status renders without panic (neutral `·` glyph).
        let txt = render_evolve(&runs, 90, 8);
        assert!(txt.contains("1 runs"), "renders the partial run:\n{txt}");
    }

    // ── habits pane (/habits, P2 Life Track) ────────────────────────────
    fn render_habits(rows: &[super::HabitRow], w: u16, h: u16) -> String {
        let backend = TestBackend::new(w, h);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| super::render_habits_pane(f, f.area(), rows)).unwrap();
        buffer_text(&term.backend().buffer().clone())
    }

    fn habit_summary(
        slug: &str,
        cur: u16,
        longest: u16,
        d7: u32,
        d30: u32,
        last: Option<&str>,
    ) -> crate::capture_habit_wire::HabitSummary {
        use crate::capture_habit_wire::{HabitStreak, HabitSummary};
        HabitSummary {
            habit_slug: slug.into(),
            last_7d_count: d7,
            last_30d_count: d30,
            last_checkin_at: last.map(|s| s.to_string()),
            streak: HabitStreak {
                habit_slug: slug.into(),
                current_streak: cur,
                longest_streak: longest,
                last_checkin_at: last.map(|s| s.to_string()),
            },
        }
    }

    #[test]
    fn habit_rows_sort_active_first_then_7d_then_slug() {
        let summaries = vec![
            habit_summary("read", 0, 7, 1, 4, Some("2026-05-26T09:00:00Z")),
            habit_summary("water", 5, 12, 6, 21, Some("2026-05-29T08:00:00Z")),
            habit_summary("quit_smoke", 5, 5, 3, 9, Some("2026-05-29T07:00:00Z")),
        ];
        let rows = super::habit_rows_from_summaries(&summaries);
        // current_streak desc → water/quit_smoke (5) before read (0);
        // tie on 5 broken by last_7d desc → water (6) before quit_smoke (3).
        assert_eq!(rows[0].slug, "water");
        assert_eq!(rows[1].slug, "quit_smoke");
        assert_eq!(rows[2].slug, "read");
    }

    #[test]
    fn habit_last_when_slices_mmdd_or_never() {
        assert_eq!(super::habit_last_when(Some("2026-05-29T08:00:00Z")), "05-29");
        let never = super::habit_last_when(None);
        assert!(never == "never" || never == "從未", "got {never:?}");
    }

    #[test]
    fn habits_pane_renders_streaks_and_counts() {
        let rows = super::habit_rows_from_summaries(&[
            habit_summary("water", 5, 12, 6, 21, Some("2026-05-29T08:00:00Z")),
            habit_summary("read", 0, 7, 1, 4, Some("2026-05-26T09:00:00Z")),
        ]);
        let txt = render_habits(&rows, 90, 10);
        // Locale + CJK-cell-padding robust: the zh title renders "追 蹤 2 個"
        // (double-width chars padded), so compare against whitespace-stripped.
        let compact: String = txt.split_whitespace().collect();
        assert!(compact.contains("2tracked") || compact.contains("追蹤2"), "title:\n{txt}");
        assert!(txt.contains("water") && txt.contains("read"), "slugs:\n{txt}");
        // current streak, best, windows, last-check MM-DD all present.
        assert!(txt.contains('5') && txt.contains("12"), "streak/best:\n{txt}");
        assert!(compact.contains("7d6") && compact.contains("30d21"), "windows:\n{txt}");
        assert!(txt.contains("05-29"), "last-check MM-DD:\n{txt}");
    }

    #[test]
    fn habits_pane_empty_state() {
        let txt = render_habits(&[], 90, 8);
        let compact: String = txt.split_whitespace().collect();
        assert!(compact.contains("0tracked") || compact.contains("追蹤0"), "empty title:\n{txt}");
        assert!(
            compact.contains("Nohabitstrackedyet") || compact.contains("尚未追蹤"),
            "empty hint:\n{txt}"
        );
    }

    #[test]
    fn goals_pane_selection_is_highlighted() {
        let rows = vec![
            super::GoalRow { label: "L1".into(), text: "alpha".into(), done: false },
            super::GoalRow { label: "L2".into(), text: "bravo".into(), done: false },
        ];
        // Selecting row 1 vs row 0 must produce different buffers (REVERSED
        // style toggles cell modifiers, which TestBackend records).
        let backend0 = TestBackend::new(40, 6);
        let mut t0 = Terminal::new(backend0).unwrap();
        t0.draw(|f| super::render_goals_pane(f, f.area(), &rows, 0)).unwrap();
        let backend1 = TestBackend::new(40, 6);
        let mut t1 = Terminal::new(backend1).unwrap();
        t1.draw(|f| super::render_goals_pane(f, f.area(), &rows, 1)).unwrap();
        assert_ne!(
            t0.backend().buffer(), t1.backend().buffer(),
            "selecting a different row should change the rendered buffer"
        );
    }

    #[test]
    fn cluster_status_glyph_text_mapping() {
        let base = super::ClusterRow {
            name: "n".into(), is_local: false, pinged: true, online: true,
            degraded: false, version: "1".into(), tasks: 0, last_seen: "now".into(),
            caps: vec![],
        };
        assert_eq!(super::cluster_status(&base).1, "online");
        let degraded = super::ClusterRow { degraded: true, ..base.clone() };
        assert_eq!(super::cluster_status(&degraded).1, "degraded");
        let offline = super::ClusterRow { online: false, ..base.clone() };
        assert_eq!(super::cluster_status(&offline).1, "offline");
        let unpinged = super::ClusterRow { pinged: false, online: false, ..base.clone() };
        assert_eq!(super::cluster_status(&unpinged).1, "configured");
    }

    #[test]
    fn cluster_pad_to_width_is_display_aware() {
        // ASCII: pads to exact width.
        assert_eq!(super::pad_to_width("abc", 6), "abc   ");
        // Already wider → truncates (with …) to the budget, never overflows.
        let t = super::pad_to_width("an-extremely-long-node-name", 10);
        assert_eq!(unicode_width::UnicodeWidthStr::width(t.as_str()), 10);
        // CJK (2 cells each): 2 chars = 4 cells → pad to 6 adds 2 spaces, total 6 cells.
        let c = super::pad_to_width("節點", 6);
        assert_eq!(unicode_width::UnicodeWidthStr::width(c.as_str()), 6);
    }

    #[test]
    fn cluster_trunc_handles_long_names() {
        assert_eq!(super::trunc("short", 16), "short");
        let long = super::trunc("an-extremely-long-peer-node-name", 16);
        assert!(long.chars().count() <= 16 && long.ends_with('…'), "got {long:?}");
    }

    fn render_focus(view: Option<&super::FocusView>, w: u16, h: u16) -> String {
        let backend = TestBackend::new(w, h);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| super::render_focus_pane(f, f.area(), view)).unwrap();
        buffer_text(&term.backend().buffer().clone())
    }

    #[test]
    fn focus_pane_renders_active_timer() {
        let v = super::FocusView {
            task: "refactor mesh ping path".into(),
            planned_min: 25,
            remaining_secs: 18 * 60 + 42,
            interruptions: 2,
            started_at: "14:07".into(),
            recording: false,
        };
        let txt = render_focus(Some(&v), 70, 10);
        assert!(txt.contains("phantom · focus") && txt.contains("25 min timer"), "title:\n{txt}");
        assert!(txt.contains("18:42") && txt.contains("left"), "timer:\n{txt}");
        assert!(txt.contains("refactor mesh ping path"), "task missing");
        assert!(txt.contains("interruptions: 2") && txt.contains("14:07"), "meta:\n{txt}");
        assert!(!txt.contains("REC"), "should not show REC when recording=false");
    }

    #[test]
    fn focus_pane_renders_recording_and_no_session() {
        let v = super::FocusView {
            task: "x".into(), planned_min: 50, remaining_secs: 5,
            interruptions: 0, started_at: "09:00".into(), recording: true,
        };
        assert!(render_focus(Some(&v), 70, 10).contains("REC"), "REC indicator when recording");
        let none = render_focus(None, 70, 8);
        // Locale-robust: hint is i18n-translated; strip whitespace (CJK cell-padding).
        let compact: String = none.split_whitespace().collect();
        assert!(
            compact.contains("Noactivefocussession") || compact.contains("目前沒有進行中的focus時段"),
            "no-session hint:\n{none}"
        );
        assert!(none.contains("phantom focus start"), "no-session cmd hint");
    }

    #[test]
    fn focus_fmt_mmss() {
        assert_eq!(super::fmt_mmss(0), "00:00");
        assert_eq!(super::fmt_mmss(65), "01:05");
        assert_eq!(super::fmt_mmss(1122), "18:42");
        assert_eq!(super::fmt_mmss(6249), "104:09"); // minutes uncapped
    }

    #[test]
    fn focus_pane_narrow_width_no_overflow() {
        let v = super::FocusView {
            task: "ship the narrow-width focus pane".into(),
            planned_min: 25,
            remaining_secs: 18 * 60 + 42,
            interruptions: 3,
            started_at: "14:07".into(),
            recording: true,
        };
        let txt = render_focus(Some(&v), 30, 24);
        // Every rendered line must fit within the pane width (no overflow).
        for line in txt.lines() {
            assert!(
                line.chars().count() <= 30,
                "line exceeds width 30: {:?} ({} chars)",
                line,
                line.chars().count()
            );
        }
    }

    #[test]
    fn focus_pane_long_task_text_does_not_panic_or_overflow() {
        let v = super::FocusView {
            task: "x".repeat(400),
            planned_min: 25,
            remaining_secs: 1500,
            interruptions: 9999,
            started_at: "23:59".into(),
            recording: true,
        };
        // Render at a very narrow width: must not panic and must not overflow.
        let txt = render_focus(Some(&v), 24, 14);
        for line in txt.lines() {
            assert!(
                line.chars().count() <= 24,
                "line exceeds width 24: {:?} ({} chars)",
                line,
                line.chars().count()
            );
        }
        // The recording marker should still survive truncation.
        assert!(txt.contains("REC"), "REC indicator at narrow width:\n{txt}");
    }

    #[test]
    fn cluster_relative_unix_buckets() {
        assert_eq!(super::relative_unix(0), "—");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert_eq!(super::relative_unix(now), "now");
        assert_eq!(super::relative_unix(now - 12), "12s ago");
        assert_eq!(super::relative_unix(now - 180), "3m ago");
        assert_eq!(super::relative_unix(now - 7200), "2h ago");
    }

    #[test]
    fn renders_empty_state_with_box_drawing() {
        let state = fresh_state();
        let buf = render(&state, 80, 24);
        let text = buffer_text(&buf);
        // ratatui's Block draws unicode box chars somewhere in the layout
        let has_box =
            text.contains('│') || text.contains('─') || text.contains('┌') || text.contains('└');
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
            "expected typed text in buffer; got:\n{}",
            text,
        );
    }

    /// V11 ship-blocker: TUI must clear cells properly on terminal
    /// resize so old content doesn't leak through.
    ///
    /// We can't directly assert "Clear was called" without instrumenting
    /// the render path — instead we test the OBSERVABLE consequence:
    /// rendering the same state at two different sizes both produces
    /// internally-consistent buffers with no uninitialised cells.
    /// (P0 — doc 31 §6.1, doc 29 §4 V11)
    #[test]
    fn clear_widget_called_on_resize() {
        let state = fresh_state();
        let small = render(&state, 60, 20);
        let large = render(&state, 100, 30);

        // Both renders produce non-empty content.
        let small_text = buffer_text(&small);
        let large_text = buffer_text(&large);
        assert!(
            !small_text.trim().is_empty(),
            "small render must not be empty"
        );
        assert!(
            !large_text.trim().is_empty(),
            "large render must not be empty"
        );

        // Larger render contains strictly more cells.
        let small_area = small.area().width as u32 * small.area().height as u32;
        let large_area = large.area().width as u32 * large.area().height as u32;
        assert!(
            large_area > small_area,
            "large area ({}) must exceed small area ({})",
            large_area,
            small_area
        );

        // No cell in either buffer contains a control character.
        // Uncleared / corrupted cells would surface here.
        for (label, buf) in [("small", &small), ("large", &large)] {
            for y in 0..buf.area().height {
                for x in 0..buf.area().width {
                    let cell = buf.cell((x, y)).unwrap();
                    let sym = cell.symbol();
                    assert!(
                        sym.is_empty() || sym.chars().all(|c| !c.is_control()),
                        "{} buffer: corrupted cell at ({},{}): {:?}",
                        label,
                        x,
                        y,
                        sym
                    );
                }
            }
        }
    }

    /// V11 ship-blocker: streaming output must survive a terminal resize
    /// mid-stream without panicking and produce internally-consistent
    /// buffers at every intermediate size.
    ///
    /// Real-world scenario: user is mid-prompt, the assistant is streaming
    /// a long answer into `TranscriptItem::AssistantPartial`, and the user
    /// (or their tiling WM) resizes the terminal. The render loop must:
    ///   1. Not panic on the layout math at any size.
    ///   2. Not leave control chars or partial UTF-8 in the buffer.
    ///   3. Keep at least *some* of the streamed content visible at the
    ///      final size (assumes the terminal is large enough to show one
    ///      line of transcript).
    ///
    /// We simulate the resize by rendering the SAME mutating state at three
    /// sizes — 80×24 (initial), 60×20 (shrunk), 100×30 (expanded) — while
    /// the AssistantPartial grows between each render. (P0 — doc 31 §6.1,
    /// doc 29 §4 V11)
    #[test]
    fn streaming_during_resize_no_crash() {
        let mut state = fresh_state();

        // Frame 1: short partial at 80×24. This is "stream just started".
        state.transcript.push(TranscriptItem::AssistantPartial(
            "STREAM-MARKER-A: lorem ipsum dolor sit amet, ".into(),
        ));
        let buf_a = render(&state, 80, 24);

        // Frame 2: partial has grown; terminal shrinks to 60×20 mid-stream.
        // The fact that we mutate the SAME state and re-render at a smaller
        // size is the resize scenario that historically panicked when the
        // Paragraph wrap math underflowed on tight widths.
        if let Some(TranscriptItem::AssistantPartial(buf)) = state.transcript.last_mut() {
            buf.push_str("consectetur adipiscing elit, sed do eiusmod tempor ");
            buf.push_str("incididunt ut labore et dolore magna aliqua. ");
        }
        let buf_b = render(&state, 60, 20);

        // Frame 3: partial has grown further; terminal expands to 100×30.
        // The previous render's cells must not leak through — Clear-on-
        // resize parity. Also includes a CJK char to surface any width
        // math regression that the earlier pure-ASCII frame would hide.
        if let Some(TranscriptItem::AssistantPartial(buf)) = state.transcript.last_mut() {
            buf.push_str("STREAM-MARKER-Z: ut enim 你好 ad minim veniam.");
        }
        let buf_c = render(&state, 100, 30);

        // ── Invariants across all three frames ───────────────────────
        for (label, buf) in [
            ("A 80x24", &buf_a),
            ("B 60x20", &buf_b),
            ("C 100x30", &buf_c),
        ] {
            // 1. Buffer must not be empty.
            let text = buffer_text(buf);
            assert!(
                !text.trim().is_empty(),
                "{}: render produced empty buffer — likely panicked/aborted internally",
                label,
            );

            // 2. No control character ever lands in a cell. Uncleared bytes
            //    from a previous render would surface as ESC/CR/etc. here.
            for y in 0..buf.area().height {
                for x in 0..buf.area().width {
                    let cell = buf.cell((x, y)).unwrap();
                    let sym = cell.symbol();
                    assert!(
                        sym.is_empty() || sym.chars().all(|c| !c.is_control()),
                        "{}: control char at ({},{}): {:?}",
                        label,
                        x,
                        y,
                        sym,
                    );
                }
            }
        }

        // 3. Final frame contains the latest streamed marker. If a panic
        //    earlier had been swallowed by ratatui's draw closure, this
        //    line would be missing.
        let text_c = buffer_text(&buf_c);
        assert!(
            text_c.contains("STREAM-MARKER-Z"),
            "final 100x30 render should show latest streamed text; got:\n{}",
            text_c,
        );
    }

    /// V11 ship-blocker: `/mouse on` and `/mouse off` must round-trip
    /// through the slash handler and leave `mouse_capture_pending` in
    /// the expected state so the main loop's "apply pending mouse-
    /// capture toggle" block (see ~line 836-848 in this file) flips
    /// the terminal backend on the next frame.
    ///
    /// Bug profile (caught by this test on 2026-05-18): the `/mouse`
    /// arm of `handle_tui_slash` previously held `app.lock()` for the
    /// entire `match arg { ... }` block, then called the `push` /
    /// `push_err` closures defined at the top of the function — which
    /// re-acquire the same `std::sync::Mutex` → deadlock. Users hit
    /// this every time they ran `/mouse on|off` and had to Ctrl-C the
    /// TUI. The accompanying fix scopes the lock to a small block that
    /// computes the message, drops the guard, then pushes. This test
    /// would hang indefinitely against the buggy code.
    ///
    /// Coverage matrix:
    ///   active=false + "on"  → pending = Some(true)   (turn on)
    ///   active=true  + "on"  → pending = None         (already on)
    ///   active=true  + "off" → pending = Some(false)  (turn off)
    ///   active=false + "off" → pending = None         (already off)
    ///   "status"             → pending = None         (read-only)
    ///   "garbage"            → pending = None + Error transcript entry
    ///
    /// (P0 — doc 31 §6.1, doc 29 §4 V11)
    #[tokio::test]
    async fn mouse_capture_toggle_round_trip() {
        // Use a private ConversationStore + temp dir so this test never
        // touches the user's real ~/.phantom-mesh. Same pattern as
        // `clear_slash_sets_pending_interrupt_flag` further down.
        let dir = std::env::temp_dir().join(format!("phantom_v11_mouse_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let conversations = crate::session::ConversationStore::new_with_dir(dir.clone());
        let runtime = crate::agent::AgentRuntime::new(crate::config::AgentsConfig::default());
        let cost = crate::cost::CostTracker::new();

        // ── Case 1: capture currently OFF, user runs `/mouse on` ──
        let app = wrap(fresh_state());
        assert!(!app.lock().unwrap().mouse_capture_active);
        assert!(app.lock().unwrap().mouse_capture_pending.is_none());
        handle_tui_slash(&app, &runtime, &conversations, &cost, "/mouse on").await;
        assert_eq!(
            app.lock().unwrap().mouse_capture_pending,
            Some(true),
            "`/mouse on` while OFF must arm pending=Some(true)",
        );

        // ── Case 2: capture already ON, `/mouse on` is a no-op for pending ──
        let app = wrap(fresh_state());
        app.lock().unwrap().mouse_capture_active = true;
        handle_tui_slash(&app, &runtime, &conversations, &cost, "/mouse on").await;
        assert!(
            app.lock().unwrap().mouse_capture_pending.is_none(),
            "`/mouse on` while already ON must not re-arm pending",
        );

        // ── Case 3: capture currently ON, user runs `/mouse off` ──
        let app = wrap(fresh_state());
        app.lock().unwrap().mouse_capture_active = true;
        handle_tui_slash(&app, &runtime, &conversations, &cost, "/mouse off").await;
        assert_eq!(
            app.lock().unwrap().mouse_capture_pending,
            Some(false),
            "`/mouse off` while ON must arm pending=Some(false)",
        );

        // ── Case 4: capture already OFF, `/mouse off` is a no-op ──
        let app = wrap(fresh_state());
        handle_tui_slash(&app, &runtime, &conversations, &cost, "/mouse off").await;
        assert!(
            app.lock().unwrap().mouse_capture_pending.is_none(),
            "`/mouse off` while already OFF must not arm pending",
        );

        // ── Case 5: `/mouse status` (and bare `/mouse`) must be read-only ──
        for variant in ["/mouse status", "/mouse"] {
            let app = wrap(fresh_state());
            app.lock().unwrap().mouse_capture_active = true;
            handle_tui_slash(&app, &runtime, &conversations, &cost, variant).await;
            assert!(
                app.lock().unwrap().mouse_capture_pending.is_none(),
                "{:?} must NOT mutate mouse_capture_pending",
                variant,
            );
            // And it should still leave the active flag untouched.
            assert!(
                app.lock().unwrap().mouse_capture_active,
                "{:?} must not flip mouse_capture_active directly",
                variant,
            );
        }

        // ── Case 6: unknown arg pushes an Error item, doesn't arm pending ──
        let app = wrap(fresh_state());
        handle_tui_slash(&app, &runtime, &conversations, &cost, "/mouse banana").await;
        assert!(
            app.lock().unwrap().mouse_capture_pending.is_none(),
            "garbage arg must not arm pending",
        );
        let pushed_error = matches!(
            app.lock().unwrap().transcript.last(),
            Some(TranscriptItem::Error(msg)) if msg.contains("banana"),
        );
        assert!(
            pushed_error,
            "garbage arg should surface as an Error transcript entry mentioning the arg",
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// V11 ship-blocker: CJK input must navigate correctly through
    /// multi-byte UTF-8 chars. `prev_char_boundary` / `next_char_boundary`
    /// are the underlying helpers; getting them wrong means the input
    /// cursor lands on partial bytes and ratatui's Paragraph wraps
    /// incorrectly. (P0 — doc 31 §6.1, doc 29 §4 V11)
    #[test]
    fn cjk_paragraph_width_correct() {
        // 4 CJK chars × 3 bytes each = 12 bytes total.
        let s = "你好世界";
        assert_eq!(s.len(), 12, "CJK string should be exactly 12 bytes");

        // Backward: end → start of last char → ... → 0.
        assert_eq!(
            prev_char_boundary(s, 12),
            9,
            "prev from byte 12 → 9 (start of 界)"
        );
        assert_eq!(
            prev_char_boundary(s, 10),
            9,
            "prev from mid-char snaps to char start"
        );
        assert_eq!(prev_char_boundary(s, 9), 6);
        assert_eq!(prev_char_boundary(s, 6), 3);
        assert_eq!(prev_char_boundary(s, 3), 0);

        // Forward: 0 → 3 → 6 → 9 → 12.
        assert_eq!(next_char_boundary(s, 0), 3);
        assert_eq!(next_char_boundary(s, 3), 6);
        assert_eq!(next_char_boundary(s, 6), 9);
        assert_eq!(next_char_boundary(s, 9), 12);
        // At end, next stays at end.
        assert_eq!(next_char_boundary(s, 12), 12);

        // Mixed ASCII + CJK: "a你b好" = 1+3+1+3 = 8 bytes.
        let s = "a你b好";
        assert_eq!(s.len(), 8);
        assert_eq!(next_char_boundary(s, 0), 1, "a→你 boundary");
        assert_eq!(next_char_boundary(s, 1), 4, "你→b boundary");
        assert_eq!(next_char_boundary(s, 4), 5, "b→好 boundary");
        assert_eq!(next_char_boundary(s, 5), 8, "好→end boundary");
        assert_eq!(prev_char_boundary(s, 8), 5);
        assert_eq!(prev_char_boundary(s, 5), 4);
        assert_eq!(prev_char_boundary(s, 4), 1);
        assert_eq!(prev_char_boundary(s, 1), 0);
    }

    #[test]
    fn slash_completions_filter_by_prefix() {
        let matches: Vec<&&str> = SLASH_COMMANDS_TUI
            .iter()
            .filter(|c| c.starts_with("/co"))
            .collect();
        assert!(
            matches.iter().any(|c| **c == "/cost"),
            "/co prefix should include /cost"
        );
        // Sanity: unrelated commands are excluded
        assert!(
            !matches.iter().any(|c| **c == "/help"),
            "/co should not include /help"
        );
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
            "Up arrow should bring the newest history entry into the input; got:\n{}",
            text,
        );
    }

    /// Drives the real `handle_key(Up)` path end-to-end so a regression
    /// in cursor/history wiring fails here instead of only being seen in
    /// the tmux selftest. Mirrors the selftest case: two history entries,
    /// press Up twice, expect input == oldest of the two.
    #[test]
    fn handle_key_up_arrow_recalls_history_end_to_end() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut state = fresh_state();
        state.history.push("older-entry".into());
        state.history.push("newer-entry".into());
        let app = wrap(state);

        let up = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        let _ = handle_key(&app, up);
        assert_eq!(
            app.lock().unwrap().input,
            "newer-entry",
            "first Up should pull the newest history entry into the input"
        );
        let _ = handle_key(&app, up);
        assert_eq!(
            app.lock().unwrap().input,
            "older-entry",
            "second Up should walk one entry further back"
        );
    }

    #[test]
    fn handle_key_esc_closes_open_pane_without_clearing_input() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut state = fresh_state();
        state.cost_view = true; // a full-body pane is open
        state.input = "half-typed".into();
        state.cursor = 10;
        let app = wrap(state);
        let action = handle_key(&app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        let s = app.lock().unwrap();
        assert!(!s.cost_view, "Esc must close the open pane (footer promises 'esc: back to chat')");
        // pane-close returns early, so the input line is NOT also cleared.
        assert_eq!(s.input, "half-typed", "closing a pane must not clobber the input");
        assert!(matches!(action, KeyAction::None));
    }

    #[test]
    fn handle_key_esc_clears_input_when_no_pane_open() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut state = fresh_state();
        state.input = "abc".into();
        state.cursor = 3;
        let app = wrap(state);
        let _ = handle_key(&app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        let s = app.lock().unwrap();
        assert_eq!(s.input, "", "with no pane open, Esc clears the input line (unchanged behavior)");
        assert_eq!(s.cursor, 0);
    }

    #[test]
    fn handle_key_esc_while_running_cancels_before_closing_pane() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut state = fresh_state();
        state.running = true; // an in-flight request
        state.cost_view = true; // ...with a pane also open
        let app = wrap(state);
        let action = handle_key(&app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        // Cancelling the request wins; the pane stays open (a 2nd Esc closes it).
        assert!(matches!(action, KeyAction::Cancel));
        assert!(
            app.lock().unwrap().cost_view,
            "while cancelling a request, Esc must not also close the pane"
        );
    }

    #[test]
    fn handle_key_goals_pane_arrows_move_selection_clamped() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut state = fresh_state();
        state.goals_view = true;
        state.goals_rows = vec![
            super::GoalRow { label: "L1".into(), text: "a".into(), done: false },
            super::GoalRow { label: "L2".into(), text: "b".into(), done: false },
        ];
        state.goals_selected = 0;
        let app = wrap(state);
        let down = || KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        let up = || KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        let _ = handle_key(&app, down());
        assert_eq!(app.lock().unwrap().goals_selected, 1, "Down moves selection");
        let _ = handle_key(&app, down()); // clamp at last row (len-1 = 1)
        assert_eq!(app.lock().unwrap().goals_selected, 1, "Down clamps at last row");
        let _ = handle_key(&app, up());
        let _ = handle_key(&app, up()); // clamp at 0
        assert_eq!(app.lock().unwrap().goals_selected, 0, "Up clamps at 0");
    }

    #[test]
    fn handle_key_goals_pane_space_marks_selected_done() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let _g = crate::env_lock::acquire();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("EVOLVE-GOALS.md");
        std::fs::write(&path, "## Pending\n- [ ] ship A\n- [ ] ship B\n\n## Done\n").unwrap();
        std::env::set_var("PHANTOM_EVOLVE_GOALS", &path);

        let mut state = fresh_state();
        state.goals_view = true;
        state.goals_rows = super::goal_rows_from_file(); // 2 pending rows
        state.goals_selected = 0; // select "ship A"
        let app = wrap(state);
        let _ = handle_key(&app, KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));

        // The file now has "ship A" checked; the reloaded rows reflect it.
        let on_disk = std::fs::read_to_string(&path).unwrap();
        std::env::remove_var("PHANTOM_EVOLVE_GOALS");
        assert!(on_disk.contains("[x]") && on_disk.contains("ship A"), "ship A marked done on disk:\n{on_disk}");
        let s = app.lock().unwrap();
        assert!(
            s.goals_rows.iter().any(|r| r.text.contains("ship A") && r.done),
            "reloaded rows show ship A as done: {:?}",
            s.goals_rows
        );
    }

    #[test]
    fn shift_date_handles_boundaries_and_bad_input() {
        assert_eq!(super::shift_date("2026-05-15", 1), "2026-05-16");
        assert_eq!(super::shift_date("2026-05-15", -1), "2026-05-14");
        assert_eq!(super::shift_date("2026-05-31", 1), "2026-06-01", "month rollover");
        assert_eq!(super::shift_date("2026-01-01", -1), "2025-12-31", "year rollover");
        assert_eq!(super::shift_date("2026-03-01", -1), "2026-02-28", "non-leap Feb");
        // Bad input → returned unchanged (no panic).
        assert_eq!(super::shift_date("not-a-date", 1), "not-a-date");
    }

    #[test]
    fn handle_key_review_pane_arrows_step_the_date() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut state = fresh_state();
        state.review_view = true;
        state.review_data = Some(super::ReviewView {
            date: "2026-05-15".into(),
            state: super::ReviewState::Empty,
            event_count: 0,
            rows: Vec::new(),
            flagged: false,
        });
        let app = wrap(state);
        let _ = handle_key(&app, KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(
            app.lock().unwrap().review_data.as_ref().unwrap().date,
            "2026-05-14",
            "Left steps the date back a day"
        );
        let _ = handle_key(&app, KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        let _ = handle_key(&app, KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(
            app.lock().unwrap().review_data.as_ref().unwrap().date,
            "2026-05-16",
            "Right steps the date forward"
        );
    }

    // MAC-CUJ02 TUI interactive lifecycle — drive a representative user journey
    // through the REAL handle_key dispatcher on a single AppState: type → edit
    // (Backspace / Ctrl-A / Ctrl-E / Ctrl-U) → switch agent (Tab) → submit
    // (Enter → KeyAction::Submit + history) → close a pane (Esc) → exit
    // (Ctrl-D on empty input). Complements the per-pane handle_key_* tests with
    // an end-to-end keystroke flow, asserting state transitions + return actions.
    #[test]
    fn handle_key_full_lifecycle_type_edit_switch_submit_exit() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let ch = |c: char| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
        let ctrl = |c: char| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
        let plain = |code: KeyCode| KeyEvent::new(code, KeyModifiers::NONE);

        let app = wrap(fresh_state());

        // 1. Type "helloX".
        for c in "helloX".chars() {
            assert!(matches!(handle_key(&app, ch(c)), KeyAction::None));
        }
        assert_eq!(app.lock().unwrap().input, "helloX");
        assert_eq!(app.lock().unwrap().cursor, 6);

        // 2. Backspace removes the stray 'X'.
        let _ = handle_key(&app, plain(KeyCode::Backspace));
        assert_eq!(app.lock().unwrap().input, "hello");
        assert_eq!(app.lock().unwrap().cursor, 5);

        // 3. Ctrl-A to line start, Ctrl-E back to end (cursor moves only).
        let _ = handle_key(&app, ctrl('a'));
        assert_eq!(app.lock().unwrap().cursor, 0, "Ctrl-A → line start");
        let _ = handle_key(&app, ctrl('e'));
        assert_eq!(app.lock().unwrap().cursor, 5, "Ctrl-E → line end");

        // 4. Ctrl-U deletes from cursor to start → empties the line.
        let _ = handle_key(&app, ctrl('u'));
        assert_eq!(app.lock().unwrap().input, "", "Ctrl-U cleared to line start");
        assert_eq!(app.lock().unwrap().cursor, 0);

        // 5. Tab cycles the active agent (master[0] → coder[1]).
        let before = app.lock().unwrap().agent_idx;
        let _ = handle_key(&app, plain(KeyCode::Tab));
        let after = app.lock().unwrap().agent_idx;
        assert_eq!(after, (before + 1) % AGENTS.len(), "Tab cycles agent on empty input");

        // 6. Type a prompt (with surrounding space) and submit with Enter.
        for c in "  do it  ".chars() {
            let _ = handle_key(&app, ch(c));
        }
        let action = handle_key(&app, plain(KeyCode::Enter));
        match action {
            KeyAction::Submit(p) => assert_eq!(p, "do it", "Enter submits the trimmed prompt"),
            other => panic!("Enter on a non-empty prompt must Submit, got {other:?}"),
        }
        {
            let s = app.lock().unwrap();
            assert_eq!(s.input, "", "submit clears the input line");
            assert_eq!(s.cursor, 0);
            assert_eq!(s.history.last().map(String::as_str), Some("do it"), "prompt recorded in history");
        }

        // 7. Open a pane, then Esc closes it (does not clear input / exit).
        app.lock().unwrap().goals_view = true;
        let action = handle_key(&app, plain(KeyCode::Esc));
        assert!(matches!(action, KeyAction::None));
        assert!(!app.lock().unwrap().goals_view, "Esc closes the open pane");

        // 8. Ctrl-D on an empty input line exits.
        assert!(app.lock().unwrap().input.is_empty());
        let action = handle_key(&app, ctrl('d'));
        assert!(matches!(action, KeyAction::Exit), "Ctrl-D on empty input exits");
    }

    #[test]
    fn events_dir_has_encrypted_detects_age_magic() {
        let dir = tempfile::tempdir().unwrap();
        let ev = dir.path().join("events");
        let plain = ev.join("evt-plain");
        std::fs::create_dir_all(&plain).unwrap();
        std::fs::write(plain.join("meta.json"), b"{\"event_id\":\"x\"}").unwrap();
        assert!(!super::events_dir_has_encrypted(&ev), "plaintext meta → false");
        let enc = ev.join("evt-enc");
        std::fs::create_dir_all(&enc).unwrap();
        std::fs::write(enc.join("meta.json"), b"age-encryption.org/v1\n<ciphertext>").unwrap();
        assert!(super::events_dir_has_encrypted(&ev), "age-magic meta → true");
    }

    #[test]
    fn review_view_for_empty_vs_locked() {
        // No events at all → Empty (NOT Locked — the bug was showing Locked
        // here / for plaintext events when no identity.key existed).
        let dir = tempfile::tempdir().unwrap();
        let v = super::review_view_for(dir.path(), "2026-05-29");
        assert_eq!(v.state, super::ReviewState::Empty, "no events → Empty");

        // A genuinely age-encrypted event + no identity.key → Locked.
        let enc = dir.path().join("events").join("evt-enc");
        std::fs::create_dir_all(&enc).unwrap();
        std::fs::write(enc.join("meta.json"), b"age-encryption.org/v1\nXX").unwrap();
        let v = super::review_view_for(dir.path(), "2026-05-29");
        assert_eq!(v.state, super::ReviewState::Locked, "encrypted + no key → Locked");
    }

    #[test]
    fn handle_key_focus_pane_i_logs_interruption_s_stops() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let _g = crate::env_lock::acquire();
        let home = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", home.path());
        // Start a real disk-backed session under the temp HOME.
        crate::life_node::focus_session::start(home.path(), 25, Some("test".into()), vec![])
            .unwrap();

        let mut state = fresh_state();
        state.focus_view = true;
        state.focus_data = super::focus_view_from_state(); // Some (active)
        let app = wrap(state);

        // 'i' → one interruption logged.
        let _ = handle_key(&app, KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
        let after_i = crate::life_node::focus_session::status(home.path());
        assert_eq!(
            after_i.map(|s| s.interruptions.len()),
            Some(1),
            "i logs an interruption"
        );

        // 's' → session ends (status now None).
        let _ = handle_key(&app, KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
        assert!(
            crate::life_node::focus_session::status(home.path()).is_none(),
            "s stops the session"
        );
        std::env::remove_var("HOME");
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
                "CJK glyph '{}' should appear in render; got:\n{}",
                ch,
                text,
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
            assert!(
                text.contains(needle),
                "{} missing from render:\n{}",
                needle,
                text
            );
        }
        // And they shouldn't all collapse onto one row — count rows that
        // contain a 'line ' prefix
        let line_rows = text
            .lines()
            .filter(|l| {
                l.contains("line one") || l.contains("line two") || l.contains("line three")
            })
            .count();
        assert!(
            line_rows >= 3,
            "expected each input line on its own row; got {} rows. Text:\n{}",
            line_rows,
            text,
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
        let rows_with_xs = text
            .lines()
            .filter(|line| line.chars().filter(|&c| c == 'x').count() > 30)
            .count();
        assert!(
            rows_with_xs >= 2,
            "Expected long input to wrap to >=2 visible rows; got {} rows of x's. Text:\n{}",
            rows_with_xs,
            text,
        );
    }

    #[test]
    fn renders_with_a_transcript_item() {
        let mut state = fresh_state();
        state
            .transcript
            .push(TranscriptItem::User("hi from test".into()));
        let buf = render(&state, 80, 24);
        let text = buffer_text(&buf);
        // We can't assume exact rendering of a User item without committing
        // to the format, but the literal substring should make it through.
        assert!(
            text.contains("hi from test"),
            "User transcript item should render somewhere; got:\n{}",
            text,
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
            spinner_col,
            phantom_col,
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
             Set PHANTOM_MAX_TOKENS=16384 and re-run."
                .into(),
        ));

        let buf = render(&state, 100, 24);
        let text = buffer_text(&buf);

        assert!(
            text.contains("⚠"),
            "warning glyph should appear in transcript; got:\n{}",
            text,
        );
        assert!(
            text.contains("Response truncated"),
            "warning message should appear in transcript; got:\n{}",
            text,
        );
        assert!(
            text.contains("PHANTOM_MAX_TOKENS"),
            "env var hint should be visible to the user; got:\n{}",
            text,
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
                "earlier line {} {}",
                i,
                "z".repeat(150),
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
             input frame; got:\n{}",
            text,
        );
        // And the input lines must also render — confirms layout split correctly.
        for needle in ["a", "b", "c", "d", "e"] {
            assert!(
                text.lines()
                    .any(|line| line.trim() == needle || line.contains(needle)),
                "input line {:?} should render; got:\n{}",
                needle,
                text,
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
                "earlier message {} {}",
                i,
                "x".repeat(200)
            )));
            state.transcript.push(TranscriptItem::Assistant(format!(
                "earlier reply  {} {}",
                i,
                "y".repeat(200)
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
             overflow; got:\n{}",
            text,
        );
    }

    /// Render two consecutive frames on the SAME backend. The live TUI's
    /// alt-screen buffer persists between draws, so cells not painted by
    /// the second frame still show the first frame's content. Reproduces
    /// the bleed-through bug; the fresh-backend `render()` helper above
    /// would mask it.
    fn render_two_frames(s1: &AppState, s2: &AppState, w: u16, h: u16) -> ratatui::buffer::Buffer {
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
            "BEGIN_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA_END".into(),
        ));

        let mut s2 = fresh_state();
        s2.transcript.push(TranscriptItem::Assistant("S".into()));

        let buf = render_two_frames(&s1, &s2, 80, 24);
        let text = buffer_text(&buf);

        assert!(
            !text.contains("AAAAAAAA"),
            "stale wide content from frame 1 leaked into frame 2; got:\n{}",
            text,
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
            C:\\Users\\you\\\n\
            ├── gpu_convolution.py\n\
            ├── gpu_output_input.png\n\
            └── gpu_output\\\n\
                ├── 模糊.png\n\
                ├── 銳化.png\n\
                └── 拉普拉斯.png\n\
            ```\n\
            \n\
            END-OF-MARKDOWN-XYZQ-7732"
            .to_string();

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
        let mut b = ratatui::buffer::Buffer::empty(ratatui::layout::Rect::new(
            0,
            0,
            width,
            lines.len() as u16,
        ));
        for (row, line) in lines.iter().enumerate() {
            // Buffer::set_string lays out display-width-aware (CJK = 2).
            b.set_string(0, row as u16, *line, Style::default());
        }
        b
    }

    fn sel(anchor: (u16, u16), cursor: (u16, u16)) -> Selection {
        Selection {
            anchor,
            cursor,
            dragging: false,
        }
    }

    #[test]
    fn extract_single_row_selection_returns_substring() {
        let buf = make_buf(20, &["hello world here"]);
        let s = sel((6, 0), (10, 0)); // "world"
        assert_eq!(extract_selection_text(&buf, s), "world");
    }

    #[test]
    fn extract_multi_row_selection_joins_with_newlines_and_trims_pad() {
        let buf = make_buf(
            20,
            &["first row aaaaaaaa", "middle row bbb", "last row cccccc"],
        );
        let s = sel((6, 0), (8, 2)); // from col 6 of row 0 to col 8 of row 2
        let got = extract_selection_text(&buf, s);
        // Row 0: "row aaaaaaaa" (col 6..end, trimmed of trailing pad)
        // Row 1: "middle row bbb" (full, trimmed)
        // Row 2: "last row " (col 0..=8, trimmed of trailing space → "last row")
        let expected = "row aaaaaaaa\nmiddle row bbb\nlast row";
        assert_eq!(
            got, expected,
            "multi-row selection text doesn't match. got:\n{}\nexpected:\n{}",
            got, expected
        );
    }

    #[test]
    fn extract_handles_anchor_below_cursor_normalizing_order() {
        // User dragged from bottom-right UP to top-left. Selection.normalized
        // should swap them so extraction still goes top→bottom.
        let buf = make_buf(20, &["row zero", "row one"]);
        let s = sel((4, 1), (0, 0)); // anchor=lower, cursor=upper
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
        let s1 = sel((5, 3), (6, 3)); // 1 cell right
        let s2 = sel((5, 3), (5, 4)); // 1 row down
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
        assert!(
            matches!(action, KeyAction::None),
            "first idle Ctrl-C must NOT exit; got {:?}",
            std::mem::discriminant(&action)
        );
        let s = app.lock().unwrap();
        assert!(
            s.last_ctrl_c_at.is_some(),
            "first press must arm the window"
        );
        let last_msg = s.transcript.last();
        assert!(
            matches!(last_msg, Some(TranscriptItem::System(m)) if m.contains("again")),
            "expected hint transcript entry, got {:?}",
            last_msg
        );
    }

    #[test]
    fn second_ctrl_c_within_window_exits() {
        let app = wrap(fresh_state());
        let _ = handle_key(&app, ctrl_c_event());
        let action = handle_key(&app, ctrl_c_event());
        assert!(
            matches!(action, KeyAction::Exit),
            "second Ctrl-C inside the window must Exit"
        );
    }

    #[test]
    fn first_ctrl_c_running_cancels() {
        let mut s = fresh_state();
        s.running = true;
        let app = wrap(s);
        let action = handle_key(&app, ctrl_c_event());
        assert!(
            matches!(action, KeyAction::Cancel),
            "first Ctrl-C while running must Cancel, not Exit"
        );
        assert!(app.lock().unwrap().last_ctrl_c_at.is_some());
    }

    #[test]
    fn unrelated_key_disarms_double_tap() {
        let app = wrap(fresh_state());
        let _ = handle_key(&app, ctrl_c_event());
        // Type a single char — should disarm.
        let _ = handle_key(&app, KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        assert!(
            app.lock().unwrap().last_ctrl_c_at.is_none(),
            "an unrelated keystroke must clear the arming window"
        );
        let action = handle_key(&app, ctrl_c_event());
        assert!(
            matches!(action, KeyAction::None),
            "after disarm, next Ctrl-C is treated as a fresh first press"
        );
    }

    #[test]
    fn ctrl_c_with_meaningful_selection_copies_not_arms() {
        let mut s = fresh_state();
        s.selection = Some(sel((1, 1), (5, 1)));
        let app = wrap(s);
        let action = handle_key(&app, ctrl_c_event());
        assert!(
            matches!(action, KeyAction::CopySelection(_)),
            "selection-priority Ctrl-C must copy"
        );
        // Selection-copy path must NOT arm the exit window — otherwise a
        // user copying twice could quit.
        assert!(
            app.lock().unwrap().last_ctrl_c_at.is_none(),
            "copy path must not arm the exit window"
        );
    }

    /// **WIN P0 / V11 / doc 32 §3** — pin the full Ctrl-C double-tap-to-exit
    /// contract along its time axis. The previous tests cover the "second
    /// press inside the window exits" and "first idle press warns" cases
    /// but never exercise the 2-second threshold itself: a stale first
    /// press from > 2 s ago must NOT combine with a fresh second press
    /// to exit. We can't wait 2 s in a unit test (and shouldn't rely on
    /// real wall-clock), so we splice the `last_ctrl_c_at` field directly
    /// with `Instant::checked_sub` to simulate elapsed time.
    ///
    /// Asserts, in order:
    /// 1. Two presses with a sub-window gap → Exit on the second press.
    /// 2. Two presses with a > 2 s gap → second press does NOT exit
    ///    (the stale window has expired and the second press becomes a
    ///    fresh "first press" that warns and re-arms).
    /// 3. After (2), a third press still inside the new window → Exit.
    #[test]
    fn ctrl_c_double_tap_exits() {
        // Case 1: rapid double-tap exits.
        let app = wrap(fresh_state());
        let _ = handle_key(&app, ctrl_c_event());
        let action = handle_key(&app, ctrl_c_event());
        assert!(
            matches!(action, KeyAction::Exit),
            "two Ctrl-C inside the 2 s window must Exit; got {:?}",
            std::mem::discriminant(&action)
        );

        // Case 2: stale first press does NOT carry over past the window.
        // Splice the timestamp to "3 s ago" without sleeping.
        let app = wrap(fresh_state());
        let _ = handle_key(&app, ctrl_c_event());
        {
            let mut s = app.lock().unwrap();
            let stale = Instant::now()
                .checked_sub(Duration::from_secs(3))
                .expect("clock should not be at the epoch in a test");
            s.last_ctrl_c_at = Some(stale);
        }
        let action = handle_key(&app, ctrl_c_event());
        assert!(
            matches!(action, KeyAction::None),
            "Ctrl-C 3 s after the first press must NOT exit \
             (stale window expired); got {:?}",
            std::mem::discriminant(&action)
        );

        // …but the press in case 2 should itself re-arm the window.
        let armed_at = {
            let s = app.lock().unwrap();
            s.last_ctrl_c_at.expect("re-armed window must be Some")
        };
        assert!(
            armed_at.elapsed() < Duration::from_secs(2),
            "re-arming press must set last_ctrl_c_at to ~now, not the stale value"
        );

        // Case 3: fresh second press inside the new window exits.
        let action = handle_key(&app, ctrl_c_event());
        assert!(
            matches!(action, KeyAction::Exit),
            "after a re-arm, a prompt second press must Exit; got {:?}",
            std::mem::discriminant(&action)
        );
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

    use rand::rngs::StdRng;
    use rand::{Rng, SeedableRng};

    /// Non-character KeyCodes worth randomly throwing at handle_key.
    /// Excluding KeyCode::Char (which we generate separately) and the
    /// less-common variants (Modifier, Media, etc.) that aren't in the
    /// realistic input distribution.
    fn fuzz_keycodes() -> &'static [KeyCode] {
        &[
            KeyCode::Backspace,
            KeyCode::Enter,
            KeyCode::Tab,
            KeyCode::BackTab,
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::Home,
            KeyCode::End,
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Insert,
            KeyCode::Delete,
            KeyCode::Esc,
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
            KeyModifiers::SHIFT | KeyModifiers::ALT,
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
                0 => rng.gen_range(0x4E00u32..0x9FFFu32),   // CJK Unified
                1 => rng.gen_range(0x1F600u32..0x1F64Fu32), // emoticons
                _ => rng.gen_range(0x0080u32..0x07FFu32),   // Latin-ext + Greek + Cyrillic
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
            String::from("a".repeat(10_000)), // very long
            String::from("中".repeat(5_000)), // long CJK
            String::from("a\nb\nc\nd\ne"),    // multi-line
        ];
        for input in weird_inputs {
            for cursor_offset in 0..=input.len() {
                // Skip cursor positions mid-codepoint — handle_key trusts
                // the cursor invariant. We only verify no panic at valid
                // boundaries.
                if !input.is_char_boundary(cursor_offset) {
                    continue;
                }
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
            (1u16, 1u16), // 1×1 — degenerate
            (1, 100),     // 1 wide × 100 tall
            (200, 1),     // 200 wide × 1 tall
            (40, 12),     // smallest practical
            (200, 500),   // huge
            (300, 80),    // ultrawide
            (10, 200),    // narrow phone-ish
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
                1 => TranscriptItem::System(format!(
                    "system line {} with some longer content to exercise wrapping",
                    i
                )),
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
            &huge_line, // single very-long line
        ];
        for s in &stressors {
            state
                .transcript
                .push(TranscriptItem::Assistant((*s).to_string()));
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
        state
            .transcript
            .push(TranscriptItem::Assistant("test row 0".into()));
        state
            .transcript
            .push(TranscriptItem::Assistant("中文 row 1".into()));
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

    // ── Issue #71: tui-history file-size guard ────────────────────────────

    #[test]
    fn load_tui_history_under_limit_ok() {
        // A small, well-formed history file loads its lines normally —
        // the size guard must not regress the happy path.
        use std::io::Write as _;
        let dir = std::env::temp_dir();
        let path = dir.join("phantom_t49_history_small.txt");
        {
            let mut f = std::fs::File::create(&path).unwrap();
            writeln!(f, "first prompt").unwrap();
            writeln!(f, "second prompt").unwrap();
            writeln!(f, "third prompt").unwrap();
        }

        let entries = load_tui_history_from(&path);
        assert_eq!(
            entries,
            vec![
                "first prompt".to_string(),
                "second prompt".to_string(),
                "third prompt".to_string(),
            ]
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_tui_history_over_limit_graceful() {
        // Build a sparse file whose metadata reports a size larger than
        // MAX_TUI_HISTORY_BYTES, without actually writing those bytes.
        // The loader must skip the load entirely (returning an empty Vec)
        // rather than panicking from an unbounded fs::read.
        let dir = std::env::temp_dir();
        let path = dir.join("phantom_t49_history_huge.txt");
        {
            let f = std::fs::File::create(&path).unwrap();
            // 1 byte over the limit — exercises the strict ">" guard.
            f.set_len(MAX_TUI_HISTORY_BYTES + 1).unwrap();
        }

        let entries = load_tui_history_from(&path);
        assert!(
            entries.is_empty(),
            "over-limit history file should yield empty Vec, not panic; got {} entries",
            entries.len()
        );

        let _ = std::fs::remove_file(&path);
    }

    // ── V12 HIGH (TUI-2): tui-history file perms ──────────────────────────

    /// On Unix, `OpenOptions::create(true)` honours the process umask, so a
    /// stock `0o022` umask lands the file at `0o644` — world-readable.
    /// Other accounts on the same host could then read the user's prompt
    /// history (which routinely contains pasted credentials, internal URLs,
    /// or in-flight secrets). `append_tui_history_to` must chmod the file
    /// to `0o600` immediately after creating it. Mirrors the equivalent
    /// guarantee in `core/src/auth.rs` for `auth.json`.
    #[cfg(unix)]
    #[test]
    fn tui_history_file_is_chmod_0600_on_create() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir();
        let path = dir.join("phantom_d3_tui2_history_perms.txt");
        let _ = std::fs::remove_file(&path); // start clean

        append_tui_history_to(&path, "secret-prompt-marker");

        let meta = std::fs::metadata(&path).expect("history file should exist after append");
        let mode = meta.permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "tui-history must be owner-only (0o600); got 0o{:o}",
            mode,
        );

        let _ = std::fs::remove_file(&path);
    }

    /// Migration check: a pre-existing 0o644 history file (left over from
    /// an older phantom build before this fix) gets retightened the next
    /// time we append to it. Catches a regression where the chmod only
    /// fires on the very first create.
    #[cfg(unix)]
    #[test]
    fn tui_history_chmod_retightens_existing_loose_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir();
        let path = dir.join("phantom_d3_tui2_history_migrate.txt");
        let _ = std::fs::remove_file(&path);

        // Seed a loose (0o644) file as if an old build had created it.
        std::fs::write(&path, "older-history-line\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o644,
            "pre-condition: file starts at 0o644",
        );

        append_tui_history_to(&path, "new-line-after-fix");

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "append on a pre-existing 0o644 file should retighten to 0o600; got 0o{:o}",
            mode,
        );

        let _ = std::fs::remove_file(&path);
    }

    /// Compaction also runs `fs::write`, which on some platforms (and on
    /// any file recreated out-of-band) can land at the umask-default
    /// perms. `maybe_compact_tui_history_at` must therefore re-chmod
    /// after every write.
    #[cfg(unix)]
    #[test]
    fn tui_history_compaction_preserves_0600() {
        use std::io::Write as _;
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir();
        let path = dir.join("phantom_d3_tui2_history_compact.txt");
        let _ = std::fs::remove_file(&path);

        // Produce >TUI_HISTORY_MAX*2 lines so compaction actually fires.
        {
            let mut f = std::fs::File::create(&path).unwrap();
            for i in 0..(TUI_HISTORY_MAX * 2 + 5) {
                writeln!(f, "line-{}", i).unwrap();
            }
        }
        // Start at 0o644 to simulate a stale-perms file pre-fix.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        maybe_compact_tui_history_at(&path);

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "compaction should leave history at 0o600; got 0o{:o}",
            mode,
        );

        let _ = std::fs::remove_file(&path);
    }

    // ── TUI-1 (V12 HIGH): /clear and /resume must cancel current_task ────
    //
    // Bug: when a user issues `/clear` or `/resume` while the agent is mid-
    // stream, the slash handler used to mutate session state but leave the
    // running JoinHandle alive. Tokens from session A then streamed into
    // session B's transcript, and `conv.append(...)` saved an interrupted
    // turn into the WRONG conversation file. The fix routes through a new
    // `AppState::pending_interrupt` flag that the slash handler sets and
    // the run_loop's top-of-iteration block (0z) honours by firing
    // `InterruptHandle::interrupt(None)` and `JoinHandle::abort()`.

    /// Spawn a long-running fake "agent task" and verify that the same
    /// cancellation sequence run_loop executes on `pending_interrupt`
    /// stops it within a single 50 ms iteration (one frame budget at
    /// 30 fps is 33 ms; we allow 100 ms slack for CI flake). Mirrors the
    /// real run_loop path verbatim: cooperative interrupt → abort →
    /// drain channel → clear running flag.
    #[tokio::test]
    async fn pending_interrupt_cancels_inflight_task_within_one_frame() {
        let interrupt = InterruptHandle::new();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<UiEvent>();

        // Fake streaming task that races the interrupt: emits a token
        // every 10 ms forever unless cancelled. This is the structural
        // analog of the AgentRuntime callback path (see spawn_agent_turn
        // in run_loop) so the test is testing the cancellation contract,
        // not the agent internals.
        let ih = interrupt.clone();
        let handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = ih.cancelled() => return,
                    _ = tokio::time::sleep(std::time::Duration::from_millis(10)) => {
                        let _ = tx.send(UiEvent::Token("tok".into()));
                    }
                }
            }
        });

        // Give the task a chance to emit a few tokens, then simulate the
        // slash handler setting the flag.
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        let app = wrap(fresh_state());
        {
            let mut s = app.lock().unwrap();
            s.pending_interrupt = Some(());
            s.running = true;
        }

        // ── Replay run_loop's 0z block verbatim ─────────────────────
        let take_pending = {
            let mut s = app.lock().unwrap();
            s.pending_interrupt.take()
        };
        assert!(take_pending.is_some(), "/clear path must set the flag");
        interrupt.interrupt(None);
        handle.abort();
        while rx.try_recv().is_ok() {}
        {
            let mut s = app.lock().unwrap();
            s.running = false;
        }

        // Within one frame, the task must be done (either completed via
        // the cancelled() select arm, or aborted hard). Without the
        // fix, the task ran forever and the await below times out.
        let stopped = tokio::time::timeout(std::time::Duration::from_millis(100), handle).await;
        assert!(
            stopped.is_ok(),
            "task must finish within one frame after pending_interrupt fires"
        );
        assert!(
            !app.lock().unwrap().running,
            "running flag must be cleared after cancellation"
        );
        // No leaked tokens reach the UI after the drain.
        assert!(
            rx.try_recv().is_err(),
            "channel must be drained so post-cancel tokens don't leak into new session"
        );
    }

    /// Direct assertion that the `/clear` slash handler sets
    /// pending_interrupt. Uses a private ConversationStore in a temp dir
    /// so the test doesn't touch the user's real ~/.phantom-mesh.
    #[tokio::test]
    async fn clear_slash_sets_pending_interrupt_flag() {
        let dir = std::env::temp_dir().join(format!("phantom_tui1_clear_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let conversations = crate::session::ConversationStore::new_with_dir(dir.clone());
        let runtime = crate::agent::AgentRuntime::new(crate::config::AgentsConfig::default());
        let cost = crate::cost::CostTracker::new();
        let app = wrap(fresh_state());

        // Sanity: not set yet.
        assert!(app.lock().unwrap().pending_interrupt.is_none());

        handle_tui_slash(&app, &runtime, &conversations, &cost, "/clear").await;

        assert!(
            app.lock().unwrap().pending_interrupt.is_some(),
            "/clear must set pending_interrupt so run_loop kills the in-flight task"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Symmetric coverage for `/resume` — same bug profile: tokens from
    /// the previous session would bleed into the newly-resumed one.
    #[tokio::test]
    async fn resume_slash_sets_pending_interrupt_flag() {
        let dir = std::env::temp_dir().join(format!("phantom_tui1_resume_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        // Seed a session file so /resume has a target to switch to.
        let target_id = "tui1-target-session";
        std::fs::write(
            dir.join(format!("{}.jsonl", target_id)),
            "{\"role\":\"user\",\"content\":\"hi\"}\n{\"role\":\"assistant\",\"content\":\"hello\"}\n",
        ).unwrap();

        let conversations = crate::session::ConversationStore::new_with_dir(dir.clone());
        let runtime = crate::agent::AgentRuntime::new(crate::config::AgentsConfig::default());
        let cost = crate::cost::CostTracker::new();
        let app = wrap(fresh_state());

        assert!(app.lock().unwrap().pending_interrupt.is_none());

        handle_tui_slash(
            &app,
            &runtime,
            &conversations,
            &cost,
            &format!("/resume {}", target_id),
        )
        .await;

        let s = app.lock().unwrap();
        assert!(
            s.pending_interrupt.is_some(),
            "/resume must set pending_interrupt so run_loop kills the in-flight task"
        );
        assert_eq!(s.chat_id, target_id, "/resume should have switched chat_id");
        drop(s);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **WIN P0 / V11 / doc 31 §6.1, doc 29 §4 V11** — Tab on an empty
    /// input must cycle through the configured agent roster
    /// (`master → coder → reviewer → researcher → master …`).
    ///
    /// The TUI exposes `AGENTS: &[&str]` (a 4-entry compile-time roster)
    /// plus `AppState.agent_idx`. The Tab handler in `handle_key` falls
    /// through to `s.cycle_agent()` whenever the current input token does
    /// NOT start with `/` (slash-command completion) or `@` (file/peer
    /// completion). Cycling must:
    ///   1. Advance `agent_idx` by exactly 1 per press, and
    ///   2. Wrap from the last agent back to index 0 — i.e. pressing Tab
    ///      `AGENTS.len()` times returns to the starting agent.
    ///
    /// This pins the keybinding so a regression that, e.g., made Tab a
    /// no-op when input was empty or skipped agents would fail here. We
    /// drive the real `handle_key(KeyCode::Tab)` path end-to-end (not
    /// `cycle_agent()` directly) so the dispatch logic is exercised too.
    #[test]
    fn tab_cycles_through_4_agents() {
        // Sanity-check the compile-time roster — if someone changes
        // AGENTS in future this test should fail loudly rather than
        // silently asserting against the wrong N.
        assert_eq!(
            AGENTS.len(),
            4,
            "expected 4 configured agents (master/coder/reviewer/researcher); \
             roster changed to {:?} — update test name + assertion if intentional",
            AGENTS
        );
        assert_eq!(
            AGENTS,
            &["master", "coder", "reviewer", "researcher"],
            "agent roster drifted: {:?}",
            AGENTS
        );

        let app = wrap(fresh_state());
        let tab = KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE);

        // Starts on agent 0 (master).
        assert_eq!(
            app.lock().unwrap().agent_idx,
            0,
            "fresh state must start on agent 0"
        );
        assert_eq!(app.lock().unwrap().agent_name(), "master");

        // One Tab advances to agent 1 (coder).
        let _ = handle_key(&app, tab);
        assert_eq!(
            app.lock().unwrap().agent_idx,
            1,
            "one Tab on empty input must advance to agent 1; got {}",
            app.lock().unwrap().agent_idx
        );
        assert_eq!(app.lock().unwrap().agent_name(), "coder");

        // Three more Tabs walk through reviewer → researcher → wrap to master.
        for expected in [2usize, 3, 0] {
            let _ = handle_key(&app, tab);
            let got = app.lock().unwrap().agent_idx;
            assert_eq!(
                got,
                expected,
                "Tab cycle: expected agent_idx {}, got {} (current agent: {:?})",
                expected,
                got,
                app.lock().unwrap().agent_name()
            );
        }
        // After AGENTS.len() (= 4) presses total, we must be back where we started.
        assert_eq!(
            app.lock().unwrap().agent_idx,
            0,
            "full cycle must wrap to 0; 4 Tab presses landed on {}",
            app.lock().unwrap().agent_idx
        );
        assert_eq!(app.lock().unwrap().agent_name(), "master");

        // One more Tab proves the cycle continues past the wrap (not a one-shot).
        let _ = handle_key(&app, tab);
        assert_eq!(
            app.lock().unwrap().agent_idx,
            1,
            "cycling past wrap must continue advancing; got {}",
            app.lock().unwrap().agent_idx
        );
    }

    // ── Bug A: provider-error render leak ───────────────────────────────
    //
    // A provider failure surfaces as a `TranscriptItem::Error(text)`. Real
    // provider errors are (a) long single lines (a 429 body inlined into one
    // message) and (b) genuinely multi-line (the upstream JSON blob appended
    // after a `\n`). The invariant Bug A violates: NO rendered row may be
    // wider than the pane — error text must wrap/clip within the transcript
    // pane, never leak into the right margin or over adjacent rows. Asserted
    // at a normal width (80) and a narrow width (40), using the same
    // fresh-backend `render()` + `buffer_text()` harness the other render
    // tests use (full `ui()` draw → in-memory Buffer).
    fn provider_error_text() -> String {
        // (a) a realistic long single line (~300 chars, no newlines) AND
        // (b) a multi-line error with an embedded `\n` (the upstream blob).
        let long_single = format!(
            "provider groq failed: 429 Too Many Requests — rate limited, retry in 6000ms; {}",
            "x".repeat(300)
        );
        format!(
            "{}\n  upstream: {{\"error\":{{\"code\":\"rate_limit_exceeded\",\"message\":\"Rate limit reached for model llama-3.3-70b-versatile on tokens per minute (TPM): Limit 6000, Used 5999, Requested 1200. Please try again in 6.012s. Visit https://console.groq.com/docs/rate-limits for more information.\"}}}}",
            long_single
        )
    }

    #[test]
    fn transcript_provider_error_never_overflows_width() {
        for &(w, h) in &[(80u16, 24u16), (40u16, 24u16)] {
            let mut state = fresh_state();
            state
                .transcript
                .push(super::TranscriptItem::Error(provider_error_text()));
            let buf = render(&state, w, h);
            let txt = buffer_text(&buf);
            for (n, line) in txt.lines().enumerate() {
                // Every buffer row is exactly `w` cells; `buffer_text` joins
                // rows with '\n', so any horizontal leak shows up as a line
                // whose char-count exceeds `w`.
                assert!(
                    line.chars().count() <= w as usize,
                    "Bug A: provider-error transcript line {n} leaks past pane width {w}: {:?}",
                    line
                );
            }
        }
    }

    #[test]
    fn transcript_multiple_provider_errors_never_overflow_width() {
        // Two stacked errors (a failover that tried two providers) — each must
        // stay inside the pane; the second must not inherit a leaked margin.
        for &(w, h) in &[(80u16, 30u16), (40u16, 30u16)] {
            let mut state = fresh_state();
            state
                .transcript
                .push(super::TranscriptItem::Error(provider_error_text()));
            state.transcript.push(super::TranscriptItem::Error(format!(
                "provider openrouter failed: 502 Bad Gateway — {}",
                "y".repeat(280)
            )));
            let buf = render(&state, w, h);
            let txt = buffer_text(&buf);
            for (n, line) in txt.lines().enumerate() {
                assert!(
                    line.chars().count() <= w as usize,
                    "Bug A: stacked provider-error line {n} leaks past pane width {w}: {:?}",
                    line
                );
            }
        }
    }

    // ===================================================================
    // Chat-screen pane scenario tests (P2 coverage). These drive the full
    // `ui(f, &state)` render through the shared `render`/`buffer_text`
    // helpers and assert on substrings that the pane render fns actually
    // emit (verified by reading render_status / render_input /
    // render_sidebar / render_transcript bodies before writing these).
    // ===================================================================

    #[test]
    fn status_bar_shows_active_agent_name() {
        // render_status emits "agent: " + s.agent_name(); agent_idx selects
        // from AGENTS = [master, coder, reviewer, researcher]. Each index
        // must surface its own name in the top bar.
        for (idx, expected) in AGENTS.iter().enumerate() {
            let mut state = fresh_state();
            state.agent_idx = idx;
            let text = buffer_text(&render(&state, 100, 24));
            assert!(
                text.contains("agent: "),
                "status bar should label the active agent; got:\n{}",
                text,
            );
            assert!(
                text.contains(*expected),
                "status bar should show agent '{}' for agent_idx {}; got:\n{}",
                expected,
                idx,
                text,
            );
        }
    }

    #[test]
    fn status_bar_shows_version_and_brand() {
        // The status bar always prints "phantom v{CARGO_PKG_VERSION}".
        let state = fresh_state();
        let text = buffer_text(&render(&state, 100, 24));
        let expected = format!("phantom v{}", env!("CARGO_PKG_VERSION"));
        assert!(
            text.contains(&expected),
            "status bar should show '{}'; got:\n{}",
            expected,
            text,
        );
    }

    #[test]
    fn status_bar_shows_session_cost() {
        // render_status emits "cost: ${:.4} session" from s.session_cost.
        let mut state = fresh_state();
        state.session_cost = 1.2345;
        let text = buffer_text(&render(&state, 120, 24));
        assert!(
            text.contains("cost: $1.2345 session"),
            "status bar should show formatted session cost; got:\n{}",
            text,
        );
    }

    #[test]
    fn status_bar_reflects_running_vs_idle() {
        // While running the bar appends " · streaming…"; idle it does not.
        let mut running = fresh_state();
        running.running = true;
        let running_text = buffer_text(&render(&running, 120, 24));
        assert!(
            running_text.contains("streaming"),
            "running status bar should show a streaming indicator; got:\n{}",
            running_text,
        );

        let idle = fresh_state();
        let idle_text = buffer_text(&render(&idle, 120, 24));
        assert!(
            !idle_text.contains("streaming"),
            "idle status bar must NOT show the streaming indicator; got:\n{}",
            idle_text,
        );
    }

    #[test]
    fn input_pane_empty_shows_keybinding_hint() {
        // Empty input renders the placeholder hint line. Both the English
        // and CJK translations start with the literal "Enter", so anchor on
        // that to stay language-independent.
        let state = fresh_state();
        let text = buffer_text(&render(&state, 100, 24));
        assert!(
            text.contains("Enter"),
            "empty input pane should show the keybinding hint; got:\n{}",
            text,
        );
    }

    #[test]
    fn input_pane_shows_typed_text() {
        // Non-empty input is rendered verbatim inside the input box.
        let mut state = fresh_state();
        state.input = "hello agent".into();
        state.cursor = state.input.len();
        let text = buffer_text(&render(&state, 100, 24));
        assert!(
            text.contains("hello agent"),
            "input pane should echo typed text; got:\n{}",
            text,
        );
    }

    #[test]
    fn input_pane_long_text_never_overflows_width() {
        // A single very long ASCII line must wrap inside the bordered input
        // box rather than leaking past the pane width, at narrow + wide.
        for &w in &[40u16, 80u16] {
            let mut state = fresh_state();
            state.input = "x".repeat(400);
            state.cursor = state.input.len();
            let text = buffer_text(&render(&state, w, 24));
            for (n, line) in text.lines().enumerate() {
                assert!(
                    line.chars().count() <= w as usize,
                    "input line {n} leaks past pane width {w}: {:?}",
                    line,
                );
            }
        }
    }

    #[test]
    fn sidebar_hidden_when_not_visible() {
        // sidebar_visible=false → ui() renders only the transcript; the
        // sidebar's " targets " title must be absent.
        let mut state = fresh_state();
        state.sidebar_visible = false;
        let text = buffer_text(&render(&state, 120, 24));
        assert!(
            !text.contains("targets"),
            "sidebar title must not appear when sidebar_visible=false; got:\n{}",
            text,
        );
    }

    #[test]
    fn sidebar_visible_lists_agents_and_title() {
        // sidebar_visible=true at a wide-enough terminal (>= 62 cols) shows
        // the " targets " title and every local agent name.
        let mut state = fresh_state();
        state.sidebar_visible = true;
        let text = buffer_text(&render(&state, 120, 24));
        assert!(
            text.contains("targets"),
            "visible sidebar should show its title; got:\n{}",
            text,
        );
        for name in AGENTS {
            assert!(
                text.contains(name),
                "visible sidebar should list agent '{}'; got:\n{}",
                name,
                text,
            );
        }
    }

    #[test]
    fn sidebar_lists_remote_peers() {
        // Peers from sidebar_peers render in the REMOTE section.
        let mut state = fresh_state();
        state.sidebar_visible = true;
        state.sidebar_peers = vec!["z13-node".into(), "mac-mini".into()];
        let text = buffer_text(&render(&state, 120, 24));
        assert!(
            text.contains("z13-node") && text.contains("mac-mini"),
            "visible sidebar should list remote peers; got:\n{}",
            text,
        );
    }

    #[test]
    fn sidebar_active_agent_gets_marker_glyph() {
        // The active agent (agent_idx) is prefixed with the "◆" marker in
        // the sidebar list. A non-default index proves it tracks agent_idx.
        let mut state = fresh_state();
        state.sidebar_visible = true;
        state.agent_idx = 1; // coder
        let text = buffer_text(&render(&state, 120, 24));
        assert!(
            text.contains("◆"),
            "active-agent marker glyph should appear in sidebar; got:\n{}",
            text,
        );
    }

    #[test]
    fn transcript_renders_mixed_item_kinds_together() {
        // System + Error + Assistant items must all surface in the body, each
        // with its distinguishing glyph (✗ for Error, ● for Assistant).
        let mut state = fresh_state();
        state
            .transcript
            .push(TranscriptItem::System("session restored".into()));
        state
            .transcript
            .push(TranscriptItem::Assistant("here is the answer".into()));
        state
            .transcript
            .push(TranscriptItem::Error("provider timed out".into()));
        let text = buffer_text(&render(&state, 100, 24));
        assert!(
            text.contains("session restored"),
            "System item text should render; got:\n{}",
            text,
        );
        assert!(
            text.contains("here is the answer") && text.contains("●"),
            "Assistant item + glyph should render; got:\n{}",
            text,
        );
        assert!(
            text.contains("provider timed out") && text.contains("✗"),
            "Error item + glyph should render; got:\n{}",
            text,
        );
    }

    #[test]
    fn transcript_empty_does_not_panic() {
        // An empty transcript is the launch state; rendering must not panic
        // and the chrome (input hint) is still present.
        let state = fresh_state();
        let text = buffer_text(&render(&state, 80, 24));
        assert!(
            text.contains("Enter"),
            "empty-transcript launch screen should still show input hint; got:\n{}",
            text,
        );
    }
}
