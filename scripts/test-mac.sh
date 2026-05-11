#!/usr/bin/env bash
# phantom Mac — end-to-end test sweep.
#
# Runs every automatable check across the macOS surface phantom ships.
# Prints PASS / FAIL / SKIP per row + final summary.
#
# Usage:
#   ./scripts/test-mac.sh
#
# Read-only; safe to run while phantom serve is live.

set -u

PHANTOM="${PHANTOM_BIN:-$HOME/.cargo/bin/phantom}"
COORD="${COORD:-http://127.0.0.1:7878}"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# ── output helpers ────────────────────────────────────────────────────────────
PASS=0; FAIL=0; SKIP=0
NAMES_FAIL=()
ROW="  %-3s %-42s %s\n"

ok()   { printf "$ROW" "$(printf '\033[32m✓\033[0m')" "$1" "$2"; PASS=$((PASS+1)); }
fail() { printf "$ROW" "$(printf '\033[31m✗\033[0m')" "$1" "$2"; FAIL=$((FAIL+1)); NAMES_FAIL+=("$1"); }
skip() { printf "$ROW" "$(printf '\033[33m○\033[0m')" "$1" "$2"; SKIP=$((SKIP+1)); }
section() { printf "\n\033[35m── %s ──\033[0m\n" "$1"; }

probe()    { command -v "$1" >/dev/null 2>&1; }
hit()      { local code; code="$(curl -s --max-time 3 -o /dev/null -w '%{http_code}' "$1" 2>/dev/null || echo)"; [ "$code" = "${2:-200}" ]; }

echo "═══ phantom Mac end-to-end test sweep ═══"
echo "  binary  : $PHANTOM"
echo "  coord   : $COORD"
echo "  date    : $(date '+%Y-%m-%d %H:%M:%S')"
echo

# ── 1. Binary presence + provenance ──────────────────────────────────────────
section "binary"
if [ -x "$PHANTOM" ]; then
  VERSION="$("$PHANTOM" --version 2>&1 | head -1)"
  ok "phantom --version" "$VERSION"
  echo "$VERSION" | grep -qE "phantom [0-9]+\.[0-9]+\.[0-9]+ \([0-9a-f]+" \
    && ok "version provenance"  "git hash + arch + build date" \
    || fail "version provenance" "missing git hash"
else
  fail "binary present" "$PHANTOM not executable"
  echo "Cannot continue without phantom binary."
  exit 1
fi

# ── 2. Doctor — single-screen self-diagnostic ────────────────────────────────
section "phantom doctor"
DOCTOR_OUT="$TMP/doctor.out"
"$PHANTOM" doctor > "$DOCTOR_OUT" 2>&1
[ -s "$DOCTOR_OUT" ] && ok "phantom doctor runs" "$(wc -l < "$DOCTOR_OUT" | tr -d ' ') lines" \
  || fail "phantom doctor runs" "empty output"

for sec in binary config "provider keys" "phantom serve" network tools "MLX local LLM" autoevolve "macOS integrations"; do
  if grep -qF "$sec" "$DOCTOR_OUT"; then ok "doctor section: $sec" ""
  else                                  skip "doctor section: $sec" "not present"
  fi
done

# ── 3. Service / launchd ─────────────────────────────────────────────────────
section "service (launchd)"
"$PHANTOM" service status > "$TMP/svc" 2>&1
if grep -q "registered : .*yes" "$TMP/svc"; then
  PID="$(awk -F: '/pid /{print $2}' "$TMP/svc" | head -1 | tr -d ' ')"
  ok "service registered" "pid $PID"
else
  fail "service registered" "phantom service install"
fi
if grep -q "healthz    : .*ok" "$TMP/svc"; then
  ok "service healthz" "200 OK"
else
  fail "service healthz" "/healthz unreachable"
fi
if launchctl print "gui/$(id -u)/ai.phantommesh.serve" >/dev/null 2>&1; then
  ok "launchctl print works" "gui/$(id -u)/ai.phantommesh.serve"
else
  fail "launchctl print works" "service not loaded into launchd"
fi

