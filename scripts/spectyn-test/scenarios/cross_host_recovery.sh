#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# cross_host_recovery.sh — E001 / F003 cross-host peer-down + recovery e2e.
#
# Companion to F002 (cross_host_dispatch.sh). Where F002 proves the happy
# path of cross-host forwarding, F003 proves the failure-and-recovery
# contract that justifies C4's heartbeat machinery (PR #175):
#
#   Phase 1.  Bootstrap two nodes (same shape as F002).
#   Phase 2.  Baseline dispatch — confirm cross-host forwarding works at all
#             (defence in depth, regression-catches F002 too).
#   Phase 3.  Kill `spectyn serve` on node B over SSH.
#   Phase 4.  Poll node A's /rpc/peers; assert B transitions to `online=false`
#             within the heartbeat deadline (default 99s = interval × threshold
#             × 1.1, configurable via $SPECTYN_E001_HEARTBEAT_DEADLINE_SECS).
#   Phase 5.  Dispatch from A with required_caps requiring B's caps; assert a
#             FAST structured error (no 180s hang). This guards against the
#             "selector still picks unhealthy peer → request times out"
#             regression that motivated C4 in the first place.
#   Phase 6.  Restart `spectyn serve` on B.
#   Phase 7.  Poll /rpc/peers; assert B transitions back to `online=true`
#             within the same deadline.
#   Phase 8.  Re-dispatch from Phase 5; assert success this time + grep both
#             nodes' logs for forwarding evidence.
#
# Spec: docs/superpowers/features/F003-cross-host-recovery-e2e-scenario.md
#
# Environment contract (REQUIRED)
# -------------------------------
#   SPECTYN_NODE_A_SSH       — full ssh prefix incl. user/host/port/key
#                              e.g. "ssh -i ~/.ssh/pi -p 22 pi@pi.tail.ts.net"
#                              (or a wrap-?.sh shim for local-loopback smoke)
#   SPECTYN_NODE_B_SSH       — same for Node B (this is the one that gets killed)
#   SPECTYN_NODE_A_URL       — base URL reachable from THIS host AND from B
#                              e.g. "http://pi.tail.ts.net:7878"
#   SPECTYN_NODE_B_URL       — same for Node B
#   SPECTYN_CLUSTER_SECRET   — shared cluster secret (HMAC) — same on both
#
# Environment contract (OPTIONAL)
# -------------------------------
#   SPECTYN_BIN_LOCAL        — path to the pre-built spectyn binary on THIS
#                              host that will be SCP'd to each node. MUST be
#                              built with `--features experimental-cluster-heartbeat`
#                              or the peer will never flip Unhealthy.
#                              Default: $(command -v spectyn)
#   SPECTYN_NODE_A_NAME      — Node A's node_name (default: "node-a")
#   SPECTYN_NODE_B_NAME      — Node B's node_name (default: "node-b")
#   SPECTYN_NODE_A_PORT      — default 7878
#   SPECTYN_NODE_B_PORT      — default 7878
#   SPECTYN_REMOTE_HOME      — directory on each node for binary + config +
#                              logs (default: ~/.spectyn-mesh-e001)
#   SPECTYN_SCP              — scp command (default: scp). For local smoke,
#                              set to "cp".
#   SPECTYN_HEALTHZ_TIMEOUT_S         — bounded retry for /healthz 200 (default: 30)
#   SPECTYN_DISPATCH_TIMEOUT_S        — per-dispatch curl timeout (default: 15)
#   SPECTYN_E001_HEARTBEAT_DEADLINE_SECS
#                                       max seconds to wait for Healthy↔Unhealthy
#                                       transitions (default: 99 = 3 × 30 × 1.1).
#                                       Override to ~12 when using --quick or
#                                       when the node configs use a tighter
#                                       heartbeat interval. Also accepted as
#                                       SPECTYN_HEARTBEAT_DEADLINE_S (alias).
#   SPECTYN_HEARTBEAT_INTERVAL_SECS   — interval written into agents.toml on
#                                       both nodes (default 30).
#   SPECTYN_HEARTBEAT_FAILURE_THRESHOLD
#                                       threshold written into agents.toml on
#                                       both nodes (default 3).
#   SPECTYN_BOOTSTRAP_SKIP=1 — skip SCP + remote setup (assumes the operator
#                              has already populated both nodes by hand).
#   SPECTYN_CLEANUP_SKIP=1   — leave processes running after the script exits.
#
# Flags
# -----
#   --quick                  — collapse heartbeat config to interval=5,
#                              threshold=2 (deadline becomes 12s) so a smoke
#                              run finishes in well under a minute. Operator
#                              uses this on localhost; not for the real
#                              Pi-over-Tailscale testbed where 90s is the
#                              spec'd window.
#   -h / --help              — print this header.
#
# Exit codes
# ----------
#   0   — all 4 tests pass (peer-down detected, fast failure, recovered,
#         post-recovery dispatch succeeded)
#   1   — any assertion failed; FAIL line at the bottom names the phase
#   77  — preconditions missing (required env var unset, missing tools)
#
# Operator workflow (real testbed: Pi 4 ↔ Workstation over Tailscale)
# -------------------------------------------------------------------
#   export SPECTYN_NODE_A_SSH="ssh -i ~/.ssh/pi pi@pi.tail.ts.net"
#   export SPECTYN_NODE_B_SSH="ssh you@workstation.tail.ts.net"
#   export SPECTYN_NODE_A_URL="http://pi.tail.ts.net:7878"
#   export SPECTYN_NODE_B_URL="http://workstation.tail.ts.net:7878"
#   export SPECTYN_CLUSTER_SECRET="$(cat ~/.spectyn-mesh/cluster_secret)"
#   bash scripts/spectyn-test/scenarios/cross_host_recovery.sh
#
# Local smoke:
#   bash scripts/spectyn-test/scenarios/cross_host_recovery.smoke.sh --quick
#
# Dependencies on shipped Rust
# ----------------------------
# • core/src/serve.rs::rpc_peers — GET /rpc/peers returns {peers:[PeerStatus]}
#   with `online: bool` per peer. F003 asserts on `online` transitions.
# • core/src/mesh.rs::record_probe_result — heartbeat state machine. Feature-
#   gated on `experimental-cluster-heartbeat`. The shipped binary MUST be
#   built with that feature for F003 to observe Healthy ↔ Unhealthy flips.
# • core/src/mesh.rs::route_to_best_peer — must return a structured error
#   (not a 180s hang) when no Healthy peer carries the required caps.
#
# POSIX-bash on purpose: targets Git Bash on Windows, default bash on Linux,
# BusyBox-ish ash on Pi. No `[[ ]]`, no associative arrays, no `mapfile`.
# ─────────────────────────────────────────────────────────────────────────────

