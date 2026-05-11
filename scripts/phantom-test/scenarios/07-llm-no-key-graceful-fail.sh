#!/usr/bin/env bash
source "$PHANTOM_TEST_LIB/common.sh"

scenario "LLM — missing provider key fails gracefully (no panic / clean error)"
require_cmd "$PHANTOM_BIN"

# We unset OPENCODE_API_KEY (and the other common ones) for ONE invocation,
# then verify phantom returns a non-zero exit + a recognizable error message
# rather than panicking or hanging. We do NOT touch global env — the unset
# scope is just this subprocess.

step "running phantom repl with all common LLM keys unset"

set +u  # the unset patterns are tolerant of already-empty
out=$(
  env -u ANTHROPIC_API_KEY \
      -u OPENAI_API_KEY \
      -u GROQ_API_KEY \
      -u GEMINI_API_KEY \
      -u OPENROUTER_API_KEY \
      -u OPENCODE_API_KEY \
    "$PHANTOM_BIN" repl --agent master -c "ping" 2>&1
)
ec=$?
set -u

# Acceptable outcomes (any one is fine):
#   - non-zero exit
#   - output mentions "key", "auth", "401", "provider", "missing", or "no API key"
clean=$(printf '%s' "$out" | sed -E 's/\x1b\[[0-9;]*m//g')

if [ "$ec" -ne 0 ]; then
  pass "exit code $ec (non-zero, as expected when no key)"
else
  warn "exit code 0 — agent may have used a fallback provider"
fi

if echo "$clean" | grep -qiE 'key|auth|401|provider|missing|unauthor'; then
  pass "stderr/stdout mentions auth/key/provider error"
else
  fail "no recognizable auth-related error in output: ${clean:0:300}"
fi

# Should not panic. "panicked at" is the Rust panic signature.
ASSERT_NOT_CONTAINS "$clean" "panicked at" "no Rust panic"
ASSERT_NOT_CONTAINS "$clean" "RUST_BACKTRACE" "no backtrace dump"

[ "$PHANTOM_TEST_FAILED" -eq 0 ]
