#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# three_node_demo.sh — v0.6.0 three-node demo e2e scenario.
#
# THE central proof artifact for the v0.6.0 demo (DEMO P1):
#   "Routed N tasks across 3 nodes / 3 architectures / 3 OSes / Tailscale-only
#    network."
#
# Runbook: docs/superpowers/runbooks/three-node-demo-bring-up.md
# Sibling : scripts/phantom-test/scenarios/cross_host_dispatch.sh (F002, 2 nodes)
#
# What it proves
# --------------
# Given THREE phantom-mesh nodes on the same Tailscale tailnet with NON-
# OVERLAPPING capabilities — workstation = [gpu], cloud = [always_on,big_disk],
# mobile = [mobile,camera] — this script fires four dispatches that each
# require capability sets only one of the three nodes has, and asserts each
# task is routed to the right node:
#
#   1. A → required_caps=[always_on] → dispatched_to=cloud
#   2. A → required_caps=[mobile]    → dispatched_to=mobile
#   3. C → required_caps=[gpu]       → dispatched_to=workstation
#   4. B → required_caps=[camera]    → dispatched_to=mobile
#
# Then it prints a narrative table that the operator can paste straight into
# the PR / announcement post.
#
# IMPORTANT DIFFERENCE vs F002:
# F002 (`cross_host_dispatch.sh`) SCPs a binary + renders agents.toml on each
# node from this host. This script does NEITHER — it assumes all three nodes
# are already running (the operator already ran `setup-oracle-ampere.sh` and
# `setup-android-termux.sh`, and the workstation already has phantom). We
# only POST dispatches and assert.
#
# Environment contract (REQUIRED)
# -------------------------------
#   PHANTOM_NODE_A_URL       — Workstation base URL, e.g.
#                              http://workstation.tail.ts.net:7878
#   PHANTOM_NODE_B_URL       — Cloud  base URL, e.g. http://cloud.tail.ts.net:7878
#   PHANTOM_NODE_C_URL       — Mobile base URL, e.g. http://mobile.tail.ts.net:7878
#   PHANTOM_CLUSTER_SECRET   — same 64-hex secret all three nodes were configured with
#
# Environment contract (OPTIONAL)
# -------------------------------
#   PHANTOM_NODE_A_NAME      — workstation's node_name (default: "workstation")
#   PHANTOM_NODE_B_NAME      — cloud's node_name       (default: "cloud")
#   PHANTOM_NODE_C_NAME      — mobile's node_name      (default: "mobile")
#   PHANTOM_DISPATCH_TIMEOUT_S — per-dispatch curl timeout (default: 20)
#   PHANTOM_HEALTHZ_TIMEOUT_S  — preflight /healthz timeout (default: 15)
#
# Exit codes
# ----------
#   0   — all four dispatches routed correctly
#   1   — at least one assertion failed
#   77  — preconditions missing
#
# Operator workflow
# -----------------
#   export PHANTOM_NODE_A_URL=http://workstation.tail.ts.net:7878
#   export PHANTOM_NODE_B_URL=http://cloud.tail.ts.net:7878
#   export PHANTOM_NODE_C_URL=http://mobile.tail.ts.net:7878
#   export PHANTOM_CLUSTER_SECRET=$(cat ~/.phantom-mesh/cluster_secret)
#   bash scripts/phantom-test/scenarios/three_node_demo.sh
#
# ─────────────────────────────────────────────────────────────────────────────

set -u

# ── pretty output ────────────────────────────────────────────────────────────
if [ -t 1 ] && [ "${NO_COLOR:-}" = "" ]; then
  C_RESET=$'\033[0m'; C_DIM=$'\033[2m'; C_BOLD=$'\033[1m'
  C_RED=$'\033[31m'; C_GREEN=$'\033[32m'; C_YELLOW=$'\033[33m'; C_CYAN=$'\033[36m'
else
  C_RESET=''; C_DIM=''; C_BOLD=''
  C_RED=''; C_GREEN=''; C_YELLOW=''; C_CYAN=''
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
    sed -n '2,60p' "$0" | sed -E 's/^# ?//'
    exit 0
    ;;
esac

# ── preconditions ────────────────────────────────────────────────────────────
title "three_node_demo · preconditions"

REQUIRED_ENV="PHANTOM_NODE_A_URL PHANTOM_NODE_B_URL PHANTOM_NODE_C_URL PHANTOM_CLUSTER_SECRET"
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

# Pick a working python (same logic as cross_host_dispatch.sh — Git Bash on
# Windows ships python.exe; Linux/macOS prefer python3; the WindowsApps
# python3 stub opens a Store popup, reject it).
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
step "python: $PY"

# Defaults
NODE_A_NAME="${PHANTOM_NODE_A_NAME:-workstation}"
NODE_B_NAME="${PHANTOM_NODE_B_NAME:-cloud}"
NODE_C_NAME="${PHANTOM_NODE_C_NAME:-mobile}"
HEALTHZ_TIMEOUT="${PHANTOM_HEALTHZ_TIMEOUT_S:-15}"
DISPATCH_TIMEOUT="${PHANTOM_DISPATCH_TIMEOUT_S:-20}"