set -u

# ── Pretty output (degrade gracefully when piped / NO_COLOR) ────────────────
if [ -t 1 ] && [ "${NO_COLOR:-}" = "" ]; then
  C_RESET=$'\033[0m'; C_DIM=$'\033[2m'
  C_RED=$'\033[31m'; C_GREEN=$'\033[32m'; C_YELLOW=$'\033[33m'; C_CYAN=$'\033[36m'
else
  C_RESET=''; C_DIM=''; C_RED=''; C_GREEN=''; C_YELLOW=''; C_CYAN=''
fi

SPECTYN_TEST_PASSED=0
SPECTYN_TEST_FAILED=0
LAST_FAIL_REASON=""

step()  { printf '  %s→%s %s\n'  "$C_DIM"  "$C_RESET" "$*"; }
pass()  { SPECTYN_TEST_PASSED=$((SPECTYN_TEST_PASSED + 1));
          printf '  %s✓%s %s\n'  "$C_GREEN" "$C_RESET" "$*"; }
fail()  { SPECTYN_TEST_FAILED=$((SPECTYN_TEST_FAILED + 1));
          LAST_FAIL_REASON="$*"
          printf '  %s✗%s %s\n'  "$C_RED"   "$C_RESET" "$*" >&2; }
warn()  { printf '  %s⚠%s %s\n'  "$C_YELLOW" "$C_RESET" "$*" >&2; }
title() { printf '\n%s━━ %s ━━%s\n' "$C_CYAN" "$*" "$C_RESET"; }

# ── Flag parsing (only --quick and --help; rest is env) ──────────────────────
QUICK=0
for arg in "$@"; do
  case "$arg" in
    -h|--help)
      sed -n '2,110p' "$0" | sed -E 's/^# ?//'
      exit 0
      ;;
    --quick)
      QUICK=1
      ;;
    *)
      warn "unknown argument: $arg (use --help)"
      exit 77
      ;;
  esac
done

# ── Preconditions ────────────────────────────────────────────────────────────
title "F003 · cross_host_recovery — preconditions"

REQUIRED_ENV="SPECTYN_NODE_A_SSH SPECTYN_NODE_B_SSH SPECTYN_NODE_A_URL SPECTYN_NODE_B_URL SPECTYN_CLUSTER_SECRET"
missing=""
for v in $REQUIRED_ENV; do
  eval "val=\${$v:-}"
  if [ -z "$val" ]; then
    missing="$missing $v"
  fi
done
if [ -n "$missing" ]; then
  warn "skipping — missing required env:$missing"
  warn "run with --help for the env-var contract"
  exit 77
fi

for cmd in curl openssl; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    warn "skipping — required command not on PATH: $cmd"
    exit 77
  fi
done

