#!/usr/bin/env bash
# harness.sh — run phantom-test scenarios and aggregate results.
#
# Usage:
#   scripts/phantom-test/harness.sh                   # run all scenarios
#   scripts/phantom-test/harness.sh 02 04             # run scenarios whose name starts with these prefixes
#   scripts/phantom-test/harness.sh --list            # print scenario list, no execution
#
# Exit codes: 0 = all pass, 1 = any fail, 2 = no scenarios matched.
#
# Each scenario is a standalone bash script in scenarios/. The scenario:
#   - sources lib/common.sh (which sets up assert helpers + per-scenario tmp)
#   - calls scenario "<title>"
#   - calls assertion helpers (ASSERT_*); they update PHANTOM_TEST_PASSED/FAILED
#   - exits with 0/1/77 (77 = skipped; e.g. dependency missing)
#
# The runner spawns each scenario in a subshell so a scenario crash never kills
# the whole run.

set -u

HERE="$(cd "$(dirname "$0")" && pwd)"
SCENARIO_DIR="$HERE/scenarios"
LIB_DIR="$HERE/lib"

# Make lib path available to scenarios via env (so they don't need fragile
# relative-path source statements).
export PHANTOM_TEST_LIB="$LIB_DIR"

if [ "${NO_COLOR:-}" = "" ] && [ -t 1 ]; then
  C_RESET=$'\033[0m'; C_BOLD=$'\033[1m'
  C_RED=$'\033[31m'; C_GREEN=$'\033[32m'; C_YELLOW=$'\033[33m'; C_CYAN=$'\033[36m'
else
  C_RESET=''; C_BOLD=''; C_RED=''; C_GREEN=''; C_YELLOW=''; C_CYAN=''
fi

# ── Argument parsing ─────────────────────────────────────────────────────────
LIST_ONLY=0
RETRY_FAILED=0
RETRY_COOLDOWN_S="${PHANTOM_TEST_RETRY_COOLDOWN_S:-30}"
PATTERNS=()
for arg in "$@"; do
  case "$arg" in
    --list|-l) LIST_ONLY=1 ;;
    --retry-failed) RETRY_FAILED=1 ;;
    --help|-h)
      sed -n '2,15p' "$0" | sed 's/^# \{0,1\}//'
      printf '\nFlags:\n'
      printf '  --retry-failed   re-run any FAIL scenario once after a %ds cooldown\n' "$RETRY_COOLDOWN_S"
      printf '                   (helpful for free-tier LLM rate-limit flakes; the\n'
      printf '                    retried run is reported as PASS-RETRY or FAIL)\n'
      printf '  PHANTOM_TEST_RETRY_COOLDOWN_S=<sec>  override the 30s cooldown\n'
      exit 0 ;;
    *) PATTERNS+=("$arg") ;;
  esac
done

# Discover scenarios. Sort by filename so the numeric prefix orders them.
mapfile -t ALL_SCENARIOS < <(find "$SCENARIO_DIR" -maxdepth 1 -name '*.sh' -type f | sort)

# Filter by pattern (matches prefix of basename).
if [ "${#PATTERNS[@]}" -gt 0 ]; then
  FILTERED=()
  for s in "${ALL_SCENARIOS[@]}"; do
    base="$(basename "$s")"
    for p in "${PATTERNS[@]}"; do
      if [[ "$base" == "$p"* ]]; then
        FILTERED+=("$s")
        break
      fi
    done
  done
  SCENARIOS=("${FILTERED[@]}")
else
  SCENARIOS=("${ALL_SCENARIOS[@]}")
fi

if [ "${#SCENARIOS[@]}" -eq 0 ]; then
  echo "no scenarios matched" >&2
  exit 2
fi

if [ "$LIST_ONLY" -eq 1 ]; then
  for s in "${SCENARIOS[@]}"; do
    base="$(basename "$s")"
    title=$(grep -m1 -E '^scenario ' "$s" 2>/dev/null | sed -E 's/^scenario "?([^"]*)"?.*/\1/')
    printf '  %-46s %s\n' "$base" "${title:-(no title)}"
  done
  exit 0
fi

# ── Run loop ─────────────────────────────────────────────────────────────────
total=0; passed=0; failed=0; skipped=0; retried=0
RESULTS=()
declare -a FAILED_SCENARIOS=()
START_TS=$(date +%s)

for s in "${SCENARIOS[@]}"; do
  total=$((total + 1))
  base="$(basename "$s")"
  printf '\n%s┏━━ %s ━━┓%s\n' "$C_CYAN" "$base" "$C_RESET"

  # Run scenario in a subshell so failures don't affect us. Capture exit code.
  bash "$s"
  ec=$?

  case $ec in
    0)  passed=$((passed + 1));  RESULTS+=("${C_GREEN}PASS${C_RESET}  $base") ;;
    77) skipped=$((skipped + 1)); RESULTS+=("${C_YELLOW}SKIP${C_RESET}  $base") ;;
    *)  failed=$((failed + 1));  RESULTS+=("${C_RED}FAIL${C_RESET}  $base (exit $ec)")
        FAILED_SCENARIOS+=("$s") ;;
  esac
done

# ── Optional retry pass for flaky FAILs ──────────────────────────────────────
# Helps with free-tier LLM rate limiting + transient cluster RPC hiccups.
# Retry happens AFTER all scenarios have run, so the retry sees a calmer
# system. A retry that succeeds is reported as PASS-RETRY, distinct from
# first-try PASS, so reviewers can see which scenarios needed a second go.
if [ "$RETRY_FAILED" -eq 1 ] && [ "${#FAILED_SCENARIOS[@]}" -gt 0 ]; then
  printf '\n%s┄┄ retry pass: %d failed scenario(s) after %ds cooldown ┄┄%s\n' \
    "$C_YELLOW" "${#FAILED_SCENARIOS[@]}" "$RETRY_COOLDOWN_S" "$C_RESET"
  sleep "$RETRY_COOLDOWN_S"

  for s in "${FAILED_SCENARIOS[@]}"; do
    base="$(basename "$s")"
    printf '\n%s┏━━ retry: %s ━━┓%s\n' "$C_CYAN" "$base" "$C_RESET"
    bash "$s"
    ec=$?
    if [ "$ec" -eq 0 ]; then
      retried=$((retried + 1))
      passed=$((passed + 1))
      failed=$((failed - 1))
      # Replace the FAIL entry with a PASS-RETRY entry.
      for i in "${!RESULTS[@]}"; do
        case "${RESULTS[$i]}" in
          *"FAIL"*"$base"*) RESULTS[$i]="${C_YELLOW}PASS-RETRY${C_RESET}  $base" ;;
        esac
      done
    fi
  done
fi

ELAPSED=$(( $(date +%s) - START_TS ))

# ── Summary ──────────────────────────────────────────────────────────────────
printf '\n%s════════ summary ════════%s\n' "$C_BOLD" "$C_RESET"
for r in "${RESULTS[@]}"; do printf '  %s\n' "$r"; done
extra=""
if [ "$retried" -gt 0 ]; then
  extra=$(printf ' · %s%d%s passed-on-retry' "$C_YELLOW" "$retried" "$C_RESET")
fi
printf '\n  %s%d%s passed · %s%d%s failed · %s%d%s skipped%s · %ds elapsed\n\n' \
  "$C_GREEN" "$passed" "$C_RESET" \
  "$C_RED" "$failed" "$C_RESET" \
  "$C_YELLOW" "$skipped" "$C_RESET" \
  "$extra" \
  "$ELAPSED"

[ "$failed" -eq 0 ] || exit 1
exit 0
