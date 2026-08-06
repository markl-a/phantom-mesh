#!/usr/bin/env bash
# Windows-only: `spectyn service` is the Scheduled-Task wrapper that auto-
# starts `spectyn serve` at user logon. We just check that `spectyn service
# status` responds — registration is up to the user via `spectyn service
# install`.

selftest_feature_meta() {
  echo "name=service-windows"
  echo "priority=P2"
  echo "requires=windows"
  echo "description=spectyn service status responds on Windows (Scheduled Task)"
  echo "hints=core/src/bin/spectyn.rs core/src/platform"
}

selftest_requires() {
  case "$(uname -s)" in
    MINGW*|MSYS*|CYGWIN*) return 0 ;;
    *) echo "not Windows (Git Bash / MSYS / Cygwin)" >&2; return 1 ;;
  esac
}

selftest_run() {
  local out="$SELFTEST_ARTIFACTS/service.out"
  T_REPRO="$(printf '%q' "$SPECTYN") service status"
  T_ARTIFACT="$out"
  if "$SPECTYN" service status > "$out" 2>&1; then
    t_pass "spectyn service status" "exit 0"
  else
    rc=$?
    # Not installed is fine; we only care that the subcommand is wired up.
    if grep -qiE 'not (registered|installed)|no scheduled task' "$out"; then
      t_pass "spectyn service status" "responds (not installed)"
    else
      t_fail "spectyn service status" "exit $rc — see artifact"
    fi
  fi
}
