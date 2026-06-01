#!/bin/sh
# scripts/install.sh — F500 unified first-touch installer for phantom-mesh.
#
# This is the "stranger-friendly" entry point served at
#   https://phantommesh.io/install
#
# Use:
#   curl -fsSL https://phantommesh.io/install | sh
#
# What it does (no questions asked):
#   1. Detects OS (Linux / macOS) + arch (x86_64 / aarch64).
#   2. Maps to the matching R2 binary name (see
#      .github/workflows/publish-phantom-binary.yml for the publish side).
#   3. Downloads the binary to a tmpdir.
#   4. Verifies SHA256 via the shared _verify-download.sh helper (the
#      F-CRIT-3 / PR #111 contract: fail-closed on missing sidecar or
#      mismatch, refuse plain http://).
#   5. Installs to $HOME/.phantom-mesh/bin/phantom (matches what
#      `phantom service install` expects on every OS).
#   6. Best-effort symlink into $HOME/.local/bin/phantom and a PATH hint
#      in the user's shell rc — never breaks the install if either fails.
#   7. Prints exactly one final line: `Run \`phantom\` to start.`
#
# What it does NOT do:
#   - sudo (default install path is per-user, never /usr/local/bin).
#   - eval any downloaded data.
#   - touch ~/.phantom-mesh/agents.toml — F501 wizard owns provider config.
#   - bootstrap a cluster — that's still install-mac.sh / install-phantom-windows.ps1.
#
# Env knobs:
#   PHANTOM_INSTALL_BASE      Base URL to fetch from. Default:
#                             https://phantommesh.io. Set to
#                             https://staging.example.com (or similar) for
#                             pre-L1 testing.
#   PHANTOM_INSTALL_DRY_RUN   If 1: print detected OS/arch and the URL we
#                             would download, then exit 0 without writing
#                             anything to disk.
#   PHANTOM_ALLOW_INSECURE    See _verify-download.sh — opt out of HTTPS.
#   PHANTOM_SKIP_VERIFY       See _verify-download.sh — opt out of SHA256.
#                             Both are NOISY warnings, NOT silent.
#
# F-CRIT-3 invariants preserved:
#   - HTTPS only (require_https) — refuses plain http:// downloads.
#   - SHA256 sidecar verified BEFORE chmod +x or move into PATH.
#   - Fail-closed on missing sidecar (PHANTOM_SKIP_VERIFY=1 to override,
#     prints a loud stderr warning).
#
# POSIX bash compatible (no bash 5+ features, runs under dash too).

set -eu

# ── Config ─────────────────────────────────────────────────────────────────
INSTALL_BASE="${PHANTOM_INSTALL_BASE:-https://phantommesh.io}"
# Strip trailing slash for clean concatenation.
INSTALL_BASE="${INSTALL_BASE%/}"
DIST_BASE="$INSTALL_BASE/dist"

CFG_DIR="$HOME/.phantom-mesh"
INSTALL_DIR="$CFG_DIR/bin"
TARGET_BIN="$INSTALL_DIR/phantom"
LINK_DIR="$HOME/.local/bin"
LINK_PATH="$LINK_DIR/phantom"

DRY_RUN="${PHANTOM_INSTALL_DRY_RUN:-0}"

# ── Logging ────────────────────────────────────────────────────────────────
log()  { printf '  %s\n' "$*"; }
warn() { printf '  ! %s\n' "$*" >&2; }
err()  { printf '  x %s\n' "$*" >&2; }

