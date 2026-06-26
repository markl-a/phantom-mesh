#!/usr/bin/env bash
# status.sh — Stage-1 stand-in for `phantom dev status`: show pending governance
# escalations (needs-human proposals) + recent deviation-handler outcomes, so an
# owner-notified escalation isn't a silent line in a file.
#
# Usage: status.sh [--all]
set -uo pipefail
STATE_DIR="${PHANTOM_STATE_DIR:-${HOME}/.phantom-mesh}"
PROPOSALS="${STATE_DIR}/deviation-proposals.jsonl"
LEDGER="${STATE_DIR}/dev-loop-log.jsonl"
N="${1:-}"; [ "$N" = "--all" ] && LIMIT=100000 || LIMIT=10

echo "=== phantom dev status (governance) — ${STATE_DIR} ==="
if [ -s "$PROPOSALS" ]; then
  c="$(grep -c . "$PROPOSALS" 2>/dev/null || echo 0)"
  echo "🚩 ${c} needs-human proposal(s) — resolve, then: deviation-handler.sh --spec <f> --reset"
  tail -n "$LIMIT" "$PROPOSALS" | sed 's/^/   /'
else
  echo "✅ no pending needs-human proposals."
fi
echo
if [ -s "$LEDGER" ]; then
  echo "recent dev-loop outcomes (last ${LIMIT}):"
  tail -n "$LIMIT" "$LEDGER" | sed 's/^/   /'
else
  echo "(no dev-loop-log yet)"
fi
