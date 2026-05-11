#!/usr/bin/env bash
source "$PHANTOM_TEST_LIB/common.sh"
source "$PHANTOM_TEST_LIB/inspect.sh"

scenario "phantom doctor — baseline health check"
require_cmd "$PHANTOM_BIN"

step "running phantom doctor"
out=$(doctor_summary)

ASSERT_CONTAINS "$out" "binary"  "doctor section: binary"
ASSERT_CONTAINS "$out" "agents.toml" "doctor section: config"
ASSERT_CONTAINS "$out" "phantom serve" "doctor section: serve"
ASSERT_CONTAINS "$out" "Tailscale" "doctor section: network"

# Healthz must succeed if the Scheduled Task is up. Skip-soft if down.
if echo "$out" | grep -q "healthz: 200"; then
  pass "phantom serve healthz returned 200"
else
  warn "phantom serve not running — subsequent serve scenarios will fail"
fi

# At least ONE LLM provider key must be present (else cluster won't function).
if echo "$out" | grep -qE '✓ (Anthropic|OpenAI|Groq|Gemini|OpenRouter|OpenCode)'; then
  pass "at least one LLM provider key detected"
else
  fail "no LLM provider key in env or agents.toml — agents will all fail"
fi

[ "$PHANTOM_TEST_FAILED" -eq 0 ]
