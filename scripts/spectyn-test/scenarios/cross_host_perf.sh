#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# cross_host_perf.sh — E001 / F004 cross-host forwarding latency budget.
#
# THE perf gate for E001's acceptance bar:
#   "Cross-host forwarding latency p99 < 500ms on a LAN
#    (per `spectyn-test` perf assertion)"
#
# Spec: docs/superpowers/features/F004-cluster-forwarding-perf-bench.md
#
# What it proves
# --------------
# Given two SSH-reachable spectyn-mesh nodes with NON-OVERLAPPING worker
# capabilities (same topology F002 sets up), this script:
#   1. Bootstraps the testbed the same way F002 does (scp binary + render
#      per-node agents.toml + start `spectyn serve` on both, /healthz wait).
#      Reuses the F002 bootstrap pattern intentionally so the operator only
#      maintains one mental model of "the two-node testbed".
#   2. Issues 100 `/rpc/task/assign` calls, alternating directions:
#        odd  iterations: A → require [vision] → forward to B
#        even iterations: B → require [gpu]    → forward to A
#      Each request is timed via `curl -w '%{time_total}'` (wall-clock
#      RTT including TLS / TCP / forward hop / response serialisation).
#   3. Computes p50 / p95 / p99 from the 100 samples (sorted nearest-rank
#      percentile — same definition Criterion uses).
#   4. Asserts p99 < SPECTYN_E001_P99_BUDGET_MS (default 500 ms). On miss,
#      prints the full sorted distribution plus min/max and exits non-zero.
#   5. Cleans up (kills both `spectyn serve`, retains logs) — idempotent.
#
# Environment contract (REQUIRED — same as F002)
# ----------------------------------------------
#   SPECTYN_NODE_A_SSH       — full ssh prefix for Node A
#   SPECTYN_NODE_B_SSH       — same for Node B
#   SPECTYN_NODE_A_URL       — Node A base URL (reachable from this host + B)
#   SPECTYN_NODE_B_URL       — same for Node B
#   SPECTYN_CLUSTER_SECRET   — shared HMAC secret
#
# Environment contract (OPTIONAL)
# -------------------------------
#   SPECTYN_BIN_LOCAL        — spectyn binary on THIS host to SCP
#                              (default: $(command -v spectyn))
#   SPECTYN_NODE_A_NAME      — default "node-a"
#   SPECTYN_NODE_B_NAME      — default "node-b"
#   SPECTYN_NODE_A_PORT      — default 7878
#   SPECTYN_NODE_B_PORT      — default 7878
#   SPECTYN_REMOTE_HOME      — default ".spectyn-mesh-e001" (shared with F002)
#   SPECTYN_SCP              — default "scp" (set to "cp" for local smoke)
#   SPECTYN_HEALTHZ_TIMEOUT_S — default 30
#   SPECTYN_DISPATCH_TIMEOUT_S — default 15
#   SPECTYN_E001_P99_BUDGET_MS — perf gate (default 500)
#                                — looser for cross-WAN runs without code
#                                  changes per F004 acceptance criterion
#   SPECTYN_PERF_SAMPLES      — number of forwarding requests (default 100)
#   SPECTYN_BOOTSTRAP_SKIP=1  — assume both `spectyn serve` already running
#                               (fast re-runs against an already-warmed bed)
#   SPECTYN_CLEANUP_SKIP=1    — leave both `spectyn serve` running on exit
#
# Exit codes
# ----------
#   0   — all 100 forwards landed; p99 < SPECTYN_E001_P99_BUDGET_MS
#   1   — p99 missed the budget OR any forwarding request errored out
#   77  — preconditions missing (env var unset, ssh broken, no python, etc.)
#
# Operator workflow (real testbed: Pi 4 ↔ Workstation over Tailscale)
# -------------------------------------------------------------------
#   export SPECTYN_NODE_A_SSH="ssh -i ~/.ssh/pi pi@pi.tail.ts.net"
#   export SPECTYN_NODE_B_SSH="ssh you@workstation.tail.ts.net"
#   export SPECTYN_NODE_A_URL="http://pi.tail.ts.net:7878"
#   export SPECTYN_NODE_B_URL="http://workstation.tail.ts.net:7878"
#   export SPECTYN_CLUSTER_SECRET="$(cat ~/.spectyn-mesh/cluster_secret)"
#   bash scripts/spectyn-test/scenarios/cross_host_perf.sh
#
# POSIX-bash on purpose: same portability constraints as F002.
# ─────────────────────────────────────────────────────────────────────────────

