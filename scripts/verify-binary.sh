#!/usr/bin/env bash
# verify-binary.sh — health-check a phantom binary
#
# Usage:
#   ./scripts/verify-binary.sh <binary-path> [options]
#
# Options:
#   --expect-version <semver>   fail if phantom --version --short != <semver>
#   --quick                     skip phantom doctor (just version + exists)
#   --full                      include phantom selftest --p0-only (needs LLM key)
#   --json                      machine-readable JSON output
#   -v | --verbose              print each check's full output
#   -h | --help                 this help
#
# Exit codes:
#   0   all checks passed
#   1   one or more checks failed
#   2   argument / usage error

set -u
SCRIPT_NAME="verify-binary.sh"
SCRIPT_VERSION="0.1.0"

BINARY=""
EXPECT_VERSION=""
QUICK=false
FULL=false
JSON_OUT=false
VERBOSE=false
START_TIME=$(date +%s)

print_help() {
  sed -n '1,/^$/p' "$0" | sed 's/^# \{0,1\}//'
  exit 0
}

# parse args
while [[ $# -gt 0 ]]; do
  case "$1" in
    --expect-version) EXPECT_VERSION="${2:-}"; shift 2 ;;
    --quick)          QUICK=true; shift ;;
    --full)           FULL=true; shift ;;
    --json)           JSON_OUT=true; shift ;;
    -v|--verbose)     VERBOSE=true; shift ;;
    -h|--help)        print_help ;;
    -*)               echo "unknown option: $1" >&2; exit 2 ;;
    *)                BINARY="$1"; shift ;;
  esac
done

if [[ -z "$BINARY" ]]; then
  echo "usage: $SCRIPT_NAME <binary-path> [options]" >&2
  echo "       run with --help for details" >&2
  exit 2
fi

# results accumulator
declare -a CHECK_NAMES=()
declare -a CHECK_STATUS=()  # pass / fail / skip
declare -a CHECK_DETAILS=()
EXIT_CODE=0

record() {
  local name="$1"
  local status="$2"
  local detail="$3"
  CHECK_NAMES+=("$name")
  CHECK_STATUS+=("$status")
  CHECK_DETAILS+=("$detail")
  if [[ "$status" == "fail" ]]; then EXIT_CODE=1; fi
}

# Check 1: binary exists + executable
if [[ -f "$BINARY" ]]; then
  if [[ -x "$BINARY" ]]; then
    record "binary_exists_executable" "pass" "$BINARY"
  else
    record "binary_exists_executable" "fail" "$BINARY exists but not executable"
  fi
else
  record "binary_exists_executable" "fail" "$BINARY does not exist"
  # short-circuit: nothing else can run
  EXIT_CODE=1
fi

# helper: run a command, capture exit + output (only continues if Check 1 passed)
run_phantom() {
  local args="$1"
  local outvar="$2"
  local exitvar="$3"
  if [[ "${CHECK_STATUS[0]:-fail}" == "fail" ]]; then
    eval "$exitvar=127"
    eval "$outvar=''"
    return
  fi
  local _out
  local _exit
  _out=$("$BINARY" $args 2>&1) || _exit=$?
  _exit="${_exit:-0}"
  eval "$outvar=\"\$_out\""
  eval "$exitvar=$_exit"
}

# Check 2: phantom --version
if [[ "${CHECK_STATUS[0]:-fail}" == "pass" ]]; then
  run_phantom "--version" VERSION_OUT VERSION_EXIT
  if [[ "$VERSION_EXIT" -eq 0 ]]; then
    record "version_runs" "pass" "$(echo "$VERSION_OUT" | head -1)"
  else
    record "version_runs" "fail" "exit=$VERSION_EXIT output=$VERSION_OUT"
  fi
fi

# Check 3: phantom --version --short matches SemVer
if [[ "${CHECK_STATUS[1]:-fail}" == "pass" ]]; then
  run_phantom "--version --short" SHORT_OUT SHORT_EXIT
  SHORT_VER=$(echo "$SHORT_OUT" | tr -d '[:space:]')
  if [[ "$SHORT_EXIT" -eq 0 && "$SHORT_VER" =~ ^[0-9]+\.[0-9]+\.[0-9]+ ]]; then
    record "version_short_semver" "pass" "$SHORT_VER"
  else
    record "version_short_semver" "fail" "got: '$SHORT_VER' (exit $SHORT_EXIT)"
  fi

  # Check 4: expect-version match (optional)
  if [[ -n "$EXPECT_VERSION" ]]; then
    if [[ "$SHORT_VER" == "$EXPECT_VERSION" ]]; then
      record "version_match_expected" "pass" "$SHORT_VER == $EXPECT_VERSION"
    else
      record "version_match_expected" "fail" "got '$SHORT_VER', expected '$EXPECT_VERSION'"
    fi
  else
    record "version_match_expected" "skip" "no --expect-version given"
  fi
else
  record "version_short_semver" "skip" "version_runs failed"
  record "version_match_expected" "skip" "version_runs failed"
