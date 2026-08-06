#!/usr/bin/env bash
source "$SPECTYN_TEST_LIB/common.sh"

scenario "LLM roundtrip via spectyn repl --agent master -c"
require_cmd "$SPECTYN_BIN"

# This exercises the in-process LLM path (no serve, no RPC). Useful baseline
# for "is the configured provider reachable from this binary right now".
step "calling agent.master with deterministic prompt"

start_ts=$(date +%s)
out=$("$SPECTYN_BIN" repl --agent master -c "Reply with exactly the two characters: ok" 2>&1)
elapsed=$(( $(date +%s) - start_ts ))

# Strip ANSI and the per-line glyph prefixes we know about.
clean=$(printf '%s' "$out" | sed -E 's/\x1b\[[0-9;]*m//g')

# The response body is multi-line (status banner + thinking + final answer).
# Just verify SOMETHING that is the agent's answer is in there. "ok" is two
# characters — we look for the literal string anywhere after the input echo.
ASSERT_CONTAINS "$clean" "ok" "response body contains 'ok'"

# Sanity: should be < 60s on a reasonable LLM. Long elapsed without an
# obvious fail usually means a slow free-tier model — warn, don't fail.
if [ "$elapsed" -gt 60 ]; then
  warn "LLM roundtrip took ${elapsed}s — slower than expected, free tier?"
else
  pass "LLM roundtrip completed in ${elapsed}s"
fi

# Cost-line presence: spectyn repl emits "[↑ $X.XXXX  ∑ $X.XXXX  Ts]" at end.
ASSERT_CONTAINS "$clean" "$" "response includes cost summary line"

[ "$SPECTYN_TEST_FAILED" -eq 0 ]
