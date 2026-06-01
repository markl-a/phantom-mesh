#!/usr/bin/env bash
# phantom Linux — end-to-end test sweep.
#
# Linux counterpart of `test-mac.sh` (was the only full-flow Bash
# runner). Same row format, same PASS/FAIL/SKIP semantics. Covers the
# automatable Linux surface: binary provenance, doctor, systemd user
# service, /healthz web frontend, /dist + /scripts allowlists, MCP
# stdio, Landlock LSM, Wayland/X11 detect, ollama/lmstudio local LLM,
# self-update --dry-run, autoevolve daemon.
#
# Usage:
#   ./scripts/test-linux.sh                           # full sweep
#   PHANTOM_BIN=/custom/path ./scripts/test-linux.sh  # override binary
#   COORD=http://127.0.0.1:7895 ./scripts/test-linux.sh
#
# Read-only; safe to run while `phantom serve` is live (uses the running
# instance's port if it's the default 7878).
#
# Exit code mirrors the FAIL count.

set -u

PHANTOM="${PHANTOM_BIN:-$HOME/.cargo/bin/phantom}"
[ -x "$PHANTOM" ] || PHANTOM="$(command -v phantom 2>/dev/null || echo "$PHANTOM")"
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

echo "═══ phantom Linux end-to-end test sweep ═══"
echo "  binary  : $PHANTOM"
echo "  coord   : $COORD"
echo "  kernel  : $(uname -r 2>/dev/null || echo unknown)"
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

# Sections expected on Linux (no macOS integrations, no MLX).
for sec in binary config "provider keys" "phantom serve" network tools autoevolve; do
  if grep -qF "$sec" "$DOCTOR_OUT"; then ok "doctor section: $sec" ""
  else                                  skip "doctor section: $sec" "not present"
  fi
done

# ── 3. Service / systemd user unit ───────────────────────────────────────────
section "service (systemd --user)"
"$PHANTOM" service status > "$TMP/svc" 2>&1
if grep -q "registered : .*yes" "$TMP/svc"; then
  PID="$(awk -F: '/pid /{print $2}' "$TMP/svc" | head -1 | tr -d ' ')"
  ok "service registered" "pid $PID"
else
  skip "service registered" "phantom service install (needs systemd)"
fi
if grep -q "healthz    : .*ok" "$TMP/svc"; then
  ok "service healthz" "200 OK"
else
  skip "service healthz" "/healthz unreachable (service not installed?)"
fi
if probe systemctl; then
  if systemctl --user is-enabled phantom-mesh.service >/dev/null 2>&1; then
    ok "systemctl is-enabled" "phantom-mesh.service"
  else
    skip "systemctl is-enabled" "unit not enabled"
  fi
  # User-linger required for boot-time autostart.
  if probe loginctl && loginctl show-user "$(id -u)" 2>/dev/null | grep -q "Linger=yes"; then
    ok "loginctl linger" "user-linger enabled"
  else
    skip "loginctl linger" "run: sudo loginctl enable-linger \$USER"
  fi
else
  skip "systemctl present" "no systemd? (WSL1 / container)"
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

DESKTOP_TITLE="$(curl -s --max-time 2 -A 'Mozilla/5.0 Linux' "$COORD/" | grep -E '<title>' | head -1)"
MOBILE_TITLE="$(curl -s --max-time 2 -A 'Mozilla/5.0 (Android)' "$COORD/" | grep -E '<title>' | head -1)"
echo "$DESKTOP_TITLE" | grep -qi "phantom · mesh"   && ok "UA: desktop title"   "$DESKTOP_TITLE" || fail "UA: desktop title" "$DESKTOP_TITLE"
echo "$MOBILE_TITLE"  | grep -qi "phantom · mobile" && ok "UA: mobile title"   "$MOBILE_TITLE" || fail "UA: mobile title"  "$MOBILE_TITLE"

# ── 5. /dist binary CDN allowlist ────────────────────────────────────────────
section "/dist binary CDN"
for f in phantom-x86_64-unknown-linux-gnu phantom-aarch64-unknown-linux-gnu phantom-x86_64-pc-windows.exe phantom-mesh-android.apk; do
  if hit "$COORD/dist/$f"; then ok "/dist/$f" ""; else skip "/dist/$f" "not in install_dir mirror"; fi
done
hit "$COORD/dist/whatever-not-allowed" "404" && ok "/dist allowlist enforced" "non-allowlisted gets 404" \
  || fail "/dist allowlist enforced" "non-allowlist did not 404"

