#!/usr/bin/env bash
# scripts/_verify-download.sh
#
# Shared shell helpers for SHA256-verified binary downloads in install scripts.
# Sourced (not executed) by install-mac.sh, termux-setup.sh, setup-cloud-linux.sh,
# update-daemon.sh, and dev/deploy-gcp.sh.
#
# Functions provided:
#   require_https <url>          — exit 1 if URL is plain http:// (unless
#                                  PHANTOM_ALLOW_INSECURE=1 is set)
#   sha256_local  <path>         — print lowercase hex sha256 of a local file
#                                  (uses sha256sum on Linux/Termux, shasum on Mac)
#   verify_sha256 <bin> <url>    — download <url>.sha256 over HTTPS, compare
#                                  against sha256_local <bin>, delete the
#                                  binary + exit 1 on mismatch
#
# Threat model: see docs/install-binary-verification.md
#
# Env opt-outs (use with extreme caution):
#   PHANTOM_ALLOW_INSECURE=1  — allow plain http:// download URLs
#   PHANTOM_SKIP_VERIFY=1     — skip SHA256 verification entirely (loudly warns)

set -u

# Detect which sha256 tool is available. We do this once per source.
_PHANTOM_SHA256_TOOL=""
if command -v sha256sum >/dev/null 2>&1; then
  _PHANTOM_SHA256_TOOL="sha256sum"
elif command -v shasum >/dev/null 2>&1; then
  _PHANTOM_SHA256_TOOL="shasum -a 256"
else
  echo "    ✗ no sha256sum / shasum found — install coreutils or perl-digest-sha256" >&2
fi

require_https() {
  # Usage: require_https <url>
  local url="$1"
  case "$url" in
    https://*)
      return 0
      ;;
    http://*)
      if [ "${PHANTOM_ALLOW_INSECURE:-0}" = "1" ]; then
        echo "    ⚠ PHANTOM_ALLOW_INSECURE=1 — accepting plain http:// URL ($url)" >&2
        echo "      THIS DISABLES MITM PROTECTION. Set PHANTOM_ALLOW_INSECURE=0 to" >&2
        echo "      force HTTPS." >&2
        return 0
      fi
      echo "    ✗ Refusing to download binary over plain http://" >&2
      echo "      URL: $url" >&2
      echo "      Use an https:// URL, or set PHANTOM_ALLOW_INSECURE=1 explicitly" >&2
      echo "      (only safe on a trusted tailnet — see" >&2
      echo "      docs/install-binary-verification.md)." >&2
      return 1
      ;;
    *)
      echo "    ✗ Unsupported URL scheme: $url" >&2
      return 1
      ;;
  esac
}

sha256_local() {
  # Usage: sha256_local <path>
  # Prints lowercase hex sha256 to stdout, nothing else.
  local path="$1"
  if [ -z "$_PHANTOM_SHA256_TOOL" ]; then
    echo "    ✗ sha256 tool not available — cannot verify $path" >&2
    return 1
  fi
  $_PHANTOM_SHA256_TOOL "$path" | awk '{print tolower($1)}'
}

verify_sha256() {
  # Usage: verify_sha256 <local-binary> <download-url>
  # Downloads "<download-url>.sha256", compares its first whitespace-delimited
  # field against sha256_local <local-binary>. On mismatch: delete the binary
  # and return 1. On match: return 0.
  local bin="$1"
  local url="$2"
  local sums_url="${url}.sha256"

  if [ "${PHANTOM_SKIP_VERIFY:-0}" = "1" ]; then
    echo "    ⚠ PHANTOM_SKIP_VERIFY=1 — SKIPPING SHA256 verification of $bin" >&2
    echo "      This means a MITM or compromised mirror can replace the" >&2
    echo "      phantom binary. Do not use except on an air-gapped first install" >&2
    echo "      where the sums file isn't published yet." >&2
    return 0
  fi

  if [ ! -f "$bin" ]; then
    echo "    ✗ verify_sha256: local binary not found: $bin" >&2
    return 1
  fi

  if ! require_https "$sums_url"; then
    rm -f "$bin"
    return 1
  fi

  local sums_file
  sums_file="$(mktemp -t phantom-sha256.XXXXXX 2>/dev/null || mktemp)"
  # Fail-closed: any download error (network, 404, redirect loop) deletes the
  # binary and aborts. We use --fail to turn HTTP 4xx/5xx into a non-zero exit.
  if ! curl -fsSL --max-time 30 "$sums_url" -o "$sums_file"; then
    echo "    ✗ Could not fetch SHA256 sidecar at $sums_url" >&2
    echo "      Refusing to install an unverified binary." >&2
    echo "      If you genuinely need to bypass this, set PHANTOM_SKIP_VERIFY=1" >&2
    echo "      (NOT recommended)." >&2
    rm -f "$sums_file" "$bin"
    return 1
  fi

  # The sidecar might be either:
  #   - a single line "<hex>  <filename>"  (sha256sum format)
  #   - a single line of just "<hex>"
  # Take the first whitespace-delimited field of the first non-empty line.
  local expected
  expected="$(awk 'NF { print tolower($1); exit }' "$sums_file")"
  rm -f "$sums_file"

  if [ -z "$expected" ] || ! echo "$expected" | grep -Eq '^[0-9a-f]{64}$'; then
    echo "    ✗ SHA256 sidecar at $sums_url is malformed (got: $expected)" >&2
    rm -f "$bin"
    return 1
  fi

  local actual
  actual="$(sha256_local "$bin")" || { rm -f "$bin"; return 1; }

  if [ "$expected" != "$actual" ]; then
    echo "    ✗ SHA256 mismatch for $bin" >&2
    echo "      expected: $expected" >&2
    echo "      actual:   $actual" >&2
    echo "      Source:   $sums_url" >&2
    echo "      The downloaded binary has been deleted." >&2
    rm -f "$bin"
    return 1
  fi

  echo "    ✓ sha256 verified ($expected)"
  return 0
}