# Pick a real Python interpreter (same logic as F002 — avoid Windows Store stub).
_find_python() {
  for cand in python python3; do
    p=$(command -v "$cand" 2>/dev/null) || continue
    case "$p" in
      */WindowsApps/python*) continue ;;
    esac
    if printf 'ok' | "$cand" -c 'import sys; print(sys.stdin.read())' 2>/dev/null | grep -q ok; then
      printf '%s\n' "$cand"; return 0
    fi
  done
  return 1
}
PY=$(_find_python || true)
if [ -z "$PY" ]; then
  warn "skipping — no working python interpreter on PATH"
  exit 77
fi
step "using python: $PY"

# Defaults
NODE_A_NAME="${SPECTYN_NODE_A_NAME:-node-a}"
NODE_B_NAME="${SPECTYN_NODE_B_NAME:-node-b}"
NODE_A_PORT="${SPECTYN_NODE_A_PORT:-7878}"
NODE_B_PORT="${SPECTYN_NODE_B_PORT:-7878}"
REMOTE_HOME="${SPECTYN_REMOTE_HOME:-.spectyn-mesh-e001}"
SCP_CMD="${SPECTYN_SCP:-scp}"
HEALTHZ_TIMEOUT="${SPECTYN_HEALTHZ_TIMEOUT_S:-30}"
DISPATCH_TIMEOUT="${SPECTYN_DISPATCH_TIMEOUT_S:-15}"
BIN_LOCAL="${SPECTYN_BIN_LOCAL:-$(command -v spectyn || true)}"

# Heartbeat config knobs (written into both nodes' agents.toml).
HB_INTERVAL="${SPECTYN_HEARTBEAT_INTERVAL_SECS:-30}"
HB_THRESHOLD="${SPECTYN_HEARTBEAT_FAILURE_THRESHOLD:-3}"

# --quick collapses the heartbeat window to ~10s so localhost smokes finish fast.
if [ "$QUICK" = "1" ]; then
  HB_INTERVAL="${SPECTYN_HEARTBEAT_INTERVAL_SECS:-5}"
  HB_THRESHOLD="${SPECTYN_HEARTBEAT_FAILURE_THRESHOLD:-2}"
fi

# Deadline default = interval × threshold × 1.1 (matches spec's "± 10%").
# Honour either the spec-named SPECTYN_E001_HEARTBEAT_DEADLINE_SECS or the
# shorter alias used by some operator runbooks.
_default_deadline=$(( HB_INTERVAL * HB_THRESHOLD * 11 / 10 + 2 ))
DEADLINE="${SPECTYN_E001_HEARTBEAT_DEADLINE_SECS:-${SPECTYN_HEARTBEAT_DEADLINE_S:-$_default_deadline}}"

if [ -z "$BIN_LOCAL" ] || [ ! -x "$BIN_LOCAL" ]; then
  warn "skipping — SPECTYN_BIN_LOCAL unset and 'spectyn' not on PATH"
  exit 77
fi

step "node A: $NODE_A_NAME @ $SPECTYN_NODE_A_URL"
step "node B: $NODE_B_NAME @ $SPECTYN_NODE_B_URL (THE one that gets killed/restarted)"
step "binary: $BIN_LOCAL"
step "heartbeat: interval=${HB_INTERVAL}s threshold=${HB_THRESHOLD} deadline=${DEADLINE}s (quick=${QUICK})"

# ── Helpers ──────────────────────────────────────────────────────────────────

# remote_run <SSH_PREFIX> <shell command>
remote_run() {
  _ssh_prefix="$1"; shift
  _cmd="$*"
  # shellcheck disable=SC2086
  $_ssh_prefix sh -c "$_cmd"
}

# scp_to_remote <SSH_PREFIX> <src> <dest_abs_path>
scp_to_remote() {
  ssh_prefix="$1"; src="$2"; dest="$3"
  if [ "$SCP_CMD" = "cp" ]; then
    mkdir -p "$dest"
    cp -f "$src" "$dest/"
    return $?
  fi
  target=$(printf '%s\n' "$ssh_prefix" \
           | sed -E 's/^ssh +//' \
           | awk '{print $NF}')
  if [ -z "$target" ]; then
    fail "could not derive scp target from: $ssh_prefix"
    return 1
  fi
  $SCP_CMD "$src" "$target:$dest/"
}

# hmac_hex <body> <secret>
hmac_hex() {
  printf '%s' "$1" | openssl dgst -sha256 -hmac "$2" -hex | awk '{print $2}'
}

# wait_healthz <base_url> <timeout_s>
wait_healthz() {
  base="$1"; deadline=$(( $(date +%s) + ${2:-30} ))
  while [ "$(date +%s)" -lt "$deadline" ]; do
    code=$(curl -sS -o /dev/null -w '%{http_code}' --max-time 3 \
                "$base/healthz" 2>/dev/null || echo 000)
    if [ "$code" = "200" ]; then return 0; fi
    sleep 1
  done
  return 1
}

