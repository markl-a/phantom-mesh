#!/usr/bin/env bash
# test-subagent-parity.sh — verifies spectyn's `subagent` and
# `parallel_tasks` tools produce output interchangeable with Claude
# Code's `Agent` tool when `format: "raw"` is requested.
#
# This script DOES make real LLM calls (against whatever provider the
# user's `agent.master` config points at — typically opencode free tier
# or groq free tier). Cost: ~$0 for free tiers, < $0.05 worst case
# with paid Anthropic. max_rounds=1 + tiny prompt keeps it bounded.
#
# What we test (the 5 parity dimensions from
# _planning-audit/07-SUBAGENT-PARITY-PLAN.md):
#   1. subagent default (wrapped) → has [subagent: ...] header
#   2. subagent format=raw → NO header, just the agent's text
#   3. subagent format=json → parses as JSON object with {agent, output, status}
#   4. subagent_type alias accepted in place of agent
#   5. description field accepted (not blocking)
#   6. parallel_tasks format=json → JSON array, one entry per task
#
# Skip with SPECTYN_PARITY_SKIP_LLM=1 (only runs the input-shape tests
# that don't need LLM dispatch — useful for CI without API keys).

set -u
SPECTYN="${SPECTYN:-$(command -v spectyn)}"
[ -z "$SPECTYN" ] && { echo "✗ spectyn not on PATH"; exit 2; }
SKIP_LLM="${SPECTYN_PARITY_SKIP_LLM:-0}"

PASS=0
FAIL=0

# Drive spectyn mcp with a JSON-RPC sequence; return the response text.
_drive() {
    printf '%s\n' "$1" | timeout 60 "$SPECTYN" mcp 2>/dev/null
}

_init='{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"parity","version":"1"}}}'

# Extract the `text` field from the most recent tools/call response.
# spectyn MCP returns tool results as `{"content":[{"text":"…","type":"text"}], "isError":...}`.
_extract_text() {
    python3 -c '
import json, sys, re
data = sys.stdin.read()
# Find the last response with content[].text
last_text = None
for line in data.split("\n"):
    if not line.strip(): continue
    try:
        obj = json.loads(line)
    except Exception:
        continue
    result = obj.get("result", {})
    content = result.get("content", [])
    for item in content:
        if item.get("type") == "text" and item.get("text") is not None:
            last_text = item["text"]
if last_text is None:
    print("__NO_TEXT__")
else:
    print(last_text)
'
}

_check() {
    local name="$1" cond="$2" detail="$3"
    if [ "$cond" = "yes" ]; then
        printf '  %-44s PASS\n' "$name"
        PASS=$((PASS + 1))
    else
        printf '  %-44s FAIL\n' "$name"
        FAIL=$((FAIL + 1))
        [ -n "$detail" ] && echo "$detail" | head -5 | sed 's/^/      /'
    fi
}

echo "━━ spectyn subagent parity tests ━━"
echo "  binary: $SPECTYN · skip_llm: $SKIP_LLM"
echo ""

# ── Tier 1: schema-only checks (don't need LLM dispatch) ────────────────────

# Verify the tool schema advertises new fields (description, format,
# subagent_type). Read the tools/list response and grep.
list_req='{"jsonrpc":"2.0","id":2,"method":"tools/list"}'
list_out=$(_drive "$_init"$'\n'"$list_req")

if echo "$list_out" | grep -q '"format"'; then
    _check "schema: subagent advertises 'format'" yes ""
else
    _check "schema: subagent advertises 'format'" no "$list_out"
fi
if echo "$list_out" | grep -q '"subagent_type"'; then
    _check "schema: subagent advertises 'subagent_type' alias" yes ""
else
    _check "schema: subagent advertises 'subagent_type' alias" no "$list_out"
fi
if echo "$list_out" | grep -q '"description":[^,]*Short label'; then
    _check "schema: subagent advertises 'description' field" yes ""
else
    _check "schema: subagent advertises 'description' field" no "$list_out"
fi

# Negative: subagent without agent OR subagent_type → clear error message
neg='{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"subagent","arguments":{"prompt":"x"}}}'
out=$(_drive "$_init"$'\n'"$neg" | _extract_text)
if echo "$out" | grep -qi "missing.*agent\|missing.*subagent_type"; then
    _check "neg: missing agent → clean error" yes ""
else
    _check "neg: missing agent → clean error" no "$out"
fi

# ── Tier 2: actual LLM dispatch (skip with SPECTYN_PARITY_SKIP_LLM=1) ───────