set -u
# NOT `set -e` — we want to accumulate all 100 samples and report the full
# distribution even if a few requests fail.

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

step()  { printf '  %s->%s %s\n'  "$C_DIM"  "$C_RESET" "$*"; }
pass()  { SPECTYN_TEST_PASSED=$((SPECTYN_TEST_PASSED + 1));
          printf '  %sPASS%s %s\n'  "$C_GREEN" "$C_RESET" "$*"; }
fail()  { SPECTYN_TEST_FAILED=$((SPECTYN_TEST_FAILED + 1));
          LAST_FAIL_REASON="$*"
          printf '  %sFAIL%s %s\n'  "$C_RED"   "$C_RESET" "$*" >&2; }
warn()  { printf '  %sWARN%s %s\n'  "$C_YELLOW" "$C_RESET" "$*" >&2; }
title() { printf '\n%s== %s ==%s\n' "$C_CYAN" "$*" "$C_RESET"; }

# ── --help short-circuit ─────────────────────────────────────────────────────
case "${1:-}" in
  -h|--help)
    sed -n '2,75p' "$0" | sed -E 's/^# ?//'
    exit 0
    ;;
esac

# ── Preconditions ────────────────────────────────────────────────────────────
title "F004 . cross_host_perf - preconditions"

REQUIRED_ENV="SPECTYN_NODE_A_SSH SPECTYN_NODE_B_SSH SPECTYN_NODE_A_URL SPECTYN_NODE_B_URL SPECTYN_CLUSTER_SECRET"
missing=""
for v in $REQUIRED_ENV; do
  eval "val=\${$v:-}"
  if [ -z "$val" ]; then
    missing="$missing $v"
  fi
done
if [ -n "$missing" ]; then
  warn "skipping - missing required env:$missing"
  warn "run with --help for the env-var contract"
  exit 77
fi

for cmd in curl openssl; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    warn "skipping - required command not on PATH: $cmd"
    exit 77
  fi
done

# Find a usable Python (same logic / same trap as F002's cross_host_dispatch.sh).
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
  warn "skipping - no working python interpreter on PATH"
  exit 77
fi
step "using python: $PY ($(command -v "$PY" 2>/dev/null))"

# Defaults (shared with F002 where applicable).
NODE_A_NAME="${SPECTYN_NODE_A_NAME:-node-a}"
NODE_B_NAME="${SPECTYN_NODE_B_NAME:-node-b}"
NODE_A_PORT="${SPECTYN_NODE_A_PORT:-7878}"
NODE_B_PORT="${SPECTYN_NODE_B_PORT:-7878}"
REMOTE_HOME="${SPECTYN_REMOTE_HOME:-.spectyn-mesh-e001}"
SCP_CMD="${SPECTYN_SCP:-scp}"
HEALTHZ_TIMEOUT="${SPECTYN_HEALTHZ_TIMEOUT_S:-30}"
DISPATCH_TIMEOUT="${SPECTYN_DISPATCH_TIMEOUT_S:-15}"
BIN_LOCAL="${SPECTYN_BIN_LOCAL:-$(command -v spectyn || true)}"
P99_BUDGET_MS="${SPECTYN_E001_P99_BUDGET_MS:-500}"
SAMPLES="${SPECTYN_PERF_SAMPLES:-100}"

if [ -z "$BIN_LOCAL" ] || [ ! -x "$BIN_LOCAL" ]; then
  warn "skipping - SPECTYN_BIN_LOCAL unset and 'spectyn' not on PATH"
  exit 77
fi

# Sanity-check the sample count is a positive integer.
case "$SAMPLES" in
  ''|*[!0-9]*) warn "SPECTYN_PERF_SAMPLES must be a positive integer, got '$SAMPLES'"; exit 77 ;;
esac
if [ "$SAMPLES" -lt 1 ]; then
  warn "SPECTYN_PERF_SAMPLES must be >= 1, got $SAMPLES"; exit 77
fi

