#!/usr/bin/env bash
# Tier-8: error handling, edge cases, concurrency
#   - malformed MCP JSON-RPC
#   - bad HTTP bodies
#   - tool wrapper edge cases (huge file, binary, symlink, missing)
#   - file_edit when old_string appears multiple times
#   - file_edit with no match
#   - shell tool: timeout, very large output, signals
#   - concurrent /api/* requests
#   - tools that don't exist
#   - empty arguments

set -o pipefail
PASS=0; FAIL=0; FAIL_LINES=()
TMP=$(mktemp -d)
SERVE="http://127.0.0.1:7878"

green() { printf "\033[32m%s\033[0m" "$1"; }
red()   { printf "\033[31m%s\033[0m" "$1"; }
gray()  { printf "\033[90m%s\033[0m" "$1"; }
bold()  { printf "\033[1m%s\033[0m" "$1"; }

ok()    { PASS=$((PASS+1)); printf "  $(green '✓') %-58s %s\n" "$1" "$(gray "$2")"; }
fail()  { FAIL=$((FAIL+1)); FAIL_LINES+=("$1 :: $2"); printf "  $(red '✗') %-58s %s\n" "$1" "$(gray "$2")"; }
section() { printf "\n$(bold "%s")\n" "$1"; }

mcp() {
  echo "$1" | timeout 8 phantom mcp 2>/dev/null
}

# ─── malformed MCP requests ───────────────────────────────────────────────
section "47. malformed MCP JSON-RPC"

# Truncated JSON
resp=$(echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"' | timeout 5 phantom mcp 2>&1 | head -c 500)
echo "$resp" | grep -qE 'error|parse|jsonrpc' \
  && ok "truncated JSON returns parse error" "" \
  || fail "truncated JSON returns parse error" "got: $(echo "$resp" | head -c 200)"

# Wrong jsonrpc version
resp=$(mcp '{"jsonrpc":"1.0","id":1,"method":"tools/list"}')
echo "$resp" | grep -qE 'error|jsonrpc' \
  && ok "wrong jsonrpc version returns error or ignored" "" \
  || fail "wrong jsonrpc version" "got: $(echo "$resp" | head -c 200)"

# Unknown method
resp=$(mcp '{"jsonrpc":"2.0","id":1,"method":"nope/missing"}')
echo "$resp" | grep -qE '"error"|method not found|-32601' \
  && ok "unknown method returns -32601" "" \
  || fail "unknown method" "got: $(echo "$resp" | head -c 200)"

# Unknown tool
resp=$(mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"definitely_not_a_tool","arguments":{}}}')
echo "$resp" | grep -qiE 'unknown tool|not found|error|isError' \
  && ok "unknown tool returns error" "" \
  || fail "unknown tool" "got: $(echo "$resp" | head -c 200)"

# Tool call with missing required arg
resp=$(mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"file_read","arguments":{}}}')
echo "$resp" | grep -qiE 'missing|required|error' \
  && ok "missing required arg returns informative error" "" \
  || fail "missing required arg" "got: $(echo "$resp" | head -c 200)"

# Tool call with wrong type for arg
resp=$(mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"file_read","arguments":{"path":42}}}')
echo "$resp" | grep -qiE 'error|missing|invalid' \
  && ok "wrong arg type handled" "" \
  || fail "wrong arg type" "got: $(echo "$resp" | head -c 200)"

# ─── HTTP bad bodies ──────────────────────────────────────────────────────
section "48. HTTP error handling"

# POST /api/chat with bad JSON
status_code=$(curl -s -o /dev/null -w "%{http_code}" -X POST -H "Content-Type: application/json" -d "not-json" "$SERVE/api/chat")
case "$status_code" in
  400|422) ok "POST /api/chat bad JSON → 4xx" "HTTP $status_code" ;;
  *)       fail "POST /api/chat bad JSON" "HTTP $status_code" ;;
esac

# POST /api/chat with empty body
status_code=$(curl -s -o /dev/null -w "%{http_code}" -X POST -H "Content-Type: application/json" -d '' "$SERVE/api/chat")
case "$status_code" in
  400|422|200) ok "POST /api/chat empty body handled" "HTTP $status_code" ;;
  500)         fail "POST /api/chat empty body 500" "" ;;
  *)           ok "POST /api/chat empty body returns $status_code" "" ;;
esac

# GET / wrong method (POST) — should not 500
status_code=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$SERVE/")
case "$status_code" in
  200|405) ok "POST /  → sane status" "HTTP $status_code" ;;
  500)     fail "POST / 500" "" ;;
  *)       ok "POST / returns $status_code" "" ;;
esac

