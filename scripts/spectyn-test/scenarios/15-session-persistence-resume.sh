#!/usr/bin/env bash
source "$SPECTYN_TEST_LIB/common.sh"

scenario "REPL session — first turn persists, --session resume recalls it"
require_cmd "$SPECTYN_BIN"

if [ -z "${OPENCODE_API_KEY:-}${ANTHROPIC_API_KEY:-}${OPENROUTER_API_KEY:-}${GROQ_API_KEY:-}${GEMINI_API_KEY:-}" ]; then
  warn "no real LLM key in env — skipping (session resume needs the agent to actually reason)"
  exit 77
fi

# Use an explicit session id we control, so we don't have to parse stdout
# to find which file spectyn wrote to. The id space is shared with the
# default cwd-hashed sessions, so we pick a clearly-test-prefixed name.
sid="spectyn-test-resume-$$-$(date +%s)"
SPECTYN_CONFIG_DIR="${SPECTYN_CONFIG_DIR:-$HOME/.spectyn-mesh}"
session_file="$SPECTYN_CONFIG_DIR/conversations/${sid}.jsonl"

# Ensure cleanup whether we pass or fail.
trap 'rm -f "$session_file"' EXIT

# A unique passphrase per run rules out cross-run cache hits.
phrase="XYZQ-$(date +%s)-RESUME"

step "turn 1: ask master to remember '$phrase'"
out1=$("$SPECTYN_BIN" repl --agent master --session "$sid" -c \
  "Remember this exact phrase verbatim: '$phrase'. Just acknowledge with the word OK." \
  2>&1 | sed -E 's/\x1b\[[0-9;]*m//g')

# The session file must now exist with at least the user+assistant pair.
if [ ! -f "$session_file" ]; then
  fail "session file not created at $session_file"
  exit 1
fi
pass "session file written: $session_file ($(stat -c %s "$session_file") bytes)"

if grep -q "$phrase" "$session_file"; then
  pass "passphrase persisted in JSONL"
else
  fail "passphrase NOT in JSONL — session writer skipped the user message?"
fi

step "turn 2: resume same --session and ask master to recall the phrase"
out2=$("$SPECTYN_BIN" repl --agent master --session "$sid" -c \
  "What was the exact phrase I asked you to remember in the previous turn? Reply with just the phrase." \
  2>&1 | sed -E 's/\x1b\[[0-9;]*m//g')

# Final reply must contain the passphrase — proves history was loaded into
# the second invocation's prompt.
ASSERT_CONTAINS "$out2" "$phrase" "agent recalled passphrase across invocations"

# Session file should now contain BOTH turns.
turn_count=$(grep -c '"role":' "$session_file")
if [ "$turn_count" -ge 4 ]; then
  pass "session has $turn_count role-tagged records (>= 4 expected: user/asst × 2 turns)"
else
  fail "session only has $turn_count records — turn 2 did not append"
fi

[ "$SPECTYN_TEST_FAILED" -eq 0 ]
