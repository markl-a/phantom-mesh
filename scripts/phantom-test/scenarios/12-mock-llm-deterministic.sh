#!/usr/bin/env bash
source "$PHANTOM_TEST_LIB/common.sh"
source "$PHANTOM_TEST_LIB/mock.sh"

scenario "Mock LLM — deterministic, no network, scripted responses"
require_cmd "$PHANTOM_BIN"

# Ensure cleanup even if the scenario fails midway.
trap 'mock_stop' EXIT

step "starting mock LLM server on :$MOCK_PORT"
if ! mock_start; then
  fail "mock server did not come up"
  exit 1
fi
pass "mock server PID $(mock_pid) listening"

step "verifying mock /v1/models endpoint responds"
models=$(curl -sS --max-time 3 "http://127.0.0.1:$MOCK_PORT/v1/models")
ASSERT_CONTAINS "$models" "mock-instant" "mock /v1/models returns expected id"

# Build a temp cwd with the mock agents.toml; phantom prefers ./agents.toml
# over $HOME/.phantom-mesh/agents.toml.
agents_dir=$(mock_temp_agents_dir)
step "using temp agents.toml at $agents_dir"

# ── Test 1: ping → pong ────────────────────────────────────────────────────
step "test 1: 'ping' should produce scripted 'pong'"
out=$(cd "$agents_dir" && "$PHANTOM_BIN" repl --agent master -c "ping" 2>&1 \
      | sed -E 's/\x1b\[[0-9;]*m//g')
ASSERT_CONTAINS "$out" "pong" "ping → pong"

# ── Test 2: '2 + 2' regex match → '4' ──────────────────────────────────────
step "test 2: '2 + 2' should produce scripted '4'"
out=$(cd "$agents_dir" && "$PHANTOM_BIN" repl --agent master -c "What is 2 + 2?" 2>&1 \
      | sed -E 's/\x1b\[[0-9;]*m//g')
ASSERT_CONTAINS "$out" "4" "regex match → 4"

# ── Test 3: unmatched prompt → default ─────────────────────────────────────
step "test 3: unmatched prompt should hit the default reply"
out=$(cd "$agents_dir" && "$PHANTOM_BIN" repl --agent master -c "this prompt has no match" 2>&1 \
      | sed -E 's/\x1b\[[0-9;]*m//g')
ASSERT_CONTAINS "$out" "MOCK" "unmatched → default 'MOCK: no scripted response'"

# ── Test 4: cost should be reported (mock returns usage) ───────────────────
step "test 4: cost summary line is present (mock returns usage tokens)"
ASSERT_CONTAINS "$out" "$" "cost line present"

[ "$PHANTOM_TEST_FAILED" -eq 0 ]
