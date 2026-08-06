#!/usr/bin/env bash
# Permission DSL self-test — verifies `spectyn doctor`'s [permissions]
# section behaves correctly across the four canonical states:
#   1. no [permissions] block      → "no rules → allow all"
#   2. valid rules                 → "N rules parsed (X deny, Y ask, Z allow)"
#   3. malformed rule              → "parse error"
#   4. blanket Deny on a tool      → tool listed in "statically denied"

selftest_feature_meta() {
  echo "name=permission-dsl"
  echo "priority=P1"
  echo "requires=spectyn-doctor"
  echo "description=spectyn doctor [permissions] section: empty/parsed/error/static-deny states"
  echo "hints=core/src/permission.rs core/src/bin/spectyn.rs docs/PERMISSIONS.md"
}

_perm_setup_home() {
  local block="$1"
  local td; td=$(mktemp -d)
  mkdir -p "$td/.spectyn-mesh"
  cat > "$td/.spectyn-mesh/agents.toml" <<EOF
[core]
host = "127.0.0.1"
port = 7878

[providers.anthropic]
type    = "anthropic"
api_key = "sk-ant-test-fake-key"

[agent.master]
provider     = "anthropic"
instructions = "test"
tools        = ["shell", "file_read", "web_fetch"]

$block
EOF
  echo "$td"
}

selftest_run() {
  local td out

  # 1. Empty permissions ⇒ legacy "allow all" message
  td=$(_perm_setup_home "")
  out="$SELFTEST_ARTIFACTS/perm-empty.txt"
  HOME="$td" "$SPECTYN" doctor > "$out" 2>&1
  T_ARTIFACT="$out"
  # Locale-robust: doctor's [permissions] line is i18n'd — en "no rules → allow
  # all", zh-TW "未設定規則 → 全部允許". Match either so this passes on the zh-TW
  # canary Mac (LANG=zh_TW) as well as English CI (z13).
  T_REPRO="HOME=$td $SPECTYN doctor 2>&1 | grep -E 'allow all|全部允許'"
  if grep -qE "no rules → allow all|全部允許" "$out"; then
    t_pass "empty permissions → allow-all message" ""
  else
    t_fail "empty permissions → allow-all message" "missing 'allow all' / '全部允許'"
  fi
  rm -rf "$td"

  # 2. Valid rules ⇒ "rules parsed (X deny, Y ask, Z allow)"
  td=$(_perm_setup_home '[permissions]
deny  = ["Read(./.env)"]
ask   = ["Bash"]
allow = ["Bash(git status)", "Read(./README.md)"]')
  out="$SELFTEST_ARTIFACTS/perm-parsed.txt"
  HOME="$td" "$SPECTYN" doctor > "$out" 2>&1
  T_ARTIFACT="$out"
  T_REPRO="HOME=$td $SPECTYN doctor 2>&1 | grep 'rules parsed'"
  if grep -qE "rules parsed \([0-9]+ deny, [0-9]+ ask, [0-9]+ allow\)" "$out"; then
    t_pass "valid rules → 'rules parsed' line" ""
  else
    t_fail "valid rules → 'rules parsed' line" "no parsed-count line"
  fi
  rm -rf "$td"

  # 3. Malformed rule ⇒ "parse error" surfaced
  td=$(_perm_setup_home '[permissions]
deny = ["Bash(unterminated-spec"]')
  out="$SELFTEST_ARTIFACTS/perm-error.txt"
  HOME="$td" "$SPECTYN" doctor > "$out" 2>&1
  T_ARTIFACT="$out"
  T_REPRO="HOME=$td $SPECTYN doctor 2>&1 | grep 'parse error'"
  if grep -qi "parse error" "$out"; then
    t_pass "malformed rule → 'parse error' surfaces" ""
  else
    t_fail "malformed rule → 'parse error' surfaces" "no parse-error message"
  fi
  rm -rf "$td"

  # 4. Blanket Deny ⇒ tool appears in "statically denied" list
  td=$(_perm_setup_home '[permissions]
deny = ["WebFetch"]')
  out="$SELFTEST_ARTIFACTS/perm-static.txt"
  HOME="$td" "$SPECTYN" doctor > "$out" 2>&1
  T_ARTIFACT="$out"
  T_REPRO="HOME=$td $SPECTYN doctor 2>&1 | grep 'statically denied'"
  if grep -q "statically denied.*web_fetch" "$out"; then
    t_pass "blanket Deny → statically-denied list" ""
  else
    t_fail "blanket Deny → statically-denied list" "tool not listed"
  fi
  rm -rf "$td"
}