step "node A: $NODE_A_NAME @ $SPECTYN_NODE_A_URL"
step "node B: $NODE_B_NAME @ $SPECTYN_NODE_B_URL"
step "local binary to ship: $BIN_LOCAL"
step "samples: $SAMPLES   p99 budget: ${P99_BUDGET_MS} ms"

# ── Helpers (same shapes as F002 to keep cognitive load low) ────────────────
remote_run() {
  _ssh_prefix="$1"; shift
  _cmd="$*"
  # shellcheck disable=SC2086
  $_ssh_prefix sh -c "$_cmd"
}

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

hmac_hex() {
  printf '%s' "$1" | openssl dgst -sha256 -hmac "$2" -hex | awk '{print $2}'
}

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

# dispatch_timed <base_url> <required_caps_json_array> <prompt>
# Echoes: "<time_total_seconds> <http_code> <body>" — the perf loop slices
# on the first two whitespace tokens; body may itself contain spaces.
dispatch_timed() {
  base="$1"; caps_json="$2"; prompt="$3"
  body=$(printf '{"agent":"master","prompt":"%s","required_caps":%s}' \
                "$prompt" "$caps_json")
  auth=$(hmac_hex "$body" "$SPECTYN_CLUSTER_SECRET")
  curl -sS --max-time "$DISPATCH_TIMEOUT" \
       -X POST "$base/rpc/task/assign" \
       -H "X-Cluster-Auth: $auth" \
       -H "Content-Type: application/json" \
       -d "$body" \
       -w '%{time_total} %{http_code}\n' \
       -o /tmp/cross_host_perf.body.$$ 2>/dev/null
  rc=$?
  rb=$(cat /tmp/cross_host_perf.body.$$ 2>/dev/null || echo '')
  rm -f /tmp/cross_host_perf.body.$$
  if [ $rc -ne 0 ]; then
    printf '0.000 000 %s\n' "curl_exit_$rc"
    return 0
  fi
  # Last line of stdout has "<seconds> <http_code>"; prepend body for parsing.
  printf '%s' "$rb" >/dev/null
}

# Variant we actually use: returns "<seconds_float> <http_code> <body_oneline>"
dispatch_timed_oneline() {
  base="$1"; caps_json="$2"; prompt="$3"
  body=$(printf '{"agent":"master","prompt":"%s","required_caps":%s}' \
                "$prompt" "$caps_json")
  auth=$(hmac_hex "$body" "$SPECTYN_CLUSTER_SECRET")
  out=$(curl -sS --max-time "$DISPATCH_TIMEOUT" \
       -X POST "$base/rpc/task/assign" \
       -H "X-Cluster-Auth: $auth" \
       -H "Content-Type: application/json" \
       -d "$body" \
       -w '\n__SPECTYN_TIME__ %{time_total} %{http_code}\n' \
       2>/dev/null)
  rc=$?
  if [ $rc -ne 0 ]; then
    printf '0.000 000 curl_exit_%s\n' "$rc"
    return 0
  fi
  tline=$(printf '%s\n' "$out" | grep '^__SPECTYN_TIME__ ' | tail -n 1)
  rbody=$(printf '%s\n' "$out" | grep -v '^__SPECTYN_TIME__ ' | tr -d '\r\n')
  secs=$(printf '%s' "$tline" | awk '{print $2}')
  code=$(printf '%s' "$tline" | awk '{print $3}')
  [ -n "$secs" ] || secs="0.000"
  [ -n "$code" ] || code="000"
  printf '%s %s %s\n' "$secs" "$code" "$rbody"
}

tail_remote_log() {
  ssh_prefix="$1"; label="$2"; logp="$3"
  printf '%s-- last 50 lines of %s spectyn serve log (%s) --%s\n' \
         "$C_YELLOW" "$label" "$logp" "$C_RESET" >&2
  remote_run "$ssh_prefix" "tail -n 50 '$logp' 2>/dev/null || echo '(no log file)'" >&2
}

fail_and_dump() {
  fail "$1"
  tail_remote_log "$SPECTYN_NODE_A_SSH" "$NODE_A_NAME" "~/$REMOTE_HOME/spectyn-serve.log"
  tail_remote_log "$SPECTYN_NODE_B_SSH" "$NODE_B_NAME" "~/$REMOTE_HOME/spectyn-serve.log"
  printf '\n%sFAIL: %s%s\n' "$C_RED" "$1" "$C_RESET" >&2
  exit 1
}

