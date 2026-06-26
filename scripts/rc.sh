#!/usr/bin/env bash
# rc.sh — run a command and report its REAL exit code, unfakeably.
#
# Why: agents (and humans) too often claim "tests passed" without proof. This
# wrapper captures full output to a log AND prints an unmissable sentinel line
# carrying the true exit code, so a "green" claim must cite RC=0 from here.
# It never masks the exit code (no `| tail`, no `&&echo ok`); the pipeline's
# real status is recovered via PIPESTATUS.
#
# Usage:
#   scripts/rc.sh <label> -- <command...>
#   scripts/rc.sh check -- cargo check --bin phantom
#   scripts/rc.sh login -- phantom login google
#
# Output: tees to /tmp/phantom-rc-<label>-<timestamp>.log and prints:
#   === RC_SENTINEL label=<label> exit=<code> log=<path> ===
# Exits with the wrapped command's exit code.

set -uo pipefail

label="${1:-run}"
shift || true
if [ "${1:-}" = "--" ]; then shift; fi

if [ "$#" -eq 0 ]; then
  echo "usage: scripts/rc.sh <label> -- <command...>" >&2
  exit 2
fi

ts="$(date +%Y%m%d-%H%M%S)"
log="/tmp/phantom-rc-${label}-${ts}.log"

# Run, stream to terminal AND log. PIPESTATUS[0] = the command's real RC.
"$@" 2>&1 | tee "$log"
rc="${PIPESTATUS[0]}"

echo "=== RC_SENTINEL label=${label} exit=${rc} log=${log} ==="
exit "$rc"