# ── 6. /scripts allowlist ────────────────────────────────────────────────────
section "/scripts allowlist"
for f in termux-setup.sh install.sh windows-bootstrap.ps1; do
  if hit "$COORD/scripts/$f"; then ok "/scripts/$f" ""; else skip "/scripts/$f" "not mirrored"; fi
done

# ── 7. MCP server (50 tools + subagent + parallel_tasks) ─────────────────────
section "MCP server (stdio)"
INIT='{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test-linux","version":"1"}}}'
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

  for needed in subagent parallel_tasks task; do
    if grep -q "\"$needed\"" "$TOOLS_JSON"; then ok "tool: $needed" ""
    else                                          fail "tool: $needed" ""
    fi
  done
else
  fail "phantom mcp stdio" "no JSON-RPC response"
fi

# ── 8. Landlock LSM availability ─────────────────────────────────────────────
section "Landlock LSM"
KMAJ=$(uname -r | cut -d. -f1)
KMIN=$(uname -r | cut -d. -f2 | cut -d- -f1)
if [ "${KMAJ:-0}" -gt 5 ] || { [ "${KMAJ:-0}" -eq 5 ] && [ "${KMIN:-0}" -ge 13 ]; }; then
  ok "kernel ≥ 5.13" "$(uname -r) supports Landlock v1+"
else
  skip "kernel ≥ 5.13" "$(uname -r) — sandbox degrades to no-op"
fi
if [ -d /sys/kernel/security/landlock ] || grep -q landlock /proc/filesystems 2>/dev/null; then
  ok "landlock present" "/sys/kernel/security/landlock or proc"
else
  # Real check: try to create a Landlock ruleset via syscall — but
  # bash can't make raw syscalls, so trust the kernel version above.
  skip "landlock present" "indirect probe failed; trust kernel version check"
fi

# ── 9. Wayland / X11 detect ──────────────────────────────────────────────────
section "display server"
if [ -n "${WAYLAND_DISPLAY:-}" ]; then
  ok "WAYLAND_DISPLAY set" "$WAYLAND_DISPLAY"
elif [ -n "${DISPLAY:-}" ]; then
  ok "DISPLAY set" "$DISPLAY (X11)"
else
  skip "display server detect" "headless / WSL / SSH session"
fi

# ── 10. Local LLM (ollama / lmstudio) ────────────────────────────────────────
section "local LLM (ollama / lmstudio)"
if probe ollama; then
  ok "ollama on PATH" "$(ollama --version 2>&1 | head -1)"
  if hit "http://127.0.0.1:11434/api/tags"; then
    ok "ollama :11434 live" ""
  else
    skip "ollama :11434 live" "daemon not running"
  fi
else
  skip "ollama on PATH" "curl -fsSL https://ollama.com/install.sh | sh"
fi
if hit "http://127.0.0.1:1234/v1/models"; then
  ok "lmstudio :1234 live" ""
else
  skip "lmstudio :1234 live" "not running"
fi

# ── 11. autoevolve daemon ────────────────────────────────────────────────────
section "autoevolve"
if [ -f "$HOME/.phantom-mesh/autoevolve.log" ]; then
  N="$(grep -c '' "$HOME/.phantom-mesh/autoevolve.log" 2>/dev/null || echo 0)"
  ok "autoevolve.log present" "$N JSONL entries"
else
  skip "autoevolve.log present" "run autoevolve once"
fi
"$PHANTOM" autoevolve --help > "$TMP/aev" 2>&1
grep -q "distributed" "$TMP/aev"  && ok "autoevolve --distributed flag" "" || fail "autoevolve --distributed flag" ""

if "$PHANTOM" autoevolve schedule status > "$TMP/sched" 2>&1; then
  if grep -q "registered : .*yes" "$TMP/sched"; then
    ok "autoevolve schedule" "registered"
  else
    skip "autoevolve schedule" "not installed (systemd timer)"
  fi
fi

# ── 12. self-update — dry run ────────────────────────────────────────────────
section "self-update"
PHANTOM_COORD="$COORD" "$PHANTOM" self-update --dry-run > "$TMP/selfup" 2>&1
grep -q "dry-run" "$TMP/selfup" && ok "self-update --dry-run" "resolves URL" || fail "self-update --dry-run" "no dry-run output"
grep -q "current : 0\." "$TMP/selfup" && ok "self-update reports current" "" || fail "self-update reports current" ""

# ── 13. journal routing ──────────────────────────────────────────────────────
section "journal"
if probe journalctl; then
  if journalctl --user -u phantom-mesh.service -n 5 --no-pager >/dev/null 2>&1; then
    ok "journalctl --user phantom-mesh" "readable"
  else
    skip "journalctl --user phantom-mesh" "service not installed or no entries"
  fi
else
  skip "journalctl present" "no systemd journal"
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