# ── Detect OS + arch ───────────────────────────────────────────────────────
detect_target() {
  # Sets: OS_KIND (linux|darwin), ARCH_KIND (x86_64|aarch64),
  #       R2_OBJECT (the R2 object name, see publish-phantom-binary.yml).
  uname_s="$(uname -s 2>/dev/null || echo unknown)"
  uname_m="$(uname -m 2>/dev/null || echo unknown)"

  case "$uname_s" in
    Linux)  OS_KIND="linux" ;;
    Darwin) OS_KIND="darwin" ;;
    *)
      err "Unsupported OS: $uname_s"
      err "  This installer supports Linux and macOS only."
      err "  Windows users: use the install.ps1 one-liner instead:"
      err "    irm https://phantommesh.io/install.ps1 | iex"
      exit 1
      ;;
  esac

  case "$uname_m" in
    x86_64|amd64) ARCH_KIND="x86_64" ;;
    aarch64|arm64) ARCH_KIND="aarch64" ;;
    *)
      err "Unsupported arch: $uname_m"
      err "  Supported: x86_64 (Linux), aarch64 (Linux + macOS Apple Silicon)."
      exit 1
      ;;
  esac

  # Map to the R2 object name that publish-phantom-binary.yml uploads.
  # Spec (F500): linux-x86_64, aarch64-unknown-linux-gnu, aarch64-apple-darwin.
  # Intel Macs intentionally excluded (matches install-mac.sh).
  case "$OS_KIND-$ARCH_KIND" in
    linux-x86_64)   R2_OBJECT="phantom-linux-x86_64" ;;
    linux-aarch64)  R2_OBJECT="phantom-aarch64-unknown-linux-gnu" ;;
    darwin-aarch64) R2_OBJECT="phantom-aarch64-apple-darwin" ;;
    darwin-x86_64)
      err "Intel Macs are not supported (Apple Silicon only)."
      err "  See scripts/install-mac.sh, which makes the same call."
      exit 1
      ;;
    *)
      err "No prebuilt binary for $OS_KIND-$ARCH_KIND."
      exit 1
      ;;
  esac
}

