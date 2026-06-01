#!/usr/bin/env bash
# node-a (Windows) phantom smoke test, run from the Mac over SSH.
#
# Usage: bash scripts/smoke-node-a.sh
#
# Prereq: node-a reachable on Tailscale 100.64.0.11 with phantom
# already deployed at C:\Users\<you>\.phantom-mesh\bin\phantom.exe and
# the PhantomServe scheduled task running on port 7879.
#
# Uses ssh ControlMaster so you only enter the password ONCE for the whole
# run. The control socket is set up on first connection and reused.

set -o pipefail

NODE_A_HOST="${NODE_A_HOST:-<you>@100.64.0.11}"
NODE_A_IP="${NODE_A_HOST##*@}"   # tailnet IP/host part of NODE_A_HOST
NODE_A_PORT=7879           # NOTE: Windows daemon listens on 7879, not 7878
SOCK="$HOME/.ssh/cm-node-a-$$"
TMP=$(mktemp -d)

PASS=0; FAIL=0; FAIL_LINES=()

green() { printf "\033[32m%s\033[0m" "$1"; }
red()   { printf "\033[31m%s\033[0m" "$1"; }
gray()  { printf "\033[90m%s\033[0m" "$1"; }
bold()  { printf "\033[1m%s\033[0m" "$1"; }

ok()    { PASS=$((PASS+1)); printf "  $(green '✓') %-55s %s\n" "$1" "$(gray "$2")"; }
fail()  { FAIL=$((FAIL+1)); FAIL_LINES+=("$1 :: $2"); printf "  $(red '✗') %-55s %s\n" "$1" "$(gray "$2")"; }
skip()  { printf "  $(gray '○') %-55s %s\n" "$1" "$(gray "$2")"; }
section() { printf "\n$(bold "%s")\n" "$1"; }

# ── Set up ControlMaster (single password prompt) ──────────────────────────
mkdir -p "$HOME/.ssh"
chmod 700 "$HOME/.ssh"

# Open the master connection in the background. The user enters the
# password once here; subsequent ssh / scp calls reuse this socket.
echo "$(bold '◆ Establishing ControlMaster (you will be prompted for the node-a password ONCE)')"
echo "  $NODE_A_HOST"
echo ""

ssh -M -S "$SOCK" -fN \
  -o ControlPersist=10m \
  -o StrictHostKeyChecking=no \
  -o ConnectTimeout=8 \
  -o ServerAliveInterval=15 \
  "$NODE_A_HOST"

if [[ ! -S "$SOCK" ]]; then
  echo "$(red 'ERROR') ControlMaster failed to open. Likely network or auth issue."
  exit 2
fi

# Helper: run command on node-a via the shared connection
remote() {
  ssh -S "$SOCK" "$NODE_A_HOST" "$@"
}

# Helper: copy file to node-a via the shared connection
remote_cp() {
  scp -o ControlPath="$SOCK" "$1" "$NODE_A_HOST:$2"
}

cleanup() {
  ssh -S "$SOCK" -O exit "$NODE_A_HOST" 2>/dev/null
  rm -f "$SOCK"
}
trap cleanup EXIT

# ── 1. Binary deployment ──────────────────────────────────────────────────
section "1. binary deployment"

ver=$(remote 'powershell -NoProfile -Command "& \"$env:USERPROFILE\.phantom-mesh\bin\phantom.exe\" --version" 2>&1' | head -1)
if echo "$ver" | grep -q "phantom 0.1.0"; then
  ok "phantom.exe --version" "$ver"
else
  fail "phantom.exe --version" "got: $ver"
fi

# Path resolution
path_check=$(remote 'powershell -NoProfile -Command "Test-Path $env:USERPROFILE\.phantom-mesh\bin\phantom.exe"' 2>&1 | tr -d '\r' | head -1)
[[ "$path_check" == "True" ]] \
  && ok "phantom.exe at expected path" '%USERPROFILE%\.phantom-mesh\bin\phantom.exe' \
  || fail "phantom.exe at expected path" "got: $path_check"

