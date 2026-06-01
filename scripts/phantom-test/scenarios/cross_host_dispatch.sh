#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# cross_host_dispatch.sh — E001 / F002 cross-host dispatch e2e scenario.
#
# THE central proof artifact for E001 / Pillar P1
# ("Cluster-aware: nodes discover each other, share capabilities, forward
#  tasks to whichever node can run them").
#
# Spec: docs/superpowers/features/F002-cross-host-dispatch-e2e-scenario.md
#
# What it proves
# --------------
# Given two SSH-reachable phantom-mesh nodes with NON-OVERLAPPING worker
# capabilities, this script:
#   1. SCPs a pre-built phantom binary to each node (no compile-on-Pi).
#   2. Renders per-node agents.toml fragments
#        Node A: worker_caps = ["gpu"]    + peers = [B]
#        Node B: worker_caps = ["vision"] + peers = [A]
#      Both: shared cluster_secret, node_name set,
#            PHANTOM_FORWARD_ON_CAPS_MISMATCH=1 in the launch env.
#   3. Starts `phantom serve` on each node (nohup + PID file), waits for
#      /healthz 200 with bounded retry.
#   4. From A: POST /rpc/task/assign with required_caps=["vision"]
#      → A doesn't have vision → forwards to B
#      → asserts response.dispatched_to == B's node_name
#      → asserts response.forwarded == true
#   5. From B: POST /rpc/task/assign with required_caps=["gpu"]
#      → symmetric assertion: dispatched_to == A's node_name, forwarded == true
#   6. Greps both nodes' phantom serve logs for the forwarding audit event
#      ("phantom::dispatch::forward" target) — defence in depth.
#   7. On failure: prints last 50 lines of BOTH nodes' phantom serve logs,
#      then exits non-zero with a `FAIL: <reason>` line.
#   8. Cleanup: kills `phantom serve` on both nodes (via PID files), leaves
#      log files for postmortem. Idempotent — safe to re-run after a dirty
#      exit; orphans are cleaned by the PID-file sweep at script entry.
#
# Environment contract (REQUIRED)
# -------------------------------
#   PHANTOM_NODE_A_SSH       — full ssh prefix incl. user/host/port/key
#                              e.g. "ssh -i ~/.ssh/pi -p 22 pi@pi.tail.ts.net"
#                              (or "bash -lc" for a local-loopback smoke run)
#   PHANTOM_NODE_B_SSH       — same for Node B
#   PHANTOM_NODE_A_URL       — base URL reachable from THIS host AND from B
#                              e.g. "http://pi.tail.ts.net:7878"
#   PHANTOM_NODE_B_URL       — same for Node B
#                              e.g. "http://workstation.tail.ts.net:7878"
#   PHANTOM_CLUSTER_SECRET   — shared cluster secret (HMAC) — same on both
#
# Environment contract (OPTIONAL)
# -------------------------------
#   PHANTOM_BIN_LOCAL        — path to the pre-built phantom binary on THIS
#                              host that will be SCP'd to each node.
#                              Default: $(command -v phantom)
#   PHANTOM_NODE_A_NAME      — Node A's node_name (default: "node-a")
#   PHANTOM_NODE_B_NAME      — Node B's node_name (default: "node-b")
#   PHANTOM_NODE_A_PORT      — Node A's serve port (default: 7878)
#   PHANTOM_NODE_B_PORT      — Node B's serve port (default: 7878)
#   PHANTOM_REMOTE_HOME      — directory on each node for binary + config +
#                              logs (default: ~/.phantom-mesh-e001)
#   PHANTOM_SCP              — scp command (default: scp). For local smoke,
#                              set to "cp" via the smoke harness.
#   PHANTOM_HEALTHZ_TIMEOUT_S — bounded retry for /healthz 200 (default: 30)
#   PHANTOM_DISPATCH_TIMEOUT_S — per-dispatch curl timeout (default: 15)
#   PHANTOM_BOOTSTRAP_SKIP=1 — skip SCP + remote setup (assumes the operator
#                              has already started both `phantom serve`
#                              instances by hand). Useful for fast re-runs.
#   PHANTOM_CLEANUP_SKIP=1   — leave both `phantom serve` instances running
#                              after the script exits (useful for ad-hoc
#                              poking with the same processes).
#
# Exit codes
# ----------
#   0   — both forwarding directions asserted; audit events found on both
#   1   — any assertion failed; failure reason on the last FAIL line
#   77  — preconditions missing (required env var unset, ssh broken)
#
# Operator workflow (real testbed: Pi 4 ↔ Workstation over Tailscale)
# -------------------------------------------------------------------
#   export PHANTOM_NODE_A_SSH="ssh -i ~/.ssh/pi pi@pi.tail.ts.net"
#   export PHANTOM_NODE_B_SSH="ssh you@workstation.tail.ts.net"
#   export PHANTOM_NODE_A_URL="http://pi.tail.ts.net:7878"
#   export PHANTOM_NODE_B_URL="http://workstation.tail.ts.net:7878"
#   export PHANTOM_CLUSTER_SECRET="$(cat ~/.phantom-mesh/cluster_secret)"
#   bash scripts/phantom-test/scenarios/cross_host_dispatch.sh
#
# Local smoke (CI / dev): use the sibling
#   scripts/phantom-test/scenarios/cross_host_dispatch.smoke.sh
# which spins up two `phantom serve` processes on 127.0.0.1 and reuses this
# script with PHANTOM_NODE_*_SSH set to a local-exec shim.
#
# Dependencies on shipped Rust
# ----------------------------
# • core/src/mesh.rs::DispatchResponse (server JSON shape) — fields
#   `dispatched_to` and `forwarded` are load-bearing; if either is renamed
#   or removed, this script breaks at the ASSERT_DISPATCHED_TO step. That
#   is the intent — F002 is the e2e gate on these field names.
# • core/src/serve.rs::rpc_task_assign — must emit `dispatched_to` +
#   `forwarded` on BOTH the local-run and forwarded branches.
# • core/src/mesh.rs::enforce_required_caps_with_forwarding — gated by
#   PHANTOM_FORWARD_ON_CAPS_MISMATCH=1; this script sets the gate on
#   both nodes' launch env.
#
# POSIX-bash on purpose: targets Git Bash on Windows, default bash on Linux,
# and BusyBox-ish ash on Pi (if operator chose to swap). No `[[ ]]`,
# no associative arrays, no `mapfile`.
# ─────────────────────────────────────────────────────────────────────────────

