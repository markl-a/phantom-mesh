#!/usr/bin/env bash
# node-dev-loop.sh — component C: one node's claim->build->push loop.
# Pulls caps-matched specs from the shared backlog, runs the crew executor in an
# isolated worktree, and (on a clean on-node gate) the fleet conductor lands it
# (Main tier = staged for operator; satellites push). NOTE: direct-push-to-main is
# intentionally NOT done here (main is operator-gated) — see C1 open items.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
PHANTOM="${PHANTOM_BIN:-phantom}"
# Keep a standalone/local run consistent with the develop flow (default main =
# the landing target); goal-develop.sh exports this, but set it here too so a
# bare `node-dev-loop.sh` invocation also coordinates on one base.
export BACKLOG_BASE="${BACKLOG_BASE:-main}"
MAX="${NODE_DEV_MAX_TASKS:-3}"   # tasks per invocation (a /goal round bounds this)

done_n=0
while [ "$done_n" -lt "$MAX" ]; do
  id="$(bash "$HERE/backlog.sh" next || true)"      # caps-matched, this node
  [ -n "$id" ] || { echo "node-dev-loop: backlog empty for this node"; break; }
  bash "$HERE/backlog.sh" claim "$id" || { echo "claim race on $id, skip"; continue; }
  echo "node-dev-loop: claimed $id"
  if "$PHANTOM" fleet run --executor crew --once "$id" 2>&1; then
    bash "$HERE/backlog.sh" done "$id"
    done_n=$((done_n+1))
  else
    bash "$HERE/backlog.sh" release "$id"   # let another node retry; never fake done
    echo "node-dev-loop: $id escalated/failed, released"
  fi
done
echo "node-dev-loop: completed $done_n task(s) on $(hostname -s 2>/dev/null || hostname)"