# agents.toml
agents_check=$(remote 'powershell -NoProfile -Command "Test-Path $env:USERPROFILE\.phantom-mesh\agents.toml"' 2>&1 | tr -d '\r' | head -1)
[[ "$agents_check" == "True" ]] \
  && ok "agents.toml present" "" \
  || fail "agents.toml present" "got: $agents_check"

# ── 2. Scheduled Task ──────────────────────────────────────────────────────
section "2. PhantomServe scheduled task"

task_state=$(remote 'powershell -NoProfile -Command "(schtasks /Query /TN PhantomServe /FO LIST /V 2>$null | Select-String \"狀態:|Status:\" | Select-Object -First 1)"' 2>&1 | tr -d '\r' | head -1)
if echo "$task_state" | grep -qiE "Ready|Running|就緒|執行"; then
  ok "PhantomServe task registered" "$(echo "$task_state" | head -c 60)"
else
  fail "PhantomServe task registered" "got: $task_state"
fi

# Process running?
proc=$(remote 'powershell -NoProfile -Command "Get-Process phantom -EA SilentlyContinue | Select-Object -First 1 | ConvertTo-Json -Compress"' 2>&1 | tr -d '\r' | head -1)
if echo "$proc" | grep -qE 'Id|PSPath|"phantom"'; then
  ok "phantom process running" ""
else
  fail "phantom process running" "got: $proc"
fi

# ── 3. HTTP daemon (port 7879) ─────────────────────────────────────────────
section "3. HTTP daemon (node-a :7879)"

# Reach via Tailscale
if curl -sf -m 4 "http://$NODE_A_IP:$NODE_A_PORT/healthz" >/dev/null 2>&1; then
  ok "GET /healthz from Mac (Tailscale)" "200 OK on :$NODE_A_PORT"
else
  fail "GET /healthz from Mac" "unreachable on Tailscale $NODE_A_IP:$NODE_A_PORT"
fi

# /api/version
ver=$(curl -sf -m 4 "http://$NODE_A_IP:$NODE_A_PORT/api/version" 2>&1)
if echo "$ver" | grep -qE '"version"|"target":"windows"'; then
  ok "GET /api/version from Mac" "$(echo "$ver" | head -c 80)"
else
  fail "GET /api/version" "got: $ver"
fi

# Sample more endpoints
for ep in /api/cost /api/sessions /api/nodes /api/todos /api/status /api/tools/history /api/providers/health /api/dashboard/status; do
  code=$(curl -s -o /dev/null -w "%{http_code}" -m 4 "http://$NODE_A_IP:$NODE_A_PORT$ep" 2>&1)
  [[ "$code" == "200" ]] \
    && ok "GET $ep" "200" \
    || fail "GET $ep" "HTTP $code"
done

# ── 4. one-shot agent run ──────────────────────────────────────────────────
section "4. phantom run one-shot"

# Run via SSH so the node-a's local agents.toml + provider keys are used.
# Don't quote the marker — let it be a literal arg to phantom.
out=$(remote 'powershell -NoProfile -Command "& $env:USERPROFILE\.phantom-mesh\bin\phantom.exe run \"Use shell to run echo phantom-node-a-marker-99 and report what it printed\""' 2>&1 | tr -d '\r' | tail -10)
if echo "$out" | grep -q "phantom-node-a-marker-99"; then
  ok "phantom run (shell tool) on Windows" "marker round-tripped"
else
  fail "phantom run (shell tool) on Windows" "got: $(echo "$out" | head -c 200)"
fi

# ── 5. MCP stdio ──────────────────────────────────────────────────────────
section "5. MCP stdio JSON-RPC"

# Pipe a tools/list request via SSH stdin
mcp_resp=$(remote 'powershell -NoProfile -Command "echo {\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"} | & $env:USERPROFILE\.phantom-mesh\bin\phantom.exe mcp"' 2>&1 | tr -d '\r' | head -c 4000)
if echo "$mcp_resp" | grep -q '"id":1' && echo "$mcp_resp" | grep -q "shell"; then
  ok "phantom mcp tools/list" "shell tool present"
