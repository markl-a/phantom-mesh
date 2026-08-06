#!/usr/bin/env bash
source "$SPECTYN_TEST_LIB/common.sh"
source "$SPECTYN_TEST_LIB/inspect.sh"

scenario "spectyn doctor — baseline health check"
require_cmd "$SPECTYN_BIN"

step "running spectyn doctor"
out=$(doctor_summary)

ASSERT_CONTAINS "$out" "binary"  "doctor section: binary"
ASSERT_CONTAINS "$out" "agents.toml" "doctor section: config"
ASSERT_CONTAINS "$out" "spectyn serve" "doctor section: serve"
ASSERT_CONTAINS "$out" "Tailscale" "doctor section: network"

# Healthz must succeed if the Scheduled Task is up. Skip-soft if down.
if echo "$out" | grep -q "healthz: 200"; then
  pass "spectyn serve healthz returned 200"
else
  warn "spectyn serve not running — subsequent serve scenarios will fail"
fi

# At least ONE LLM provider key must be present (else cluster won't function).
if echo "$out" | grep -qE '✓ (Anthropic|OpenAI|Groq|Gemini|OpenRouter|OpenCode)'; then
  pass "at least one LLM provider key detected"
else
  fail "no LLM provider key in env or agents.toml — agents will all fail"
fi

[ "$SPECTYN_TEST_FAILED" -eq 0 ]
