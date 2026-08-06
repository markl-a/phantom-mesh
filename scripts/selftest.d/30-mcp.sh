#!/usr/bin/env bash
# Validate the stdio MCP server: handshake, tools/list, and minimum tool count.
# Mirrors scripts/validate-mcp.sh but as a structured selftest feature.
#
# Pure-bash parsing — works on Mac, Linux, and Git Bash on Windows without
# needing python3 or jq. We do "good enough" tool detection by grepping the
# emitted JSON for the canonical `"name":"<tool>"` shape; that's all the
# stdio output uses, so the heuristic is reliable in practice.

selftest_feature_meta() {
  echo "name=mcp"
  echo "priority=P1"
  echo "requires=mcp"
  echo "description=spectyn mcp stdio server handshake + tools/list"
  echo "hints=core/src/mcp.rs core/src/tools core/src/bin/spectyn.rs"
}

# Resolve a `timeout`-equivalent. macOS doesn't ship GNU timeout; users often
# install coreutils via brew which gives `gtimeout`. Git Bash has `timeout`.
_mcp_find_timeout() {
  if command -v timeout  >/dev/null 2>&1; then echo "timeout";  return 0; fi
  if command -v gtimeout >/dev/null 2>&1; then echo "gtimeout"; return 0; fi
  return 1
}

selftest_requires() {
  if ! _mcp_find_timeout >/dev/null; then
    echo "neither 'timeout' nor 'gtimeout' on PATH (mac: brew install coreutils)" >&2
    return 1
  fi
}

selftest_run() {
  local TIMEOUT; TIMEOUT="$(_mcp_find_timeout)"
  local init list out
  init='{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"selftest","version":"1"}}}'
  list='{"jsonrpc":"2.0","id":2,"method":"tools/list"}'
  out="$SELFTEST_ARTIFACTS/mcp.out"
  printf '%s\n%s\n' "$init" "$list" | "$TIMEOUT" 15 "$SPECTYN" mcp > "$out" 2>"$SELFTEST_ARTIFACTS/mcp.err" || true

  # Pre-set repro/artifact so each t_pass/t_fail attaches them in the report.
  T_ARTIFACT="$out"
  T_REPRO="printf '%s\n%s\n' '$init' '$list' | $TIMEOUT 15 $(printf '%q' "$SPECTYN") mcp"

  # 1. Handshake response
  if grep -q '"serverInfo"' "$out"; then
    t_pass "mcp initialize" "serverInfo present"
  else
    t_fail "mcp initialize" "no serverInfo (see $out)"
    return
  fi

  # 2. Each must-have tool gets its own row, so a missing one tells you
  # exactly which it is — better debug signal than a single composite check.
  local missing=0 tool
  for tool in subagent parallel_tasks shell file_read git_status; do
    T_ARTIFACT="$out"
    T_REPRO="grep -E '\"name\"[[:space:]]*:[[:space:]]*\"$tool\"' $out"
    if grep -qE "\"name\"[[:space:]]*:[[:space:]]*\"$tool\"" "$out"; then
      t_pass "tool present: $tool" ""
    else
      t_fail "tool present: $tool" "not in tools/list output"
      missing=$((missing+1))
    fi
  done

  # 3. Loose count check via grep — counts unique tool names. The MCP wire
  # format puts each tool's `"name":"…"` on the same JSON line, so unique-by-
  # `sort -u` is a fair proxy for "how many tools were registered".
  local count
  count=$(grep -oE '"name"[[:space:]]*:[[:space:]]*"[a-zA-Z0-9_]+"' "$out" \
            | sort -u | wc -l | tr -d ' ')
  T_ARTIFACT="$out"
  T_REPRO="grep -oE '\"name\"[[:space:]]*:[[:space:]]*\"[a-zA-Z0-9_]+\"' $out | sort -u | wc -l"
  if [ "$count" -ge 40 ]; then
    t_pass "tools/list count" "$count unique names (≥ 40)"
  else
    t_fail "tools/list count" "got $count (want ≥ 40)"
  fi
}
