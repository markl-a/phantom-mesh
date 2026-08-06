#!/usr/bin/env bash
# Validate that `spectyn mcp` (the stdio MCP server) is healthy enough to be
# used as a subagent provider by Claude Code and Codex CLI.
#
# Checks (in order):
#   1. binary exists, --version works
#   2. initialize handshake returns serverInfo
#   3. tools/list returns ≥ 40 tools, including subagent + parallel_tasks
#   4. tools/call subagent succeeds and returns non-empty output
#      (this is the regression check for init_global() in the mcp path)
#
# Usage:
#   ./scripts/validate-mcp.sh                       # uses ~/.cargo/bin/spectyn
#   SPECTYN_BIN=/path/to/spectyn ./scripts/validate-mcp.sh
#
# Exit codes: 0 = healthy, 1 = any check failed.

set -u
BIN="${SPECTYN_BIN:-$HOME/.cargo/bin/spectyn}"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

ok()   { printf "  \033[32m✓\033[0m %s\n" "$1"; }
fail() { printf "  \033[31m✗\033[0m %s\n" "$1"; FAILED=1; }
FAILED=0

echo "spectyn mcp validate — $(date '+%Y-%m-%d %H:%M:%S')"
echo "binary: $BIN"

# 1. Binary
if [ ! -x "$BIN" ]; then fail "binary not executable"; exit 1; fi
VER=$("$BIN" --version 2>&1 | head -1)
ok "binary runs ($VER)"

# 2+3. initialize + tools/list
INIT='{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"validate-mcp","version":"1"}}}'
LIST='{"jsonrpc":"2.0","id":2,"method":"tools/list"}'
printf '%s\n%s\n' "$INIT" "$LIST" | timeout 15 "$BIN" mcp > "$TMP/list.out" 2>"$TMP/list.err"

python3 - "$TMP/list.out" <<'PY' && ok "tools/list — 48 tools, subagent + parallel_tasks present" || fail "tools/list malformed (see $TMP/list.out)"
import sys, json
out = open(sys.argv[1]).read().splitlines()
got_init = got_list = False
n = 0; has_sub = has_par = False
for line in out:
    if not line.strip(): continue
    try: o = json.loads(line)
    except: continue
    if o.get("id") == 1 and "result" in o and "serverInfo" in o["result"]: got_init = True
    if o.get("id") == 2 and "result" in o:
        tools = o["result"].get("tools", [])
        n = len(tools)
        names = {t["name"] for t in tools}
        has_sub = "subagent" in names
        has_par = "parallel_tasks" in names
        got_list = True
sys.exit(0 if (got_init and got_list and n >= 40 and has_sub and has_par) else 1)
PY

# 4. Real subagent call — regression check for init_global()
CALL='{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"subagent","arguments":{"agent":"master","prompt":"reply with just OK","max_secs":45,"max_rounds":2}}}'
printf '%s\n%s\n' "$INIT" "$CALL" | timeout 60 "$BIN" mcp > "$TMP/call.out" 2>"$TMP/call.err"

if grep -q "runtime not initialised" "$TMP/call.out" "$TMP/call.err" 2>/dev/null; then
  fail "subagent call returned 'runtime not initialised' — init_global() missing in mcp path"
elif grep -q '"isError":false' "$TMP/call.out"; then
  ok "subagent call succeeded (real LLM round-trip)"
else
  fail "subagent call did not return success (see $TMP/call.out)"
fi

# 5. Show registration in Claude Code + Codex (informational)
echo ""
echo "registration status:"
if grep -q '"spectyn"' "$HOME/.claude.json" 2>/dev/null; then
  ok "Claude Code: spectyn present in ~/.claude.json"
else
  fail "Claude Code: spectyn NOT in ~/.claude.json — run: claude mcp add spectyn $BIN mcp"
fi
if grep -q '\[mcp_servers.spectyn\]' "$HOME/.codex/config.toml" 2>/dev/null; then
  ok "Codex: [mcp_servers.spectyn] present in ~/.codex/config.toml"
else
  fail "Codex: [mcp_servers.spectyn] NOT in ~/.codex/config.toml — run: codex mcp add spectyn -- $BIN mcp"
fi

echo ""
if [ "$FAILED" = 0 ]; then
  echo "✅ all checks passed — spectyn is callable as a subagent from Claude Code and Codex."
  exit 0
else
  echo "❌ some checks failed — see messages above."
  exit 1
fi
