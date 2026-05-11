#!/usr/bin/env bash
source "$PHANTOM_TEST_LIB/common.sh"

scenario "phantom mcp — stdio server returns initialize + tools/list"
require_cmd "$PHANTOM_BIN"

# MCP is line-delimited JSON-RPC over stdin/stdout. We send:
#   1. initialize
#   2. notifications/initialized (acknowledge handshake)
#   3. tools/list
# Then close stdin and let the server exit naturally.

step "spawning phantom mcp + writing 3 JSON-RPC frames"
out=$(printf '%s\n%s\n%s\n' \
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"phantom-test","version":"1.0"}}}' \
    '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
    '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
    | timeout 10 "$PHANTOM_BIN" mcp 2>&1)
ec=$?
step "phantom mcp exit=$ec"

# A timeout (124) is OK — mcp normally only exits when stdin closes, and on
# Windows the pipe-close detection can lag. We just need to have collected
# the responses by then.
if [ "$ec" -ne 0 ] && [ "$ec" -ne 124 ]; then
    fail "unexpected exit code: $ec"
    printf '%s\n' "$out" | head -10 | sed 's/^/    /'
    exit 1
fi

# Show first few hundred bytes of stdout for the operator running the test.
step "first 400 bytes of mcp output:"
printf '%s\n' "$out" | head -c 400 | sed 's/^/    /'
echo

# Two structural assertions on the JSON-RPC stream:
ASSERT_CONTAINS "$out" '"jsonrpc":"2.0"' "saw at least one JSON-RPC envelope"
ASSERT_CONTAINS "$out" '"protocolVersion"' "initialize response carries protocolVersion"

# tools/list response should mention several known built-in tool names.
# Pull out the tool names with a tolerant grep — we don't try to parse the
# whole response since it can be very long.
expected_tools="shell file_read file_write content_search glob_search"
seen=0
for t in $expected_tools; do
    if printf '%s\n' "$out" | grep -q "\"name\":\"$t\""; then
        seen=$((seen + 1))
    fi
done
if [ "$seen" -ge 3 ]; then
    pass "tools/list mentions $seen of 5 expected tool names"
else
    fail "tools/list missing too many built-in tools (only $seen / 5 found)"
fi

[ "$PHANTOM_TEST_FAILED" -eq 0 ]
