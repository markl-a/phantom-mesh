#!/usr/bin/env bash
# tmux black-box tests for the ratatui TUI. Each assertion runs in its own
# session so failures are isolated, and the captured pane is dumped into the
# feature's artifacts dir whenever an assertion misses.
#
# Migrated from scripts/smoke-mac.sh (sections 7) — same coverage but each
# check now flows through the selftest framework's repro/artifact pipeline.

selftest_feature_meta() {
  echo "name=tui"
  echo "priority=P1"
  echo "requires=tmux"
  echo "description=ratatui TUI launches, slash commands respond, history persists"
  echo "hints=core/src/tui.rs"
}

selftest_requires() {
  t_have tmux || { echo "tmux not on PATH — install: brew install tmux" >&2; return 1; }
}

# ── helpers ─────────────────────────────────────────────────────────────────
_tui_n=0
_tui_launch() {
  _tui_n=$((_tui_n+1))
  local sess="phantom-selftest-tui-$$-$_tui_n"
  tmux kill-session -t "$sess" 2>/dev/null || true
  tmux new-session -d -s "$sess" -x 120 -y 40 "$PHANTOM" 2>/dev/null
  sleep 2  # ratatui first paint + onboarding-skip
  echo "$sess"
}

_tui_cleanup() {
  local sess="$1"
  tmux send-keys -t "$sess" Escape 2>/dev/null
  sleep 0.2
  tmux send-keys -t "$sess" "/exit" Enter 2>/dev/null
  sleep 0.4
  tmux kill-session -t "$sess" 2>/dev/null || true
}

# _tui_assert <test-name> <regex> <session> [<artifact-slug>]
_tui_assert() {
  local name="$1" pattern="$2" sess="$3"
  local slug="${4:-$(echo "$name" | tr ' /<>' '----')}"
  local cap="$SELFTEST_ARTIFACTS/${slug}.txt"
  tmux capture-pane -t "$sess" -p > "$cap" 2>/dev/null
  T_ARTIFACT="$cap"
  T_REPRO="tmux new-session -d -s tui-debug -x 120 -y 40 $(printf '%q' "$PHANTOM") && sleep 2 && tmux send-keys -t tui-debug ... && tmux capture-pane -t tui-debug -p"
  if grep -qE "$pattern" "$cap"; then
    t_pass "$name" "matched /$pattern/"
  else
    t_fail "$name" "no /$pattern/ — see artifact"
  fi
}

# ── checks ──────────────────────────────────────────────────────────────────
selftest_run() {
  local sess

  # 1. Launch — chrome chars (box drawing) should be visible
  sess=$(_tui_launch)
  _tui_assert "TUI launch" 'phantom|>|│|╭|─' "$sess" "launch"
  _tui_cleanup "$sess"

  # 2. /help renders the slash-command palette
  sess=$(_tui_launch)
  tmux send-keys -t "$sess" "/help" Enter; sleep 1.5
  _tui_assert "TUI /help shows commands" '/cost|/exit|/provider|/model|/copy|commands' "$sess" "help"
  _tui_cleanup "$sess"

  # 3. Typed input renders in the input area
  sess=$(_tui_launch)
  tmux send-keys -t "$sess" "phantom-typing-marker-99"; sleep 0.8
  _tui_assert "TUI typing renders" 'phantom-typing-marker-99' "$sess" "typing"
  _tui_cleanup "$sess"

  # 4. Tab completion — single-match path: `/cos<Tab>` → `/cost`. The previous
  # `/co<Tab>` test asserted a candidate-list popup that the TUI never had
  # (LCP of /cost+/copy is `/co`, so Tab is correctly a no-op there).
  sess=$(_tui_launch)
  tmux send-keys -t "$sess" "/cos"; sleep 0.5
  tmux send-keys -t "$sess" Tab;    sleep 0.5
  _tui_assert "TUI Tab completes /cos→/cost" '/cost' "$sess" "tab-complete"
  _tui_cleanup "$sess"

  # 5. /provider lists configured providers
  sess=$(_tui_launch)
  tmux send-keys -t "$sess" "/provider" Enter; sleep 1.5
  _tui_assert "TUI /provider lists providers" 'groq|anthropic|gemini|openrouter|mlx|provider' "$sess" "provider"
  _tui_cleanup "$sess"

  # 6. /model shows the current model
  sess=$(_tui_launch)
  tmux send-keys -t "$sess" "/model" Enter; sleep 1.5
  _tui_assert "TUI /model shows model" 'model|llama|claude|gemini|gpt|mlx' "$sess" "model"
  _tui_cleanup "$sess"

  # 7. /cost shows session cost summary
  sess=$(_tui_launch)
  tmux send-keys -t "$sess" "/cost" Enter; sleep 1.5
  _tui_assert "TUI /cost shows summary" 'cost|spent|tokens|0\.|\$|session' "$sess" "cost"
  _tui_cleanup "$sess"

  # 8. Esc clears typed-but-not-submitted input
  sess=$(_tui_launch)
  tmux send-keys -t "$sess" "this-should-vanish"; sleep 0.5
  tmux send-keys -t "$sess" Escape; sleep 0.5
  local cap="$SELFTEST_ARTIFACTS/esc.txt"
  tmux capture-pane -t "$sess" -p > "$cap" 2>/dev/null
  T_ARTIFACT="$cap"
  T_REPRO="see scripts/selftest.d/35-tui.sh case 8"
  if grep -q "this-should-vanish" "$cap"; then
    t_fail "TUI Esc clears input" "string still visible — see artifact"
  else
    t_pass "TUI Esc clears input" "input area cleared"
  fi
  _tui_cleanup "$sess"

  # 9. History persistence: file durability + Up-arrow recall in fresh session
  local marker="hist-marker-$$-$RANDOM"
  local hist_file="$HOME/.phantom-mesh/tui-history"
  sess=$(_tui_launch)
  tmux send-keys -t "$sess" "$marker" Enter; sleep 0.6
  tmux send-keys -t "$sess" Escape;          sleep 0.4
  _tui_cleanup "$sess"; sleep 0.5

  T_ARTIFACT="$hist_file"
  T_REPRO="grep -F '$marker' $hist_file"
  if [ -f "$hist_file" ] && grep -qF "$marker" "$hist_file"; then
    t_pass "TUI history file persisted" "$marker found in $(basename "$hist_file")"
  else
    t_fail "TUI history file persisted" "$marker not in $hist_file"
    return  # if file durability failed, recall test is moot
  fi

  # tui_cleanup sent /exit, which is also recorded; press Up twice.
  sess=$(_tui_launch)
  tmux send-keys -t "$sess" Up; sleep 0.4
  tmux send-keys -t "$sess" Up; sleep 0.6
  _tui_assert "TUI Up arrow recalls history" "$marker" "$sess" "history-recall"
  _tui_cleanup "$sess"
}