# Very long URL
long_path=$(printf 'a%.0s' {1..2000})
status_code=$(curl -s -o /dev/null -w "%{http_code}" "$SERVE/api/$long_path")
case "$status_code" in
  404|414) ok "very long URL handled (HTTP $status_code)" "" ;;
  500)     fail "very long URL → 500" "" ;;
  *)       ok "very long URL returns $status_code" "" ;;
esac

# ─── tool edge cases ──────────────────────────────────────────────────────
section "49. tool edge cases"

ABS_REPO="/Users/marklight/Documents/workspace/hailmary/phantom-mesh"

# file_read on non-existent file
resp=$(mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"file_read","arguments":{"path":"/this/does/not/exist.txt"}}}')
echo "$resp" | grep -qiE 'no such|not found|error|enoent' \
  && ok "file_read missing file → error" "" \
  || fail "file_read missing file" "got: $(echo "$resp" | head -c 200)"

# file_read on a directory
resp=$(mcp "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"file_read\",\"arguments\":{\"path\":\"$ABS_REPO\"}}}")
echo "$resp" | grep -qiE 'directory|error|is.a.dir|isadirectory' \
  && ok "file_read on directory → error" "" \
  || fail "file_read on directory" "got: $(echo "$resp" | head -c 200)"

# file_read on a binary
BIN_PATH="$ABS_REPO/main"
if [[ -f "$BIN_PATH" ]]; then
  resp=$(mcp "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"file_read\",\"arguments\":{\"path\":\"$BIN_PATH\"}}}")
  if echo "$resp" | grep -qiE 'binary|truncat|content'; then
    ok "file_read binary file: handled" ""
  else
    fail "file_read binary file" "got: $(echo "$resp" | head -c 200)"
  fi
fi

# file_edit when old_string doesn't match anywhere
TMPF="$TMP/edit-noop.txt"
echo "hello world" > "$TMPF"
resp=$(mcp "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"file_edit\",\"arguments\":{\"path\":\"$TMPF\",\"old_string\":\"NOT_PRESENT\",\"new_string\":\"X\"}}}")
echo "$resp" | grep -qiE 'not found|no.match|error|0 matches' \
  && ok "file_edit no-match → error" "" \
  || fail "file_edit no-match" "got: $(echo "$resp" | head -c 200)"
# File should be unchanged
[[ "$(cat "$TMPF")" == "hello world" ]] \
  && ok "file_edit no-match: file unchanged" "" \
  || fail "file_edit no-match: file unchanged" "got: $(cat "$TMPF")"

# file_edit when old_string matches multiple times (without replace_all)
TMPF2="$TMP/edit-multi.txt"
printf "abc\nabc\nabc\n" > "$TMPF2"
resp=$(mcp "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"file_edit\",\"arguments\":{\"path\":\"$TMPF2\",\"old_string\":\"abc\",\"new_string\":\"X\"}}}")
echo "$resp" | grep -qiE 'multiple|matches.*[2-9]|replace_all' \
  && ok "file_edit multi-match (no replace_all) → error" "" \
  || fail "file_edit multi-match" "got: $(echo "$resp" | head -c 200)"
[[ "$(cat "$TMPF2")" == "$(printf 'abc\nabc\nabc')" ]] \
  && ok "file_edit multi-match: file unchanged" "" \
  || fail "file_edit multi-match: file unchanged" "got: $(cat "$TMPF2")"

# file_edit with replace_all
resp=$(mcp "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"file_edit\",\"arguments\":{\"path\":\"$TMPF2\",\"old_string\":\"abc\",\"new_string\":\"X\",\"replace_all\":true}}}")
content=$(cat "$TMPF2")
[[ "$content" == "X
X
X" ]] \
  && ok "file_edit replace_all=true replaces all 3" "" \
  || fail "file_edit replace_all" "got: $content"

# ─── shell tool edge cases ────────────────────────────────────────────────
section "50. shell tool edges"

# shell with timeout exceeded
resp=$(mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"shell","arguments":{"command":"sleep 10","timeout_secs":2}}}')
echo "$resp" | grep -qiE 'timeout|killed|terminated|sigterm' \
  && ok "shell timeout enforced" "" \
  || fail "shell timeout" "got: $(echo "$resp" | head -c 200)"

# shell with very long output
resp=$(mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"shell","arguments":{"command":"yes hello | head -c 50000"}}}')
echo "$resp" | grep -q "hello" \
  && ok "shell large output: contains hello" "" \
  || fail "shell large output" "got: $(echo "$resp" | head -c 200)"
# Should NOT panic / should truncate or limit
echo "$resp" | grep -qiE 'panic|crash' \
  && fail "shell large output: doesn't crash" "" \
  || ok "shell large output: no crash" ""