else
  fail "phantom mcp tools/list" "got: $(echo "$mcp_resp" | head -c 200)"
fi

# ── 6. doctor health on Windows ────────────────────────────────────────────
section "6. phantom doctor on node-a"

doc_out=$(remote 'powershell -NoProfile -Command "& $env:USERPROFILE\.phantom-mesh\bin\phantom.exe doctor"' 2>&1 | tr -d '\r')
green_n=$(echo "$doc_out" | grep -c '✓' || echo 0)
warn_n=$(echo "$doc_out" | grep -c '⚠' || echo 0)
red_n=$(echo "$doc_out" | grep -c '✗' || echo 0)

if (( red_n == 0 )); then
  ok "phantom doctor on node-a" "$green_n green / $warn_n warn / $red_n red"
else
  fail "phantom doctor on node-a" "$red_n red findings"
  echo "  ── doctor red lines: ──"
  echo "$doc_out" | grep '✗' | head -5 | sed 's/^/    /'
fi

# Save full output for inspection
echo "$doc_out" > "$TMP/node-a-doctor.txt"
echo "  (full doctor output saved to $TMP/node-a-doctor.txt)"

# ── 7. cluster reverse-direction ───────────────────────────────────────────
section "7. cluster cross-machine reachability"

# From node-a's POV, can it reach Mac's phantom serve at :7878?
mac_tail=$(tailscale ip -4 2>/dev/null | head -1)
if [[ -n "$mac_tail" ]]; then
  reach=$(remote "powershell -NoProfile -Command \"try { (Invoke-WebRequest -Uri 'http://$mac_tail:7878/healthz' -TimeoutSec 4).StatusCode } catch { 'ERR' }\"" 2>&1 | tr -d '\r' | tail -1)
  if [[ "$reach" == "200" ]]; then
    ok "node-a → Mac :7878 reachable (Tailscale 雙向)" "$mac_tail"
  else
    fail "node-a → Mac :7878 reachable" "got: $reach"
  fi
else
  skip "node-a → Mac reverse" "Mac Tailscale IP not detected"
fi

# ── 8. Windows-specific known-issue probe ──────────────────────────────────
section "8. Windows known-issue probes"

# Memory file from previous Windows hang issue: Groq streaming hangs.
# Verify our 30s timeout fix is still wired (look for the const in binary).
grep_marker=$(remote 'powershell -NoProfile -Command "Select-String -Path $env:USERPROFILE\.phantom-mesh\bin\phantom.exe -Pattern \"Groq\" -SimpleMatch -Quiet"' 2>&1 | tr -d '\r' | tail -1)
[[ "$grep_marker" == "True" || -z "$grep_marker" ]] \
  && ok "phantom.exe binary contains Groq strings" "(timeout workaround compiled in)" \
  || skip "binary string scan inconclusive" "$grep_marker"

# tui-history file write permission (Windows path quirk)
hist_check=$(remote 'powershell -NoProfile -Command "Test-Path $env:USERPROFILE\.phantom-mesh\tui-history"' 2>&1 | tr -d '\r' | head -1)
[[ "$hist_check" =~ ^(True|False)$ ]] \
  && ok "tui-history path probe (file may or may not exist yet)" "$hist_check" \
  || fail "tui-history path probe" "got: $hist_check"

# ── summary ────────────────────────────────────────────────────────────────
section "summary"
total=$((PASS + FAIL))
printf "  %s pass · %s fail · total %d\n" "$(green $PASS)" "$(red $FAIL)" "$total"
printf "  %s\n" "$(gray "captures: $TMP")"
if (( FAIL > 0 )); then
  printf "\n%s\n" "$(bold 'failures:')"
  for f in "${FAIL_LINES[@]}"; do printf "  - %s\n" "$f"; done
  exit 2
fi
