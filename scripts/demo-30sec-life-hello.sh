#!/usr/bin/env bash
# E006 — 30-second Life Hello demo
# Shows a stranger what Spectyn's Life Track does in under 30 seconds.
#
# Usage:
#   ./scripts/demo-30sec-life-hello.sh
#
# Prerequisites:
#   - spectyn binary at core/target/release/spectyn (or on PATH)
#   - spectyn serve running: nohup spectyn serve &
#   - GEMINI_API_KEY set (or read from ~/.spectyn-mesh/agents.toml)
#   - identity.key at ~/.spectyn-mesh/identity.key

set -euo pipefail

SPECTYN="${SPECTYN_BIN:-$(dirname "$0")/../core/target/release/spectyn}"
if ! command -v "$SPECTYN" &>/dev/null && [ ! -x "$SPECTYN" ]; then
    SPECTYN="spectyn"
fi

DATE_TODAY="$(date +%Y-%m-%d)"

# Pull Gemini key from agents.toml if not already in env
if [ -z "${GEMINI_API_KEY:-}" ]; then
    GEMINI_API_KEY="$(grep -A3 "\[providers\.gemini\]" ~/.spectyn-mesh/agents.toml 2>/dev/null \
        | grep api_key | sed 's/.*api_key = "\(.*\)"/\1/')"
    export GEMINI_API_KEY
fi

echo ""
echo "╔═══════════════════════════════════════════════════╗"
echo "║   Spectyn — 30-second Life Hello demo             ║"
echo "║   Life Track: capture → analyze → coach review    ║"
echo "╚═══════════════════════════════════════════════════╝"
echo ""
echo "── Step 1: log today's food ───────────────────────"
"$SPECTYN" event capture \
    --kind food_log \
    --text "Caesar salad with grilled chicken — light lunch, good protein" \
    --tag fat_loss

echo ""
echo "── Step 2: log a focus session ────────────────────"
"$SPECTYN" event capture \
    --kind focus_session \
    --text "50 min deep work on spectyn-mesh E006, no distractions" \
    --tag focus

echo ""
echo "── Step 3: log evening habit ──────────────────────"
"$SPECTYN" event capture \
    --kind habit_check \
    --text "evening walk 30 min around the park" \
    --tag habit

echo ""
echo "── Coach review for today ($DATE_TODAY) ────────────"
"$SPECTYN" coach review --date "$DATE_TODAY"

echo ""
echo "Demo complete. Total events logged: 3"
echo "Try: spectyn coach review --date $DATE_TODAY"
