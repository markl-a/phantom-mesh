#!/usr/bin/env bash
# tdd-run.sh — run a single named test, report red/green + exit code.
#
# Usage:
#   tdd-run.sh <test::path::name>           # cargo test --lib <name>
#   tdd-run.sh <test::path::name> --bin     # cargo test --bin phantom (rare)
#
# Returns the cargo exit code (0 green, non-zero red).

set -u
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
TEST_NAME="${1:-}"
TARGET_KIND="${2:-lib}"

if [[ -z "$TEST_NAME" ]]; then
  echo "usage: tdd-run.sh <test_name> [--lib|--bin]" >&2
  exit 2
fi

case "$TARGET_KIND" in
  --lib|lib) cargo_args=(test --lib "$TEST_NAME") ;;
  --bin|bin) cargo_args=(test --bin phantom "$TEST_NAME") ;;
  *) cargo_args=(test --lib "$TEST_NAME") ;;
esac

# isolated target dir to dodge Windows AV / share builds across runs
: "${CARGO_TARGET_DIR:=$REPO_ROOT/target-tdd}"
export CARGO_TARGET_DIR

cd "$REPO_ROOT/core"
"${CARGO:-cargo}" "${cargo_args[@]}" -- --test-threads=1 2>&1 | tail -40
exit_code=${PIPESTATUS[0]}

if [[ $exit_code -eq 0 ]]; then
  printf '\n\033[32m🟢 GREEN\033[0m  %s\n' "$TEST_NAME"
else
  printf '\n\033[31m🔴 RED  \033[0m  %s (exit %d)\n' "$TEST_NAME" "$exit_code"
fi

exit $exit_code
