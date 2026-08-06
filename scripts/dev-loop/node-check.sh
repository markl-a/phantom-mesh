#!/bin/sh
# node-check.sh — runs LOCALLY on one fleet node, piped to its LOGIN shell over
# ssh (`ssh <node> "zsh -ls"` / `"...bash.exe" -ls" < node-check.sh`). Because it
# arrives on stdin and runs natively, there is ZERO cross-shell quoting to fight,
# and a login shell gives the real PATH (macOS AIs live in ~/.local/bin via
# .zprofile; git-bash login PATH on Windows). POSIX sh so bash AND zsh run it the
# same. It only INSPECTS — no arming, no writes. Output = KEY=VAL lines the
# conductor (goal-setup.sh) parses.

host=$(hostname 2>/dev/null || echo unknown)

# Which dev AI CLIs answer `command -v` here (login PATH).
ais=""
for t in codex opencode agy claude; do
  if command -v "$t" >/dev/null 2>&1; then ais="$ais$t,"; fi
done
ais=${ais%,}

# Declared platform caps (set at arm time → backlog routing).
caps=""
if [ -f "$HOME/.spectyn-mesh/caps" ]; then
  caps=$(tr '\n,' '  ' < "$HOME/.spectyn-mesh/caps" 2>/dev/null | tr -s ' ')
fi

# Repo present? probe the per-machine candidate paths (path varies by node).
repo=""
for d in \
  "$HOME/Projects/spectyn-mesh" \
  "$HOME/Projects/spectyn-mesh" \
  "$HOME/pm-node" \
  "/d/Projects/spectyn-mesh" \
  "/c/Users/$USER/pm-node" \
  "$HOME/Documents/GitHub/hailmary/spectyn-mesh"; do
  if [ -d "$d/core" ] || [ -d "$d/.git" ]; then repo="$d"; break; fi
done

# Warm build cache? (cargo target present somewhere we'd reuse)
cache=no
if [ -n "$repo" ]; then
  if [ -d "$repo/core/target" ] || [ -d "$repo/target" ] || [ -d /root/pm-wsl-target ]; then
    cache=yes
  fi
fi

printf 'HOST=%s\n' "$host"
printf 'AIS=%s\n' "$ais"
printf 'CAPS=%s\n' "$caps"
printf 'REPO=%s\n' "$repo"
printf 'CACHE=%s\n' "$cache"