step "node A (workstation, gpu)        : $NODE_A_NAME @ $PHANTOM_NODE_A_URL"
step "node B (cloud,       always_on)  : $NODE_B_NAME @ $PHANTOM_NODE_B_URL"
step "node C (mobile,      mobile)     : $NODE_C_NAME @ $PHANTOM_NODE_C_URL"

# ── helpers ──────────────────────────────────────────────────────────────────
hmac_hex() {
  printf '%s' "$1" | openssl dgst -sha256 -hmac "$2" -hex | awk '{print $2}'
}

wait_healthz() {
  base="$1"; deadline=$(( $(date +%s) + ${2:-15} ))
  while [ "$(date +%s)" -lt "$deadline" ]; do
    code=$(curl -sS -o /dev/null -w '%{http_code}' --max-time 3 \
                "$base/healthz" 2>/dev/null || echo 000)
    if [ "$code" = "200" ]; then return 0; fi
    sleep 1
  done
  return 1
}

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

# ── preflight: all three /healthz reachable ─────────────────────────────────
title "three_node_demo · preflight — all three /healthz reachable"
preflight_one() {
  LBL="$1"; URL="$2"
  step "node $LBL: GET $URL/healthz (timeout ${HEALTHZ_TIMEOUT}s)"
  if wait_healthz "$URL" "$HEALTHZ_TIMEOUT"; then
    pass "node $LBL: /healthz 200"
  else
    fail "node $LBL: /healthz never returned 200 within ${HEALTHZ_TIMEOUT}s"
  fi
}
preflight_one "$NODE_A_NAME" "$PHANTOM_NODE_A_URL"
preflight_one "$NODE_B_NAME" "$PHANTOM_NODE_B_URL"
preflight_one "$NODE_C_NAME" "$PHANTOM_NODE_C_URL"

if [ "$PHANTOM_TEST_FAILED" -gt 0 ]; then
  printf '\n%sFAIL: preflight failed — at least one node is not serving; aborting before dispatch%s\n' \
         "$C_RED" "$C_RESET" >&2
  exit 1
fi

# Coax peer inventory refresh so dispatch has fresh peer caps.
for U in "$PHANTOM_NODE_A_URL" "$PHANTOM_NODE_B_URL" "$PHANTOM_NODE_C_URL"; do
  curl -sS --max-time 5 "$U/cluster/peers" >/dev/null 2>&1 || true
done
sleep 2

# ── per-dispatch assertion helper ───────────────────────────────────────────
# Args: <label> <from_url> <caps_json_array> <expected_node_name>
# Records into RESULTS_* arrays for the final narrative table.
RESULTS_FROM=""
RESULTS_CAPS=""
RESULTS_TO=""
RESULTS_FW=""
RESULTS_OK=""
record() {
  RESULTS_FROM="${RESULTS_FROM}|${1}"
  RESULTS_CAPS="${RESULTS_CAPS}|${2}"
  RESULTS_TO="${RESULTS_TO}|${3}"
  RESULTS_FW="${RESULTS_FW}|${4}"
  RESULTS_OK="${RESULTS_OK}|${5}"
}

run_dispatch_test() {
  LBL="$1"; FROM_NAME="$2"; FROM_URL="$3"; CAPS_JSON="$4"; EXPECTED="$5"
  title "$LBL — from $FROM_NAME require $CAPS_JSON → expect dispatched_to=$EXPECTED"
  RESP=$(dispatch_with_caps "$FROM_URL" "$CAPS_JSON" "$LBL from $FROM_NAME require $CAPS_JSON")
  step "raw response: $RESP"
  DT=$(json_field "$RESP" dispatched_to)
  FW=$(json_field "$RESP" forwarded)
  JID=$(json_field "$RESP" job_id)
  ERR=$(json_field "$RESP" error)
  step "parsed: dispatched_to=[$DT] forwarded=[$FW] job_id=[$JID] error=[$ERR]"

  if [ -n "$ERR" ]; then
    fail "$LBL: server returned error: $ERR"
    record "$FROM_NAME" "$CAPS_JSON" "(error: $ERR)" "-" "FAIL"
    return
  fi
  if [ -z "$JID" ]; then
    fail "$LBL: no job_id in response"
    record "$FROM_NAME" "$CAPS_JSON" "(no job_id)" "-" "FAIL"
    return
  fi
  if [ "$DT" != "$EXPECTED" ]; then
    fail "$LBL: expected dispatched_to='$EXPECTED', got '$DT'"
    record "$FROM_NAME" "$CAPS_JSON" "$DT" "$FW" "FAIL"
    return
  fi
  pass "$LBL: routed to $DT (forwarded=$FW, job=$JID)"
  record "$FROM_NAME" "$CAPS_JSON" "$DT" "$FW" "OK"
}

