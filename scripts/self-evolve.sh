#!/usr/bin/env bash

# ⛔ GATED (2026-06-07): 閘門前禁跑(ACCEL doc §③)。
if [ "${PHANTOM_GATE_PASSED:-0}" != "1" ] && [ "${1:-}" != "--break-glass" ]; then
  echo "GATED: partner app 未過 7 天真用閘門 (docs/ACCEL-FRAMEWORK-AS-PHANTOM-FEATURE.md §③)。" >&2
  echo "解法: 過閘後 PHANTOM_GATE_PASSED=1;或 --break-glass(手動逃生,輸出只准寫 ~/.phantom-mesh/dev-loop-log.jsonl)。" >&2
  exit 2
fi

# phantom self-evolve: run phantom evolve on its own codebase
# Usage: ./scripts/self-evolve.sh [rounds]
#
# Uses single-node evolve (not --distributed) because cargo/Rust only
# available on Mac and node-a (Linux). Windows nodes (node-a, laptop) can't
# run cargo tasks.
set -euo pipefail

REPO="${PHANTOM_MESH_REPO:-$(cd "$(dirname "$0")/.." && pwd)}"
BINARY="$REPO/core/target/release/phantom"
ROUNDS="${1:-3}"
LOG="$REPO/scripts/evolve.log"

cd "$REPO/core"

echo "$(date): self-evolve started (rounds=$ROUNDS)" | tee -a "$LOG"

# Gather context: test failures, clippy warnings, TODOs
CONTEXT=$(
  cargo test --lib -- --test-threads=1 2>&1 | grep -E "FAILED|error\[" | head -20 || true
  cargo clippy --all-targets 2>&1 | grep "^warning:" | head -30 || true
  grep -rn "TODO\|FIXME\|unimplemented!" src/ --include="*.rs" | grep -v "//.*TODO" | head -15 || true
  git log --oneline -5 || true
)

GOAL="You are working on the phantom-mesh Rust codebase at $REPO/core.

Context (test failures, clippy warnings, TODOs):
$CONTEXT

Your task — work through these in order:
1. Run 'cargo test --lib -- --test-threads=1 2>&1 | tail -20' to identify failures
2. Fix any failing tests or compilation errors
3. Run 'cargo clippy --all-targets 2>&1 | grep warning | head -30' and fix easy clippy warnings
   (collapsible_if, manual_strip, needless_collect, etc.)
4. If all tests pass, run 'cargo build --release 2>&1 | tail -5' to verify
5. Use git_commit to commit any changes made

Working directory: $REPO/core
Tools available: shell, file_read, file_edit, content_search, glob_search, git_commit"

echo "Goal prepared, running evolve (rounds=$ROUNDS)..." | tee -a "$LOG"
"$BINARY" evolve "$GOAL" --rounds "$ROUNDS" 2>&1 | tee -a "$LOG"

echo "$(date): self-evolve done" | tee -a "$LOG"
