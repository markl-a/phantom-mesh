#!/usr/bin/env bash
# Comprehensive TUI input behaviour test.
#
# Each test launches a fresh spectyn in tmux, sends keystrokes, captures the
# pane, and reports OK / FAIL with the captured row and cursor position.
# Goal: surface remaining layout/edit bugs end-to-end.

set -o pipefail

PASS=0; FAIL=0; FAIL_LINES=()
TMP=$(mktemp -d)

green()  { printf "\033[32m%s\033[0m" "$1"; }
red()    { printf "\033[31m%s\033[0m" "$1"; }
gray()   { printf "\033[90m%s\033[0m" "$1"; }
bold()   { printf "\033[1m%s\033[0m" "$1"; }

ok()   { PASS=$((PASS+1)); printf "  $(green '✓') %-50s %s\n" "$1" "$(gray "$2")"; }
fail() { FAIL=$((FAIL+1)); FAIL_LINES+=("$1 :: $2"); printf "  $(red '✗') %-50s %s\n" "$1" "$(gray "$2")"; }

# Run a spectyn session, send keystrokes, return the captured input row + cursor_x.
# Args: <name> <description> <keystrokes...>
# Special: TYPE:str, KEY:keyname, WAIT:secs
run_test() {
  local name="$1"; shift
  local desc="$1"; shift
  local sess="phantest-$$-$RANDOM"
  tmux new-session -d -s "$sess" -x 80 -y 25 spectyn 2>/dev/null
  sleep 1.5
  while [ $# -gt 0 ]; do
    case "$1" in
      "TYPE:"*) tmux send-keys -t "$sess" -l "${1#TYPE:}" ;;
      "KEY:"*)  tmux send-keys -t "$sess" "${1#KEY:}" ;;
      "WAIT:"*) sleep "${1#WAIT:}" ;;
    esac
    sleep 0.12
    shift
  done
  sleep 0.3
  TMUX_CAP=$(tmux capture-pane -t "$sess" -p)
  TMUX_INPUT_ROW=$(echo "$TMUX_CAP" | grep -E '^│' | head -1)
  TMUX_INPUT_ROW2=$(echo "$TMUX_CAP" | grep -E '^│' | head -2 | tail -1)
  TMUX_TITLE=$(echo "$TMUX_CAP" | grep -E '^┌' | head -1)
  TMUX_CURSOR=$(tmux display-message -t "$sess" -p '#{cursor_x},#{cursor_y}')
  tmux kill-session -t "$sess" 2>/dev/null
  echo "$TMUX_CAP" > "$TMP/${name}.txt"
}

section() { printf "\n$(bold "%s")\n" "$1"; }

# ─── basic input ──────────────────────────────────────────────────────────
section "1. basic input"

run_test "ascii-typing" "type 'hello'" "TYPE:hello"
[[ "$TMUX_INPUT_ROW" == *"hello"* && "${TMUX_CURSOR%%,*}" == "6" ]] \
  && ok "ascii typing renders" "cursor_x=6" \
  || fail "ascii typing renders" "got: $TMUX_INPUT_ROW cursor=$TMUX_CURSOR"

run_test "cjk-typing" "type 你好世界 (4 CJK chars)" "TYPE:你好世界"
[[ "$TMUX_INPUT_ROW" == *"你好世界"* && "${TMUX_CURSOR%%,*}" == "9" ]] \
  && ok "CJK typing cursor at col 9" "cursor_x=9 (8 cells + 1 border)" \
  || fail "CJK typing cursor at col 9" "got: $TMUX_INPUT_ROW cursor=$TMUX_CURSOR"

run_test "mixed-typing" "type 'spectyn 是個 AI'" "TYPE:spectyn 是個 AI"
[[ "$TMUX_INPUT_ROW" == *"spectyn 是個 AI"* ]] \
  && ok "mixed ASCII+CJK typing" "" \
  || fail "mixed ASCII+CJK typing" "got: $TMUX_INPUT_ROW"

# ─── cursor mid-text editing ──────────────────────────────────────────────
section "2. cursor mid-text editing"

run_test "left-then-insert" "hello, Left x2, type X" "TYPE:hello" "KEY:Left" "KEY:Left" "TYPE:X"
[[ "$TMUX_INPUT_ROW" == *"helXlo"* ]] \
  && ok "left-then-insert produces helXlo" "" \
  || fail "left-then-insert produces helXlo" "got: $TMUX_INPUT_ROW"

run_test "left-cjk-boundary" "你好, Left x1 — cursor between 你 and 好" "TYPE:你好" "KEY:Left"
[[ "${TMUX_CURSOR%%,*}" == "3" ]] \
  && ok "Left from CJK end → cursor at col 3" "cursor between 你 and 好" \
  || fail "Left from CJK end → cursor at col 3" "got cursor=$TMUX_CURSOR (expected 3)"

