#!/usr/bin/env bash
# SPEC-01 §9/G2 drift-reject gate: every PR must point at a capability.
#
# Closes SPEC-01 §9 ("Every PR must point at a capability slug — no slug =
# drift = reject") + [G2]. A PR body MUST contain a literal line of the form
#
#     Serves: <pillar>.<slug>
#
# where <pillar>.<slug> is one of the 23 sub-capabilities defined in
# docs/superpowers/specs/v060-deep-spec/SPEC-01-FOUNDATION-bigGoal-mapping.md §8
# (the 23-CAP taxonomy). This script greps the PR body for such a line and
# exits non-zero if it is absent or names an unknown slug.
#
# Body source (first match wins):
#   1. path given as $1                      (a file containing the PR body)
#   2. $PR_BODY_FILE env var                 (a file path)
#   3. $PR_BODY env var                      (the body text inline)
#
# Allowlist below is embedded (no network / no SPEC parse at CI time) and is
# the union of SPEC-01 §8 main body (8 new/redefined) + §8.A appendix (15
# carried-over) + §8.6 INFRA cap. Keep in sync with SPEC-01 §8 when the
# taxonomy changes (it is version-locked for v0.6.0).

set -euo pipefail

# --- 23-CAP allowlist (SPEC-01 §8) + INFRA cap ------------------------------
# P1 跨裝置 Mesh (6)
ALLOWED_SLUGS=(
  P1.peer-wire P1.cap-adv P1.mdns P1.air-gap P1.cross-os P1.durable-resume
  # P2 多模態理解 (3)
  P2.food P2.audio P2.multimodal-trait
  # P3 進化網 (5)
  P3.tiered-memory P3.provider P3.mcp P3.hermes-loop P3.skill-sync
  # P4 加密為先 (5)
  P4.identity P4.age-encrypt P4.wipe P4.byom P4.sandbox
  # X cross-pillar (3) + INFRA (1)
  X.coach X.30s-hello X.otel X.worktree X.release-infra
)

# --- locate PR body ---------------------------------------------------------
body=""
if [ "${1:-}" != "" ] && [ -f "$1" ]; then
  body="$(cat "$1")"
elif [ "${PR_BODY_FILE:-}" != "" ] && [ -f "${PR_BODY_FILE}" ]; then
  body="$(cat "${PR_BODY_FILE}")"
elif [ "${PR_BODY:-}" != "" ]; then
  body="${PR_BODY}"
else
  echo "::error::check-pr-serves: no PR body provided (pass a file as \$1, or set PR_BODY_FILE / PR_BODY)."
  echo "Add a line 'Serves: <pillar>.<slug>' to the PR description — see SPEC-01 §8 for valid slugs."
  exit 2
fi

# --- extract the Serves: line ----------------------------------------------
# Tolerate leading whitespace and optional markdown bold (**Serves:**).
# Capture everything after the colon, trim, take the first whitespace token.
serves_raw="$(printf '%s\n' "$body" \
  | grep -iE '^[[:space:]]*\*{0,2}serves\*{0,2}[[:space:]]*:' \
  | head -n1 || true)"

if [ -z "$serves_raw" ]; then
  echo "::error::check-pr-serves: no 'Serves:' line found in PR body."
  echo "SPEC-01 §9: every PR must name a capability it serves — no slug = drift = reject."
  echo "Add: Serves: <pillar>.<slug>   (valid slugs: ${ALLOWED_SLUGS[*]})"
  exit 1
fi

# Strip the 'Serves:' prefix and markdown, grab the first token.
claimed="$(printf '%s' "$serves_raw" \
  | sed -E 's/^[[:space:]]*\*{0,2}[Ss]erves\*{0,2}[[:space:]]*:[[:space:]]*//' \
  | sed -E 's/[`*]//g' \
  | awk '{print $1}')"

if [ -z "$claimed" ]; then
  echo "::error::check-pr-serves: 'Serves:' line is present but names no slug."
  echo "Found: $serves_raw"
  echo "Expected: Serves: <pillar>.<slug>   (valid slugs: ${ALLOWED_SLUGS[*]})"
  exit 1
fi

# --- validate against allowlist --------------------------------------------
for slug in "${ALLOWED_SLUGS[@]}"; do
  if [ "$claimed" = "$slug" ]; then
    echo "OK: PR serves '$claimed' (valid SPEC-01 §8 capability)."
    exit 0
  fi
done

echo "::error::check-pr-serves: 'Serves: $claimed' is not a known SPEC-01 §8 capability slug."
echo "Valid slugs (23-CAP taxonomy + INFRA): ${ALLOWED_SLUGS[*]}"
echo "Fix the slug or add the capability to SPEC-01 §8 first."
exit 1