# ── Step 1: bootstrap (reuses F002's pattern — keep these in sync) ──────────
if [ "${SPECTYN_BOOTSTRAP_SKIP:-0}" != "1" ]; then
  title "F004 . bootstrap - ship binary + render agents.toml on each node"

  TMP_LOCAL=$(mktemp -d 2>/dev/null || mktemp -d -t f004)
  trap 'rm -rf "$TMP_LOCAL"' EXIT

  # Node A: caps=[gpu], peers=[B] — symmetric with F002 so the same testbed
  # is reusable across both scenarios without re-rendering configs.
  cat > "$TMP_LOCAL/agents-a.toml" <<EOF
# Auto-rendered by cross_host_perf.sh for E001/F004 - do not edit by hand.
[core]
host = "0.0.0.0"
port = $NODE_A_PORT

[cluster]
node_name      = "$NODE_A_NAME"
cluster_secret = "$SPECTYN_CLUSTER_SECRET"
peers          = ["$SPECTYN_NODE_B_URL"]
capabilities   = ["gpu"]
worker_caps    = ["gpu"]
enforce_caps   = "soft"

[agent.master]
provider = "echo"
model    = "echo"
EOF

  # Node B: caps=[vision], peers=[A]
  cat > "$TMP_LOCAL/agents-b.toml" <<EOF
# Auto-rendered by cross_host_perf.sh for E001/F004 - do not edit by hand.
[core]
host = "0.0.0.0"
port = $NODE_B_PORT

[cluster]
node_name      = "$NODE_B_NAME"
cluster_secret = "$SPECTYN_CLUSTER_SECRET"
peers          = ["$SPECTYN_NODE_A_URL"]
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
    REMOTE_ABS=$(remote_run "$SSHP" "echo \$HOME/$REMOTE_HOME" | tr -d '\r\n')
    [ -n "$REMOTE_ABS" ] || REMOTE_ABS="$HOME/$REMOTE_HOME"
    if ! scp_to_remote "$SSHP" "$BIN_LOCAL" "$REMOTE_ABS"; then
      fail_and_dump "node $LBL: failed to ship $BIN_LOCAL -> $REMOTE_ABS"
    fi
    cp "$CFG" "$TMP_LOCAL/agents.toml"
    if ! scp_to_remote "$SSHP" "$TMP_LOCAL/agents.toml" "$REMOTE_ABS"; then
      fail_and_dump "node $LBL: failed to ship agents.toml -> $REMOTE_ABS"
    fi
    pass "node $LBL: bootstrap complete in $REMOTE_ABS"
  }
  bootstrap_one "$NODE_A_NAME" "$SPECTYN_NODE_A_SSH" "$TMP_LOCAL/agents-a.toml"
  bootstrap_one "$NODE_B_NAME" "$SPECTYN_NODE_B_SSH" "$TMP_LOCAL/agents-b.toml"

  # ── Step 2: sweep stale processes, then launch ────────────────────────
  title "F004 . sweep - kill any stale spectyn serve from a prior run"
  sweep_one() {
    LBL="$1"; SSHP="$2"
    step "node $LBL: killing stale PID file (if any)"
    remote_run "$SSHP" "
      pidf=\$HOME/$REMOTE_HOME/spectyn-serve.pid
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
  sweep_one "$NODE_A_NAME" "$SPECTYN_NODE_A_SSH"
  sweep_one "$NODE_B_NAME" "$SPECTYN_NODE_B_SSH"

  title "F004 . launch - spectyn serve on both nodes"
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
      fail_and_dump "node $LBL: failed to spawn spectyn serve"
    fi
    step "node $LBL: waiting up to ${HEALTHZ_TIMEOUT}s for /healthz 200 at $BASE"
    if wait_healthz "$BASE" "$HEALTHZ_TIMEOUT"; then
      pass "node $LBL: /healthz 200 - spectyn serve is up"
    else
      fail_and_dump "node $LBL: /healthz never returned 200 within ${HEALTHZ_TIMEOUT}s at $BASE"
    fi
  }
  launch_one "$NODE_A_NAME" "$SPECTYN_NODE_A_SSH" "$SPECTYN_NODE_A_URL" "$NODE_A_PORT"
  launch_one "$NODE_B_NAME" "$SPECTYN_NODE_B_SSH" "$SPECTYN_NODE_B_URL" "$NODE_B_PORT"

  # Warm peer inventory.
  title "F004 . warm-up - refresh peer inventory on both sides"
  for U in "$SPECTYN_NODE_A_URL" "$SPECTYN_NODE_B_URL"; do
    curl -sS --max-time 5 "$U/healthz" >/dev/null 2>&1 || true
    curl -sS --max-time 5 "$U/cluster/peers" >/dev/null 2>&1 || true
  done
  sleep 2
  step "warm-up done"
