#!/usr/bin/env bash
# goal-develop.sh — component F: one D-stage round across the fleet.
#   1. plan    — FEATURE-MATRIX -> shared backlog
#   2. fan out — each node runs its claim->crew->land loop (in parallel)
#   3. harvest — adversarial sample-audit of what landed this round
# Max-auto in a reversible envelope: Main-tier landing STAGES for the operator
# (no direct main push); the audit only FLAGS, never force-reverts. Run repeatedly
# (the /goal develop skill loops) until the matrix is green.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
# The develop flow's coordination base — specs, claims, dev branches, and the
# harvest fetch ALL use this one branch (default main = the landing/integration
# target). Override with BACKLOG_BASE=<branch> for a dedicated dev base.
export BACKLOG_BASE="${BACKLOG_BASE:-main}"
NODES_FILE="${PHANTOM_FLEET_NODES:-$HOME/.phantom-mesh/fleet.nodes}"
START_REF="$(git -C "$ROOT" rev-parse HEAD)"

echo "== goal-develop round: plan =="
bash "$HERE/matrix-to-backlog.sh"

echo "== goal-develop round: fan out node loops =="
if [ ! -f "$NODES_FILE" ]; then
  echo "goal-develop: no fleet node list at $NODES_FILE — running the LOCAL node only"
  bash "$HERE/node-dev-loop.sh" || true
else
  pids=()
  while read -r label target shell caps role; do
    case "$label" in ''|\#*) continue;; esac
    if [ "$target" = "local" ]; then
      ( bash "$HERE/node-dev-loop.sh" ) & pids+=("$!")
    else
      # Pipe the node loop over SSH per docs/mesh/FLEET-SSH.md (login shell).
      # Forward the LOCAL BACKLOG_BASE into the remote command (env doesn't cross
      # SSH by default) so every node coordinates on the same base. Outer
      # double-quotes expand $BACKLOG_BASE here; inner single-quotes are the
      # remote bash -lc arg (a branch name has no shell-hostile chars).
      ( ssh -o ConnectTimeout=20 "$target" \
          "bash -lc 'cd ~/Projects/phantom-mesh && BACKLOG_BASE=$BACKLOG_BASE bash scripts/dev-loop/node-dev-loop.sh'" \
        ) & pids+=("$!")
    fi
  done < "$NODES_FILE"
  for p in "${pids[@]:-}"; do [ -n "$p" ] && wait "$p" || true; done   # a node failing never aborts the round
fi

echo "== goal-develop round: harvest + audit =="
git -C "$ROOT" fetch origin "$BACKLOG_BASE" --quiet || true
bash "$HERE/audit-landed.sh" "$START_REF" || true

echo "== goal-develop round: review branches pushed this round =="
git -C "$ROOT" fetch origin "+refs/heads/fleet/*:refs/remotes/origin/fleet/*" --prune --quiet 2>/dev/null || true
git -C "$ROOT" for-each-ref --format='  %(refname:short)' refs/remotes/origin/fleet/ 2>/dev/null \
  | sed 's#origin/##' || true
echo "  (review + merge these to $BACKLOG_BASE yourself — main is operator-gated)"

echo "== goal-develop round: done (started at $START_REF) =="
