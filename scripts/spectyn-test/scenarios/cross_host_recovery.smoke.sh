#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# cross_host_recovery.smoke.sh — local-loopback smoke for the F003 scenario.
#
# Spins up two `spectyn serve` processes on 127.0.0.1:7878 and 127.0.0.1:7879,
# wires the F003 env-var contract to point at them, and re-uses the SAME
# cross_host_recovery.sh script with a local-exec shim as the SSH stand-in
# and `cp` as the SCP stand-in.
#
# Goal: validate that the F003 bash plumbing (env contract, peer-online
# polling, kill/restart cycle, cleanup) is correct independently of a real
# testbed, so that operator-side failures are real failures and not script
# bugs.
#
# `--quick` is the default for smoke runs — it sets HB_INTERVAL=5 +
# HB_THRESHOLD=2 (deadline ~12s) so the full Healthy→Unhealthy→Healthy
# cycle finishes well under a minute on localhost.
#
# Usage:
#   bash scripts/spectyn-test/scenarios/cross_host_recovery.smoke.sh --quick
#
# Env knobs:
#   SPECTYN_BIN_LOCAL    — path to spectyn binary (default: target/release
#                          or PATH). MUST be built with --features
#                          experimental-cluster-heartbeat.
#   SPECTYN_SMOKE_KEEP=1 — don't delete the scratch dir on exit
#
# Exit codes mirror cross_host_recovery.sh (0 / 1 / 77).
# ─────────────────────────────────────────────────────────────────────────────

set -u

HERE="$(cd "$(dirname "$0")" && pwd)"
SCRIPT="$HERE/cross_host_recovery.sh"

if [ ! -f "$SCRIPT" ]; then
  echo "smoke: cannot find $SCRIPT" >&2
  exit 2
fi

# --help short-circuit (delegates to the main script's helper text).
case "${1:-}" in
  -h|--help)
    exec bash "$SCRIPT" --help
    ;;
esac

# Pick the spectyn binary to test.
if [ -n "${SPECTYN_BIN_LOCAL:-}" ] && [ -x "$SPECTYN_BIN_LOCAL" ]; then
  BIN="$SPECTYN_BIN_LOCAL"
elif [ -x "${CARGO_TARGET_DIR:-target}/release/spectyn.exe" ]; then
  BIN="${CARGO_TARGET_DIR:-target}/release/spectyn.exe"
elif [ -x "${CARGO_TARGET_DIR:-target}/release/spectyn" ]; then
  BIN="${CARGO_TARGET_DIR:-target}/release/spectyn"
elif [ -x "core/target/release/spectyn.exe" ]; then
  BIN="core/target/release/spectyn.exe"
elif [ -x "core/target/release/spectyn" ]; then
  BIN="core/target/release/spectyn"
elif command -v spectyn >/dev/null 2>&1; then
  BIN="$(command -v spectyn)"
else
  echo "smoke: no spectyn binary found — set SPECTYN_BIN_LOCAL=path/to/spectyn" >&2
  echo "       (must be built with --features experimental-cluster-heartbeat)" >&2
  exit 77
fi

echo "smoke: using binary $BIN"

# Per-run sandboxed home dirs.
TMP=$(mktemp -d 2>/dev/null || mktemp -d -t f003-smoke)
A_HOME="$TMP/node-a"
B_HOME="$TMP/node-b"
mkdir -p "$A_HOME" "$B_HOME"
echo "smoke: scratch dir $TMP"

cleanup() {
  REMOTE_HOME_NAME=".spectyn-mesh-e001-smoke"
  for HD in "$A_HOME" "$B_HOME"; do
    pidf="$HD/$REMOTE_HOME_NAME/spectyn-serve.pid"
    if [ -f "$pidf" ]; then
      pid=$(cat "$pidf" 2>/dev/null || true)
      if [ -n "$pid" ]; then
        if command -v taskkill.exe >/dev/null 2>&1; then
          taskkill.exe //F //PID "$pid" //T >/dev/null 2>&1 || true
        else
          kill "$pid" 2>/dev/null || true
          sleep 1
          kill -9 "$pid" 2>/dev/null || true
        fi
      fi
    fi
  done
  if [ "${SPECTYN_SMOKE_KEEP:-0}" = "1" ]; then
    echo "smoke: SPECTYN_SMOKE_KEEP=1 — leaving $TMP for inspection"
  else
    rm -rf "$TMP" 2>/dev/null || true
  fi
}
trap cleanup EXIT

# SSH-stand-in wrappers (same pattern as F002 smoke).
WRAP_A="$TMP/wrap-a.sh"
WRAP_B="$TMP/wrap-b.sh"

cat > "$WRAP_A" <<EOF_WRAPPER_A
#!/usr/bin/env bash
export HOME="$A_HOME"
mkdir -p "\$HOME"
if [ "\$1" = "sh" ] && [ "\$2" = "-c" ]; then
  shift 2
  exec sh -c "\$*"
fi
exec "\$@"
EOF_WRAPPER_A

cat > "$WRAP_B" <<EOF_WRAPPER_B
#!/usr/bin/env bash
export HOME="$B_HOME"
mkdir -p "\$HOME"
if [ "\$1" = "sh" ] && [ "\$2" = "-c" ]; then
  shift 2
  exec sh -c "\$*"
fi
exec "\$@"
EOF_WRAPPER_B
chmod +x "$WRAP_A" "$WRAP_B"

# Wire F003 env contract to local loopback.
export SPECTYN_NODE_A_SSH="$WRAP_A"
export SPECTYN_NODE_B_SSH="$WRAP_B"
export SPECTYN_NODE_A_URL="http://127.0.0.1:7878"
export SPECTYN_NODE_B_URL="http://127.0.0.1:7879"
export SPECTYN_NODE_A_PORT=7878
export SPECTYN_NODE_B_PORT=7879
export SPECTYN_NODE_A_NAME="node-a-smoke"
export SPECTYN_NODE_B_NAME="node-b-smoke"
export SPECTYN_CLUSTER_SECRET="f003-smoke-secret"
export SPECTYN_BIN_LOCAL="$BIN"
export SPECTYN_REMOTE_HOME=".spectyn-mesh-e001-smoke"
export SPECTYN_SCP="cp"
export SPECTYN_HEALTHZ_TIMEOUT_S="${SPECTYN_HEALTHZ_TIMEOUT_S:-25}"
export SPECTYN_SMOKE_MODE=1

echo "smoke: A=$SPECTYN_NODE_A_URL HOME=$A_HOME"
echo "smoke: B=$SPECTYN_NODE_B_URL HOME=$B_HOME"

# Forward any extra args (notably --quick) to the main scenario script.
bash "$SCRIPT" "$@"
ec=$?
echo "smoke: cross_host_recovery.sh exit=$ec"
exit $ec