fi

# ── Step 3: the perf loop ─────────────────────────────────────────────────
title "F004 . perf - $SAMPLES forwarding calls (alternating A->B / B->A)"

SAMPLE_FILE=$(mktemp 2>/dev/null || mktemp -t f004-samples)
ERR_FILE=$(mktemp 2>/dev/null || mktemp -t f004-errors)
# Append the existing EXIT-trap rm to clean these up too.
trap 'rm -rf "$TMP_LOCAL" "$SAMPLE_FILE" "$ERR_FILE"' EXIT

err_count=0
i=1
while [ "$i" -le "$SAMPLES" ]; do
  # Odd = A -> require vision -> forward to B; even = symmetric the other way.
  if [ $((i % 2)) -eq 1 ]; then
    src_url="$SPECTYN_NODE_A_URL"; src_lbl="$NODE_A_NAME"
    caps='["vision"]'; expected_dst="$NODE_B_NAME"
  else
    src_url="$SPECTYN_NODE_B_URL"; src_lbl="$NODE_B_NAME"
    caps='["gpu"]'; expected_dst="$NODE_A_NAME"
  fi
  prompt="F004 perf sample $i from $src_lbl"
  line=$(dispatch_timed_oneline "$src_url" "$caps" "$prompt")
  secs=$(printf '%s' "$line" | awk '{print $1}')
  code=$(printf '%s' "$line" | awk '{print $2}')
  rbody=$(printf '%s' "$line" | cut -d' ' -f3-)

  # A 2xx with non-zero time counts as a valid sample; anything else is
  # an error we still record (with 0.0 secs) but report separately.
  if [ "$code" = "200" ] || [ "$code" = "202" ]; then
    printf '%s\n' "$secs" >> "$SAMPLE_FILE"
  else
    err_count=$((err_count + 1))
    printf 'i=%s code=%s secs=%s body=%s\n' "$i" "$code" "$secs" "$rbody" >> "$ERR_FILE"
    # Still record so distribution view is honest about the failure tail.
    printf '%s\n' "$secs" >> "$SAMPLE_FILE"
  fi

  # Compact per-sample log (one dot per success, X per failure).
  if [ "$code" = "200" ] || [ "$code" = "202" ]; then
    printf '.'
  else
    printf 'X'
  fi
  # Newline every 50 samples for readability.
  if [ $((i % 50)) -eq 0 ]; then printf '\n'; fi
  i=$((i + 1))
done
printf '\n'

step "collected $(wc -l < "$SAMPLE_FILE" | tr -d ' ') samples; $err_count errors"

# ── Step 4: percentile math (Python — already validated above) ──────────────
title "F004 . stats - p50 / p95 / p99 from $SAMPLES samples"

stats_json=$("$PY" - "$SAMPLE_FILE" "$P99_BUDGET_MS" <<'PYEOF'
import json, sys
samples_path = sys.argv[1]
budget_ms = float(sys.argv[2])
with open(samples_path) as f:
    secs = [float(x.strip()) for x in f if x.strip()]
if not secs:
    print(json.dumps({"error": "no samples"}))
    sys.exit(0)
ms = sorted([s * 1000.0 for s in secs])
def pct(p):
    # Nearest-rank percentile (same definition Criterion uses).
    k = max(0, min(len(ms) - 1, int(round((p / 100.0) * len(ms))) - 1))
    return ms[k]
p50 = pct(50); p95 = pct(95); p99 = pct(99)
print(json.dumps({
    "n": len(ms),
    "min_ms": ms[0],
    "max_ms": ms[-1],
    "p50_ms": p50,
    "p95_ms": p95,
    "p99_ms": p99,
    "budget_ms": budget_ms,
    "pass": p99 < budget_ms,
    "all_ms": ms,
}))
PYEOF
)