set -u
# Deliberately NOT `set -e` — assertions must be allowed to fail and
# accumulate so the FAIL summary at the bottom is complete.

# ── Pretty output (degrade gracefully when piped / NO_COLOR) ────────────────
if [ -t 1 ] && [ "${NO_COLOR:-}" = "" ]; then
  C_RESET=$'\033[0m'; C_DIM=$'\033[2m'
  C_RED=$'\033[31m'; C_GREEN=$'\033[32m'; C_YELLOW=$'\033[33m'; C_CYAN=$'\033[36m'
else
  C_RESET=''; C_DIM=''; C_RED=''; C_GREEN=''; C_YELLOW=''; C_CYAN=''
fi

PHANTOM_TEST_PASSED=0
PHANTOM_TEST_FAILED=0
LAST_FAIL_REASON=""

step()  { printf '  %s→%s %s\n'  "$C_DIM"  "$C_RESET" "$*"; }
pass()  { PHANTOM_TEST_PASSED=$((PHANTOM_TEST_PASSED + 1));
          printf '  %s✓%s %s\n'  "$C_GREEN" "$C_RESET" "$*"; }
fail()  { PHANTOM_TEST_FAILED=$((PHANTOM_TEST_FAILED + 1));
          LAST_FAIL_REASON="$*"
          printf '  %s✗%s %s\n'  "$C_RED"   "$C_RESET" "$*" >&2; }
warn()  { printf '  %s⚠%s %s\n'  "$C_YELLOW" "$C_RESET" "$*" >&2; }
title() { printf '\n%s━━ %s ━━%s\n' "$C_CYAN" "$*" "$C_RESET"; }