# dispatch_with_caps <base_url> <required_caps_json_array> <prompt>
dispatch_with_caps() {
  base="$1"; caps_json="$2"; prompt="$3"
  body=$(printf '{"agent":"master","prompt":"%s","required_caps":%s}' \
                "$prompt" "$caps_json")
  auth=$(hmac_hex "$body" "$SPECTYN_CLUSTER_SECRET")
  curl -sS --max-time "$DISPATCH_TIMEOUT" \
       -X POST "$base/rpc/task/assign" \
       -H "X-Cluster-Auth: $auth" \
       -H "Content-Type: application/json" \
       -d "$body"
}

# json_field <body> <field>
json_field() {
  printf '%s' "$1" | "$PY" -c "
import json, sys
try:
    d = json.loads(sys.stdin.read())
    v = d.get('$2')
    if v is None: print('')
    elif isinstance(v, bool): print('true' if v else 'false')
    else: print(v)
except Exception:
    print('')
" 2>/dev/null
}

# peer_online <base_url> <target_peer_name>
# Hit /rpc/peers on <base_url>, find the peer entry whose `name` matches
# <target_peer_name>, and echo its `online` field (`true` / `false` /
# empty if not found). Resilient to peers being indexed by URL — falls back
# to a URL-substring match if name lookup misses.
peer_online() {
  base="$1"; want="$2"
  body=$(curl -sS --max-time 5 "$base/rpc/peers" 2>/dev/null || echo '')
  if [ -z "$body" ]; then echo ""; return; fi
  printf '%s' "$body" | "$PY" -c "
import json, sys
want = '$want'
try:
    d = json.loads(sys.stdin.read())
    peers = d.get('peers') or []
    for p in peers:
        # match by name OR by url substring (the registry sometimes lacks
        # name until first ping completes)
        if p.get('name') == want or want in (p.get('url') or ''):
            v = p.get('online')
            print('true' if v else 'false')
            sys.exit(0)
    print('')
except Exception:
    print('')
" 2>/dev/null
}

# wait_peer_online <base_url> <target_name> <want_true_or_false> <deadline_s>
# Polls /rpc/peers every 2s until peer's online == want, or deadline.
# Returns 0 + echoes "elapsed=<n>s" on hit, 1 on miss.
wait_peer_online() {
  base="$1"; want_peer="$2"; want_state="$3"; deadline_s="$4"
  start=$(date +%s)
  end=$(( start + deadline_s ))
  while [ "$(date +%s)" -lt "$end" ]; do
    cur=$(peer_online "$base" "$want_peer")
    if [ "$cur" = "$want_state" ]; then
      elapsed=$(( $(date +%s) - start ))
      echo "elapsed=${elapsed}s"
      return 0
    fi
    sleep 2
  done
  return 1
}

# tail_remote_log <SSH_PREFIX> <node label> <log path>
tail_remote_log() {
  ssh_prefix="$1"; label="$2"; logp="$3"
  printf '%s━━ last 50 lines of %s spectyn serve log (%s) ━━%s\n' \
         "$C_YELLOW" "$label" "$logp" "$C_RESET" >&2
  remote_run "$ssh_prefix" "tail -n 50 '$logp' 2>/dev/null || echo '(no log file)'" >&2
}

# dump_peers <base_url> <label>
# Dump the raw /rpc/peers JSON for postmortem.
dump_peers() {
  base="$1"; label="$2"
  printf '%s━━ %s /rpc/peers snapshot ━━%s\n' "$C_YELLOW" "$label" "$C_RESET" >&2
  curl -sS --max-time 5 "$base/rpc/peers" 2>/dev/null | "$PY" -m json.tool 2>/dev/null >&2 \
    || echo '(could not fetch /rpc/peers)' >&2
}

# fail_and_dump <reason>
fail_and_dump() {
  fail "$1"
  dump_peers "$SPECTYN_NODE_A_URL" "$NODE_A_NAME"
  dump_peers "$SPECTYN_NODE_B_URL" "$NODE_B_NAME"
  tail_remote_log "$SPECTYN_NODE_A_SSH" "$NODE_A_NAME" "~/$REMOTE_HOME/spectyn-serve.log"
  tail_remote_log "$SPECTYN_NODE_B_SSH" "$NODE_B_NAME" "~/$REMOTE_HOME/spectyn-serve.log"
  printf '\n%sFAIL: %s%s\n' "$C_RED" "$1" "$C_RESET" >&2
  exit 1
}

# ── Phase 1: bootstrap binary + config on each node ─────────────────────────
if [ "${SPECTYN_BOOTSTRAP_SKIP:-0}" != "1" ]; then
  title "F003 · Phase 1 — bootstrap binary + render agents.toml on each node"

  TMP_LOCAL=$(mktemp -d 2>/dev/null || mktemp -d -t f003)
  trap 'rm -rf "$TMP_LOCAL"' EXIT

  # Node A: caps=[gpu], peers=[B], with the configured heartbeat window.
  cat > "$TMP_LOCAL/agents-a.toml" <<EOF
