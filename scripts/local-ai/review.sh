#!/usr/bin/env bash
# review.sh — LOCAL >=2-AI review gate (EXECUTION-PLAN M3, reframed to the local model).
#
# Reviews a git diff with >=2 DIFFERENT local AI CLIs (via the local-ai skill's ask.sh)
# and requires consensus. No SSH — the reviewers run on THIS machine, so there is no
# transport layer (and none of the 4KB-stdin / PowerShell-quoting pitfalls the
# cross-machine scripts/dev-cluster/review.sh has).
#
# B2 (reviewer selection): the AUTHOR is this Claude Code (`claude`), so claude is
#   excluded — reviewers are the machine's OTHER installed AI CLIs (codex/opencode/agy);
#   at least 2 DIFFERENT ones are required.
# B3 (consensus, owner-set 2026-06-09): UNANIMITY — every selected reviewer must APPROVE
#   — WITH a ROUND-CAP: after REVIEW_MAX_ROUNDS non-passing reviews of the same branch it
#   escalates to NEEDS-HUMAN instead of looping forever (the strict-unanimity gate was
#   found not to converge against a maximally-strict reviewer; governance R5 consensus +
#   R4 stuck -> halt + notify). A passing review resets the branch's round counter.
#
# Usage:
#   review.sh [GIT_RANGE]                  default HEAD~1..HEAD
#   review.sh --staged                     review staged changes
#   REVIEWERS="codex opencode" review.sh   force the reviewer tools (space-separated)
#   review.sh --reset                      clear this branch's round counter, then exit
#
# Exit: 0 = consensus APPROVE; 1 = CHANGES REQUESTED (under the cap);
#       2 = NEEDS-HUMAN (round-cap hit); 3 = setup error (no diff / <2 reviewers).

set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ASK="${LOCAL_AI_ASK:-${HERE}/../../.claude/skills/local-ai/ask.sh}"
MAX_ROUNDS="${REVIEW_MAX_ROUNDS:-3}"
MAX_LINES="${REVIEW_MAX_LINES:-800}"
STATE_DIR="${HOME}/.phantom-mesh"

[ -x "$ASK" ] || { echo "review: local-ai ask.sh not found/executable at $ASK" >&2; exit 3; }

branch="$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo detached)"
counter="${STATE_DIR}/review-rounds-$(printf '%s' "$branch" | tr '/ ' '__').count"

if [ "${1:-}" = "--reset" ]; then
  rm -f "$counter"; echo "review: round counter reset for branch '$branch'"; exit 0
fi

# ── the diff ─────────────────────────────────────────────────────────────────
RANGE="${1:-HEAD~1..HEAD}"
if [ "$RANGE" = "--staged" ]; then DIFF="$(git diff --staged)"; RANGE="(staged)"
else DIFF="$(git diff "$RANGE" 2>/dev/null)"; fi
[ -n "$DIFF" ] || { echo "review: empty diff for '$RANGE' — nothing to review" >&2; exit 3; }
TOTAL=$(printf '%s\n' "$DIFF" | wc -l | tr -d ' '); NOTE=""
if [ "$TOTAL" -gt "$MAX_LINES" ]; then
  DIFF="$(printf '%s\n' "$DIFF" | head -n "$MAX_LINES")"; NOTE=" (truncated to first ${MAX_LINES}/${TOTAL} lines)"
fi

# ── reviewers (B2): >=2 different non-author tools that are installed ─────────
if [ -n "${REVIEWERS:-}" ]; then read -r -a POOL <<< "$REVIEWERS"
else POOL=(); for t in codex opencode agy; do command -v "$t" >/dev/null 2>&1 && POOL+=("$t"); done; fi
# de-dupe + drop the author (claude)
REVIEWERS_USE=()
for t in "${POOL[@]}"; do
  [ "$t" = "claude" ] && continue
  skip=0; for u in "${REVIEWERS_USE[@]:-}"; do [ "$u" = "$t" ] && skip=1; done
  [ "$skip" = 0 ] && REVIEWERS_USE+=("$t")
