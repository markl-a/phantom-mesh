#!/usr/bin/env bash
source "$SPECTYN_TEST_LIB/common.sh"

scenario "spectyn swarm — fan-out to all online peers + synthesize"
require_cmd "$SPECTYN_BIN"

if [ -z "${OPENCODE_API_KEY:-}${ANTHROPIC_API_KEY:-}${OPENROUTER_API_KEY:-}${GROQ_API_KEY:-}${GEMINI_API_KEY:-}" ]; then
  warn "no LLM key in env — synthesis step needs one; skipping"
  exit 77
fi

# Confirm at least one online peer (other than self) exists, else swarm is
# just a local one-shot and there's no fan-out to verify.
self="$(rpc_url 2>/dev/null || echo "http://${SPECTYN_HOST}:${SPECTYN_PORT}")"
online_peers=$("$SPECTYN_BIN" peer list 2>/dev/null \
  | sed -E 's/\x1b\[[0-9;]*m//g' \
  | awk '/online/ && /^http/ {print $1}' \
  | grep -v -F "${self}" | wc -l | tr -d ' \n')

if [ "${online_peers:-0}" -lt 1 ]; then
  warn "no online non-self peer — swarm has nothing to fan to; skipping"
  exit 77
fi
step "$online_peers online non-self peer(s) detected"

step "swarming a deterministic prompt"
out=$("$SPECTYN_BIN" swarm "Reply with exactly your hostname and nothing else." 2>&1 \
      | sed -E 's/\x1b\[[0-9;]*m//g')

# Print the tail so a human reader of the test log can see the synthesis text.
printf '%s\n' "$out" | tail -25 | sed 's/^/    /'

# Structural signals (synthesis text itself is non-deterministic):
ASSERT_CONTAINS "$out" "Discovering peers" "swarm hit discovery phase"
ASSERT_CONTAINS "$out" "Dispatching" "swarm hit dispatch phase"
ASSERT_CONTAINS "$out" "Results" "swarm collected results"
ASSERT_CONTAINS "$out" "Synthesis" "swarm produced a synthesis"
ASSERT_CONTAINS "$out" "swarm complete" "swarm reached terminal state"

# Each online peer should produce a `peer ... job done` line.
done_lines=$(printf '%s\n' "$out" | grep -c "peer .* job done" || true)
if [ "$done_lines" -ge "$online_peers" ]; then
  pass "$done_lines peer(s) reported job done (>= $online_peers expected)"
else
  fail "only $done_lines peer 'job done' lines (expected >= $online_peers)"
fi

[ "$SPECTYN_TEST_FAILED" -eq 0 ]
