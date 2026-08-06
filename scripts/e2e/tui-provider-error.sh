#!/usr/bin/env bash
# tui-provider-error.sh — G-E2E-4 / Bug A: drive the REAL TUI in a REAL
# pseudo-terminal (tmux) and capture the rendered frames, so we can catch the
# provider-error "render leak" that the headless ratatui TestBackend tests
# CANNOT reach.
#
# Why a PTY: `spectyn tui` bails unless `stdin().is_terminal()` (tui.rs), so a
# piped-stdin test can't drive it. tmux gives a genuine PTY + a real screen we
# can read back with `tmux capture-pane` (the actual rendered cells, after
# ratatui has written escape sequences to the terminal — exactly the layer where
# Bug A would manifest as overflow / stray escapes / broken box borders).
#
# Bug A hypothesis: when a provider call fails, the error text leaks past its
# pane (long unwrapped line, or raw ANSI in an upstream error body) and corrupts
# the screen. We trigger it deterministically: an isolated HOME with NO provider
# key, then submit a prompt → the agent's fallback chain exhausts → an Error
# transcript item is rendered. We then assert the screen invariants.
#
# This is L2.5 of the pyramid (real PTY, real binary, real render) — it ADDS to
# the cargo render tests + full-lifecycle-mac.sh; it does not replace them.
#
# Usage:
#   scripts/e2e/tui-provider-error.sh                 # builds debug binary if needed
#   SPECTYN_BIN=/path/to/spectyn scripts/e2e/tui-provider-error.sh
#   KEEP=1 scripts/e2e/tui-provider-error.sh          # keep HOME + frames + tmux log
#   COLS=120 ROWS=40 scripts/e2e/tui-provider-error.sh
#
# Honesty: if tmux is missing, or the binary won't build, or the TUI never draws
# a frame, the script FAILs loudly (exit!=0) rather than skipping silently.
set -uo pipefail

COLS="${COLS:-100}"
ROWS="${ROWS:-30}"
SESSION="spectyn-tui-bugA-$$"
TS="$(date +%Y%m%d-%H%M%S)"
OUT="${TMPDIR:-/tmp}/spectyn-tui-bugA-$TS"
mkdir -p "$OUT"
FRAMES="$OUT/frames"; mkdir -p "$FRAMES"
RUN_HOME="$OUT/home"; mkdir -p "$RUN_HOME/.spectyn-mesh"

note() { printf '%s\n' "$*"; }
fail() { note "✗ FAIL: $*"; finish 1; }

cleanup() {
  tmux kill-session -t "$SESSION" 2>/dev/null || true
}
finish() {
  local rc="$1"
  cleanup
  if [ "${KEEP:-0}" = "1" ] || [ "$rc" -ne 0 ]; then
    note "artifacts kept in: $OUT"
  else
    rm -rf "$OUT"
  fi
  exit "$rc"
}
trap 'cleanup' EXIT INT TERM

# ── Preconditions ───────────────────────────────────────────────────────────
command -v tmux >/dev/null 2>&1 || fail "tmux not found — required for the PTY (brew install tmux)"

BIN="${SPECTYN_BIN:-}"
if [ -z "$BIN" ]; then
  REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
  # cargo runs from core/ (the crate root — there is NO root workspace), so the
  # binary lands in core/target/, NOT REPO_ROOT/target. Pointing at the latter
  # would run a stale/absent binary after a fresh build (codex review F2).
  BIN="$REPO_ROOT/core/target/debug/spectyn"
  if [ ! -x "$BIN" ]; then
    note "building spectyn (debug) ..."
    ( cd "$REPO_ROOT/core" && cargo build --bin spectyn ) || fail "cargo build --bin spectyn failed"
  fi
fi
[ -x "$BIN" ] || fail "spectyn binary not found/executable at $BIN"
note "bin:  $BIN"
note "home: $RUN_HOME (isolated, no provider key)"
note "pty:  tmux ${COLS}x${ROWS}"

# Scrub every provider key from the child env so the fallback chain is GUARANTEED
# to exhaust → provider error. (We pass a clean env into the tmux session.)
UNSET_KEYS="GROQ_API_KEY ANTHROPIC_API_KEY OPENAI_API_KEY GEMINI_API_KEY GOOGLE_API_KEY OPENROUTER_API_KEY MISTRAL_API_KEY DEEPSEEK_API_KEY SPECTYN_MESH_GROQ_API_KEY SPECTYN_MESH_ANTHROPIC_API_KEY SPECTYN_MESH_GEMINI_API_KEY"
UNSET_ARGS=""
for k in $UNSET_KEYS; do UNSET_ARGS="$UNSET_ARGS -u $k"; done

# ── Launch the TUI in a real PTY ────────────────────────────────────────────
# `env -u <keys> HOME=... spectyn tui` inside tmux. tmux allocates the PTY so
# is_terminal() is true and the full-screen ratatui app runs for real.
# shellcheck disable=SC2086
tmux new-session -d -s "$SESSION" -x "$COLS" -y "$ROWS" \
  "env $UNSET_ARGS HOME='$RUN_HOME' SPECTYN_LOG=error '$BIN' tui 2>'$OUT/tui.stderr'"