done
if [ "${#REVIEWERS_USE[@]}" -lt 2 ]; then
  echo "review: need >=2 different local reviewer AIs (have: ${REVIEWERS_USE[*]:-none}). Install/enable another (codex/opencode/agy)." >&2
  exit 3
fi

PROMPT="You are a strict, senior code reviewer. Review the git diff below for CORRECTNESS
bugs, regressions, and broken edge cases that a green build could still hide. Be specific:
name the file and the exact problem. Skip style nits. If it is solid, say so in one line.
End your reply with EXACTLY one final line, nothing after it:
  VERDICT: APPROVE          (if you'd let this merge)
  VERDICT: REQUEST_CHANGES  (if anything above needs fixing first)

--- git diff ${RANGE}${NOTE} ---
${DIFF}"

verdict_of() { printf '%s\n' "$1" | grep -oaE 'VERDICT: (APPROVE|REQUEST_CHANGES)' | tail -1 | awk '{print $2}'; }

echo "=== local review gate: ${RANGE}${NOTE} ==="
echo "  author    : claude (this Claude Code — excluded, no self-review)"
echo "  reviewers : ${REVIEWERS_USE[*]}  (consensus = ALL must APPROVE; round-cap ${MAX_ROUNDS})"
echo

approvals=0; changes=0; voters=0; verdicts=()
for t in "${REVIEWERS_USE[@]}"; do
  out="$("$ASK" "$t" "$PROMPT" 2>&1)"
  v="$(verdict_of "$out")"
  echo "────────── ${t} ──────────"
  printf '%s\n' "$out" | tail -n 25
  echo
  verdicts+=("${t}:${v:-<no verdict / abstain>}")
  case "$v" in
    APPROVE)         approvals=$((approvals+1)); voters=$((voters+1)) ;;
    REQUEST_CHANGES) changes=$((changes+1));     voters=$((voters+1)) ;;
    *) ;;  # no parseable verdict = abstain (flaky tool / non-response); reported, not blocking
  esac
done
abstains=$(( ${#REVIEWERS_USE[@]} - voters ))

echo "=== consensus ==="
for vv in "${verdicts[@]}"; do echo "  $vv"; done
echo "  (voted: ${voters}, approve: ${approvals}, request-changes: ${changes}, abstain: ${abstains})"

# Consensus = UNANIMITY among reviewers that actually voted, with a quorum of >=2
# real verdicts. An abstain (no parseable verdict, e.g. a flaky free model that
# returned nothing) is reported but does NOT block — otherwise one unreliable tool
# would deadlock the gate forever. A REQUEST_CHANGES from anyone still blocks.
if [ "$voters" -ge 2 ] && [ "$changes" -eq 0 ] && [ "$approvals" -eq "$voters" ]; then
  rm -f "$counter"
  echo "  RESULT: ✅ APPROVED — all ${voters} voting reviewers APPROVE (${abstains} abstained)"
  exit 0
fi
if [ "$voters" -lt 2 ]; then
  echo "  RESULT: ⚠️ INSUFFICIENT — only ${voters} reviewer(s) returned a verdict (need >=2)."
  # fall through to round-cap so repeated non-responses still escalate
fi

# not passing → advance the round counter and decide cap
round=$(( $( [ -f "$counter" ] && cat "$counter" 2>/dev/null || echo 0 ) + 1 ))
mkdir -p "$STATE_DIR"; printf '%s' "$round" > "$counter"
if [ "$round" -ge "$MAX_ROUNDS" ]; then
  echo "  RESULT: 🚩 NEEDS-HUMAN — ${round}/${MAX_ROUNDS} rounds without consensus on '$branch'."
  echo "          (round-cap reached; stop auto-iterating and bring in a person — R4 halt+notify.)"
  exit 2
else
  echo "  RESULT: ⚠️ CHANGES REQUESTED (round ${round}/${MAX_ROUNDS} on '$branch') — fix and re-run."
  exit 1
fi
