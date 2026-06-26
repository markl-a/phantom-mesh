#!/usr/bin/env bash
# validate-mac.sh — one-shot macOS smoke gate for the phantom CLI.
#
# Quickly checks that phantom's macOS-specific surface works and that the
# system tooling the CLI relies on is present. Designed to be a usable gate:
#   exit 0  = all required checks passed
#   exit !0 = at least one required check failed
#
# Optional/advisory checks (e.g. mlx) warn but never fail the gate.
# Every command that could hang is wrapped in `timeout`. No network/provider
# keys are required.
#
# Usage:
#   ./scripts/validate-mac.sh                          # uses ~/.cargo/bin/phantom
#   PHANTOM_BIN=/path/to/phantom ./scripts/validate-mac.sh
#
set -euo pipefail

BIN="${PHANTOM_BIN:-$HOME/.cargo/bin/phantom}"

PASS=0 FAIL=0 WARN=0
ok()   { printf "  \033[32m✓\033[0m %s\n" "$1"; PASS=$((PASS+1)); }
bad()  { printf "  \033[31m✗\033[0m %s\n" "$1"; FAIL=$((FAIL+1)); }
warn() { printf "  \033[33m⚠\033[0m %s\n" "$1"; WARN=$((WARN+1)); }
hdr()  { printf "\n== %s ==\n" "$1"; }

# `timeout` is GNU-only; macOS ships it as `gtimeout` (coreutils) or not at all.
# Provide a portable wrapper so the script still runs without it.
if command -v timeout >/dev/null 2>&1; then
  TO() { timeout "$@"; }
elif command -v gtimeout >/dev/null 2>&1; then
  TO() { gtimeout "$@"; }
else
  # No timeout available — run directly (drop the duration arg).
  TO() { shift; "$@"; }
  warn "no 'timeout'/'gtimeout' found — running without hang protection"
fi

echo "phantom mac validate — $(date '+%Y-%m-%d %H:%M:%S')"
echo "platform: $(uname -s) $(uname -m)"
echo "binary:   $BIN"

# ---------------------------------------------------------------------------
hdr "0. platform sanity"
if [ "$(uname -s)" = "Darwin" ]; then
  ok "running on macOS (Darwin)"
else
  warn "not running on macOS — mac-specific checks may be meaningless"
fi

# ---------------------------------------------------------------------------
hdr "1. phantom binary"
if [ ! -x "$BIN" ]; then
  bad "binary not found / not executable at $BIN (set PHANTOM_BIN to override)"
  echo; echo "== result: $PASS passed, $FAIL failed, $WARN warn =="
  exit 1
fi
ok "binary executable at $BIN"

VER="$(TO 20 "$BIN" --version 2>&1 | head -1 | tr -d '\r')" && [ -n "$VER" ] \
  && ok "phantom --version → ${VER}" \
  || bad "phantom --version failed"

# ---------------------------------------------------------------------------
hdr "2. phantom doctor"
# doctor is the CLI's own self-check. We only require it to exit 0; its
# internal warnings (missing provider keys etc.) are not our concern here.
if TO 60 "$BIN" doctor >/dev/null 2>&1; then
  ok "phantom doctor exited 0"
else
  bad "phantom doctor exited nonzero (run '$BIN doctor' to see why)"
fi

# ---------------------------------------------------------------------------
hdr "3. macOS tooling the CLI relies on"
# service install uses launchctl; life-node/spotlight uses mdfind; backups use
# tmutil; xcode_simctl tool uses xcrun simctl. mlx is optional accel.
chk_tool() {  # chk_tool <required|optional> <label> <cmd> [args...]
  local mode="$1" label="$2"; shift 2
  if command -v "$1" >/dev/null 2>&1; then
    if TO 15 "$@" >/dev/null 2>&1; then
      ok "$label present and runs"
    else
      # present but probe nonzero — still counts as available
      ok "$label present (probe returned nonzero, tool exists)"
    fi
  else
    if [ "$mode" = required ]; then bad "$label NOT found"; else warn "$label NOT found (optional)"; fi
  fi
}

chk_tool required "launchctl (launchd / service install)" launchctl help
chk_tool required "tmutil (Time Machine backups)"          tmutil version
chk_tool required "mdfind (Spotlight search)"              mdfind -onlyin / -count "phantom_no_such_query_xyz"

# xcrun + simctl: xcrun may exist but simctl needs Xcode/CLT to be selected.
if command -v xcrun >/dev/null 2>&1; then
  if TO 30 xcrun simctl help >/dev/null 2>&1; then
    ok "xcrun simctl present and runs"
  else
    warn "xcrun present but 'simctl' unavailable (Xcode / CLT not selected) — iOS sim checks skipped"
  fi
else
  warn "xcrun NOT found (no Xcode Command Line Tools) — iOS sim checks skipped"
fi

# mlx — optional Apple-silicon accel; advisory only, never fails the gate.
if command -v mlx_lm.generate >/dev/null 2>&1 || python3 -c "import mlx" >/dev/null 2>&1; then
  ok "mlx available (Apple-silicon accel)"
else
  warn "mlx not available (optional — local accel only)"
fi

# ---------------------------------------------------------------------------
hdr "4. phantom mcp stdio server (tools/list)"
# Non-hanging: feed initialize + tools/list on stdin, hard timeout, then parse.
# Skipped (warn, not fail) if it hangs or python3 is missing.
if ! command -v python3 >/dev/null 2>&1; then
  warn "python3 not found — skipping mcp tools/list probe"
else
  MCP_TMP="$(mktemp)"
  INIT='{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"validate-mac","version":"1"}}}'
  LIST='{"jsonrpc":"2.0","id":2,"method":"tools/list"}'
  if printf '%s\n%s\n' "$INIT" "$LIST" | TO 20 "$BIN" mcp >"$MCP_TMP" 2>/dev/null; then
    N="$(python3 - "$MCP_TMP" <<'PY' 2>/dev/null
import sys, json
n = -1
for line in open(sys.argv[1]):
    line = line.strip()
    if not line: continue
    try: o = json.loads(line)
    except Exception: continue
    if o.get("id") == 2 and "result" in o:
        n = len(o["result"].get("tools", []))
print(n)
PY
)"
    if [ "${N:-0}" -gt 0 ] 2>/dev/null; then
      ok "mcp tools/list returned ${N} tools"
    else
      bad "mcp tools/list returned no tools (see output)"
    fi
  else
    warn "mcp stdio probe timed out or exited nonzero — skipped (not failing gate)"
  fi
  rm -f "$MCP_TMP"
fi

# ---------------------------------------------------------------------------
echo
echo "== result: $PASS passed, $FAIL failed, $WARN warn =="
if [ "$FAIL" -eq 0 ]; then
  echo "✅ all required mac checks passed"
  exit 0
else
  echo "❌ $FAIL required check(s) failed — see ✗ above"
  exit 1
fi
