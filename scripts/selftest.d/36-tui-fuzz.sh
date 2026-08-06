#!/usr/bin/env bash
# tmux-based fuzz test: drive the real ratatui TUI through a flood of
# random keystrokes and verify the process is still alive at the end.
#
# Complements scripts/selftest.d/35-tui.sh (which checks specific
# slash commands work) and the 8 Rust-level fuzz tests in
# core/src/tui.rs (which cover handle_key + render at unit level).
# This file's job is the *integration* layer: does the real binary
# survive 100+ random keystrokes including modifier-heavy combos and
# Unicode input that the unit-level tests can't exercise without a
# real terminal pipeline?
#
# Reproducible: seed is fixed via $RANDOM seeding from a constant.
# To bisect a specific failure, replay the exact `tmux send-keys`
# sequence shown in the artifact log.

selftest_feature_meta() {
  echo "name=tui-fuzz"
  echo "priority=P2"
  echo "requires=tmux"
  echo "description=Real-terminal random-keystroke flood; verifies TUI doesn't crash under adversarial input"
  echo "hints=core/src/tui.rs scripts/selftest.d/35-tui.sh"
}

selftest_requires() {
  t_have tmux || { echo "tmux not on PATH" >&2; return 1; }
}

selftest_run() {
  local sess="spectyn-fuzz-$$"
  local pane_log="$SELFTEST_ARTIFACTS/fuzz-pane.txt"
  local keys_log="$SELFTEST_ARTIFACTS/fuzz-keys-sent.log"
  T_ARTIFACT="$pane_log"

  tmux kill-session -t "$sess" 2>/dev/null || true
  tmux new-session -d -s "$sess" -x 120 -y 40 "$SPECTYN" 2>/dev/null
  sleep 2

  # Seed bash $RANDOM deterministically so fuzz failures replay.
  RANDOM=4242

  # Random keys: mix of printable ASCII, Unicode, control combos, arrow
  # navigation, slash-commands, and Tab. 100 keystrokes is enough to
  # land in unusual states without making the test slow.
  local pool=(
    "a" "b" "c" "x" "y" "z" "0" "5" " " "/" "?" "!" "."
    "Enter" "BSpace" "Tab" "Up" "Down" "Left" "Right" "Home" "End"
    "Escape" "PageUp" "PageDown"
    "中" "文" "🎉" "👨‍👩"  # multi-byte
    "C-a" "C-e" "C-u" "C-w" "C-k" "C-l"  # ctrl combos that are NOT exit (skipping C-c on purpose — we test that elsewhere)
  )
  local n_keys=120

  : > "$keys_log"
  for ((i = 0; i < n_keys; i++)); do
    local k="${pool[RANDOM % ${#pool[@]}]}"
    tmux send-keys -t "$sess" "$k" 2>/dev/null
    echo "$k" >> "$keys_log"
    # Brief sleep every ~10 keys so the TUI has time to redraw — without
    # this, send-keys can drown the input event loop (which is in tokio)
    # and we'd be testing a hot path that doesn't match real users.
    if (( i % 10 == 0 )); then sleep 0.1; fi
  done

  # Give the TUI a moment to settle on the last keystroke.
  sleep 1.5

  # Capture pane content (this is the artifact; lets a debug session
  # see what the TUI looked like at the end of the fuzz).
  tmux capture-pane -t "$sess" -p > "$pane_log" 2>/dev/null || true

  # Is the tmux session still alive? If spectyn panicked, the session
  # would have ended (tmux kills the session when the only command
  # exits).
  T_REPRO="see $keys_log; replay with: while IFS= read -r k; do tmux send-keys -t fuzz-debug \"\$k\"; done < $keys_log"

  if tmux has-session -t "$sess" 2>/dev/null; then
    # Session alive — does it still LOOK like the TUI? A crashed spectyn
    # might leave the session up but show a stack trace instead. Look
    # for ratatui's box-drawing chars or the typing-prompt indicator
    # which the TUI always renders.
    if grep -qE '│|─|┌|└|>|❯|│' "$pane_log"; then
      t_pass "tmux session alive after 120 random keys" \
              "TUI box-drawing still rendered"
    else
      t_fail "tmux session alive but no TUI chrome visible" \
              "process up but UI looks crashed (stack trace?)"
    fi

    # Bonus: is there a panic / abort signature in the captured pane?
    # spectyn prints panics to stderr but sometimes they leak through.
    if grep -qiE "panicked at|RUST_BACKTRACE|core dumped|assertion failed" "$pane_log"; then
      t_fail "no panic signature in pane" "panic text visible — see artifact"
    else
      t_pass "no panic signature in pane" ""
    fi
  else
    t_fail "tmux session alive after 120 random keys" \
            "session died — spectyn likely panicked. see $keys_log to replay"
  fi

  # Cleanup (tmux session may already be dead).
  tmux kill-session -t "$sess" 2>/dev/null || true
}