if [ "$SKIP_LLM" = "1" ]; then
    echo ""
    echo "  (SPECTYN_PARITY_SKIP_LLM=1 — skipping LLM-dispatch tests)"
    echo "━━ result: $PASS pass, $FAIL fail ━━"
    [ "$FAIL" -eq 0 ]
    exit $?
fi

PROMPT='Reply with exactly the word OK and nothing else.'

# 1. wrapped (default) → has [subagent: ...] header
call='{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"subagent","arguments":{"agent":"master","prompt":"'"$PROMPT"'","max_rounds":1}}}'
out=$(_drive "$_init"$'\n'"$call" | _extract_text)
if echo "$out" | head -1 | grep -qE '^\[subagent:'; then
    _check "wrapped (default) → has [subagent: header" yes ""
else
    _check "wrapped (default) → has [subagent: header" no "$out"
fi

# 2. format=raw → NO header
call='{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"subagent","arguments":{"agent":"master","prompt":"'"$PROMPT"'","max_rounds":1,"format":"raw"}}}'
out=$(_drive "$_init"$'\n'"$call" | _extract_text)
if echo "$out" | head -1 | grep -qE '^\[subagent:'; then
    _check "format=raw → no [subagent: header" no "$out"
else
    _check "format=raw → no [subagent: header" yes ""
fi
if [ -n "$out" ] && [ "$out" != "__NO_TEXT__" ]; then
    _check "format=raw → non-empty output" yes ""
else
    _check "format=raw → non-empty output" no "$out"
fi

# 3. format=json → parses as JSON, has expected keys
call='{"jsonrpc":"2.0","id":12,"method":"tools/call","params":{"name":"subagent","arguments":{"agent":"master","prompt":"'"$PROMPT"'","max_rounds":1,"format":"json"}}}'
out=$(_drive "$_init"$'\n'"$call" | _extract_text)
if echo "$out" | python3 -c '
import json, sys
try:
    d = json.loads(sys.stdin.read())
    assert "agent" in d, "missing agent"
    assert "output" in d, "missing output"
    assert "status" in d, "missing status"
    print("OK")
except Exception as e:
    print("FAIL", e)
' | grep -q "^OK$"; then
    _check "format=json → parses with {agent,output,status}" yes ""
else
    _check "format=json → parses with {agent,output,status}" no "$out"
fi

# 4. subagent_type alias works
call='{"jsonrpc":"2.0","id":13,"method":"tools/call","params":{"name":"subagent","arguments":{"subagent_type":"master","prompt":"'"$PROMPT"'","max_rounds":1,"format":"raw"}}}'
out=$(_drive "$_init"$'\n'"$call" | _extract_text)
if [ -n "$out" ] && [ "$out" != "__NO_TEXT__" ] && ! echo "$out" | grep -qi "missing"; then
    _check "subagent_type alias accepted" yes ""
else
    _check "subagent_type alias accepted" no "$out"
fi

# 5. description field accepted (non-blocking)
call='{"jsonrpc":"2.0","id":14,"method":"tools/call","params":{"name":"subagent","arguments":{"agent":"master","prompt":"'"$PROMPT"'","description":"echo test","max_rounds":1,"format":"raw"}}}'
out=$(_drive "$_init"$'\n'"$call" | _extract_text)
if [ -n "$out" ] && [ "$out" != "__NO_TEXT__" ] && ! echo "$out" | grep -qi "missing\|error"; then
    _check "description field accepted (compat shim)" yes ""
else
    _check "description field accepted (compat shim)" no "$out"
fi

# 6. parallel_tasks format=json → JSON array of N entries
call='{"jsonrpc":"2.0","id":15,"method":"tools/call","params":{"name":"parallel_tasks","arguments":{"format":"json","max_rounds":1,"tasks":[{"agent":"master","prompt":"Reply with the letter A only."},{"agent":"master","prompt":"Reply with the letter B only."}]}}}'
out=$(_drive "$_init"$'\n'"$call" | _extract_text)
if echo "$out" | python3 -c '
import json, sys
try:
    arr = json.loads(sys.stdin.read())
    assert isinstance(arr, list), "not a list"
    assert len(arr) == 2, f"expected 2 entries, got {len(arr)}"
    for entry in arr:
        assert "agent" in entry, "missing agent"
        assert "label" in entry, "missing label"
        # output may be empty if LLM hiccuped; we just check key exists
        assert "output" in entry, "missing output"
    print("OK")
except Exception as e:
    print("FAIL", e)
' | grep -q "^OK$"; then
    _check "parallel_tasks format=json → array[2]" yes ""
else
    _check "parallel_tasks format=json → array[2]" no "$out"
fi

echo ""
echo "━━ result: $PASS pass, $FAIL fail ━━"
[ "$FAIL" -eq 0 ]
