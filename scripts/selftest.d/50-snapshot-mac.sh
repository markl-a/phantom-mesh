#!/usr/bin/env bash
# macOS-only: APFS snapshot subsystem. Only runs on Darwin; everywhere else
# the precondition gate skips the whole feature with a clean SKIP record.

selftest_feature_meta() {
  echo "name=snapshot-mac"
  echo "priority=P2"
  echo "requires=macos"
  echo "description=phantom snapshot list responds on macOS (APFS)"
  echo "hints=core/src/snapshot.rs core/src/platform"
}

selftest_requires() {
  if [ "$(uname)" != "Darwin" ]; then
    echo "not macOS" >&2
    return 1
  fi
}

selftest_run() {
  out="$TMP/snapshot.out"
  if "$PHANTOM" snapshot list > "$out" 2>&1; then
    t_pass "phantom snapshot list" "exit 0"
  else
    rc=$?
    # exit 1 with a "no snapshots" message is fine — the subcommand worked.
    if grep -qiE 'no snapshots|empty' "$out"; then
      t_pass "phantom snapshot list" "no snapshots yet"
    else
      t_fail "phantom snapshot list" "exit $rc (see $out)"
    fi
  fi
}
