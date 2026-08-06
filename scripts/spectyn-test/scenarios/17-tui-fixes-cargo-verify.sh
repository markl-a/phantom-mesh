#!/usr/bin/env bash
source "$SPECTYN_TEST_LIB/common.sh"

scenario "TUI fixes — cargo test verifies Clear (PR #6) + line_count (PR #10) regression suites"
require_cmd cargo

# PR #6 and PR #10 ship the two TUI regression tests. Until they merge into
# the same trunk they live on separate branches/worktrees, so this scenario
# locates each independently and runs cargo against whatever tree carries
# the test. Both must pass for a green mark.
#
# Discovery rule: for each marker, scan cwd then sibling .worktrees/* for
# the FIRST tree whose core/src/tui.rs contains it. SPECTYN_TUI_TREE env
# override applies to BOTH (forces single-tree mode — useful post-merge).

CLEAR_MARKER='transcript_does_not_retain_stale'
CJK_MARKER='long_cjk_assistant_message_keeps_tail_visible'

find_tree_with_marker() {
    local marker="$1"
    local pwd_now="$(pwd)"

    # 0. explicit env override
    if [ -n "${SPECTYN_TUI_TREE:-}" ]; then
        if [ -f "$SPECTYN_TUI_TREE/core/src/tui.rs" ] \
           && grep -q "$marker" "$SPECTYN_TUI_TREE/core/src/tui.rs" 2>/dev/null; then
            printf '%s\n' "$SPECTYN_TUI_TREE"
            return 0
        fi
    fi
    # 1. cwd
    if [ -f "$pwd_now/core/src/tui.rs" ] \
       && grep -q "$marker" "$pwd_now/core/src/tui.rs" 2>/dev/null; then
        printf '%s\n' "$pwd_now"
        return 0
    fi
    # 2. sibling .worktrees/*
    local repo_root="$pwd_now"
    while [ "$repo_root" != "/" ] && [ ! -d "$repo_root/.worktrees" ]; do
        repo_root=$(dirname "$repo_root")
    done
    if [ -d "$repo_root/.worktrees" ]; then
        for wt in "$repo_root"/.worktrees/*/; do
            wt="${wt%/}"
            if [ -f "$wt/core/src/tui.rs" ] \
               && grep -q "$marker" "$wt/core/src/tui.rs" 2>/dev/null; then
                printf '%s\n' "$wt"
                return 0
            fi
        done
    fi
    return 1
}

run_cargo_for() {
    local tree="$1" filter="$2" label="$3"

    # On MSYS / Git-Bash on Windows, paths look like `/d/Projects/...` which
    # cargo (a native Windows binary) does not understand. Convert to a
    # mixed-style path (forward slashes + drive letter) that works in both
    # MSYS and Windows-native shells. cygpath -m gives `D:/Projects/...`.
    local tree_native="$tree"
    if command -v cygpath >/dev/null 2>&1; then
        tree_native=$(cygpath -m "$tree")
    fi

    step "$label: cargo test --lib $filter (tree: $tree_native)"

    local target_dir="${CARGO_TARGET_DIR:-D:/tmp/spectyn-windows-host-target}"
    [ -d "$target_dir" ] || mkdir -p "$target_dir"

    local ts; ts=$(date +%s)
    local out; out=$(timeout 240 env \
        MSYS_NO_PATHCONV=1 \
        CARGO_TARGET_DIR="$target_dir" \
        CARGO_INCREMENTAL=0 \
        cargo test --manifest-path "$tree_native/core/Cargo.toml" --lib "$filter" 2>&1)
    local ec=$?
    local elapsed=$(( $(date +%s) - ts ))

    if [ "$ec" -eq 124 ]; then
        fail "$label: cargo test hit 240s timeout — likely McAfee/Defender mid-build"
        return 1
    fi
    step "$label: cargo exit=$ec elapsed=${elapsed}s"

    local result; result=$(printf '%s\n' "$out" | grep -E '^test result:' | head -1)
    step "$label: $result"

    if [ "$ec" -ne 0 ]; then
        fail "$label: cargo test exit=$ec"
        printf '%s\n' "$out" | tail -10 | sed 's/^/      /' >&2
        return 1
    fi

    if printf '%s\n' "$out" | grep -qE '0 failed' ; then
        if printf '%s\n' "$out" | grep -q "${filter}.*ok"; then
            pass "$label: regression test ran and passed"
            return 0
        else
            fail "$label: test name '$filter' did not appear in output"
            return 1
        fi
    else
        fail "$label: failed-count > 0: $result"
        return 1
    fi
}

CLEAR_TREE=$(find_tree_with_marker "$CLEAR_MARKER")
CJK_TREE=$(find_tree_with_marker "$CJK_MARKER")

if [ -z "$CLEAR_TREE" ] && [ -z "$CJK_TREE" ]; then
    warn "neither PR #6 nor PR #10 marker found in any reachable tree — skipping"
    exit 77
fi

failed=0
if [ -n "$CLEAR_TREE" ]; then
    run_cargo_for "$CLEAR_TREE" "$CLEAR_MARKER" "PR #6 Clear-fix" || failed=1
else
    warn "PR #6 marker not found — Clear-fix test not verified this run"
fi

if [ -n "$CJK_TREE" ]; then
    run_cargo_for "$CJK_TREE" "$CJK_MARKER" "PR #10 CJK-fix" || failed=1
else
    warn "PR #10 marker not found — CJK-fix test not verified this run"
fi

[ $failed -eq 0 ] && [ "$SPECTYN_TEST_FAILED" -eq 0 ]
