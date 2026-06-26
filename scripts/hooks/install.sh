#!/usr/bin/env bash
# Install phantom-mesh git hooks into .git/hooks/ (symlink so updates auto-apply).
# Run once per clone: bash scripts/hooks/install.sh
set -uo pipefail
ROOT="$(git rev-parse --show-toplevel 2>/dev/null)" || { echo "not a git repo"; exit 1; }
cd "$ROOT" || exit 1

mkdir -p .git/hooks
for h in pre-commit pre-push; do
  src="scripts/hooks/$h"
  dst=".git/hooks/$h"
  [ -f "$src" ] || continue
  chmod +x "$src"
  ln -sf "../../$src" "$dst"
  echo "✓ installed $dst → $src"
done
echo "Done. Bypass a single commit with: git commit --no-verify"