run_test "left-twice-cjk-boundary" "你好, Left x2 — cursor at start" "TYPE:你好" "KEY:Left" "KEY:Left"
[[ "${TMUX_CURSOR%%,*}" == "1" ]] \
  && ok "Left twice → cursor at col 1" "" \
  || fail "Left twice → cursor at col 1" "got cursor=$TMUX_CURSOR"

run_test "insert-cjk-mid" "abc, Left, type 你" "TYPE:abc" "KEY:Left" "TYPE:你"
[[ "$TMUX_INPUT_ROW" == *"ab你c"* ]] \
  && ok "insert CJK mid-text" "" \
  || fail "insert CJK mid-text" "got: $TMUX_INPUT_ROW"

# ─── backspace / delete ───────────────────────────────────────────────────
section "3. backspace / delete"

run_test "backspace-cjk-end" "你好世界, BSpace x1" "TYPE:你好世界" "KEY:BSpace"
[[ "$TMUX_INPUT_ROW" == *"你好世"* && "$TMUX_INPUT_ROW" != *"你好世界"* ]] \
  && ok "Backspace deletes whole CJK char" "" \
  || fail "Backspace deletes whole CJK char" "got: $TMUX_INPUT_ROW"

run_test "backspace-mid" "hello, Left x2, BSpace" "TYPE:hello" "KEY:Left" "KEY:Left" "KEY:BSpace"
[[ "$TMUX_INPUT_ROW" == *"helo"* && "$TMUX_INPUT_ROW" != *"hello"* ]] \
  && ok "Backspace mid-text removes char before cursor" "" \
  || fail "Backspace mid-text removes char before cursor" "got: $TMUX_INPUT_ROW"

run_test "backspace-empty" "BSpace on empty input (no-op)" "KEY:BSpace"
[[ "${TMUX_CURSOR%%,*}" == "1" ]] \
  && ok "Backspace on empty is no-op" "cursor stays at col 1" \
  || fail "Backspace on empty is no-op" "got cursor=$TMUX_CURSOR"

run_test "delete-mid" "hello, Left x3, Delete (remove 'l')" "TYPE:hello" "KEY:Left" "KEY:Left" "KEY:Left" "KEY:Delete"
[[ "$TMUX_INPUT_ROW" == *"helo"* ]] \
  && ok "Delete forward-removes char at cursor" "" \
  || fail "Delete forward-removes char at cursor" "got: $TMUX_INPUT_ROW"

# ─── home / end / ctrl-a / ctrl-e ─────────────────────────────────────────
section "4. line motion (Home/End/Ctrl-A/Ctrl-E)"

run_test "home-key" "hello, Home" "TYPE:hello" "KEY:Home"
[[ "${TMUX_CURSOR%%,*}" == "1" ]] \
  && ok "Home jumps cursor to col 1" "" \
  || fail "Home jumps cursor to col 1" "got cursor=$TMUX_CURSOR"

run_test "end-key" "hello, Home, End" "TYPE:hello" "KEY:Home" "KEY:End"
[[ "${TMUX_CURSOR%%,*}" == "6" ]] \
  && ok "End jumps cursor to col 6" "" \
  || fail "End jumps cursor to col 6" "got cursor=$TMUX_CURSOR"

run_test "ctrl-a" "hello, Ctrl-A" "TYPE:hello" "KEY:C-a"
[[ "${TMUX_CURSOR%%,*}" == "1" ]] \
  && ok "Ctrl-A jumps to start" "" \
  || fail "Ctrl-A jumps to start" "got cursor=$TMUX_CURSOR"

run_test "ctrl-e" "hello, Ctrl-A, Ctrl-E" "TYPE:hello" "KEY:C-a" "KEY:C-e"
[[ "${TMUX_CURSOR%%,*}" == "6" ]] \
  && ok "Ctrl-E jumps to end" "" \
  || fail "Ctrl-E jumps to end" "got cursor=$TMUX_CURSOR"

# ─── word ops ─────────────────────────────────────────────────────────────
section "5. word motion / Ctrl-W"

run_test "ctrl-w" "hello world, Ctrl-W" "TYPE:hello world" "KEY:C-w"
[[ "$TMUX_INPUT_ROW" == *"hello "* && "$TMUX_INPUT_ROW" != *"world"* ]] \
  && ok "Ctrl-W deletes last word" "" \
  || fail "Ctrl-W deletes last word" "got: $TMUX_INPUT_ROW"

run_test "ctrl-l" "hello, Ctrl-L (clear input)" "TYPE:hello" "KEY:C-l"
[[ "$TMUX_INPUT_ROW" != *"hello"* ]] \
  && ok "Ctrl-L clears input" "" \
  || fail "Ctrl-L clears input" "got: $TMUX_INPUT_ROW"

# ─── multi-line ───────────────────────────────────────────────────────────
section "6. multi-line input"

run_test "ctrl-j-newline" "line1, Ctrl-J, line2" "TYPE:line1" "KEY:C-j" "TYPE:line2"
if echo "$TMUX_CAP" | grep -qE '^│line1' && echo "$TMUX_CAP" | grep -qE '^│line2'; then
  ok "Ctrl-J creates new visual row" ""