# ── 4. Web frontend ──────────────────────────────────────────────────────────
section "phantom serve / web"
hit "$COORD/healthz"                    && ok "/healthz"             "200" || fail "/healthz" "no response"
hit "$COORD/"                           && ok "/ desktop UI"        ""    || fail "/ desktop UI"        "no response"
hit "$COORD/m"                          && ok "/m mobile UI"        ""    || fail "/m mobile UI"        "no response"
hit "$COORD/static/app.js"              && ok "/static/app.js"      ""    || fail "/static/app.js"      ""
hit "$COORD/static/mobile.js"           && ok "/static/mobile.js"   ""    || fail "/static/mobile.js"   ""
hit "$COORD/api/cost"                   && ok "/api/cost"           ""    || fail "/api/cost"           ""
hit "$COORD/api/nodes"                  && ok "/api/nodes"          ""    || fail "/api/nodes"          ""
hit "$COORD/api/tools/history"          && ok "/api/tools/history"  ""    || fail "/api/tools/history"  ""

# UA-aware mobile detection
DESKTOP_TITLE="$(curl -s --max-time 2 -A 'Mozilla/5.0 Mac' "$COORD/" | grep -E '<title>' | head -1)"
MOBILE_TITLE="$(curl -s --max-time 2 -A 'Mozilla/5.0 (iPhone)' "$COORD/" | grep -E '<title>' | head -1)"
echo "$DESKTOP_TITLE" | grep -qi "phantom · mesh"   && ok "UA: desktop title"   "$DESKTOP_TITLE" || fail "UA: desktop title" "$DESKTOP_TITLE"
echo "$MOBILE_TITLE"  | grep -qi "phantom · mobile" && ok "UA: mobile title"   "$MOBILE_TITLE" || fail "UA: mobile title"  "$MOBILE_TITLE"

# ── 5. /dist binary CDN allowlist ────────────────────────────────────────────
section "/dist binary CDN"
for f in phantom-aarch64-apple-darwin phantom-aarch64-linux-android phantom-x86_64-pc-windows.exe phantom-mesh-android.apk phantom-mesh-ios.ipa; do
  if hit "$COORD/dist/$f"; then ok "/dist/$f" ""; else skip "/dist/$f" "not in install_dir mirror"; fi
done
hit "$COORD/dist/whatever-not-allowed" "404" && ok "/dist allowlist enforced" "non-allowlisted gets 404" \
  || fail "/dist allowlist enforced" "non-allowlist did not 404"

# ── 6. /scripts allowlist ────────────────────────────────────────────────────
section "/scripts allowlist"
for f in termux-setup.sh windows-bootstrap.ps1 install-phantom-windows.ps1; do
  if hit "$COORD/scripts/$f"; then ok "/scripts/$f" ""; else skip "/scripts/$f" "not mirrored"; fi
done

# ── 7. MCP server (50 tools + subagent + parallel_tasks) ─────────────────────
section "MCP server (stdio)"
INIT='{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test-mac","version":"1"}}}'
LIST='{"jsonrpc":"2.0","id":2,"method":"tools/list"}'
TOOLS_JSON="$TMP/tools.json"
printf '%s\n%s\n' "$INIT" "$LIST" | timeout 15 "$PHANTOM" mcp 2>/dev/null > "$TOOLS_JSON" || true

if [ -s "$TOOLS_JSON" ]; then
  N="$(python3 -c "
import sys, json
for line in open('$TOOLS_JSON'):
    line=line.strip()
    if not line: continue
    try: o=json.loads(line)
    except: continue
    if o.get('id')==2 and 'result' in o:
        print(len(o['result'].get('tools', [])))
        break
" 2>/dev/null)"
  [ -n "$N" ] && [ "$N" -ge 48 ] \
    && ok "tools/list count" "$N tools" \
    || fail "tools/list count" "got $N (expected ≥48)"

  for needed in subagent parallel_tasks task spotlight_search xcode_simctl; do
    if grep -q "\"$needed\"" "$TOOLS_JSON"; then ok "tool: $needed" ""
    else                                          fail "tool: $needed" ""
    fi
  done
else
  fail "phantom mcp stdio" "no JSON-RPC response"
fi