# ── Load shared verifier helpers (F-CRIT-3) ───────────────────────────────
# We need require_https + verify_sha256 from _verify-download.sh. When piped
# via `curl | sh` we don't have a local scripts/ dir, so fetch the helper
# from the same base URL we're already trusting for the binary.
# Fail-closed if it can't be loaded (PHANTOM_SKIP_VERIFY=1 still requires
# the helper — it only changes the helper's behaviour).
load_verifier() {
  VERIFY_HELPER="$(mktemp 2>/dev/null || mktemp -t phantom-verify)"
  # Prefer a local copy if we have one (developer running from a checkout).
  SCRIPT_DIR=""
  case "${0:-}" in
    */*) SCRIPT_DIR="$(cd "$(dirname "$0")" 2>/dev/null && pwd 2>/dev/null || true)" ;;
  esac

  if [ -n "$SCRIPT_DIR" ] && [ -f "$SCRIPT_DIR/_verify-download.sh" ]; then
    cp "$SCRIPT_DIR/_verify-download.sh" "$VERIFY_HELPER"
  else
    HELPER_URL="$INSTALL_BASE/scripts/_verify-download.sh"
    if ! curl -fsSL --max-time 10 "$HELPER_URL" -o "$VERIFY_HELPER" 2>/dev/null; then
      err "Could not load $HELPER_URL"
      err "  Refusing to download a binary without the verifier."
      err "  If the R2 publish step has not run yet, see:"
      err "    docs/superpowers/runbooks/L1-cloudflare-creds.md"
      rm -f "$VERIFY_HELPER"
      exit 1
    fi
  fi
  # shellcheck disable=SC1090
  . "$VERIFY_HELPER"
  rm -f "$VERIFY_HELPER"
}

# ── Friendly 404 / network failure handling ───────────────────────────────
# Tells the user *what* to do, not just *that* it failed. Per F500 spec L1
# dep — operator may not have published the binary yet.
fail_missing_binary() {
  url="$1"
  err "Could not download $url"
  err ""
  err "  Most likely cause: the operator has not yet published this target"
  err "  to the R2 bucket. The L1 'Publish phantom binary to R2' workflow"
  err "  needs to run for $R2_OBJECT."
  err ""
  err "  If you ARE the operator: follow"
  err "    docs/superpowers/runbooks/L1-cloudflare-creds.md"
  err "  to add CLOUDFLARE_API_TOKEN + CLOUDFLARE_ACCOUNT_ID secrets and"
  err "  trigger:"
  err "    https://github.com/markl-a/phantom-mesh/actions/workflows/publish-phantom-binary.yml"
  err ""
  err "  In the meantime you can build from source:"
  err "    cargo install --git https://github.com/markl-a/phantom-mesh --bin phantom"
  exit 1
}

# ── Best-effort PATH wiring ───────────────────────────────────────────────
# Drops a symlink into ~/.local/bin (already on PATH for most modern
# distros + macOS via /etc/paths.d). If we cannot symlink, print a PATH
# hint. Never errors — install is already complete by this point.
wire_path() {
  bin="$1"
  mkdir -p "$LINK_DIR" 2>/dev/null || true
  if [ -d "$LINK_DIR" ]; then
    # Use ln -sf so re-runs upgrade cleanly.
    if ln -sf "$bin" "$LINK_PATH" 2>/dev/null; then
      log "linked $LINK_PATH -> $bin"
    else
      # Some filesystems (e.g. mounted SMB on macOS) refuse symlinks; fall
      # back to a copy.
      if cp "$bin" "$LINK_PATH" 2>/dev/null; then
        log "copied $bin -> $LINK_PATH"
      fi
    fi
  fi

  # Is $LINK_DIR actually on PATH?
  case ":$PATH:" in
    *":$LINK_DIR:"*) return 0 ;;
  esac

  warn "$LINK_DIR is not on your PATH."
  warn "  Add it with:"
  warn "    echo 'export PATH=\"\$HOME/.local/bin:\$PATH\"' >> ~/.bashrc"
  warn "  Then: source ~/.bashrc  (or restart your shell)"
}

# ── Main ──────────────────────────────────────────────────────────────────
main() {
  detect_target

  BIN_URL="$DIST_BASE/$R2_OBJECT"

  if [ "$DRY_RUN" = "1" ]; then
    printf '%s\n' "phantom-mesh installer (dry run)"
    printf '  detected OS:   %s\n' "$OS_KIND"
    printf '  detected arch: %s\n' "$ARCH_KIND"
    printf '  R2 object:     %s\n' "$R2_OBJECT"
    printf '  base URL:      %s\n' "$INSTALL_BASE"
    printf '  would download: %s\n' "$BIN_URL"
    printf '  would verify:  %s.sha256\n' "$BIN_URL"
    printf '  would install: %s\n' "$TARGET_BIN"
    printf '  would symlink: %s -> %s\n' "$LINK_PATH" "$TARGET_BIN"
    printf '\n'
    printf '  PHANTOM_INSTALL_DRY_RUN=1 — no files written.\n'
    exit 0
  fi

  if ! command -v curl >/dev/null 2>&1; then
    err "curl not found — install curl and retry."
    exit 1
  fi

  log "phantom-mesh installer"
  log "  target: $R2_OBJECT"

  load_verifier

  # F-CRIT-3: require_https refuses plain http:// unless the operator
  # explicitly opts out with PHANTOM_ALLOW_INSECURE=1.
  require_https "$BIN_URL" || exit 1

  mkdir -p "$INSTALL_DIR"

  # Download to a tmpdir so a mid-stream failure cannot leave a broken
  # binary in PATH.
  TMP_DIR="$(mktemp -d 2>/dev/null || mktemp -d -t phantom-installer)"
  trap 'rm -rf "$TMP_DIR"' EXIT INT TERM
  TMP_BIN="$TMP_DIR/phantom"

  log "downloading $BIN_URL ..."
  if ! curl -fsSL --max-time 120 "$BIN_URL" -o "$TMP_BIN"; then
    fail_missing_binary "$BIN_URL"
  fi

  # F-CRIT-3: verify SHA256 BEFORE chmod +x or move into PATH.
  # verify_sha256 deletes $TMP_BIN on mismatch and returns non-zero so
  # `set -e` aborts.
  verify_sha256 "$TMP_BIN" "$BIN_URL"

  chmod +x "$TMP_BIN"
  mv "$TMP_BIN" "$TARGET_BIN"

  wire_path "$TARGET_BIN"

  # Per F500 spec: last stdout line must be EXACTLY this so the wizard
  # (F501) takes over cleanly on next `phantom` invocation. No banners,
  # no extra prose.
  printf 'Run `phantom` to start.\n'
}

main "$@"
