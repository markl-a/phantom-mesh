#!/usr/bin/env bash
# tdd-loop.sh — interactive TDD dev loop.
#
# Cycle:
#   1. tdd-next.sh prints next red test
#   2. operator writes the test file + implementation
#   3. tdd-run.sh confirms green
#   4. tdd-mark-done.sh marks it done
#   5. loop until all green or operator quits
#
# Callable from any agent's bash tool (Claude Code, Gemini CLI, Codex CLI, Antigravity).

set -u
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

while true; do
  echo ""
  echo "============================================================"
  next=$("$SCRIPT_DIR/tdd-next.sh") || { echo "$next"; break; }
  test_name=$(echo "$next" | awk -F'|' '{print $2}' | xargs)

  echo "  NEXT: $next"
  echo "============================================================"
  echo ""
  echo "  1. Write the test FIRST (test-first per TDD)"
  echo "  2. Confirm RED:   $SCRIPT_DIR/tdd-run.sh '$test_name'"
  echo "  3. Implement minimum code"
  echo "  4. Confirm GREEN: $SCRIPT_DIR/tdd-run.sh '$test_name'"
  echo ""
  read -r -p "Status? [d=done(mark green), s=skip, r=run-now, q=quit]: " ans

  case "$ans" in
    d|D)
      "$SCRIPT_DIR/tdd-mark-done.sh" "$test_name"
      ;;
    s|S)
      echo "skipped (left as red)"
      # avoid infinite loop on skipped — bail out of loop
      break
      ;;
    r|R)
      "$SCRIPT_DIR/tdd-run.sh" "$test_name"
      # let operator re-decide
      ;;
    q|Q)
      echo "quit"
      break
      ;;
    *)
      echo "unrecognized — quitting"
      break
      ;;
  esac
done

"$SCRIPT_DIR/tdd-status.sh"
