#!/usr/bin/env bash
# Run the satellite mock demos (phantom-secops, phantom-mobile) when they're
# cloned alongside this repo. Each demo prints its own pass-rate, so we just
# scrape the headline metric.
#
# Auto-skips cleanly if the satellites aren't present — they're optional.

selftest_feature_meta() {
  echo "name=ecosystem"
  echo "priority=P2"
  echo "requires=ecosystem"
  echo "description=phantom-secops + phantom-mobile demo-mock satellites pass"
  echo "hints=Makefile ../phantom-secops/Makefile ../phantom-mobile/Makefile"
}

# Search for satellites in the locations the user actually clones into.
_eco_find_satellite() {
  local name="$1" repo="${name#phantom-}"
  local candidates=(
    "$(pwd)/../$name"
    "$HOME/path/to/$name"
    "$HOME/$name"
  )
  for d in "${candidates[@]}"; do
    [ -f "$d/Makefile" ] && { echo "$d"; return 0; }
  done
  return 1
}

selftest_requires() {
  if _eco_find_satellite phantom-secops >/dev/null \
     || _eco_find_satellite phantom-mobile >/dev/null; then
    return 0
  fi
  echo "neither phantom-secops nor phantom-mobile cloned as a sibling — nothing to run" >&2
  return 1
}

selftest_run() {
  # User-level PATH has cargo / brew / miniconda; without it, child make
  # invocations may not see those toolchains.
  local extra_path="$HOME/.cargo/bin:$HOME/miniconda/bin:/opt/homebrew/bin:/usr/local/bin"
  local PATH_FOR_MAKE="$extra_path:$PATH"

  for sat in phantom-secops phantom-mobile; do
    local dir; dir="$(_eco_find_satellite "$sat" 2>/dev/null)" || {
      t_skip "$sat demo-mock" "satellite not cloned"
      continue
    }
    local out="$SELFTEST_ARTIFACTS/${sat}.out"
    T_REPRO="PATH=$PATH_FOR_MAKE make -C $(printf '%q' "$dir") demo-mock"
    T_ARTIFACT="$out"
    if PATH="$PATH_FOR_MAKE" make -C "$dir" demo-mock > "$out" 2>&1 \
       && grep -qE 'Pass rate|artifacts at' "$out"; then
      headline=$(grep -E 'Pass rate|MTTD' "$out" | head -1 | sed 's/^→ *//' | cut -c1-60)
      t_pass "$sat demo-mock" "${headline:-completed}"
    else
      last3=$(tail -3 "$out" | tr '\n' ' ' | cut -c1-140)
      t_fail "$sat demo-mock" "$last3"
    fi
  done
}
