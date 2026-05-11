#!/usr/bin/env bash
# Windows-only: `phantom service` is the Scheduled-Task wrapper that auto-
# starts `phantom serve` at user logon. We just check that `phantom service
# status` responds — registration is up to the user via `phantom service
# install`.

selftest_feature_meta() {
  echo "name=service-windows"
  echo "priority=P2"
  echo "requires=windows"
  echo "description=phantom service status responds on Windows (Scheduled Task)"
  echo "hints=core/src/bin/phantom.rs core/src/platform"
}

selftest_requires() {
  case "$(uname -s)" in
    MINGW*|MSYS*|CYGWIN*) return 0 ;;
    *) echo "not Windows (Git Bash / MSYS / Cygwin)" >&2; return 1 ;;
  esac
}

selftest_run() {
  local out="$SELFTEST_ARTIFACTS/service.out"
  T_REPRO="$(printf '%q' "$PHANTOM") service status"
  T_ARTIFACT="$out"
  if "$PHANTOM" service status > "$out" 2>&1; then
    t_pass "phantom service status" "exit 0"
  else
    rc=$?
    # Not installed is fine; we only care that the subcommand is wired up.
    if grep -qiE 'not (registered|installed)|no scheduled task' "$out"; then
      t_pass "phantom service status" "responds (not installed)"
    else
      t_fail "phantom service status" "exit $rc — see artifact"
    fi
  fi
}
