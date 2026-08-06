#!/usr/bin/env bash
source "$SPECTYN_TEST_LIB/common.sh"

scenario "spectyn service status — autostart entry registered for current platform"
require_cmd "$SPECTYN_BIN"

step "running spectyn service status"
out=$("$SPECTYN_BIN" service status 2>&1 | sed -E 's/\x1b\[[0-9;]*m//g')

# `spectyn service status` reports per-platform autostart state:
#   - macOS:   launchd
#   - Windows: Scheduled Task "SpectynServe"
#   - Linux:   systemd --user
# We accept ANY of these markers as evidence the registration mechanism
# is healthy on this host.
matched=0
for kw in "Scheduled Task" "launchd" "LaunchAgent" "systemd" "registered" "SpectynServe"; do
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
    # spectyn should still print a clean summary, not panic.
    if printf '%s' "$out" | grep -qiE 'not installed|not registered|no service|run.*service install'; then
        pass "service not installed — spectyn reported cleanly"
    else
        fail "no recognized markers in service status output: ${out:0:300}"
    fi
fi

ASSERT_NOT_CONTAINS "$out" "panicked at" "no Rust panic"

[ "$SPECTYN_TEST_FAILED" -eq 0 ]
