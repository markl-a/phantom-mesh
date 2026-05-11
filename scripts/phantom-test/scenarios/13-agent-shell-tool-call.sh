#!/usr/bin/env bash
source "$PHANTOM_TEST_LIB/common.sh"

scenario "Agent tool call — shell({whoami}) end-to-end"
require_cmd "$PHANTOM_BIN"

# Skip if no LLM key in env. Real LLM is required because the mock server
# doesn't (yet) simulate OpenAI tool_calls deltas — extending it is filed
# in the README's gap table.
if [ -z "${OPENCODE_API_KEY:-}${ANTHROPIC_API_KEY:-}${OPENROUTER_API_KEY:-}${GROQ_API_KEY:-}${GEMINI_API_KEY:-}" ]; then
  warn "no real LLM key in env — skipping (this scenario needs tool-call streaming)"
  exit 77
fi

step "asking master to run 'whoami' via the shell tool"
out=$("$PHANTOM_BIN" repl --agent master -c \
  "Use the shell tool to run 'whoami', then reply with exactly that command output and nothing else." \
  2>&1 | sed -E 's/\x1b\[[0-9;]*m//g')

# Three independent signals that prove the tool path executed:
#   (a) the agent dispatched the shell tool (line: ● shell({"command":"whoami"}))
#   (b) the tool returned exit code 0 (line: [exit code: 0])
#   (c) the agent's final reply mirrors the actual `whoami` output
ASSERT_CONTAINS "$out" 'shell({"command":"whoami"})' "agent dispatched shell tool"
ASSERT_CONTAINS "$out" "exit code: 0" "shell tool exited 0"

# Compare against the actual local user — strip any domain prefix Windows
# adds (e.g. "computer\\user" → "user").
expected=$(whoami 2>/dev/null | sed -E 's|.*[/\\]||' | tr -d '\r\n' | tr '[:upper:]' '[:lower:]')
out_lc=$(printf '%s' "$out" | tr '[:upper:]' '[:lower:]')
ASSERT_CONTAINS "$out_lc" "$expected" "agent's final reply contains real whoami value '$expected'"

ASSERT_CONTAINS "$out" "$" "cost summary line present"

[ "$PHANTOM_TEST_FAILED" -eq 0 ]
