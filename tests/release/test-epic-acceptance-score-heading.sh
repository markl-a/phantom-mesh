#!/usr/bin/env bash
# test-epic-acceptance-score-heading.sh — regression test for the epic
# acceptance scoreboard's H2 detection (heading drift).
#
# Bug: scripts/release/epic-acceptance-score.sh only recognized the English
# "## Acceptance criteria" H2, but the canonical specs in
# docs/superpowers/specs/_current use the Chinese "## 驗收標準" H2 — so e.g.
# E003 scored 0/6 (DRIFT) despite having 6 acceptance checkboxes.
#
# Fixtures (tests/release/fixtures/heading-drift/):
#   E001-english-heading.md  English H2, 2 boxes, 1 ticked  → expect 1/2
#   E003-chinese-heading.md  Chinese H2, 6 boxes, 0 ticked  → expect 0/6
# Both fixtures carry an extra checkbox AFTER the next H2 that must not be
# counted (guards the section-end logic too).
#
# Run: bash tests/release/test-epic-acceptance-score-heading.sh
# Exit 0 = pass, 1 = fail.

set -u

TEST_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$TEST_DIR/../.." && pwd)
SCRIPT_SH="$REPO_ROOT/scripts/release/epic-acceptance-score.sh"
SCRIPT_PS1="$REPO_ROOT/scripts/release/epic-acceptance-score.ps1"
FIXTURES="$TEST_DIR/fixtures/heading-drift"

fail=0

assert_contains() {
    # $1 = haystack, $2 = needle, $3 = label
    case "$1" in
        *"$2"*) printf 'ok   - %s\n' "$3" ;;
        *)
            printf 'FAIL - %s\n       expected to find: %s\n' "$3" "$2"
            fail=1
            ;;
    esac
}

# --strict: if the Chinese H2 were not recognized, the E003 fixture would be
# DRIFT → exit 2. --threshold 0 so the SHIP gate itself (1/8 = 12%) cannot
# mask a parse failure with exit 1.
out=$(bash "$SCRIPT_SH" --strict --threshold 0 --specs-dir "$FIXTURES" 2>&1)
rc=$?

if [ "$rc" -ne 0 ]; then
    printf 'FAIL - scoreboard (.sh) exited %d (expected 0); output:\n%s\n' "$rc" "$out"
    exit 1
fi
printf 'ok   - scoreboard (.sh) exits 0 under --strict (no DRIFT)\n'

assert_contains "$out" '| E001 | 1    | 2     | 50  |' 'English H2 fixture counts 1/2'
assert_contains "$out" '| E003 | 0    | 6     | 0   |' 'Chinese H2 fixture counts 0/6'
assert_contains "$out" '| TOTAL| 1    | 8     | 12  |' 'total row sums both fixtures (1/8)'

# ---------- optional: PowerShell twin parity (only if pwsh is on PATH) ----------
if command -v pwsh >/dev/null 2>&1; then
    ps_out=$(pwsh -NoProfile -File "$SCRIPT_PS1" -Strict -Threshold 0 -SpecsDir "$FIXTURES" 2>&1)
    ps_rc=$?
    if [ "$ps_rc" -ne 0 ]; then
        printf 'FAIL - scoreboard (.ps1) exited %d (expected 0); output:\n%s\n' "$ps_rc" "$ps_out"
        fail=1
    else
        printf 'ok   - scoreboard (.ps1) exits 0 under -Strict (no DRIFT)\n'
        assert_contains "$ps_out" '| E003 | 0    | 6     | 0   |' 'Chinese H2 fixture counts 0/6 (.ps1 parity)'
    fi
else
    printf 'skip - pwsh not on PATH; .ps1 parity not checked\n'
fi

if [ "$fail" -ne 0 ]; then
    printf '\n--- .sh scoreboard output ---\n%s\n' "$out"
    exit 1
fi
printf 'PASS - epic-acceptance-score heading-drift test\n'
exit 0