fi

# Check 5: phantom doctor (skipped in --quick mode)
if [[ "$QUICK" == "true" ]]; then
  record "doctor_runs" "skip" "--quick mode"
  record "doctor_json_parseable" "skip" "--quick mode"
elif [[ "${CHECK_STATUS[0]:-fail}" == "pass" ]]; then
  run_phantom "doctor" DOCTOR_OUT DOCTOR_EXIT
  if [[ "$DOCTOR_EXIT" -eq 0 ]]; then
    record "doctor_runs" "pass" "exit 0; $(echo "$DOCTOR_OUT" | wc -l) lines"
  else
    record "doctor_runs" "fail" "exit=$DOCTOR_EXIT"
  fi

  # Check 6: doctor --json (best-effort; older binaries may not honor --json)
  run_phantom "doctor --json" DOCTOR_JSON_OUT DOCTOR_JSON_EXIT
  # crude JSON detection: starts with { and has "binary" or "version"
  if [[ "$DOCTOR_JSON_EXIT" -eq 0 && "$DOCTOR_JSON_OUT" =~ ^[[:space:]]*\{ ]]; then
    record "doctor_json_parseable" "pass" "valid JSON"
  elif [[ "$DOCTOR_JSON_EXIT" -eq 0 ]]; then
    record "doctor_json_parseable" "skip" "--json honored but plain output (pre-0.5.0?)"
  else
    record "doctor_json_parseable" "fail" "exit=$DOCTOR_JSON_EXIT"
  fi
fi

# Check 7: phantom selftest --p0-only (only in --full; needs LLM key)
if [[ "$FULL" == "true" && "${CHECK_STATUS[0]:-fail}" == "pass" ]]; then
  run_phantom "selftest --p0-only" SELFTEST_OUT SELFTEST_EXIT
  if [[ "$SELFTEST_EXIT" -eq 0 ]]; then
    record "selftest_p0" "pass" "exit 0"
  else
    record "selftest_p0" "fail" "exit=$SELFTEST_EXIT"
  fi
else
  record "selftest_p0" "skip" "--full not given"
fi

# Summary
END_TIME=$(date +%s)
DURATION=$((END_TIME - START_TIME))
PASS_COUNT=0
FAIL_COUNT=0
SKIP_COUNT=0
for s in "${CHECK_STATUS[@]}"; do
  case "$s" in
    pass) PASS_COUNT=$((PASS_COUNT+1)) ;;
    fail) FAIL_COUNT=$((FAIL_COUNT+1)) ;;
    skip) SKIP_COUNT=$((SKIP_COUNT+1)) ;;
  esac
done

if [[ "$JSON_OUT" == "true" ]]; then
  echo "{"
  echo "  \"script\": \"$SCRIPT_NAME\","
  echo "  \"script_version\": \"$SCRIPT_VERSION\","
  echo "  \"binary\": \"$BINARY\","
  echo "  \"duration_seconds\": $DURATION,"
  echo "  \"summary\": { \"pass\": $PASS_COUNT, \"fail\": $FAIL_COUNT, \"skip\": $SKIP_COUNT },"
  echo "  \"exit_code\": $EXIT_CODE,"
  echo "  \"checks\": ["
  for i in "${!CHECK_NAMES[@]}"; do
    sep=","
    if [[ $i -eq $((${#CHECK_NAMES[@]} - 1)) ]]; then sep=""; fi
    detail_escaped=$(echo "${CHECK_DETAILS[$i]}" | sed 's/\\/\\\\/g; s/"/\\"/g' | tr '\n' ' ')
    echo "    { \"name\": \"${CHECK_NAMES[$i]}\", \"status\": \"${CHECK_STATUS[$i]}\", \"detail\": \"$detail_escaped\" }$sep"
  done
  echo "  ]"
  echo "}"
else
  echo "phantom verify-binary $SCRIPT_VERSION"
  echo "  binary:   $BINARY"
  echo "  duration: ${DURATION}s"
  echo ""
  for i in "${!CHECK_NAMES[@]}"; do
    case "${CHECK_STATUS[$i]}" in
      pass) sym="\033[32m✓\033[0m" ;;
      fail) sym="\033[31m✗\033[0m" ;;
      skip) sym="\033[90m∘\033[0m" ;;
    esac
    printf "  %b %-30s %s\n" "$sym" "${CHECK_NAMES[$i]}" "${CHECK_DETAILS[$i]}"
  done
  echo ""
  if [[ $EXIT_CODE -eq 0 ]]; then
    printf "\033[32mPASS\033[0m: %d/%d checks passed (%d skipped)\n" "$PASS_COUNT" "$((PASS_COUNT + FAIL_COUNT))" "$SKIP_COUNT"
  else
    printf "\033[31mFAIL\033[0m: %d/%d checks passed (%d skipped, %d failed)\n" "$PASS_COUNT" "$((PASS_COUNT + FAIL_COUNT))" "$SKIP_COUNT" "$FAIL_COUNT"
  fi
fi

exit $EXIT_CODE