else
  fail "Ctrl-J creates new visual row" "$TMUX_INPUT_ROW | $TMUX_INPUT_ROW2"
fi

run_test "ml-backspace-join" "line1, Ctrl-J, BSpace (join lines)" "TYPE:line1" "KEY:C-j" "KEY:BSpace"
# After Ctrl+J then BSpace, the \n should be removed and we're back to line1
if echo "$TMUX_CAP" | grep -qE '^│line1' && ! echo "$TMUX_CAP" | grep -qE '^│line2'; then
  # Should also have only 1 content row now, second row blank
  blank_rows=$(echo "$TMUX_CAP" | grep -c -E '^│ +│$' || true)
  ok "BSpace at line start joins lines" ""
else
  fail "BSpace at line start joins lines" "got: $TMUX_INPUT_ROW | $TMUX_INPUT_ROW2"
fi

# ─── esc / cancel ─────────────────────────────────────────────────────────
section "7. Esc behaviour"

run_test "esc-clears-input" "hello, Esc" "TYPE:hello" "KEY:Escape"
[[ "$TMUX_INPUT_ROW" != *"hello"* ]] \
  && ok "Esc clears input when not running" "" \
  || fail "Esc clears input when not running" "got: $TMUX_INPUT_ROW"

# ─── slash commands (no LLM call) ─────────────────────────────────────────
section "8. slash commands"

run_test "slash-help" "/help, Enter" "TYPE:/help" "KEY:Enter" "WAIT:1"
echo "$TMUX_CAP" | grep -qE '/cost|/exit|/help' \
  && ok "/help prints command list" "" \
  || fail "/help prints command list" "see $TMP/slash-help.txt"

run_test "slash-co-tab" "/co + Tab" "TYPE:/co" "KEY:Tab"
echo "$TMUX_CAP" | grep -qE '/cost|/copy|/compact' \
  && ok "/co<Tab> shows completions" "" \
  || fail "/co<Tab> shows completions" "got: $TMUX_INPUT_ROW"

run_test "slash-cost" "/cost, Enter" "TYPE:/cost" "KEY:Enter" "WAIT:1"
echo "$TMUX_CAP" | grep -qiE 'cost|tokens|spent|session|0\.|\$' \
  && ok "/cost prints cost summary" "" \
  || fail "/cost prints cost summary" "see $TMP/slash-cost.txt"

run_test "slash-unknown" "/notarealcommand" "TYPE:/notarealcommand" "KEY:Enter" "WAIT:1"
echo "$TMUX_CAP" | grep -qiE 'unknown|/help' \
  && ok "unknown slash → 'unknown command'" "" \
  || fail "unknown slash → 'unknown command'" "see $TMP/slash-unknown.txt"

# ─── history (Up/Down) ────────────────────────────────────────────────────
section "9. history Up/Down"

# This test uses two sessions to confirm cross-restart persistence.
HIST_MARKER="HISTMARK-$$-$RANDOM"
sess1="phantest-h1-$$"
tmux new-session -d -s "$sess1" -x 80 -y 25 spectyn 2>/dev/null; sleep 1.5
tmux send-keys -t "$sess1" -l "$HIST_MARKER"
sleep 0.2
tmux send-keys -t "$sess1" Enter
sleep 1
tmux send-keys -t "$sess1" Escape; sleep 0.3
tmux send-keys -t "$sess1" "/exit" Enter; sleep 0.5
tmux kill-session -t "$sess1" 2>/dev/null
sleep 0.4

# File-level persistence
if grep -qF "$HIST_MARKER" "$HOME/.spectyn-mesh/tui-history" 2>/dev/null; then
  ok "history persists to ~/.spectyn-mesh/tui-history" ""
else
  fail "history persists to ~/.spectyn-mesh/tui-history" ""
fi

# Cross-session Up arrow recall (Up x2 to skip /exit)
sess2="phantest-h2-$$"
tmux new-session -d -s "$sess2" -x 80 -y 25 spectyn 2>/dev/null; sleep 1.5
tmux send-keys -t "$sess2" Up; sleep 0.3
tmux send-keys -t "$sess2" Up; sleep 0.3
TMUX_CAP=$(tmux capture-pane -t "$sess2" -p)
tmux kill-session -t "$sess2" 2>/dev/null
echo "$TMUX_CAP" | grep -qF "$HIST_MARKER" \
  && ok "Up arrow (x2) recalls last session marker" "$HIST_MARKER" \
  || fail "Up arrow (x2) recalls last session marker" "see $TMP/h2.txt"

# ─── summary ──────────────────────────────────────────────────────────────
section "summary"
total=$((PASS + FAIL))
printf "  %s pass · %s fail · total %d\n" "$(green $PASS)" "$(red $FAIL)" "$total"
printf "  %s\n" "$(gray "captures saved to $TMP")"
if (( FAIL > 0 )); then
  printf "\n%s\n" "$(bold 'failures:')"
  for f in "${FAIL_LINES[@]}"; do printf "  - %s\n" "$f"; done
  exit 2
fi
