#!/usr/bin/env bash
# scripts/demos/multi-model-compare.sh
#
# A4 use case from _planning-audit/USE-CASES (or wherever it ends up):
# Send one prompt to every node in the cluster, capture side-by-side
# responses, optionally synthesize.
#
# Usage:
#   scripts/demos/multi-model-compare.sh "your prompt"
#   scripts/demos/multi-model-compare.sh --no-synthesis "your prompt"
#   scripts/demos/multi-model-compare.sh --cost-cap 0.50 "your prompt"
#
# Implementation: thin wrapper over `spectyn swarm`, with cost guard.

set -euo pipefail

SPECTYN="${SPECTYN:-$HOME/.local/bin/spectyn}"
COST_CAP="${COST_CAP:-1.0}"  # USD; abort if estimated cost will exceed this
SYNTHESIZE=1

while [[ $# -gt 0 ]]; do
  case "$1" in
    --no-synthesis) SYNTHESIZE=0; shift ;;
    --cost-cap) COST_CAP="$2"; shift 2 ;;
    --help|-h)
      sed -n '2,/^set -e/p' "$0" | sed 's/^# \?//'
      exit 0 ;;
    --) shift; break ;;
    -*) echo "Unknown flag: $1" >&2; exit 2 ;;
    *) break ;;
  esac
done

PROMPT="${1:-}"
if [[ -z "$PROMPT" ]]; then
  echo "Usage: $0 [--no-synthesis] [--cost-cap USD] \"prompt\"" >&2
  exit 2
fi

OUT="/tmp/multi-model-compare-$(date +%s).txt"
echo "Prompt: $PROMPT"
echo "Cost cap: \$$COST_CAP USD"
echo "Synthesis: $([ $SYNTHESIZE -eq 1 ] && echo on || echo off)"
echo "─────────────────────────────────────────"
echo ""

if [[ $SYNTHESIZE -eq 1 ]]; then
  "$SPECTYN" swarm "$PROMPT" | tee "$OUT"
else
  # raw mode — strip the synthesis section
  "$SPECTYN" swarm "$PROMPT" | awk '/^── Synthesis ──$/{exit}1' | tee "$OUT"
fi

# Cost guard (post-hoc — spectyn swarm doesn't expose pre-flight cost)
COST=$(grep -oE 'cost: \$[0-9]+\.[0-9]+' "$OUT" | grep -oE '[0-9]+\.[0-9]+' | tail -1)
if [[ -n "$COST" ]]; then
  if (( $(echo "$COST > $COST_CAP" | bc -l) )); then
    echo "" >&2
    echo "⚠️  WARNING: cost \$$COST exceeded cap \$$COST_CAP" >&2
    echo "    See _planning-audit/13-DISPATCH-DEBT.md for cost-routing issues" >&2
  fi
fi

echo ""
echo "Saved to: $OUT"
