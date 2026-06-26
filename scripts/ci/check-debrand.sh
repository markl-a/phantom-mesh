#!/usr/bin/env bash
# check-debrand.sh — fail-closed gate for the OSS de-branding effort.
#
# Asserts that no NEW "hermes"/"openclaw" brand residue creeps into the
# shipping code tree. Every currently-known residue is enumerated in
# scripts/ci/debrand-allowlist.txt with the phase that will remove it
# (or why it is permanently allowed). Any hit NOT covered by the allowlist
# fails the gate (exit 1).
#
# GOAL: docs/superpowers/GOAL-debrand-oss.md (D2/D7). The allowlist shrinks
# as Phase 2 (backend) and Phase 3 (docs) land; the end state is: only DB
# table names + migration filenames remain.
#
# Usage:
#   scripts/ci/check-debrand.sh          # gate mode: exit 1 on un-allowlisted hits
#   scripts/ci/check-debrand.sh --self   # report mode: print counts, always exit 0
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 2

ALLOWLIST="scripts/ci/debrand-allowlist.txt"
# Shipping code tree. Docs (docs/**) are scanned once Phase 3 lands; for now
# the gate guards code only (core/ crates/ app/), which is what compiles into
# the product + bindings.
SCAN_PATHS=(core crates app)
SCAN_GLOBS=(--glob '!**/target/**' --glob '!**/build-target-*/**' --glob '!**/node_modules/**')

if [[ ! -f "$ALLOWLIST" ]]; then
  echo "check-debrand: missing allowlist $ALLOWLIST" >&2
  exit 2
fi

# All brand hits as path:line:text (rg already excludes binary/.git).
ALL_HITS="$(rg -ni 'hermes|openclaw' "${SCAN_PATHS[@]}" "${SCAN_GLOBS[@]}" 2>/dev/null)"

# Drop every line matched by an allowlist regex (regexes match the whole
# "path:line:text" line, so both path-prefix and content patterns work).
# Comments (#...) and blank lines in the allowlist are stripped first.
# NB: use a real temp file, NOT process substitution <(...) — the latter is
# broken under MSYS/Git-Bash and would make rg fail silently (false PASS).
PATTERNS_FILE="$(mktemp)"
trap 'rm -f "$PATTERNS_FILE"' EXIT
grep -vE '^[[:space:]]*(#|$)' "$ALLOWLIST" > "$PATTERNS_FILE"
VIOLATIONS="$(printf '%s\n' "$ALL_HITS" | rg -v -f "$PATTERNS_FILE")"

total_hits="$(printf '%s' "$ALL_HITS" | grep -c . || true)"
violation_count="$(printf '%s' "$VIOLATIONS" | grep -c . || true)"

if [[ "${1:-}" == "--self" ]]; then
  echo "check-debrand --self: $total_hits total brand hits, $violation_count un-allowlisted."
  printf '%s\n' "$VIOLATIONS" | grep . || echo "(no un-allowlisted hits)"
  exit 0
fi

if [[ "$violation_count" -gt 0 ]]; then
  echo "check-debrand: FAIL — $violation_count brand hit(s) not covered by $ALLOWLIST:" >&2
  printf '%s\n' "$VIOLATIONS" >&2
  echo "" >&2
  echo "If this is a legitimate deferred/immutable identifier, add a pattern to $ALLOWLIST with a phase note. Otherwise debrand it." >&2
  exit 1
fi

echo "check-debrand: PASS — all $total_hits brand hits are allowlisted (deferred/immutable)."
exit 0
