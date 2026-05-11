#!/usr/bin/env bash
# test-mcp-tools.sh — end-to-end test that phantom mcp's tools work,
# not just that they appear in tools/list.
#
# Complements scripts/selftest.d/30-mcp.sh (which only verifies the
# handshake + tool list). This script makes actual tool/call requests
# and asserts the responses look right.
#
# Designed to be run BOTH:
#   - as a CI gate (`scripts/test-mcp-tools.sh` in github actions)
#   - by Claude Code mid-session via the Bash tool, so I can confirm
#     phantom's tools are working before relying on `mcp__phantom__*`
#     calls.
#
# No external deps beyond bash, phantom, and standard POSIX text tools
# (grep / sed / cut). No jq, no python — works in Git Bash on Windows.
#
# Usage:
#   scripts/test-mcp-tools.sh           # all tests
#   scripts/test-mcp-tools.sh -v        # verbose (show JSON-RPC traffic)

set -u

VERBOSE="${1:-}"
PHANTOM="${PHANTOM:-$(command -v phantom)}"
[ -z "$PHANTOM" ] && { echo "✗ phantom not on PATH (or set PHANTOM=...)" >&2; exit 2; }

PASS=0
FAIL=0

echo "━━ phantom mcp tools/call e2e ($PHANTOM) ━━"

# Drive phantom mcp by piping a sequence of JSON-RPC lines, then grep
# the response stream for the substring we expect. The MCP server
# prints each response on its own line so substring grep is robust.
_drive() {
    local input="$1"
    local out
    out=$(printf '%s\n' "$input" | timeout 10 "$PHANTOM" mcp 2>/dev/null) || true
    if [ -n "$VERBOSE" ]; then printf '  ── REQUEST ──\n%s\n  ── RESPONSE ──\n%s\n' "$input" "$out"; fi
    printf '%s' "$out"
}

# Initialize payload — used by every test below.
_init='{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test-mcp-tools","version":"1"}}}'

_check() {
    local name="$1" condition="$2" out="$3"
    if [ "$condition" = "yes" ]; then
        printf '  %-36s %s\n' "$name" "PASS"
        PASS=$((PASS + 1))
    else
        printf '  %-36s %s\n' "$name" "FAIL"
        FAIL=$((FAIL + 1))
        echo "$out" | head -3 | sed 's/^/      /'
    fi
}

# ── 1. handshake ─────────────────────────────────────────────────────────────
out=$(_drive "$_init")
if echo "$out" | grep -q '"serverInfo"'; then _check "handshake" yes "$out"; else _check "handshake" no "$out"; fi

# ── 2. tools/list contains canonical tools ──────────────────────────────────
list_req='{"jsonrpc":"2.0","id":2,"method":"tools/list"}'
out=$(_drive "$_init"$'\n'"$list_req")
for tool in file_read file_write file_edit shell content_search git_status web_fetch; do
    if echo "$out" | grep -qE "\"name\"[[:space:]]*:[[:space:]]*\"$tool\""; then
        _check "tools/list contains $tool" yes "$out"
    else
        _check "tools/list contains $tool" no "$out"
    fi
done

# ── 3. file_read on a known file ─────────────────────────────────────────────
mkdir -p /tmp/phantom-mcp-test
echo "phantom-test-marker-line-1" > /tmp/phantom-mcp-test/marker.txt
echo "second line" >> /tmp/phantom-mcp-test/marker.txt
call='{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"file_read","arguments":{"path":"/tmp/phantom-mcp-test/marker.txt"}}}'
out=$(_drive "$_init"$'\n'"$call")
if echo "$out" | grep -q 'phantom-test-marker-line-1'; then
    _check "file_read returns content" yes "$out"
else
    _check "file_read returns content" no "$out"
fi

# ── 4. shell echo round-trip ─────────────────────────────────────────────────
call='{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"shell","arguments":{"command":"echo hello-from-mcp-tool"}}}'
out=$(_drive "$_init"$'\n'"$call")
if echo "$out" | grep -q 'hello-from-mcp-tool'; then
    _check "shell echoes back" yes "$out"
else
    _check "shell echoes back" no "$out"
fi

# ── 5. git_status responds (any non-error) ──────────────────────────────────
call='{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"git_status","arguments":{}}}'
out=$(_drive "$_init"$'\n'"$call")
if echo "$out" | grep -q '"result"' || echo "$out" | grep -qi "not a git"; then
    _check "git_status responds" yes "$out"
else
    _check "git_status responds" no "$out"
fi

# ── 6. unknown tool → clean error ────────────────────────────────────────────
call='{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"this_tool_does_not_exist","arguments":{}}}'
out=$(_drive "$_init"$'\n'"$call")
if echo "$out" | grep -q '"error"' \
   || echo "$out" | grep -qi "unknown tool" \
   || echo "$out" | grep -qi "not found"; then
    _check "unknown tool → clean error" yes "$out"
else
    _check "unknown tool → clean error" no "$out"
fi

# ── 7. file_write + file_read round-trip ────────────────────────────────────
write='{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"file_write","arguments":{"path":"/tmp/phantom-mcp-test/written.txt","content":"round-trip-marker"}}}'
read='{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"file_read","arguments":{"path":"/tmp/phantom-mcp-test/written.txt"}}}'
out=$(_drive "$_init"$'\n'"$write"$'\n'"$read")
if echo "$out" | grep -q 'round-trip-marker'; then
    _check "file_write→file_read round-trip" yes "$out"
else
    _check "file_write→file_read round-trip" no "$out"
fi

# ── cleanup + report ─────────────────────────────────────────────────────────
rm -rf /tmp/phantom-mcp-test

echo ""
echo "━━ result: $PASS pass, $FAIL fail ━━"
[ "$FAIL" -eq 0 ]