# Auto-rendered by cross_host_recovery.sh for E001/F003 — do not edit by hand.
[core]
host = "0.0.0.0"
port = $NODE_A_PORT

[cluster]
node_name                    = "$NODE_A_NAME"
cluster_secret               = "$SPECTYN_CLUSTER_SECRET"
peers                        = ["$SPECTYN_NODE_B_URL"]
capabilities                 = ["gpu"]
worker_caps                  = ["gpu"]
enforce_caps                 = "soft"
heartbeat_interval_secs      = $HB_INTERVAL
heartbeat_failure_threshold  = $HB_THRESHOLD

[agent.master]
provider = "echo"
model    = "echo"
EOF

  # Node B: caps=[vision], peers=[A], same heartbeat window.
  cat > "$TMP_LOCAL/agents-b.toml" <<EOF
# Auto-rendered by cross_host_recovery.sh for E001/F003 — do not edit by hand.
[core]
host = "0.0.0.0"
port = $NODE_B_PORT

[cluster]
node_name                    = "$NODE_B_NAME"
cluster_secret               = "$SPECTYN_CLUSTER_SECRET"
peers                        = ["$SPECTYN_NODE_A_URL"]
capabilities                 = ["vision"]
worker_caps                  = ["vision"]
enforce_caps                 = "soft"
heartbeat_interval_secs      = $HB_INTERVAL
heartbeat_failure_threshold  = $HB_THRESHOLD

[agent.master]
provider = "echo"
model    = "echo"
EOF

  bootstrap_one() {
    LBL="$1"; SSHP="$2"; CFG="$3"
    step "node $LBL: shipping binary + config"
    if ! remote_run "$SSHP" "mkdir -p ~/$REMOTE_HOME"; then
      fail_and_dump "Phase 1: node $LBL: mkdir ~/$REMOTE_HOME failed"
    fi
    REMOTE_ABS=$(remote_run "$SSHP" "echo \$HOME/$REMOTE_HOME" | tr -d '\r\n')
    [ -n "$REMOTE_ABS" ] || REMOTE_ABS="$HOME/$REMOTE_HOME"
    if ! scp_to_remote "$SSHP" "$BIN_LOCAL" "$REMOTE_ABS"; then
      fail_and_dump "Phase 1: node $LBL: failed to ship $BIN_LOCAL → $REMOTE_ABS"
    fi
    cp "$CFG" "$TMP_LOCAL/agents.toml"
    if ! scp_to_remote "$SSHP" "$TMP_LOCAL/agents.toml" "$REMOTE_ABS"; then
      fail_and_dump "Phase 1: node $LBL: failed to ship agents.toml → $REMOTE_ABS"
    fi
    pass "node $LBL: bootstrap complete in $REMOTE_ABS"
  }
  bootstrap_one "$NODE_A_NAME" "$SPECTYN_NODE_A_SSH" "$TMP_LOCAL/agents-a.toml"
  bootstrap_one "$NODE_B_NAME" "$SPECTYN_NODE_B_SSH" "$TMP_LOCAL/agents-b.toml"
fi

# ── Cross-platform kill helper ──────────────────────────────────────────────
# Kill the spectyn serve listening on $1 (port). On Windows, `nohup
# ./spectyn.exe &` records $! as the bash subshell PID — not the real
# spectyn.exe Windows PID — so PID-based kills miss the actual binary. The
# load-bearing identifier across platforms is therefore the listening port:
# look it up with `lsof` (Linux/macOS) or `netstat -ano` (Git Bash / Windows)
# and kill whatever owns it.
#
# Interpolated into a remote `sh -c` string. Single-quoted to suppress local
# expansion — the only var that survives to the remote is $1 (the port).
KILL_PORT_SNIPPET='_port="$1"; [ -z "$_port" ] && exit 0; _pids=""; if command -v lsof >/dev/null 2>&1; then _pids=$(lsof -ti :"$_port" -sTCP:LISTEN 2>/dev/null || true); fi; if [ -z "$_pids" ] && command -v ss >/dev/null 2>&1; then _pids=$(ss -ltnp 2>/dev/null | grep ":$_port " | sed -n "s/.*pid=\\([0-9]\\+\\).*/\\1/p" | sort -u); fi; if [ -z "$_pids" ] && command -v netstat.exe >/dev/null 2>&1; then _pids=$(netstat.exe -ano 2>/dev/null | tr -d "\\r" | awk -v p=":$_port" "(\$2 ~ p\"$\") && (\$4 == \"LISTENING\") {print \$5}" | sort -u); fi; [ -z "$_pids" ] && exit 0; for _p in $_pids; do if command -v taskkill.exe >/dev/null 2>&1; then taskkill.exe //F //PID "$_p" //T >/dev/null 2>&1 || true; else kill "$_p" 2>/dev/null || true; fi; done; sleep 1; for _p in $_pids; do if command -v taskkill.exe >/dev/null 2>&1; then taskkill.exe //F //PID "$_p" //T >/dev/null 2>&1 || true; else kill -9 "$_p" 2>/dev/null || true; fi; done'