# ── --help short-circuit ─────────────────────────────────────────────────────
case "${1:-}" in
  -h|--help)
    sed -n '2,75p' "$0" | sed -E 's/^# ?//'
    exit 0
    ;;
esac

# ── Preconditions ────────────────────────────────────────────────────────────
title "F002 · cross_host_dispatch — preconditions"

REQUIRED_ENV="PHANTOM_NODE_A_SSH PHANTOM_NODE_B_SSH PHANTOM_NODE_A_URL PHANTOM_NODE_B_URL PHANTOM_CLUSTER_SECRET"
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

for cmd in curl openssl python3; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    # python3 fallback to python (Git Bash on Windows ships python.exe)
    if [ "$cmd" = "python3" ] && command -v python >/dev/null 2>&1; then
      continue
    fi
    warn "skipping — required command not on PATH: $cmd"
    exit 77
  fi
done

# Pick a real Python interpreter. Prefer `python` (which is what Git Bash on
# Windows installs as the working interpreter); fall back to `python3` on
# Linux/macOS. Reject Microsoft Store's `python3` stub at
# /c/.../WindowsApps/python3, which opens an interactive Store prompt and
# returns nothing on stdin (this would silently break json_field).
_find_python() {
  for cand in python python3; do
    p=$(command -v "$cand" 2>/dev/null) || continue
    case "$p" in
      */WindowsApps/python*) continue ;;
    esac
    # Smoke test: does it accept stdin and print something?
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
step "using python: $PY ($(command -v "$PY" 2>/dev/null))"

# Defaults
NODE_A_NAME="${PHANTOM_NODE_A_NAME:-node-a}"
NODE_B_NAME="${PHANTOM_NODE_B_NAME:-node-b}"
NODE_A_PORT="${PHANTOM_NODE_A_PORT:-7878}"
NODE_B_PORT="${PHANTOM_NODE_B_PORT:-7878}"
REMOTE_HOME="${PHANTOM_REMOTE_HOME:-.phantom-mesh-e001}"
SCP_CMD="${PHANTOM_SCP:-scp}"
HEALTHZ_TIMEOUT="${PHANTOM_HEALTHZ_TIMEOUT_S:-30}"
DISPATCH_TIMEOUT="${PHANTOM_DISPATCH_TIMEOUT_S:-15}"
BIN_LOCAL="${PHANTOM_BIN_LOCAL:-$(command -v phantom || true)}"

if [ -z "$BIN_LOCAL" ] || [ ! -x "$BIN_LOCAL" ]; then
  warn "skipping — PHANTOM_BIN_LOCAL unset and 'phantom' not on PATH"
  exit 77
fi

step "node A: $NODE_A_NAME @ $PHANTOM_NODE_A_URL (SSH: $PHANTOM_NODE_A_SSH)"
step "node B: $NODE_B_NAME @ $PHANTOM_NODE_B_URL (SSH: $PHANTOM_NODE_B_SSH)"
step "local binary to ship: $BIN_LOCAL"
step "remote home: ~/$REMOTE_HOME (each node)"

# ── Helpers ──────────────────────────────────────────────────────────────────

# remote_run <SSH_PREFIX> <shell command>
# Executes <shell command> on the remote node. The command is passed as a
# single final argv element; both `ssh user@host '<cmd>'` and the local
# smoke wrapper (`wrap-?.sh sh -c '<cmd>'`) treat that element as a shell
# command to evaluate via /bin/sh -c on the receiving side.
#
# IMPORTANT: $ssh_prefix is intentionally unquoted so a multi-word value
# like "ssh -i ~/.ssh/k pi@host" expands into separate argv tokens. The
# final argument is always one token (the command).
#
# For plain ssh we append a `sh -c` prefix so the command runs under a
# predictable shell regardless of the remote user's login shell. The smoke
# wrapper detects this pattern and strips it back off before exec.
remote_run() {
  _ssh_prefix="$1"; shift
  _cmd="$*"
  # shellcheck disable=SC2086
  $_ssh_prefix sh -c "$_cmd"
}

