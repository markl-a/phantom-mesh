#!/usr/bin/env bash
# Continuous test loop for phantom-mesh (mac-m1 session, started 2026-05-01)
#
# Modes (set via INTERVAL_SECS env var, default 1800 = 30-min "C-mode"):
#   A — passive            (don't run this loop; user pings on demand)
#   B — fast-poll demo day  INTERVAL_SECS=300   # 5 min — for 5/8 / 5/9
#   C — default freeze week INTERVAL_SECS=1800  # 30 min — for 5/2 → 5/7
#
# Each iteration (mode B and C):
#   - Checks Mac daemon health + core_sha drift vs HEAD
#   - Validates /rpc/ping schema (wire_version, agents, worker_caps)
#   - Detects new commits from other sessions (node-a / Oracle / iOS / Android)
#   - Detects peer online state change (Phase 2/3 unlock signal)
#   - Verifies peer list timeout-safe (≤8s) every 4th round only
#
# Output rules:
#   - Each line on stdout becomes a notification.
#   - Silent rounds = nothing changed; no notification.
#   - Anomalies / state-changes only.
#
# Skipped per cost / scope:
#   - cargo test --lib (~60s + parallel-test flake noise; user runs manually)
#   - /rpc/squad/dispatch happy path (costs real LLM API credits each round)
#   - browser /term render check (needs Vite preview server)
#
# Stop: TaskStop on the Monitor task ID.

set -u

REPO="${REPO:-$HOME/path/to/phantom-mesh}"
DAEMON_URL=http://127.0.0.1:7878
SECRET=$(grep -E "^\s*cluster_secret\s*=" "$HOME/.phantom-mesh/agents.toml" 2>/dev/null \
  | sed 's/.*= *"//; s/"$//' || echo "")

ROUND=0
LAST_COMMITS_HASH=""
LAST_PEERS_ONLINE=""
LAST_DAEMON_SHA=""

cd "$REPO" || exit 1

while true; do
  ROUND=$((ROUND + 1))
  TS=$(date '+%H:%M:%S')

  # 1. Daemon health + SHA drift
  PING=$(curl -sfm 3 "$DAEMON_URL/rpc/ping" 2>/dev/null || echo "")
  if [ -z "$PING" ]; then
    echo "[$TS r${ROUND}] DAEMON DOWN — /rpc/ping unreachable. Run: launchctl kickstart -k gui/\$UID/ai.phantommesh.serve"
  else
    DAEMON_SHA=$(echo "$PING" | sed -n 's/.*"core_sha":"\([^"]*\)".*/\1/p')
    HEAD_SHA=$(git rev-parse --short=10 HEAD 2>/dev/null)

    # SHA drift detection — emit only on TRANSITION (avoid spam if drift persists)
    if [ "$DAEMON_SHA" != "$HEAD_SHA" ] && [ "$DAEMON_SHA" != "$LAST_DAEMON_SHA" ]; then
      echo "[$TS r${ROUND}] DAEMON STALE — daemon=$DAEMON_SHA HEAD=$HEAD_SHA. Run scripts/build-mac.sh + redeploy."
    fi
    LAST_DAEMON_SHA="$DAEMON_SHA"

    # /rpc/ping schema sanity (Squad-a + wire_version)
    SCHEMA_ERR=""
    echo "$PING" | grep -q '"wire_version":1' || SCHEMA_ERR="$SCHEMA_ERR wire_version"
    echo "$PING" | grep -q '"agents":' || SCHEMA_ERR="$SCHEMA_ERR agents"
    echo "$PING" | grep -q '"worker_caps":' || SCHEMA_ERR="$SCHEMA_ERR worker_caps"
    if [ -n "$SCHEMA_ERR" ]; then
      echo "[$TS r${ROUND}] PING SCHEMA REGRESSION — missing:$SCHEMA_ERR"
    fi
  fi

  # 2. New commits from other sessions
  git fetch --all --quiet 2>/dev/null || true
  CURR_COMMITS=$(git log --since='24 hours ago' --pretty=format:'%H' \
    origin/feat/windows origin/feat/android origin/platform/linux \
    origin/platform/ios origin/phase1-r1-foundations origin/docs/android-onboarding \
    2>/dev/null | sort -u | md5)
  if [ -n "$LAST_COMMITS_HASH" ] && [ "$CURR_COMMITS" != "$LAST_COMMITS_HASH" ]; then
    NEW=$(git log --since='30 minutes ago' --oneline \
      origin/feat/windows origin/feat/android origin/platform/linux \
      origin/platform/ios origin/phase1-r1-foundations origin/docs/android-onboarding \
      2>/dev/null | sort -u | head -5)
    if [ -n "$NEW" ]; then
      echo "[$TS r${ROUND}] NEW COMMITS from other sessions:"
      echo "$NEW" | sed 's/^/    /'
    fi
  fi
  LAST_COMMITS_HASH="$CURR_COMMITS"

  # 3. Peer online state change (unlocks Phase 2/3)
  PEERS_ONLINE=$(curl -sfm 5 "$DAEMON_URL/rpc/peers" 2>/dev/null \
    | grep -o '"online":true' | wc -l | tr -d ' ')
  PEERS_ONLINE=${PEERS_ONLINE:-0}
  if [ -n "$LAST_PEERS_ONLINE" ] && [ "$PEERS_ONLINE" != "$LAST_PEERS_ONLINE" ]; then
    if [ "$PEERS_ONLINE" -gt "$LAST_PEERS_ONLINE" ]; then
      echo "[$TS r${ROUND}] PEER UP — $LAST_PEERS_ONLINE → $PEERS_ONLINE online (Phase 2/3 may be testable)"
    else
      echo "[$TS r${ROUND}] PEER DOWN — $LAST_PEERS_ONLINE → $PEERS_ONLINE online"
    fi
  fi
  LAST_PEERS_ONLINE="$PEERS_ONLINE"

  # 4. peer list timeout-safe (Bug #23 regression check, every 4th round = ~2h)
  if [ $((ROUND % 4)) -eq 0 ]; then
    START=$(date +%s)
    timeout 8 "$HOME/.cargo/bin/phantom" peer list >/dev/null 2>&1 || true
    ELAPSED=$(( $(date +%s) - START ))
    if [ "$ELAPSED" -gt 8 ]; then
      echo "[$TS r${ROUND}] PEER LIST SLOW — ${ELAPSED}s (Bug #23 regression?)"
    fi
  fi

  sleep "${INTERVAL_SECS:-1800}"
done