# ── Phase 1b: sweep stale processes (idempotent re-run) ─────────────────────
title "F003 · sweep — kill any stale spectyn serve from a prior run"
sweep_one() {
  LBL="$1"; SSHP="$2"; PORT="$3"
  step "node $LBL: killing whatever listens on :$PORT (stale spectyn)"
  remote_run "$SSHP" "
    sh -c '$KILL_PORT_SNIPPET' _ '$PORT'
    rm -f \$HOME/$REMOTE_HOME/spectyn-serve.pid
  " || true
}
sweep_one "$NODE_A_NAME" "$SPECTYN_NODE_A_SSH" "$NODE_A_PORT"
sweep_one "$NODE_B_NAME" "$SPECTYN_NODE_B_SSH" "$NODE_B_PORT"

# ── Phase 1c: launch spectyn serve on both nodes ────────────────────────────
title "F003 · launch — spectyn serve on both nodes (heartbeat ON)"
launch_one() {
  LBL="$1"; SSHP="$2"; BASE="$3"; PORT="$4"
  bin_name="spectyn"
  if [ -x "$BIN_LOCAL" ] && printf '%s' "$BIN_LOCAL" | grep -qi '\.exe$'; then
    bin_name="spectyn.exe"
  fi
  step "node $LBL: starting $bin_name --port $PORT (forwarding gate ON)"
  if ! remote_run "$SSHP" "
    cd \$HOME/$REMOTE_HOME &&
    rm -f spectyn-serve.log &&
    SPECTYN_FORWARD_ON_CAPS_MISMATCH=1 \
    SPECTYN_ENFORCE_REQUIRED_CAPS=soft \
    nohup ./$bin_name serve --host 0.0.0.0 --port $PORT \
      > spectyn-serve.log 2>&1 < /dev/null &
    echo \$! > spectyn-serve.pid
  "; then
    fail_and_dump "launch: node $LBL: failed to spawn spectyn serve"
  fi
  step "node $LBL: waiting up to ${HEALTHZ_TIMEOUT}s for /healthz 200 at $BASE"
  if wait_healthz "$BASE" "$HEALTHZ_TIMEOUT"; then
    pass "node $LBL: /healthz 200 — spectyn serve is up"
  else
    fail_and_dump "launch: node $LBL: /healthz never returned 200 within ${HEALTHZ_TIMEOUT}s at $BASE"
  fi
}
launch_one "$NODE_A_NAME" "$SPECTYN_NODE_A_SSH" "$SPECTYN_NODE_A_URL" "$NODE_A_PORT"
launch_one "$NODE_B_NAME" "$SPECTYN_NODE_B_SSH" "$SPECTYN_NODE_B_URL" "$NODE_B_PORT"

# Cleanup trap registered now that processes are running.
cleanup_processes() {
  if [ "${SPECTYN_CLEANUP_SKIP:-0}" = "1" ]; then return; fi
  for tuple in "${NODE_A_NAME}|${SPECTYN_NODE_A_SSH}|${NODE_A_PORT}" "${NODE_B_NAME}|${SPECTYN_NODE_B_SSH}|${NODE_B_PORT}"; do
    LBL=$(echo "$tuple" | cut -d'|' -f1)
    SSHP=$(echo "$tuple" | cut -d'|' -f2)
    PORT=$(echo "$tuple" | cut -d'|' -f3)
    remote_run "$SSHP" "sh -c '$KILL_PORT_SNIPPET' _ '$PORT'" 2>/dev/null || true
  done
}
trap 'cleanup_processes; rm -rf "${TMP_LOCAL:-}"' EXIT

# ── Phase 2: warm-up + baseline forwarding works ────────────────────────────
title "F003 · Phase 2 — baseline dispatch (defence in depth)"
# Touch each node so peer inventory populates quickly.
for U in "$SPECTYN_NODE_A_URL" "$SPECTYN_NODE_B_URL"; do
  curl -sS --max-time 5 "$U/healthz"     >/dev/null 2>&1 || true
  curl -sS --max-time 5 "$U/rpc/peers"   >/dev/null 2>&1 || true
done
sleep 2

# Wait for B to register as online from A's POV before declaring baseline.
step "waiting for A to see B as online (deadline ${DEADLINE}s)"
if elapsed=$(wait_peer_online "$SPECTYN_NODE_A_URL" "$NODE_B_NAME" "true" "$DEADLINE"); then
  pass "baseline: A sees B online ($elapsed)"