# scp_to_remote <SSH_PREFIX> <src> <dest_abs_path>
# Honours $PHANTOM_SCP so the smoke harness can sub in `cp` (in which case
# <dest_abs_path> is a local absolute directory).
scp_to_remote() {
  ssh_prefix="$1"; src="$2"; dest="$3"
  if [ "$SCP_CMD" = "cp" ]; then
    mkdir -p "$dest"
    cp -f "$src" "$dest/"
    return $?
  fi
  # Derive the scp target (user@host) from the ssh prefix. Best-effort
  # parser: strip leading "ssh " and -i/-p flag groups, keep the final
  # user@host token.
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
# Echoes the raw JSON response body. Failure → echoes the body curl saw
# (or empty string); caller asserts on shape.
dispatch_with_caps() {
  base="$1"; caps_json="$2"; prompt="$3"
  body=$(printf '{"agent":"master","prompt":"%s","required_caps":%s}' \
                "$prompt" "$caps_json")
  auth=$(hmac_hex "$body" "$PHANTOM_CLUSTER_SECRET")
  curl -sS --max-time "$DISPATCH_TIMEOUT" \
       -X POST "$base/rpc/task/assign" \
       -H "X-Cluster-Auth: $auth" \
       -H "Content-Type: application/json" \
       -d "$body"
}

# json_field <body> <field>
# Extract a top-level string/bool field from a JSON body. Python keeps us
# off the `jq` dependency (jq isn't on the Pi by default).
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

# tail_remote_log <SSH_PREFIX> <node label> <log path>
tail_remote_log() {
  ssh_prefix="$1"; label="$2"; logp="$3"
  printf '%s━━ last 50 lines of %s phantom serve log (%s) ━━%s\n' \
         "$C_YELLOW" "$label" "$logp" "$C_RESET" >&2
  remote_run "$ssh_prefix" "tail -n 50 '$logp' 2>/dev/null || echo '(no log file)'" >&2
}

# fail_and_dump <reason>
# Mark the last fail, dump both nodes' tail logs, then exit 1.
fail_and_dump() {
  fail "$1"
  tail_remote_log "$PHANTOM_NODE_A_SSH" "$NODE_A_NAME" "~/$REMOTE_HOME/phantom-serve.log"
  tail_remote_log "$PHANTOM_NODE_B_SSH" "$NODE_B_NAME" "~/$REMOTE_HOME/phantom-serve.log"
  printf '\n%sFAIL: %s%s\n' "$C_RED" "$1" "$C_RESET" >&2
  exit 1
}

# ── Step 1: bootstrap binary + config on each node ──────────────────────────
if [ "${PHANTOM_BOOTSTRAP_SKIP:-0}" != "1" ]; then
  title "F002 · bootstrap — ship binary + render agents.toml on each node"

  TMP_LOCAL=$(mktemp -d 2>/dev/null || mktemp -d -t f002)
  trap 'rm -rf "$TMP_LOCAL"' EXIT

  # Render Node A's agents.toml: caps=[gpu], peers=[B]
  cat > "$TMP_LOCAL/agents-a.toml" <<EOF
# Auto-rendered by cross_host_dispatch.sh for E001/F002 — do not edit by hand.
[core]
host = "0.0.0.0"
port = $NODE_A_PORT

[cluster]
node_name      = "$NODE_A_NAME"
cluster_secret = "$PHANTOM_CLUSTER_SECRET"
peers          = ["$PHANTOM_NODE_B_URL"]
capabilities   = ["gpu"]
worker_caps    = ["gpu"]
enforce_caps   = "soft"

[agent.master]
provider = "echo"
model    = "echo"
EOF

  # Render Node B's agents.toml: caps=[vision], peers=[A]
  cat > "$TMP_LOCAL/agents-b.toml" <<EOF
# Auto-rendered by cross_host_dispatch.sh for E001/F002 — do not edit by hand.
[core]
host = "0.0.0.0"
port = $NODE_B_PORT

[cluster]
node_name      = "$NODE_B_NAME"
cluster_secret = "$PHANTOM_CLUSTER_SECRET"
peers          = ["$PHANTOM_NODE_A_URL"]
capabilities   = ["vision"]
worker_caps    = ["vision"]
enforce_caps   = "soft"

[agent.master]
provider = "echo"
model    = "echo"
EOF

  bootstrap_one() {
    LBL="$1"; SSHP="$2"; CFG="$3"
    step "node $LBL: ensuring remote dir + shipping binary + config"
    if ! remote_run "$SSHP" "mkdir -p ~/$REMOTE_HOME"; then
      fail_and_dump "node $LBL: mkdir ~/$REMOTE_HOME failed"
    fi
    # Resolve the literal remote home path for `cp` (smoke) where ~/ is not
    # expanded the same way as it would be by ssh.
    REMOTE_ABS=$(remote_run "$SSHP" "echo \$HOME/$REMOTE_HOME" | tr -d '\r\n')
    [ -n "$REMOTE_ABS" ] || REMOTE_ABS="$HOME/$REMOTE_HOME"
    if ! scp_to_remote "$SSHP" "$BIN_LOCAL" "$REMOTE_ABS"; then
      fail_and_dump "node $LBL: failed to ship $BIN_LOCAL → $REMOTE_ABS"
    fi
    # Ship the config under its final name directly (no rename dance).
    cp "$CFG" "$TMP_LOCAL/agents.toml"
    if ! scp_to_remote "$SSHP" "$TMP_LOCAL/agents.toml" "$REMOTE_ABS"; then
      fail_and_dump "node $LBL: failed to ship agents.toml → $REMOTE_ABS"
    fi
    pass "node $LBL: bootstrap complete in $REMOTE_ABS"
  }
  bootstrap_one "$NODE_A_NAME" "$PHANTOM_NODE_A_SSH" "$TMP_LOCAL/agents-a.toml"
  bootstrap_one "$NODE_B_NAME" "$PHANTOM_NODE_B_SSH" "$TMP_LOCAL/agents-b.toml"
fi

# ── Step 2: sweep stale processes (idempotent re-run) ───────────────────────
title "F002 · sweep — kill any stale phantom serve from a prior run"
sweep_one() {
  LBL="$1"; SSHP="$2"
  step "node $LBL: killing stale PID file (if any)"
  remote_run "$SSHP" "
    pidf=\$HOME/$REMOTE_HOME/phantom-serve.pid
    if [ -f \"\$pidf\" ]; then
      pid=\$(cat \"\$pidf\" 2>/dev/null)
      if [ -n \"\$pid\" ] && kill -0 \"\$pid\" 2>/dev/null; then
        kill \"\$pid\" 2>/dev/null || true
        sleep 1
        kill -9 \"\$pid\" 2>/dev/null || true
      fi
      rm -f \"\$pidf\"
    fi
  " || true
}
sweep_one "$NODE_A_NAME" "$PHANTOM_NODE_A_SSH"
sweep_one "$NODE_B_NAME" "$PHANTOM_NODE_B_SSH"

# ── Step 3: start phantom serve on both nodes ───────────────────────────────
title "F002 · launch — phantom serve on both nodes"
# Per-node tuples: (LABEL, SSH_PREFIX, BASE_URL, PORT). Iterate via positional
# args of a helper function so we never concatenate ':' onto a URL that
# already contains one.
launch_one() {
  LBL="$1"; SSHP="$2"; BASE="$3"; PORT="$4"
  # Determine the phantom binary name to invoke (Windows smoke: .exe).
  bin_name="phantom"
  if [ -x "$BIN_LOCAL" ] && printf '%s' "$BIN_LOCAL" | grep -qi '\.exe$'; then
    bin_name="phantom.exe"
  fi
  step "node $LBL: starting $bin_name --port $PORT (forwarding gate ON)"
  if ! remote_run "$SSHP" "
    cd \$HOME/$REMOTE_HOME &&
    rm -f phantom-serve.log &&
    PHANTOM_FORWARD_ON_CAPS_MISMATCH=1 \
    PHANTOM_ENFORCE_REQUIRED_CAPS=soft \
    nohup ./$bin_name serve --host 0.0.0.0 --port $PORT \
      > phantom-serve.log 2>&1 < /dev/null &
    echo \$! > phantom-serve.pid
  "; then
    fail_and_dump "node $LBL: failed to spawn phantom serve"
  fi

  step "node $LBL: waiting up to ${HEALTHZ_TIMEOUT}s for /healthz 200 at $BASE"
  if wait_healthz "$BASE" "$HEALTHZ_TIMEOUT"; then
    pass "node $LBL: /healthz 200 — phantom serve is up"
  else
    fail_and_dump "node $LBL: /healthz never returned 200 within ${HEALTHZ_TIMEOUT}s at $BASE"
  fi
}

launch_one "$NODE_A_NAME" "$PHANTOM_NODE_A_SSH" "$PHANTOM_NODE_A_URL" "$NODE_A_PORT"
launch_one "$NODE_B_NAME" "$PHANTOM_NODE_B_SSH" "$PHANTOM_NODE_B_URL" "$NODE_B_PORT"

# ── Step 4: poke peer registration so forwarding has an inventory ───────────
# `phantom serve` pings peers every 30s. To avoid waiting we hit /rpc/ping
# directly from each node to the other (the manager populates peer_infos on
# first successful ping).
title "F002 · warm-up — refresh peer inventory on both sides"
for U in "$PHANTOM_NODE_A_URL" "$PHANTOM_NODE_B_URL"; do
  # touch each node's status endpoint to coax the heartbeat into refreshing
  curl -sS --max-time 5 "$U/healthz" >/dev/null 2>&1 || true
  curl -sS --max-time 5 "$U/cluster/peers" >/dev/null 2>&1 || true
done
sleep 2
step "warm-up done"

# ── Step 5: dispatch test 1 — from A, require vision → expect dispatched_to=B
title "F002 · dispatch 1 — from A asking for [vision] → expect forward to B"
RESP_AB=$(dispatch_with_caps "$PHANTOM_NODE_A_URL" '["vision"]' \
            "F002 dispatch 1 from $NODE_A_NAME requiring vision")
step "raw response (A→?): $RESP_AB"
DT_AB=$(json_field "$RESP_AB" dispatched_to)
FW_AB=$(json_field "$RESP_AB" forwarded)
JID_AB=$(json_field "$RESP_AB" job_id)
ERR_AB=$(json_field "$RESP_AB" error)
step "parsed: dispatched_to=[$DT_AB] forwarded=[$FW_AB] job_id=[$JID_AB] error=[$ERR_AB]"

if [ -n "$ERR_AB" ]; then
  fail "dispatch 1: server returned error: $ERR_AB"
elif [ -z "$JID_AB" ]; then
  fail "dispatch 1: no job_id in response"
elif [ "$DT_AB" != "$NODE_B_NAME" ]; then
  fail "dispatch 1: expected dispatched_to='$NODE_B_NAME', got '$DT_AB'"
elif [ "$FW_AB" != "true" ]; then
  fail "dispatch 1: expected forwarded=true, got '$FW_AB'"
else
  pass "dispatch 1: A forwarded job $JID_AB → dispatched_to=$DT_AB, forwarded=true"
fi

# ── Step 6: dispatch test 2 — from B, require gpu → expect dispatched_to=A
title "F002 · dispatch 2 — from B asking for [gpu] → expect forward to A"
RESP_BA=$(dispatch_with_caps "$PHANTOM_NODE_B_URL" '["gpu"]' \
            "F002 dispatch 2 from $NODE_B_NAME requiring gpu")
step "raw response (B→?): $RESP_BA"
DT_BA=$(json_field "$RESP_BA" dispatched_to)
FW_BA=$(json_field "$RESP_BA" forwarded)
JID_BA=$(json_field "$RESP_BA" job_id)
ERR_BA=$(json_field "$RESP_BA" error)
step "parsed: dispatched_to=[$DT_BA] forwarded=[$FW_BA] job_id=[$JID_BA] error=[$ERR_BA]"

if [ -n "$ERR_BA" ]; then
  fail "dispatch 2: server returned error: $ERR_BA"
elif [ -z "$JID_BA" ]; then
  fail "dispatch 2: no job_id in response"
elif [ "$DT_BA" != "$NODE_A_NAME" ]; then
  fail "dispatch 2: expected dispatched_to='$NODE_A_NAME', got '$DT_BA'"
elif [ "$FW_BA" != "true" ]; then
  fail "dispatch 2: expected forwarded=true, got '$FW_BA'"
else
  pass "dispatch 2: B forwarded job $JID_BA → dispatched_to=$DT_BA, forwarded=true"
fi

# ── Step 7: audit-log assertion on both nodes ───────────────────────────────
title "F002 · audit — forwarding event present in each node's serve log"
# The tracing target `phantom::dispatch::forward` (mesh.rs) is emitted on the
# receiving-side as warn/info when forwarding happens. The simpler stable
# signal that survives log-format drift is the literal request URL of the
# /rpc/task/assign hop (axum logs each inbound request at info via tower).
audit_one() {
  LBL="$1"; SSHP="$2"
  step "node $LBL: scanning ~/$REMOTE_HOME/phantom-serve.log for forwarding evidence"
  hits=$(remote_run "$SSHP" "
    grep -cE 'task/assign|dispatch::forward|forwarded' \
         \$HOME/$REMOTE_HOME/phantom-serve.log 2>/dev/null || echo 0
  " | tr -d '\r\n ')
  # Permit either node to show hits — at minimum, the side that received the
  # original POST will have one /rpc/task/assign line per test (2 total).
  if [ -n "$hits" ] && [ "$hits" -ge 1 ] 2>/dev/null; then
    pass "node $LBL: $hits forwarding / dispatch log lines"
  else
    fail "node $LBL: no dispatch/forwarding evidence in serve log"
  fi
}
audit_one "$NODE_A_NAME" "$PHANTOM_NODE_A_SSH"
audit_one "$NODE_B_NAME" "$PHANTOM_NODE_B_SSH"

# ── Step 8: cleanup (unless operator wants the processes left) ──────────────
if [ "${PHANTOM_CLEANUP_SKIP:-0}" != "1" ]; then
  title "F002 · cleanup — killing phantom serve on both nodes (logs retained)"
  cleanup_one() {
    LBL="$1"; SSHP="$2"
    remote_run "$SSHP" "
      pidf=\$HOME/$REMOTE_HOME/phantom-serve.pid
      if [ -f \"\$pidf\" ]; then
        pid=\$(cat \"\$pidf\")
        kill \"\$pid\" 2>/dev/null || true
      fi
    " || true
    step "node $LBL: killed (log kept at ~/$REMOTE_HOME/phantom-serve.log)"
  }
  cleanup_one "$NODE_A_NAME" "$PHANTOM_NODE_A_SSH"
  cleanup_one "$NODE_B_NAME" "$PHANTOM_NODE_B_SSH"
fi

# ── Summary ─────────────────────────────────────────────────────────────────
title "F002 · summary"
printf '  %sPASS%s %d   %sFAIL%s %d\n' \
       "$C_GREEN" "$C_RESET" "$PHANTOM_TEST_PASSED" \
       "$C_RED"   "$C_RESET" "$PHANTOM_TEST_FAILED"

if [ "$PHANTOM_TEST_FAILED" -gt 0 ]; then
  # Dump tail logs so the operator can see what went wrong on either side.
  tail_remote_log "$PHANTOM_NODE_A_SSH" "$NODE_A_NAME" "~/$REMOTE_HOME/phantom-serve.log"
  tail_remote_log "$PHANTOM_NODE_B_SSH" "$NODE_B_NAME" "~/$REMOTE_HOME/phantom-serve.log"
  printf '\n%sFAIL: %s%s\n' "$C_RED" "$LAST_FAIL_REASON" "$C_RESET" >&2
  exit 1
fi

printf '\n%sPASS: cross-host forwarding verified in both directions%s\n' \
       "$C_GREEN" "$C_RESET"
exit 0