# ── 4 dispatch scenarios ────────────────────────────────────────────────────
# 1. From workstation, require always_on → cloud
run_dispatch_test "dispatch 1" "$NODE_A_NAME" "$PHANTOM_NODE_A_URL" '["always_on"]' "$NODE_B_NAME"
# 2. From workstation, require mobile → mobile
run_dispatch_test "dispatch 2" "$NODE_A_NAME" "$PHANTOM_NODE_A_URL" '["mobile"]'    "$NODE_C_NAME"
# 3. From mobile, require gpu → workstation
run_dispatch_test "dispatch 3" "$NODE_C_NAME" "$PHANTOM_NODE_C_URL" '["gpu"]'       "$NODE_A_NAME"
# 4. From cloud, require camera → mobile
run_dispatch_test "dispatch 4" "$NODE_B_NAME" "$PHANTOM_NODE_B_URL" '["camera"]'    "$NODE_C_NAME"

# ── narrative table ─────────────────────────────────────────────────────────
title "three_node_demo · narrative table"

# Convert the |-separated histories into aligned columns.
# (POSIX-bash on purpose: no arrays.)
printf '\n'
printf '  %s%-4s %-14s %-22s %-14s %-5s %-4s%s\n' "$C_BOLD" \
       "#" "from"  "required_caps" "dispatched_to" "fwd?" "ok"  "$C_RESET"
printf '  %s%-4s %-14s %-22s %-14s %-5s %-4s%s\n' "$C_DIM" \
       "--" "----"  "--------------" "--------------" "-----" "----" "$C_RESET"

i=0
# IFS=| split on the leading-empty trick.
oldIFS="$IFS"; IFS='|'
set -- $RESULTS_FROM; FROMS="$*"
set -- $RESULTS_CAPS; CAPSS="$*"
set -- $RESULTS_TO;   TOS="$*"
set -- $RESULTS_FW;   FWS="$*"
set -- $RESULTS_OK;   OKS="$*"
IFS="$oldIFS"

# Reparse — using `set --` then space-split because the leading "|" produced
# an empty first field. Rebuild as space-separated tokens we can iterate.
# Each cap_json contains brackets and quotes, so we substitute commas-in-list
# for ',' (no space) which keeps a single token per record.
parse_to_rows() {
  raw="$1"
  printf '%s\n' "$raw" | tr '|' '\n' | sed '/^$/d'
}

ROW_FROM=$(parse_to_rows "$RESULTS_FROM")
ROW_CAPS=$(parse_to_rows "$RESULTS_CAPS")
ROW_TO=$(parse_to_rows   "$RESULTS_TO")
ROW_FW=$(parse_to_rows   "$RESULTS_FW")
ROW_OK=$(parse_to_rows   "$RESULTS_OK")

# Print them line-by-line using paste-like coordination.
N=$(printf '%s\n' "$ROW_FROM" | wc -l | tr -d ' ')
i=1
while [ "$i" -le "$N" ]; do
  f=$(printf '%s\n' "$ROW_FROM" | sed -n "${i}p")
  c=$(printf '%s\n' "$ROW_CAPS" | sed -n "${i}p")
  t=$(printf '%s\n' "$ROW_TO"   | sed -n "${i}p")
  w=$(printf '%s\n' "$ROW_FW"   | sed -n "${i}p")
  o=$(printf '%s\n' "$ROW_OK"   | sed -n "${i}p")
  if [ "$o" = "OK" ]; then
    OK_C="$C_GREEN"
  else
    OK_C="$C_RED"
  fi
  printf '  %-4s %-14s %-22s %-14s %-5s %s%-4s%s\n' \
         "$i" "$f" "$c" "$t" "$w" "$OK_C" "$o" "$C_RESET"
  i=$((i + 1))
done

# ── summary ─────────────────────────────────────────────────────────────────
title "three_node_demo · summary"
printf '  %sPASS%s %d   %sFAIL%s %d\n' \
       "$C_GREEN" "$C_RESET" "$PHANTOM_TEST_PASSED" \
       "$C_RED"   "$C_RESET" "$PHANTOM_TEST_FAILED"

if [ "$PHANTOM_TEST_FAILED" -gt 0 ]; then
  printf '\n%sFAIL: %s%s\n' "$C_RED" "$LAST_FAIL_REASON" "$C_RESET" >&2
  exit 1
fi

printf '\n'
printf '%s━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━%s\n' \
       "$C_GREEN" "$C_RESET"
printf '%s  Routed %d tasks across 3 nodes / 3 architectures /          %s\n' \
       "$C_GREEN" "$PHANTOM_TEST_PASSED" "$C_RESET"
printf '%s  3 OSes / Tailscale-only network — DEMO P1 proven.           %s\n' \
       "$C_GREEN" "$C_RESET"
printf '%s━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━%s\n' \
       "$C_GREEN" "$C_RESET"
printf '\n'
printf '  Architectures : x86_64 (workstation) · aarch64-linux (cloud) · aarch64-android (mobile)\n'
printf '  OSes          : Windows 11           · Ubuntu 22.04 LTS       · Android (Termux)\n'
printf '  Network       : Tailscale tailnet (no port-forwarding, no public IPs needed)\n'
printf '\n'
exit 0