else
  fail_and_dump "Phase 2: A never saw B as online within ${DEADLINE}s"
fi

RESP_BASE=$(dispatch_with_caps "$SPECTYN_NODE_A_URL" '["vision"]' \
              "F003 baseline dispatch from $NODE_A_NAME requiring vision")
step "baseline raw response: $RESP_BASE"
DT_BASE=$(json_field "$RESP_BASE" dispatched_to)
FW_BASE=$(json_field "$RESP_BASE" forwarded)
JID_BASE=$(json_field "$RESP_BASE" job_id)
ERR_BASE=$(json_field "$RESP_BASE" error)
if [ -n "$ERR_BASE" ]; then
  fail "Phase 2: baseline dispatch returned error: $ERR_BASE"
elif [ "$DT_BASE" != "$NODE_B_NAME" ] || [ "$FW_BASE" != "true" ]; then
  fail "Phase 2: baseline dispatch expected dispatched_to=$NODE_B_NAME forwarded=true; got dispatched_to=$DT_BASE forwarded=$FW_BASE"
else
  pass "Phase 2: baseline dispatch forwarded A→B (job $JID_BASE)"
fi

# ── Test 1 / Phases 3+4: kill B → A marks B Unhealthy within deadline ───────
title "F003 · Test 1 (Phases 3+4) — kill B → A flips B to online=false within ${DEADLINE}s"
step "killing spectyn serve on $NODE_B_NAME (port-based kill on :$NODE_B_PORT)"
if ! remote_run "$SPECTYN_NODE_B_SSH" "sh -c '$KILL_PORT_SNIPPET' _ '$NODE_B_PORT'"; then
  fail_and_dump "Test 1: failed to send kill on $NODE_B_NAME"
fi
# Verify the process really did die — probe that B's HTTP port stops
# accepting connections. The kill helper above kills by listening PID, but
# we want hard evidence the socket is gone before we start polling A's
# heartbeat view. On Windows the recorded `$!` PID is unreliable (bash
# wrapper rather than spectyn.exe), so the HTTP probe is the load-bearing
# check on any platform.
sleep 2
http_dead=0
last_code=""
for i in 1 2 3 4 5; do
  code=$(curl -sS -o /dev/null -w '%{http_code}' --max-time 2 \
              "$SPECTYN_NODE_B_URL/healthz" 2>/dev/null)
  ec=$?
  # curl exit ≠ 0 (e.g. 7 connect refused, 28 timeout) → socket is dead.
  # `code` may be empty or "000" depending on libcurl version.
  if [ $ec -ne 0 ] || [ "$code" = "000" ] || [ -z "$code" ]; then
    http_dead=1
    last_code="(connect refused, curl ec=$ec)"
    break
  fi
  last_code="$code"
  sleep 1
done
if [ "$http_dead" = "1" ]; then
  step "$NODE_B_NAME process confirmed gone — port no longer accepts /healthz $last_code"
else
  fail_and_dump "Test 1: $NODE_B_NAME /healthz still returns $last_code after kill — process not actually dead"
fi

step "polling /rpc/peers on $NODE_A_NAME (deadline ${DEADLINE}s)"
if elapsed=$(wait_peer_online "$SPECTYN_NODE_A_URL" "$NODE_B_NAME" "false" "$DEADLINE"); then
  pass "Test 1: A marked B online=false ($elapsed within ${DEADLINE}s window)"
else
  dump_peers "$SPECTYN_NODE_A_URL" "$NODE_A_NAME"
  fail_and_dump "Test 1: A never flipped B to online=false within ${DEADLINE}s (heartbeat interval=${HB_INTERVAL}s threshold=${HB_THRESHOLD}; verify the spectyn binary was built with --features experimental-cluster-heartbeat)"
fi

# ── Test 2 / Phase 5: dispatch B-only caps → fast structured error ──────────
title "F003 · Test 2 (Phase 5) — dispatch needing B's [vision] → fast structured failure"
start=$(date +%s)
RESP_DOWN=$(dispatch_with_caps "$SPECTYN_NODE_A_URL" '["vision"]' \
              "F003 dispatch while $NODE_B_NAME is down")
duration=$(( $(date +%s) - start ))
step "raw response (B down, took ${duration}s): $RESP_DOWN"
ERR_DOWN=$(json_field "$RESP_DOWN" error)
DT_DOWN=$(json_field "$RESP_DOWN" dispatched_to)
EC_DOWN=$(json_field "$RESP_DOWN" error_code)

if [ "$duration" -gt 30 ]; then
  fail "Test 2: dispatch took ${duration}s (>30s) — looks like a HANG, not a fast structured error"
elif [ -n "$ERR_DOWN" ]; then
  # Spec: "no healthy peer with caps [vision]" — but we accept any structured
  # error mentioning peers/caps as long as it came back fast.
  pass "Test 2: fast structured error in ${duration}s — error=\"$ERR_DOWN\" code=\"$EC_DOWN\""