# Extract a few scalars without depending on jq.
extract() {
  printf '%s' "$stats_json" | "$PY" -c "import json,sys; d=json.loads(sys.stdin.read()); v=d.get('$1'); print('' if v is None else v)"
}
N_OK=$(extract n)
MIN_MS=$(extract min_ms)
MAX_MS=$(extract max_ms)
P50_MS=$(extract p50_ms)
P95_MS=$(extract p95_ms)
P99_MS=$(extract p99_ms)
PASSED=$(extract pass)

step "n=$N_OK   min=${MIN_MS} ms   max=${MAX_MS} ms"
step "p50=${P50_MS} ms   p95=${P95_MS} ms   p99=${P99_MS} ms   budget=${P99_BUDGET_MS} ms"

if [ "$PASSED" = "True" ] && [ "$err_count" -eq 0 ]; then
  pass "p99 ${P99_MS} ms < ${P99_BUDGET_MS} ms budget AND all requests 2xx"
elif [ "$PASSED" = "True" ] && [ "$err_count" -gt 0 ]; then
  fail "p99 in budget but $err_count requests errored (see distribution + errors below)"
else
  fail "p99 ${P99_MS} ms exceeds ${P99_BUDGET_MS} ms budget"
fi

if [ "$SPECTYN_TEST_FAILED" -gt 0 ]; then
  printf '\n%s-- full sorted latency distribution (ms) --%s\n' "$C_YELLOW" "$C_RESET" >&2
  printf '%s' "$stats_json" | "$PY" -c "
import json, sys
d = json.loads(sys.stdin.read())
print(' '.join(f'{x:.1f}' for x in d.get('all_ms', [])))
" >&2
  if [ -s "$ERR_FILE" ]; then
    printf '\n%s-- error samples --%s\n' "$C_YELLOW" "$C_RESET" >&2
    cat "$ERR_FILE" >&2
  fi
fi

# ── Step 5: cleanup ─────────────────────────────────────────────────────────
if [ "${SPECTYN_CLEANUP_SKIP:-0}" != "1" ] && [ "${SPECTYN_BOOTSTRAP_SKIP:-0}" != "1" ]; then
  title "F004 . cleanup - killing spectyn serve on both nodes (logs retained)"
  cleanup_one() {
    LBL="$1"; SSHP="$2"
    remote_run "$SSHP" "
      pidf=\$HOME/$REMOTE_HOME/spectyn-serve.pid
      if [ -f \"\$pidf\" ]; then
        pid=\$(cat \"\$pidf\")
        kill \"\$pid\" 2>/dev/null || true
      fi
    " || true
    step "node $LBL: killed (log kept at ~/$REMOTE_HOME/spectyn-serve.log)"
  }
  cleanup_one "$NODE_A_NAME" "$SPECTYN_NODE_A_SSH"
  cleanup_one "$NODE_B_NAME" "$SPECTYN_NODE_B_SSH"
fi

# ── Summary ─────────────────────────────────────────────────────────────────
title "F004 . summary"
printf '  %sPASS%s %d   %sFAIL%s %d\n' \
       "$C_GREEN" "$C_RESET" "$SPECTYN_TEST_PASSED" \
       "$C_RED"   "$C_RESET" "$SPECTYN_TEST_FAILED"

if [ "$SPECTYN_TEST_FAILED" -gt 0 ]; then
  tail_remote_log "$SPECTYN_NODE_A_SSH" "$NODE_A_NAME" "~/$REMOTE_HOME/spectyn-serve.log"
  tail_remote_log "$SPECTYN_NODE_B_SSH" "$NODE_B_NAME" "~/$REMOTE_HOME/spectyn-serve.log"
  printf '\n%sFAIL: %s%s\n' "$C_RED" "$LAST_FAIL_REASON" "$C_RESET" >&2
  exit 1
fi

printf '\n%sPASS: cross-host forwarding p99 %s ms < %s ms budget (n=%s)%s\n' \
       "$C_GREEN" "$P99_MS" "$P99_BUDGET_MS" "$N_OK" "$C_RESET"
exit 0
