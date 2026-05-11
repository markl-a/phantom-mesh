#!/usr/bin/env bash
# tmux-based test for double-tap Ctrl-C exit semantics.
#
# Unit-level coverage in core/src/tui.rs verifies the state-machine
# correctness (5 tests on handle_key); this file verifies the
# integration through a real terminal — which exercises the
# crossterm event-loop wiring + ratatui redraw pipeline.
#
# What we assert:
#   1. First Ctrl-C while idle → TUI does NOT exit (tmux session
#      still alive after one press).
#   2. The hint "press Ctrl-C again within 2 s to exit" appears in
#      the rendered transcript (proving the warning was shown).
#   3. Second Ctrl-C within ~1 s → TUI exits (tmux session ends).

selftest_feature_meta() {
  echo "name=tui-double-tap"
  echo "priority=P2"
  echo "requires=tmux"
  echo "description=double-tap Ctrl-C exit semantics in real terminal"
  echo "hints=core/src/tui.rs handle_key Ctrl-C double-tap"
}

selftest_requires() {
  t_have tmux || { echo "tmux not on PATH" >&2; return 1; }
}

selftest_run() {
  local sess="phantom-double-tap-$$"
  local cap="$SELFTEST_ARTIFACTS/double-tap-pane.txt"

  tmux kill-session -t "$sess" 2>/dev/null || true
  tmux new-session -d -s "$sess" -x 120 -y 40 "$PHANTOM" 2>/dev/null
  sleep 2  # ratatui first paint

  # ── 1. First Ctrl-C — should NOT exit ──────────────────────────────────
  tmux send-keys -t "$sess" C-c 2>/dev/null
  sleep 0.5

  T_ARTIFACT="$cap"
  T_REPRO="tmux new-session -d -s tap-debug -x 120 -y 40 $(printf '%q' "$PHANTOM") && sleep 2 && tmux send-keys -t tap-debug C-c"

  if tmux has-session -t "$sess" 2>/dev/null; then
    t_pass "first Ctrl-C does NOT exit" "session still alive"
  else
    t_fail "first Ctrl-C does NOT exit" "session died (regression of double-tap)"
    return
  fi

  # ── 2. Hint message visible in pane ────────────────────────────────────
  tmux capture-pane -t "$sess" -p > "$cap" 2>/dev/null
  if grep -qE "press Ctrl-C again|press.*again.*exit|2 ?s to exit" "$cap"; then
    t_pass "warning hint shown in transcript" ""
  else
    t_fail "warning hint shown in transcript" \
            "expected 'press Ctrl-C again within 2 s' or similar"
  fi

  # ── 3. Second Ctrl-C within window — SHOULD exit ──────────────────────
  tmux send-keys -t "$sess" C-c 2>/dev/null
  sleep 1.0

  if ! tmux has-session -t "$sess" 2>/dev/null; then
    t_pass "second Ctrl-C within 2s exits" "session ended cleanly"
  else
    # Session still alive — that's the regression. Report and clean up.
    tmux capture-pane -t "$sess" -p > "$cap.alive" 2>/dev/null
    t_fail "second Ctrl-C within 2s exits" \
            "session still alive after second Ctrl-C — see $cap.alive"
    tmux kill-session -t "$sess" 2>/dev/null || true
  fi
}
