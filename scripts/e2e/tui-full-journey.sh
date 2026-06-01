#!/usr/bin/env bash
# tui-full-journey.sh — G-E2E-2: drive the REAL `phantom tui` through a full
# Mac-terminal user journey in a REAL pseudo-terminal (tmux), capturing the
# rendered frame at every step + asserting each pane actually opened and the
# screen stayed intact (no overflow / no escape leak / box present).
#
# Why a PTY: `phantom tui` bails unless `stdin().is_terminal()`, and the pane-
# opening slash commands run in the async run_loop (handle_tui_slash) — NOT in
# the in-process handle_key dispatcher — so the only way to exercise the real
# interactive journey is to drive the real binary in a real terminal. tmux gives
# us that PTY plus `capture-pane` to read back the actually-rendered cells.
#
# Journey (each step: send keys → settle → capture frame → assert):
#   00 startup            → framed screen drew
#   01 /habits            → habits pane on   (transcript: "habits pane on" / 習慣面板)
#   02 /focus             → focus pane on    (transcript: "focus pane on"  / 專注面板)
#   03 /review            → review pane on   (transcript: "review pane"     / 回顧面板)
#   04 Esc                → back to chat (panes closed)
# Every captured frame is also checked for: no line wider than the pane (display
# width), no raw ESC byte in the cells, and box borders intact.
#
# This is L2.5 (real PTY, real binary, real render) and ADDS to the headless
# cargo tui_render_tests + full-lifecycle-mac.sh (non-interactive CLI).
#
# Usage:
#   scripts/e2e/tui-full-journey.sh
#   PHANTOM_BIN=/path/to/phantom KEEP=1 COLS=120 ROWS=40 scripts/e2e/tui-full-journey.sh
#
# Honesty: missing tmux / failed build / a pane that never opens / a corrupted
# frame all FAIL loudly (exit!=0). screencapture PNGs are best-effort (a
# backgrounded tmux session has no on-screen window, so they may be blank — the
# authoritative evidence is the capture-pane text frames).
set -uo pipefail

COLS="${COLS:-100}"
ROWS="${ROWS:-30}"
SESSION="phantom-tui-journey-$$"
TS="$(date +%Y%m%d-%H%M%S)"
OUT="${TMPDIR:-/tmp}/phantom-tui-journey-$TS"
mkdir -p "$OUT"
FRAMES="$OUT/frames"; mkdir -p "$FRAMES"
SHOTS="$OUT/shots"; mkdir -p "$SHOTS"
RUN_HOME="$OUT/home"; mkdir -p "$RUN_HOME/.phantom-mesh"

note() { printf '%s\n' "$*"; }
fail() { note "✗ FAIL: $*"; finish 1; }

cleanup() { tmux kill-session -t "$SESSION" 2>/dev/null || true; }
finish() {
  local rc="$1"; cleanup
  if [ "${KEEP:-0}" = "1" ] || [ "$rc" -ne 0 ]; then note "artifacts kept in: $OUT"; else rm -rf "$OUT"; fi
  exit "$rc"
}
trap 'cleanup' EXIT INT TERM

# ── Preconditions ───────────────────────────────────────────────────────────
command -v tmux >/dev/null 2>&1 || fail "tmux not found — required for the PTY (brew install tmux)"

BIN="${PHANTOM_BIN:-}"
if [ -z "$BIN" ]; then
  REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
  # cargo runs from core/ (the crate root — there is NO root workspace), so the
  # binary lands in core/target/, NOT REPO_ROOT/target. Pointing at the latter
  # would run a stale/absent binary after a fresh build (codex review F2).
  BIN="$REPO_ROOT/core/target/debug/phantom"
  if [ ! -x "$BIN" ]; then
    note "building phantom (debug) ..."
    ( cd "$REPO_ROOT/core" && cargo build --bin phantom ) || fail "cargo build --bin phantom failed"
  fi
fi
[ -x "$BIN" ] || fail "phantom binary not found/executable at $BIN"
note "bin:  $BIN"
note "home: $RUN_HOME (isolated)"
note "pty:  tmux ${COLS}x${ROWS}"

# Clean child env (no provider key needed — this journey is local panes only).
UNSET_KEYS="GROQ_API_KEY ANTHROPIC_API_KEY OPENAI_API_KEY GEMINI_API_KEY GOOGLE_API_KEY OPENROUTER_API_KEY MISTRAL_API_KEY DEEPSEEK_API_KEY PHANTOM_MESH_GROQ_API_KEY PHANTOM_MESH_ANTHROPIC_API_KEY PHANTOM_MESH_GEMINI_API_KEY"
UNSET_ARGS=""
for k in $UNSET_KEYS; do UNSET_ARGS="$UNSET_ARGS -u $k"; done

fails=0

# Measure DISPLAY width (not bytes): box-drawing + CJK are multi-byte.
maxwidth() {
  python3 -c "
import sys,unicodedata
m=0
for ln in open(sys.argv[1], encoding='utf-8', errors='replace'):
    ln=ln.rstrip('\n')
    w=sum(2 if unicodedata.east_asian_width(c) in ('W','F') else 1 for c in ln)
    if w>m: m=w
print(m)
" "$1" 2>/dev/null
}

