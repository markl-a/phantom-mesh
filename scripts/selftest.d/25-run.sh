#!/usr/bin/env bash
# Exercise the one-shot agent path: `spectyn run "<prompt>"` should drive a
# real LLM, invoke the requested tool, and print output that proves the tool
# was actually called. We use unique markers in the prompt so a successful
# run can only succeed by *actually* invoking the tool — not by the model
# hallucinating the answer.

selftest_feature_meta() {
  echo "name=run"
  echo "priority=P1"
  echo "requires=daemon network"
  echo "description=spectyn run one-shot agent invokes shell + file_read tools"
  echo "hints=core/src/agent.rs core/src/runtime.rs core/src/tools core/src/bin/spectyn.rs"
}

selftest_requires() {
  # `spectyn run` needs at least one provider key configured. Cheap proxy:
  # any non-empty *_API_KEY env var, OR an inline key in agents.toml.
  if [ -n "$ANTHROPIC_API_KEY$OPENAI_API_KEY$GROQ_API_KEY$GEMINI_API_KEY$OPENROUTER_API_KEY$OPENCODE_API_KEY$CEREBRAS_API_KEY$DEEPSEEK_API_KEY$MISTRAL_API_KEY$TOGETHER_API_KEY$NVIDIA_NIM_API_KEY" ]; then
    return 0
  fi
  if [ -f "$HOME/.spectyn-mesh/agents.toml" ] && grep -qE 'api_key[[:space:]]*=' "$HOME/.spectyn-mesh/agents.toml"; then
    return 0
  fi
  echo "no provider API key in env or ~/.spectyn-mesh/agents.toml — `spectyn run` would fail" >&2
  return 1
}

selftest_run() {
  local out marker

  # 1. shell tool — generate a unique marker the model can't have memorised
  marker="spectyn-selftest-$(date +%s)-$RANDOM"
  out="$SELFTEST_ARTIFACTS/run-shell.out"
  T_REPRO="$(printf '%q' "$SPECTYN") run \"Use shell to run 'echo $marker' and then say what it printed\""
  T_ARTIFACT="$out"
  if timeout 90 "$SPECTYN" run "Use shell to run 'echo $marker' and then say what it printed" \
       > "$out" 2>&1 && grep -q "$marker" "$out"; then
    t_pass "spectyn run (shell tool)" "agent invoked shell + reported $marker"
  else
    rc=$?
    if [ "$rc" = 124 ]; then
      t_fail "spectyn run (shell tool)" "timed out after 90s"
    else
      t_fail "spectyn run (shell tool)" "marker not in output (exit $rc)"
    fi
  fi

  # 2. file_read tool — assert a known string from the repo's README
  out="$SELFTEST_ARTIFACTS/run-file-read.out"
  T_REPRO="$(printf '%q' "$SPECTYN") run \"Use file_read to read README.md and report just its first heading\""
  T_ARTIFACT="$out"
  if timeout 90 "$SPECTYN" run "Use file_read to read README.md and report just its first heading" \
       > "$out" 2>&1 && grep -q "Spectyn Mesh" "$out"; then
    t_pass "spectyn run (file_read tool)" "agent read README and returned title"
  else
    rc=$?
    if [ "$rc" = 124 ]; then
      t_fail "spectyn run (file_read tool)" "timed out after 90s"
    else
      t_fail "spectyn run (file_read tool)" "missing 'Spectyn Mesh' in output (exit $rc)"
    fi
  fi
}