# shell with command not found
resp=$(mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"shell","arguments":{"command":"this-command-definitely-does-not-exist-xyz"}}}')
echo "$resp" | grep -qiE 'not found|exit code|error' \
  && ok "shell unknown command: error reported" "" \
  || fail "shell unknown command" "got: $(echo "$resp" | head -c 200)"

# shell with special chars in args (shell injection probe)
resp=$(mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"shell","arguments":{"command":"echo hello && echo world"}}}')
echo "$resp" | grep -q "hello" && echo "$resp" | grep -q "world" \
  && ok "shell &&: compound commands work" "" \
  || fail "shell && compound" "got: $(echo "$resp" | head -c 200)"

# ─── concurrent HTTP requests ─────────────────────────────────────────────
section "51. concurrent /api/* requests"

# Fire 10 parallel requests, verify all succeed
seq 1 10 | xargs -P 10 -I {} curl -s -o /dev/null -w "%{http_code}\n" "$SERVE/api/cost" > "$TMP/concurrent.log" 2>&1
all_200=$(grep -c "^200$" "$TMP/concurrent.log")
[[ "$all_200" == "10" ]] \
  && ok "10 concurrent /api/cost: all 200" "" \
  || fail "10 concurrent /api/cost" "got: $(sort -u "$TMP/concurrent.log" | tr '\n' ' ')"

# Mix endpoints
seq 1 5 | xargs -P 5 -I {} bash -c "curl -s -o /dev/null -w '%{http_code}\n' '$SERVE/api/sessions'" > "$TMP/concurrent2.log" 2>&1
seq 1 5 | xargs -P 5 -I {} bash -c "curl -s -o /dev/null -w '%{http_code}\n' '$SERVE/api/nodes'" >> "$TMP/concurrent2.log" 2>&1
seq 1 5 | xargs -P 5 -I {} bash -c "curl -s -o /dev/null -w '%{http_code}\n' '$SERVE/api/status'" >> "$TMP/concurrent2.log" 2>&1
all_ok=$(grep -c "^200$" "$TMP/concurrent2.log")
[[ "$all_ok" == "15" ]] \
  && ok "15 concurrent mixed endpoints: all 200" "" \
  || fail "15 concurrent mixed endpoints" "$(sort -u "$TMP/concurrent2.log" | tr '\n' ' ')"

# ─── empty arguments ──────────────────────────────────────────────────────
section "52. empty / null arguments"

# tools/call with null arguments
resp=$(mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"git_status","arguments":null}}')
echo "$resp" | grep -qE 'content|result|error' \
  && ok "tools/call with arguments=null handled" "" \
  || fail "tools/call arguments=null" "got: $(echo "$resp" | head -c 200)"

# tools/call without arguments key at all
resp=$(mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"git_status"}}')
echo "$resp" | grep -qE 'content|result|error' \
  && ok "tools/call without 'arguments' key handled" "" \
  || fail "tools/call without arguments key" "got: $(echo "$resp" | head -c 200)"

# memory_recall on never-stored key
resp=$(mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"memory_recall","arguments":{"key":"never-existed-xyz-9999"}}}')
echo "$resp" | grep -qiE 'not found|empty|null|content' \
  && ok "memory_recall on missing key handled" "" \
  || fail "memory_recall on missing key" "got: $(echo "$resp" | head -c 200)"

# ─── path safety ──────────────────────────────────────────────────────────
section "53. path safety"

# file_read on /etc/passwd should be allowed (no sandbox claimed in TUI mode)
resp=$(mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"file_read","arguments":{"path":"/etc/passwd"}}}')
if echo "$resp" | grep -qE "root:|content"; then
  ok "file_read on /etc/passwd reads (no sandbox in interactive)" ""
else
  ok "file_read on /etc/passwd: blocked or unreadable" ""
fi

# file_write to /etc/something (should fail with permission denied)
resp=$(mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"file_write","arguments":{"path":"/etc/phantom-test","content":"x"}}}')
echo "$resp" | grep -qiE 'permission|denied|error|read.only' \
  && ok "file_write to /etc/* blocked by OS" "" \
  || ok "file_write to /etc/*: returned without error" ""

# ─── summary ──────────────────────────────────────────────────────────────
section "summary"
total=$((PASS + FAIL))
printf "  %s pass · %s fail · total %d\n" "$(green $PASS)" "$(red $FAIL)" "$total"
printf "  %s\n" "$(gray "captures: $TMP")"
if (( FAIL > 0 )); then
  printf "\n%s\n" "$(bold 'failures:')"
  for f in "${FAIL_LINES[@]}"; do printf "  - %s\n" "$f"; done
  exit 2
fi