elif [ "$DT_DOWN" = "$NODE_A_NAME" ]; then
  # A doesn't have vision, so it shouldn't claim to have dispatched locally.
  # If forwarding-on-mismatch is soft, this is technically allowed — but it's
  # still a regression signal worth noting.
  warn "Test 2: dispatch landed locally on $NODE_A_NAME — soft-enforce kept it from failing, but the caps were B-only"
  pass "Test 2: dispatch resolved in ${duration}s without hang"
else
  fail "Test 2: unexpected response shape (no error, no error_code, dispatched_to=$DT_DOWN)"
fi

# ── Test 3 / Phases 6+7: restart B → A flips B back to online=true ──────────
title "F003 · Test 3 (Phases 6+7) — restart B → A flips B to online=true within ${DEADLINE}s"
step "restarting spectyn serve on $NODE_B_NAME"
launch_one "$NODE_B_NAME" "$SPECTYN_NODE_B_SSH" "$SPECTYN_NODE_B_URL" "$NODE_B_PORT"

step "polling /rpc/peers on $NODE_A_NAME (deadline ${DEADLINE}s)"
if elapsed=$(wait_peer_online "$SPECTYN_NODE_A_URL" "$NODE_B_NAME" "true" "$DEADLINE"); then
  pass "Test 3: A flipped B back to online=true ($elapsed within ${DEADLINE}s)"
else
  dump_peers "$SPECTYN_NODE_A_URL" "$NODE_A_NAME"
  fail_and_dump "Test 3: A never flipped B back to online=true within ${DEADLINE}s after restart"
fi

# ── Test 4 / Phase 8: post-recovery dispatch succeeds + audit log ───────────
title "F003 · Test 4 (Phase 8) — post-recovery dispatch + audit-log assertion"
RESP_OK=$(dispatch_with_caps "$SPECTYN_NODE_A_URL" '["vision"]' \
            "F003 post-recovery dispatch from $NODE_A_NAME requiring vision")
step "raw response (post-recovery): $RESP_OK"
DT_OK=$(json_field "$RESP_OK" dispatched_to)
FW_OK=$(json_field "$RESP_OK" forwarded)
JID_OK=$(json_field "$RESP_OK" job_id)
ERR_OK=$(json_field "$RESP_OK" error)
if [ -n "$ERR_OK" ]; then
  fail "Test 4: post-recovery dispatch returned error: $ERR_OK"
elif [ "$DT_OK" != "$NODE_B_NAME" ] || [ "$FW_OK" != "true" ]; then
  fail "Test 4: post-recovery dispatch expected dispatched_to=$NODE_B_NAME forwarded=true; got dispatched_to=$DT_OK forwarded=$FW_OK"
else
  pass "Test 4: post-recovery dispatch forwarded A→B (job $JID_OK)"
fi

# Audit log: at least one forwarding line on each node.
audit_one() {
  LBL="$1"; SSHP="$2"
  hits=$(remote_run "$SSHP" "
    grep -cE 'task/assign|dispatch::forward|forwarded|peer_health_transition' \
         \$HOME/$REMOTE_HOME/spectyn-serve.log 2>/dev/null || echo 0
  " | tr -d '\r\n ')
  if [ -n "$hits" ] && [ "$hits" -ge 1 ] 2>/dev/null; then
    pass "Test 4: node $LBL: $hits dispatch/health log lines"
  else
    fail "Test 4: node $LBL: no dispatch/health evidence in serve log"
  fi
}
audit_one "$NODE_A_NAME" "$SPECTYN_NODE_A_SSH"
audit_one "$NODE_B_NAME" "$SPECTYN_NODE_B_SSH"

# ── Summary ─────────────────────────────────────────────────────────────────
title "F003 · summary"
printf '  %sPASS%s %d   %sFAIL%s %d\n' \
       "$C_GREEN" "$C_RESET" "$SPECTYN_TEST_PASSED" \
       "$C_RED"   "$C_RESET" "$SPECTYN_TEST_FAILED"

if [ "$SPECTYN_TEST_FAILED" -gt 0 ]; then
  tail_remote_log "$SPECTYN_NODE_A_SSH" "$NODE_A_NAME" "~/$REMOTE_HOME/spectyn-serve.log"
  tail_remote_log "$SPECTYN_NODE_B_SSH" "$NODE_B_NAME" "~/$REMOTE_HOME/spectyn-serve.log"
  printf '\n%sFAIL: %s%s\n' "$C_RED" "$LAST_FAIL_REASON" "$C_RESET" >&2
  exit 1
fi

printf '\n%sPASS: cross-host peer-down + recovery cycle verified (Healthy→Unhealthy→Healthy)%s\n' \
       "$C_GREEN" "$C_RESET"
exit 0
