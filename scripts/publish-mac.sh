#!/usr/bin/env bash
# One-shot Mac release publisher — mirrors the Windows iwr|iex flow.
#
# What it does, in order:
#   1. Build (delegates to scripts/build-mac.sh — codesigns + mirrors to dist/)
#   2. Upload dist/phantom-aarch64-apple-darwin → R2 phantom-binaries/phantom-darwin-arm64
#   3. Deploy phantommesh-io Worker (registers /install.sh + Mac entry in /dist/*)
#   4. Verify via curl that /install.sh + /dist/phantom-darwin-arm64 are live
#
# After step 4, anyone on macOS (M-series) can:
#   curl -fsSL https://phantommesh.io/install.sh | sh
#
# Usage:
#   ./scripts/publish-mac.sh                  # full flow
#   SKIP_BUILD=1 ./scripts/publish-mac.sh     # reuse existing dist/ binary
#   SKIP_DEPLOY=1 ./scripts/publish-mac.sh    # binary upload only (worker stays as-is)
#
# Requires:
#   - npx wrangler authenticated (npx wrangler whoami)
#   - codesign available (Xcode CLT)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

BIN_LOCAL="dist/phantom-aarch64-apple-darwin"
BIN_R2_KEY="phantom-darwin-arm64"
PMIO_DIR="phantommesh-io"
PUBLIC_HOST="https://phantommesh.io"

step() { printf "\n\033[35m── %s ──\033[0m\n" "$1"; }
ok()   { printf "  \033[32m✓\033[0m %s\n" "$1"; }
fail() { printf "  \033[31m✗\033[0m %s\n" "$1"; exit 1; }

# ── 1. Build ──────────────────────────────────────────────────────────────
if [ "${SKIP_BUILD:-0}" = "1" ]; then
  step "1. build (skipped — SKIP_BUILD=1)"
  [ -x "$BIN_LOCAL" ] || fail "$BIN_LOCAL missing — run without SKIP_BUILD first"
else
  step "1. build via scripts/build-mac.sh"
  ./scripts/build-mac.sh
fi
SIZE_MB=$(( $(wc -c < "$BIN_LOCAL") / 1024 / 1024 ))
ok "$BIN_LOCAL ($SIZE_MB MB)"

# ── 2. Upload to R2 ───────────────────────────────────────────────────────
step "2. upload to R2 (phantom-binaries/$BIN_R2_KEY)"
npx --prefix "$PMIO_DIR" wrangler r2 object put \
  "phantom-binaries/$BIN_R2_KEY" \
  --file "$BIN_LOCAL" \
  --content-type application/octet-stream
ok "uploaded"

# ── 3. Deploy Worker ──────────────────────────────────────────────────────
if [ "${SKIP_DEPLOY:-0}" = "1" ]; then
  step "3. worker deploy (skipped — SKIP_DEPLOY=1)"
else
  step "3. deploy phantommesh.io Worker"
  ( cd "$PMIO_DIR" && npx wrangler deploy )
  ok "worker deployed"
fi

# ── 4. Verify ─────────────────────────────────────────────────────────────
step "4. verify live endpoints"

if curl -sSI "$PUBLIC_HOST/install.sh" 2>/dev/null | grep -q "^HTTP/.* 200"; then
  ok "GET $PUBLIC_HOST/install.sh → 200"
else
  fail "GET $PUBLIC_HOST/install.sh did not return 200"
fi

# /dist may take a few seconds to propagate after upload
for i in 1 2 3; do
  HEAD=$(curl -sSI "$PUBLIC_HOST/dist/$BIN_R2_KEY" 2>/dev/null)
  if echo "$HEAD" | grep -q "^HTTP/.* 200"; then
    LEN=$(echo "$HEAD" | grep -i "^content-length" | awk '{print $2}' | tr -d '\r')
    ETAG=$(echo "$HEAD" | grep -i "^etag" | awk '{print $2}' | tr -d '\r')
    ok "GET $PUBLIC_HOST/dist/$BIN_R2_KEY → 200 (Content-Length=$LEN, ETag=$ETAG)"
    break
  fi
  if [ $i -eq 3 ]; then
    fail "GET $PUBLIC_HOST/dist/$BIN_R2_KEY did not return 200 after 3 attempts (CDN may need a manual purge)"
  fi
  sleep 2
done

step "done"
echo "  Anyone on macOS arm64 can now install with:"
echo "    curl -fsSL $PUBLIC_HOST/install.sh | sh"
