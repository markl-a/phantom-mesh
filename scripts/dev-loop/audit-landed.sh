#!/usr/bin/env bash
# audit-landed.sh — component E: conductor's adversarial sample-audit of the
# commits node dev-loops landed since $SINCE. Each sampled commit's diff goes to
# TWO distinct vendors via ask.sh; a CHANGES verdict FLAGS it for the operator
# (this script never reverts — flagging only, per the reversible-envelope policy).
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
SINCE="${1:?usage: audit-landed.sh <since-ref>}"
SAMPLE="${AUDIT_SAMPLE:-3}"

mapfile -t commits < <(git -C "$ROOT" log --no-merges --format='%H' "$SINCE"..HEAD | head -n "$SAMPLE")
[ "${#commits[@]}" -gt 0 ] || { echo "audit-landed: nothing new since $SINCE"; exit 0; }
flagged=0
for c in "${commits[@]}"; do
  diff="$(git -C "$ROOT" show "$c" | head -c 12000)"
  prompt="Adversarially review this LANDED commit for a real correctness/security regression. End with VERDICT: LGTM or VERDICT: CHANGES: <why>.

$diff"
  # Capture the FULL reviewer output (no tail truncation — a verdict line scrolling
  # out of a fixed window would silently miss a CHANGES = fail-open).
  v1="$(bash "$ROOT/.claude/skills/local-ai/ask.sh" agy "$prompt" 2>/dev/null)"
  v2="$(bash "$ROOT/.claude/skills/local-ai/ask.sh" codex "$prompt" 2>/dev/null)"
  both="$v1
$v2"
  # Fail CLOSED: any CHANGES flags; otherwise an explicit LGTM is REQUIRED to pass.
  # A dead/empty/quota'd reviewer (no usable verdict from either) is inconclusive,
  # which FLAGS for the operator rather than reading as a silent pass.
  if echo "$both" | grep -qiE 'VERDICT:[[:space:]]*CHANGES'; then
    echo "AUDIT-FLAG $c — a reviewer requested changes (parked for operator)"
    flagged=$((flagged+1))
  elif echo "$both" | grep -qiE 'VERDICT:[[:space:]]*LGTM'; then
    echo "audit ok: $c"
  else
    echo "AUDIT-FLAG $c — inconclusive (no usable verdict from either reviewer; parked for operator)"
    flagged=$((flagged+1))
  fi
done
echo "audit-landed: $flagged/${#commits[@]} sampled commit(s) flagged"
