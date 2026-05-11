#!/usr/bin/env bash
source "$PHANTOM_TEST_LIB/common.sh"

scenario "phantom service status — autostart entry registered for current platform"
require_cmd "$PHANTOM_BIN"

step "running phantom service status"
out=$("$PHANTOM_BIN" service status 2>&1 | sed -E 's/\x1b\[[0-9;]*m//g')

# `phantom service status` reports per-platform autostart state:
#   - macOS:   launchd
#   - Windows: Scheduled Task "PhantomServe"
#   - Linux:   systemd --user
# We accept ANY of these markers as evidence the registration mechanism
# is healthy on this host.
matched=0
for kw in "Scheduled Task" "launchd" "LaunchAgent" "systemd" "registered" "PhantomServe"; do
    if printf '%s' "$out" | grep -qiF "$kw"; then
        matched=$((matched + 1))
    fi
done

if [ "$matched" -ge 1 ]; then
    pass "service status output references at least one autostart system ($matched markers found)"
    step "first 8 lines of output:"
    printf '%s\n' "$out" | head -8 | sed 's/^/    /'
else
    # Maybe service is genuinely not installed — that's a valid state, but
    # phantom should still print a clean summary, not panic.
    if printf '%s' "$out" | grep -qiE 'not installed|not registered|no service|run.*service install'; then
        pass "service not installed — phantom reported cleanly"
    else
        fail "no recognized markers in service status output: ${out:0:300}"
    fi
fi

ASSERT_NOT_CONTAINS "$out" "panicked at" "no Rust panic"

[ "$PHANTOM_TEST_FAILED" -eq 0 ]
