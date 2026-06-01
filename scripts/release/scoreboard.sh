#!/usr/bin/env bash
# scoreboard.sh — thin alias for epic-acceptance-score.sh
#
# F600 spec names the canonical script `epic-acceptance-score.sh`. This file
# exists because the runbook conversation and some operator habits use the
# shorter `scoreboard.sh` name. Both invocations behave identically.
#
# See `scripts/release/epic-acceptance-score.sh --help` for the full env-var
# contract, flags, and exit codes.

set -u

HERE=$(cd "$(dirname "$0")" && pwd)
exec "$HERE/epic-acceptance-score.sh" "$@"