# Wait for the first real frame. The full layout draws a box-drawing border, but
# at very narrow widths ratatui may render borderless — so also accept the brand
# status line ("spectyn ·") as proof the app drew. Either means we're live.
drew=0
for _ in $(seq 1 50); do          # up to ~10s
  sleep 0.2
  tmux capture-pane -t "$SESSION" -p > "$FRAMES/00-startup.txt" 2>/dev/null || true
  if grep -q '[─│╭╮╰╯┌┐└┘║═]' "$FRAMES/00-startup.txt" 2>/dev/null \
     || grep -q 'spectyn ·' "$FRAMES/00-startup.txt" 2>/dev/null; then
    drew=1; break
  fi
done
[ "$drew" -eq 1 ] || fail "TUI never drew in 10s (see $OUT/tui.stderr + $FRAMES/00-startup.txt)"
note "✓ TUI drew its first frame"

# ── Trigger the provider-error path: type a prompt + Enter ──────────────────
tmux send-keys -t "$SESSION" "hello there" Enter
# Give the fallback chain time to try every provider and surface the error.
sleep 3
tmux capture-pane -t "$SESSION" -p > "$FRAMES/01-after-submit.txt" 2>/dev/null || true
# A second capture a moment later (catch any late re-render / scroll).
sleep 2
tmux capture-pane -t "$SESSION" -p > "$FRAMES/02-settled.txt" 2>/dev/null || true
note "✓ submitted a prompt with no provider key; captured frames"

# ── Assertions on the REAL rendered frames ──────────────────────────────────
fails=0

# (1) No horizontal leak: every rendered line must fit within COLS. tmux pads
#     lines to the pane width, so a line LONGER than COLS means content escaped
#     its region — the core Bug A symptom.
check_no_overflow() {
  local f="$1"
  [ -f "$f" ] || return 0
  # Measure DISPLAY width, not bytes: the TUI's box-drawing chars (─│┌┐) are
  # 3 UTF-8 bytes each and CJK glyphs are 2 cells / 3 bytes, so a byte count
  # (awk `length`) wildly over-reports. Use east-asian-width so the result is
  # the real terminal-cell width tmux laid the line out at.
  local maxlen
  maxlen="$(python3 -c "
import sys,unicodedata
m=0
for ln in open(sys.argv[1], encoding='utf-8', errors='replace'):
    ln=ln.rstrip('\n')
    w=sum(2 if unicodedata.east_asian_width(c) in ('W','F') else 1 for c in ln)
    if w>m: m=w
print(m)
" "$f" 2>/dev/null)"
  if [ -z "$maxlen" ]; then
    note "  ⚠ $(basename "$f"): could not measure width (python3 missing?) — skipping overflow check"
    return 0
  fi
  if [ "$maxlen" -gt "$COLS" ]; then
    note "  ✗ $f: a line is $maxlen cells wide (> $COLS) — horizontal leak"
    fails=$((fails+1))
  else
    note "  ✓ $(basename "$f"): widest line $maxlen ≤ $COLS cells"
  fi
}

# (2) No raw escape sequences in the captured cells. capture-pane returns
#     rendered text; a stray ESC (0x1b) means an escape leaked into content.
check_no_raw_escape() {
  local f="$1"
  [ -f "$f" ] || return 0
  if LC_ALL=C grep -q $'\x1b' "$f"; then
    note "  ✗ $(basename "$f"): raw ESC byte in rendered cells (escape leak)"
    fails=$((fails+1))
  fi
}

# (3) Screen structure intact: the bordered box is still present in the settled
#     frame (a corrupted screen typically loses its borders).
check_box_intact() {
  local f="$1"
  [ -f "$f" ] || { note "  ✗ missing frame $f"; fails=$((fails+1)); return; }
  if grep -q '[─│╭╮╰╯┌┐└┘║═]' "$f"; then
    note "  ✓ $(basename "$f"): box borders still present (structure intact)"
  else
    note "  ✗ $(basename "$f"): box borders gone — screen corrupted"
    fails=$((fails+1))
  fi
}

# (4) The error was actually surfaced (proves we exercised the provider-error
#     render path, not just an idle screen). Accept the EN or ZH wording.
check_error_surfaced() {
  local f="$1"
  if grep -qiE "provider|金鑰|api key|failed|/login|登入|error" "$f" 2>/dev/null; then
    note "  ✓ provider-error / no-key messaging surfaced (render path exercised)"
  else
    # codex review F4: this is the whole point of the test (proving the
    # provider-error render path ran) — a miss must FAIL, not just warn.
    note "  ✗ no provider-error text found in $(basename "$f") — render path NOT exercised"
    fails=$((fails+1))
  fi
}

note ""
note "── assertions ──"
for f in "$FRAMES"/*.txt; do
  check_no_overflow "$f"
  check_no_raw_escape "$f"
done
check_box_intact "$FRAMES/02-settled.txt"
check_error_surfaced "$FRAMES/02-settled.txt"

# Quit the TUI cleanly (Ctrl-D on empty input → Exit).
tmux send-keys -t "$SESSION" C-d 2>/dev/null || true
sleep 0.5

note ""
if [ "$fails" -eq 0 ]; then
  note "TUI-BUGA RESULT: PASS (real PTY ${COLS}x${ROWS}; frames in $FRAMES)"
  finish 0
else
  note "TUI-BUGA RESULT: FAIL ($fails assertion(s) failed)"
  note "frames: $FRAMES   tui stderr: $OUT/tui.stderr"
  finish 1
fi