# capture <NN-label>  → grab the frame + a best-effort screenshot.
capture() {
  local label="$1"
  tmux capture-pane -t "$SESSION" -p > "$FRAMES/$label.txt" 2>/dev/null || true
  screencapture -x "$SHOTS/$label.png" 2>/dev/null || true   # best-effort
}

# assert_frame <NN-label>  → no-overflow + no-ESC + box intact on that frame.
assert_frame() {
  local label="$1" f="$FRAMES/$1.txt"
  [ -f "$f" ] || { note "  ✗ $label: no frame captured"; fails=$((fails+1)); return; }
  local w; w="$(maxwidth "$f")"
  if [ -n "$w" ] && [ "$w" -gt "$COLS" ]; then
    note "  ✗ $label: line $w cells > $COLS — horizontal leak"; fails=$((fails+1))
  fi
  if LC_ALL=C grep -q $'\x1b' "$f"; then
    note "  ✗ $label: raw ESC byte in rendered cells — escape leak"; fails=$((fails+1))
  fi
  if ! grep -q '[─│╭╮╰╯┌┐└┘║═]' "$f"; then
    note "  ✗ $label: box borders gone — screen corrupted"; fails=$((fails+1))
  fi
}

# assert_contains <NN-label> <regex> <human>  → the step's expected text showed.
assert_contains() {
  local label="$1" pat="$2" human="$3" f="$FRAMES/$1.txt"
  if grep -qE "$pat" "$f" 2>/dev/null; then
    note "  ✓ $label: $human"
  else
    note "  ✗ $label: expected $human — not found"; fails=$((fails+1))
  fi
}

# ── Launch ──────────────────────────────────────────────────────────────────
# shellcheck disable=SC2086
tmux new-session -d -s "$SESSION" -x "$COLS" -y "$ROWS" \
  "env $UNSET_ARGS HOME='$RUN_HOME' PHANTOM_LOG=error '$BIN' tui 2>'$OUT/tui.stderr'"

drew=0
for _ in $(seq 1 50); do
  sleep 0.2
  tmux capture-pane -t "$SESSION" -p > "$FRAMES/00-startup.txt" 2>/dev/null || true
  if grep -q '[─│╭╮╰╯┌┐└┘║═]' "$FRAMES/00-startup.txt" 2>/dev/null \
     || grep -q 'phantom ·' "$FRAMES/00-startup.txt" 2>/dev/null; then
    drew=1; break
  fi
done
[ "$drew" -eq 1 ] || fail "TUI never drew in 10s (see $OUT/tui.stderr + $FRAMES/00-startup.txt)"
screencapture -x "$SHOTS/00-startup.png" 2>/dev/null || true
note "✓ TUI drew its first frame"

# ── Journey ─────────────────────────────────────────────────────────────────
step() { # step <NN-label> <keys-to-send>
  local label="$1" keys="$2"
  tmux send-keys -t "$SESSION" "$keys" Enter
  sleep 1.5
  capture "$label"
}

# Close whatever pane is up before opening the next. This mirrors real usage AND
# is REQUIRED: ui() renders panes by a fixed priority (review > … > focus, tui.rs
# ~2287), and a slash command opening a second pane while one is already up does
# NOT switch the visible pane — the user must Esc back to chat first. The
# real-PTY journey surfaced this (opening /review while /focus was up left the
# focus pane on screen); Esc-between-panes is the correct interaction.
close_pane() { tmux send-keys -t "$SESSION" Escape; sleep 0.8; }

step "01-habits" "/habits"
close_pane
step "02-focus"  "/focus"
close_pane
step "03-review" "/review"

# Esc → close the last pane, back to chat.
tmux send-keys -t "$SESSION" Escape
sleep 1
capture "04-after-esc"

# ── Assertions ──────────────────────────────────────────────────────────────
note ""
note "── assertions ──"
# Every frame structurally sound.
for lbl in 00-startup 01-habits 02-focus 03-review 04-after-esc; do
  assert_frame "$lbl"
done
note "  (structure: no-overflow / no-ESC / box-intact checked on all 5 frames)"
# Each pane actually opened — assert on the box TITLE in the captured frame
# (the pane replaces the body, so the pushed transcript line is covered; the
# box title `┌ phantom · <pane>` is the visible proof). Titles are locale-
# dependent (this run is zh-Hant): habits→"習慣", focus→"focus", review→"回顧".
assert_contains "01-habits" "phantom · 習慣|phantom · habits"   "/habits opened the habits pane (title)"
assert_contains "02-focus"  "phantom · focus|phantom · 專注"    "/focus opened the focus pane (title)"
assert_contains "03-review" "phantom · daily review"           "/review opened the review pane (title)"
# Back-to-chat after the final Esc: pane title gone, chat status line shown.
assert_contains "04-after-esc" "agent: master|phantom v0"        "Esc returned to the chat screen"

# Quit cleanly (Ctrl-D on empty input → Exit).
tmux send-keys -t "$SESSION" C-d 2>/dev/null || true
sleep 0.5

note ""
if [ "$fails" -eq 0 ]; then
  note "TUI-JOURNEY RESULT: PASS (real PTY ${COLS}x${ROWS}; frames in $FRAMES)"
  finish 0
else
  note "TUI-JOURNEY RESULT: FAIL ($fails assertion(s) failed)"
  note "frames: $FRAMES   tui stderr: $OUT/tui.stderr"
  finish 1
fi