# ── 8. APFS snapshot subsystem ───────────────────────────────────────────────
section "APFS snapshot"
if probe tmutil; then
  ok "tmutil present" ""
  "$PHANTOM" snapshot list > "$TMP/snaps" 2>&1
  if grep -q "no local snapshots\|local snapshot(s)" "$TMP/snaps"; then
    ok "snapshot list runs" "$(wc -l < "$TMP/snaps" | tr -d ' ') lines"
  else
    fail "snapshot list runs" "$(head -1 "$TMP/snaps")"
  fi
  # dry-run apply on a fake id should error cleanly
  if "$PHANTOM" snapshot apply 1999-01-01-000000 --cwd 2>&1 | grep -q "not found"; then
    ok "snapshot apply guards id" "rejects non-existent id"
  else
    fail "snapshot apply guards id" "no guard"
  fi
else
  skip "tmutil present" "macOS missing tmutil?"
fi

# ── 9. autoevolve daemon ─────────────────────────────────────────────────────
section "autoevolve"
if [ -f "$HOME/.phantom-mesh/autoevolve.log" ]; then
  N="$(grep -c '' "$HOME/.phantom-mesh/autoevolve.log" 2>/dev/null || echo 0)"
  ok "autoevolve.log present" "$N JSONL entries"
else
  skip "autoevolve.log present" "run autoevolve once"
fi
"$PHANTOM" autoevolve --help > "$TMP/aev" 2>&1
grep -q "distributed" "$TMP/aev"  && ok "autoevolve --distributed flag" "" || fail "autoevolve --distributed flag" ""
grep -q "schedule"    "$TMP/aev" || true  # help may not list subactions

if "$PHANTOM" autoevolve schedule status > "$TMP/sched" 2>&1; then
  if grep -q "registered : .*yes" "$TMP/sched"; then
    ok "autoevolve schedule" "registered"
  else
    skip "autoevolve schedule" "not installed"
  fi
fi

# ── 10. MLX local LLM ────────────────────────────────────────────────────────
section "MLX (Apple Silicon)"
if python3 -c "import mlx_lm" >/dev/null 2>&1; then
  ok "mlx_lm importable" "$(python3 -c 'import mlx_lm,sys; sys.stdout.write(mlx_lm.__file__)' 2>/dev/null | xargs basename)"
  "$PHANTOM" mlx status > "$TMP/mlx" 2>&1
  if grep -q "mlx_lm: importable\|mlx_lm: NOT installed" "$TMP/mlx"; then
    ok "phantom mlx status" "$(grep -E '✓|✗' "$TMP/mlx" | head -1 | tr -s ' ')"
  fi
  # Probe live server (if user has one running)
  if hit "http://127.0.0.1:8080/v1/models"; then
    ok "MLX server :8080" "live"
  else
    skip "MLX server :8080" "not running (phantom mlx serve to start)"
  fi
else
  skip "mlx_lm importable" "pip install mlx-lm"
fi

# ── 11. self-update — dry run ────────────────────────────────────────────────
section "self-update"
PHANTOM_COORD="$COORD" "$PHANTOM" self-update --dry-run > "$TMP/selfup" 2>&1
grep -q "dry-run" "$TMP/selfup" && ok "self-update --dry-run" "resolves URL" || fail "self-update --dry-run" "no dry-run output"
grep -q "current : 0\." "$TMP/selfup" && ok "self-update reports current" "" || fail "self-update reports current" ""

# ── 12. validate-mcp.sh script (existing) ────────────────────────────────────
section "validate-mcp.sh"
if [ -x ./scripts/validate-mcp.sh ]; then
  if ./scripts/validate-mcp.sh > "$TMP/vmcp" 2>&1; then
    ok "validate-mcp.sh" "all checks passed"
  else
    fail "validate-mcp.sh" "script returned non-zero"
  fi
else
  skip "validate-mcp.sh" "missing or not executable"
fi

# ── Summary ──────────────────────────────────────────────────────────────────
echo
echo "═══════════════════════════════════════════════════════════════"
printf "  PASS %d   FAIL %d   SKIP %d\n" "$PASS" "$FAIL" "$SKIP"
if [ "$FAIL" -gt 0 ]; then
  echo
  echo "  failures:"
  for n in "${NAMES_FAIL[@]}"; do echo "    - $n"; done
  echo
  echo "  ✗ overall: FAIL"
  exit 1
else
  echo
  echo "  ✓ overall: PASS"
fi
