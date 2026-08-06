#!/bin/bash
# Run this before making the repo public.
# Prints PASS/FAIL for each check and a final go/no-go verdict.

set -uo pipefail

GREEN='\033[0;32m'
RED='\033[0;31m'
NC='\033[0m' # no colour

PASS=0
FAIL=0
TOTAL=13

pass() { echo -e "  ${GREEN}PASS${NC}  $1"; ((PASS++)); }
fail() { echo -e "  ${RED}FAIL${NC}  $1"; ((FAIL++)); }

echo "==> spectyn-mesh pre-open-source checklist"
echo ""

# ── 1. No inline API keys in tracked files ───────────────────
# Exclude example/doc files that intentionally show placeholder patterns,
# and exclude the two scripts that contain the patterns as strings to match.
result=$(git grep -E "(sk-ant-|sk-or-v1-|AIzaSy|gsk_|GOCSPX-)" \
  -- \
  ":(exclude)*.example" \
  ":(exclude)CHANGELOG*" \
  ":(exclude)scripts/clean-history.sh" \
  ":(exclude)scripts/pre-open-source-checklist.sh" \
  ":(exclude)SECURITY.md" \
  ":(exclude)docs/**" \
  2>/dev/null \
  | grep -v '\.\.\.' \
  | grep -v 'YOUR_' \
  | grep -v 'test-secret\|example-key' \
  | grep -v '\*\*\*\*' \
  | grep -v 'starts_with\|startsWith\|\.gitleaks\.' \
  || true)
if [ -z "$result" ]; then
  pass "No inline API keys in tracked files"
else
  fail "Inline API keys found in tracked files:"
  echo "$result" | sed 's/^/         /'
fi

# ── 2. agents.toml is in .gitignore ──────────────────────────
if grep -qxF 'agents.toml' .gitignore 2>/dev/null || \
   grep -q 'agents\.toml' .gitignore 2>/dev/null; then
  pass "agents.toml is in .gitignore"
else
  fail "agents.toml is NOT in .gitignore — add: echo 'agents.toml' >> .gitignore"
fi

# ── 3. agents.toml.example exists and has no real keys ───────
if [ ! -f agents.toml.example ]; then
  fail "agents.toml.example does not exist"
else
  # Only flag lines that look like real keys (not placeholder ...  or YOUR_ values)
  keys=$(grep -E "(sk-ant-|sk-or-v1-|AIzaSy|gsk_|GOCSPX-)" agents.toml.example \
    | grep -v '\.\.\.' | grep -v 'YOUR_' || true)
  if [ -n "$keys" ]; then
    fail "agents.toml.example contains what look like real API keys"
    echo "$keys" | sed 's/^/         /'
  else
    pass "agents.toml.example exists and has no real keys"
  fi
fi

# ── 4. SPECTYN.md is not empty (more than 5 lines) ───────────
if [ -f SPECTYN.md ] && [ "$(wc -l < SPECTYN.md)" -gt 5 ]; then
  pass "SPECTYN.md exists and has content ($(wc -l < SPECTYN.md) lines)"
else
  fail "SPECTYN.md is missing or too short (need > 5 lines)"
fi

# ── 5. README.md is not a placeholder (more than 50 lines) ───
if [ -f README.md ] && [ "$(wc -l < README.md)" -gt 50 ]; then
  pass "README.md exists and is substantive ($(wc -l < README.md) lines)"
else
  fail "README.md is missing or too short (need > 50 lines)"
fi

# ── 6. CHANGELOG.md exists ───────────────────────────────────
if [ -f CHANGELOG.md ]; then
  pass "CHANGELOG.md exists"
else
  fail "CHANGELOG.md does not exist — create it before launch"
fi

# ── 7. RELEASE-NOTES.md exists ───────────────────────────────
if [ -f RELEASE-NOTES.md ]; then
  pass "RELEASE-NOTES.md exists"
else
  fail "RELEASE-NOTES.md does not exist — create it before launch"
fi

# ── 8. docs/GETTING-STARTED.md exists ────────────────────────
if [ -f docs/GETTING-STARTED.md ]; then
  pass "docs/GETTING-STARTED.md exists"
else
  fail "docs/GETTING-STARTED.md does not exist — create it before launch"
fi

# ── 9. core builds (cargo build --release) ───────────────────
CARGO_DIR=""
if [ -f Cargo.toml ]; then
  CARGO_DIR="."
elif [ -f core/Cargo.toml ]; then
  CARGO_DIR="core"
elif [ -f crates/Cargo.toml ]; then
  CARGO_DIR="crates"
fi

if [ -z "$CARGO_DIR" ]; then
  fail "No Cargo.toml found in root, core/, or crates/ — cannot verify core build"
else
  echo "  ...  running cargo build --release in $CARGO_DIR/ (this may take a moment)..."
  if (cd "$CARGO_DIR" && cargo build --release 2>/dev/null); then
    pass "cargo build --release succeeded (in $CARGO_DIR/)"
  else
    fail "cargo build --release FAILED in $CARGO_DIR/ — fix build errors before launch"
  fi
fi

# ── 10. No TODO/FIXME/YOUR_* in scripts/deploy-gcp.sh ────────
if [ ! -f scripts/deploy-gcp.sh ]; then
  fail "scripts/deploy-gcp.sh does not exist"
else
  issues=$(grep -nE "(TODO|FIXME|YOUR_)" scripts/deploy-gcp.sh || true)
  if [ -z "$issues" ]; then
    pass "No TODO/FIXME/YOUR_* placeholders in scripts/deploy-gcp.sh"
  else
    fail "Unresolved placeholders in scripts/deploy-gcp.sh:"
    echo "$issues" | sed 's/^/         /'
  fi
fi

# ── 11. LICENSE file exists ───────────────────────────────────
if [ -f LICENSE ]; then
  pass "LICENSE file exists"
else
  fail "LICENSE file does not exist — add the AGPL-3.0 core license before launch"
fi

# ── 12. SECURITY.md exists ───────────────────────────────────
if [ -f SECURITY.md ]; then
  pass "SECURITY.md exists"
else
  fail "SECURITY.md does not exist — add a vulnerability disclosure policy before launch"
fi

# ── 13. .github/workflows/security.yml exists ────────────────
if [ -f .github/workflows/security.yml ]; then
  pass ".github/workflows/security.yml exists"
else
  fail ".github/workflows/security.yml does not exist — set up security scanning CI"
fi

# ── Summary ──────────────────────────────────────────────────
echo ""
echo "──────────────────────────────────────────"
if [ "$FAIL" -eq 0 ]; then
  READY="YES"
  COLOR=$GREEN
else
  READY="NO"
  COLOR=$RED
fi
echo -e "  ${PASS}/${TOTAL} checks passed.  Ready to launch: ${COLOR}${READY}${NC}"
echo "──────────────────────────────────────────"

[ "$FAIL" -eq 0 ] || exit 1
