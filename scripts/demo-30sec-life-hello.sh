#!/usr/bin/env bash
# E006 — 30-second Life Hello demo
# Shows a stranger what Phantom's Life Track does in under 30 seconds.
#
# Usage:
#   ./scripts/demo-30sec-life-hello.sh
#
# Prerequisites:
#   - phantom binary at core/target/release/phantom (or on PATH)
#   - phantom serve running: nohup phantom serve &
#   - GEMINI_API_KEY set (or read from ~/.phantom-mesh/agents.toml)
#   - identity.key at ~/.phantom-mesh/identity.key

set -euo pipefail

PHANTOM="${PHANTOM_BIN:-$(dirname "$0")/../core/target/release/phantom}"
if ! command -v "$PHANTOM" &>/dev/null && [ ! -x "$PHANTOM" ]; then
    PHANTOM="phantom"
fi

DATE_TODAY="$(date +%Y-%m-%d)"

# Pull Gemini key from agents.toml if not already in env
if [ -z "${GEMINI_API_KEY:-}" ]; then
    GEMINI_API_KEY="$(grep -A3 "\[providers\.gemini\]" ~/.phantom-mesh/agents.toml 2>/dev/null \
        | grep api_key | sed 's/.*api_key = "\(.*\)"/\1/')"
    export GEMINI_API_KEY
fi

echo ""
echo "╔═══════════════════════════════════════════════════╗"
echo "║   Phantom — 30-second Life Hello demo             ║"
echo "║   Life Track: capture → analyze → coach review    ║"
echo "╚═══════════════════════════════════════════════════╝"
echo ""
echo "── Step 1: log today's food ───────────────────────"
"$PHANTOM" event capture \
    --kind food_log \
    --text "Caesar salad with grilled chicken — light lunch, good protein" \
    --tag fat_loss

echo ""
echo "── Step 2: log a focus session ────────────────────"
"$PHANTOM" event capture \
    --kind focus_session \
    --text "50 min deep work on phantom-mesh E006, no distractions" \
    --tag focus

echo ""
echo "── Step 3: log evening habit ──────────────────────"
"$PHANTOM" event capture \
    --kind habit_check \
    --text "evening walk 30 min around the park" \
    --tag habit

echo ""
echo "── Coach review for today ($DATE_TODAY) ────────────"
"$PHANTOM" coach review --date "$DATE_TODAY"

echo ""
echo "Demo complete. Total events logged: 3"
echo "Try: phantom coach review --date $DATE_TODAY"
