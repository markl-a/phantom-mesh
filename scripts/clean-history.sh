#!/bin/bash
set -euo pipefail

# ============================================================
# WARNING: THIS SCRIPT REWRITES ALL GIT HISTORY
#
# - Every commit SHA will change
# - All collaborators MUST delete their local clone and re-clone
# - All open PRs will be broken
# - This cannot be undone (unless you restore from the backup
#   this script creates)
#
# Patterns that will be scrubbed:
#   sk-ant-    (Anthropic API keys)
#   sk-or-v1-  (OpenRouter API keys)
#   AIzaSy     (Google / Gemini API keys)
#   gsk_       (Groq API keys)
#   GOCSPX-    (Google OAuth client secrets)
#
# After running you MUST force-push:
#   git push --force-with-lease origin main
# ============================================================

DRY_RUN=false
FORCE=false

usage() {
  echo "Usage: $0 [--dry-run] [--force]"
  echo ""
  echo "  --dry-run   Show which patterns would be replaced without changing history"
  echo "  --force     Actually rewrite history (required to make changes)"
  echo ""
  echo "Without --force the script always behaves as a dry run."
  exit 0
}

for arg in "$@"; do
  case "$arg" in
    --dry-run) DRY_RUN=true ;;
    --force)   FORCE=true ;;
    --help|-h) usage ;;
    *) echo "Unknown argument: $arg"; usage ;;
  esac
done

# Without --force, behave as dry run regardless
if [ "$FORCE" = false ]; then
  DRY_RUN=true
fi

echo "============================================================"
echo "  phantom-mesh git history cleaner"
if [ "$DRY_RUN" = true ]; then
  echo "  MODE: DRY RUN — no history will be modified"
else
  echo "  MODE: LIVE — history WILL be rewritten"
fi
echo "============================================================"
echo ""

# ── Check git-filter-repo is installed ───────────────────────
if ! command -v git-filter-repo &>/dev/null; then
  echo "ERROR: git-filter-repo is not installed."
  echo ""
  echo "Install it with one of:"
  echo "  pip3 install git-filter-repo"
  echo "  brew install git-filter-repo          # macOS Homebrew"
  echo "  sudo apt install git-filter-repo      # Debian/Ubuntu"
  echo "  sudo pacman -S git-filter-repo        # Arch Linux"
  exit 1
fi

# ── Patterns that will be scrubbed ───────────────────────────
echo "Patterns that will be scrubbed from all commits:"
echo "  sk-ant-    →  REMOVED_  (Anthropic API keys)"
echo "  sk-or-v1-  →  REMOVED_  (OpenRouter API keys)"
echo "  AIzaSy     →  REMOVED_  (Google / Gemini API keys)"
echo "  gsk_       →  REMOVED_  (Groq API keys)"
echo "  GOCSPX-    →  REMOVED_  (Google OAuth client secrets)"
echo ""

# ── Dry-run: scan history for matches ────────────────────────
echo "==> Scanning history for matches..."
PATTERN='sk-ant-|sk-or-v1-|AIzaSy|gsk_|GOCSPX-'
MATCHES=$(git log --all -p 2>/dev/null | grep -E "$PATTERN" | grep -v '^Binary' || true)

if [ -z "$MATCHES" ]; then
  echo "  No matches found in git history. History is clean."
else
  echo "  Matches found:"
  git log --all -p 2>/dev/null \
    | grep -E "$PATTERN" \
    | grep -v '^Binary' \
    | sed 's/^/    /' \
    | head -50
  MATCH_COUNT=$(git log --all -p 2>/dev/null | grep -cE "$PATTERN" || true)
  echo "  Total matching lines: $MATCH_COUNT"
fi
echo ""

if [ "$DRY_RUN" = true ]; then
  echo "DRY RUN complete. To actually rewrite history run:"
  echo "  bash $0 --force"
  exit 0
fi

# ── Live mode: confirm and backup ────────────────────────────
echo "============================================================"
echo "  WARNING: You are about to REWRITE ALL GIT HISTORY."
echo "  All commit SHAs will change. This is destructive."
echo "  Collaborators must delete their local clones and re-clone."
echo "============================================================"
echo ""
read -r -p "Type YES to confirm and proceed: " CONFIRM
[[ "$CONFIRM" == "YES" ]] || { echo "Aborted."; exit 1; }

# Create a bundle backup before touching anything
BACKUP_FILE="../phantom-mesh-backup-$(date +%Y%m%d).bundle"
echo ""
echo "==> Creating backup bundle at $BACKUP_FILE ..."
git bundle create "$BACKUP_FILE" --all
echo "    Backup created: $BACKUP_FILE"
echo ""

# ── Build replacements file ───────────────────────────────────
TMPFILE=$(mktemp)
cat > "$TMPFILE" << 'REPLACEMENTS'
sk-ant-==>REMOVED_
sk-or-v1-==>REMOVED_
AIzaSy==>REMOVED_
gsk_==>REMOVED_
GOCSPX-==>REMOVED_
REPLACEMENTS

# ── Run filter-repo ───────────────────────────────────────────
echo "==> Running git-filter-repo to scrub secrets..."
git filter-repo \
  --replace-text "$TMPFILE" \
  --force

rm -f "$TMPFILE"

echo ""
echo "==> History cleaned!"
echo ""
echo "IMPORTANT next steps:"
echo "  1. Verify the result:    git log --all --oneline | head -20"
echo "  2. Force-push:           git push --force-with-lease origin main"
echo "  3. Notify collaborators: they must delete local clones and re-clone"
echo "  4. Revoke ALL previously exposed keys immediately:"
echo "       - Anthropic API keys  (console.anthropic.com)"
echo "       - OpenRouter API keys (openrouter.ai/keys)"
echo "       - Google / Gemini API keys (console.cloud.google.com)"
echo "       - Groq API keys       (console.groq.com/keys)"
echo "       - Google OAuth secrets (console.cloud.google.com → Credentials)"
echo "  5. Generate new keys for every service above"
