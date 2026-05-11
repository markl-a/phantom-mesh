#!/usr/bin/env bash
# Linux release-build helper for phantom-mesh.
#
# Per SPEC-FREEZE-V1 §6.1, the canonical Linux artefact is built on the
# target machine itself (option C: target = self) — no cross-toolchain.
# Final shape is Oracle Cloud A1 ARM (`aarch64-unknown-linux-gnu`).
#
# What this does, in order:
#   1. Detect host arch from `uname -m` (override via TARGET=<triple>)
#   2. cargo build --release --bin phantom (using the system toolchain)
#   3. Smoke-verify by running `phantom --version` with a 5s timeout
#   4. Mirror into dist/phantom-<triple> for setup-oci.sh / phantom upgrade
#
# Usage:
#   ./scripts/build-linux.sh                 # builds for host arch
#   TARGET=aarch64-unknown-linux-gnu ./scripts/build-linux.sh
#   TARGET=x86_64-unknown-linux-gnu  ./scripts/build-linux.sh
#
# Memory note: full release build needs ~1.5 GB RAM during the link
# phase. On <2 GB hosts (e.g. Oracle E2.1.Micro 1 GB) this will OOM.
# Either add swap (setup-oci.sh §swap) or build elsewhere and scp the
# binary into dist/ manually.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CARGO_DIR="$REPO_ROOT/core"
cd "$CARGO_DIR"

# ── 0. host sanity ──────────────────────────────────────────────────────
if [ "$(uname -s)" != "Linux" ]; then
  echo "✗ build-linux.sh must run on Linux — host is $(uname -s)"
  exit 1
fi

# ── 1. detect target ────────────────────────────────────────────────────
if [ -z "${TARGET:-}" ]; then
  case "$(uname -m)" in
    aarch64|arm64)  TARGET="aarch64-unknown-linux-gnu" ;;
    x86_64|amd64)   TARGET="x86_64-unknown-linux-gnu" ;;
    *)              echo "✗ unsupported host arch: $(uname -m)"; exit 1 ;;
  esac
fi

if [ "$TARGET" != "aarch64-unknown-linux-gnu" ]; then
  echo "  ⚠ target $TARGET is not SPEC-FREEZE-V1 §6.1 canonical (aarch64)."
  echo "    This binary will not match the release matrix; use only for"
  echo "    scaffolding hosts (e.g. Oracle E2.1.Micro AMD)."
  echo
fi

# ── 2. memory pre-flight ────────────────────────────────────────────────
RAM_KB=$(awk '/^MemTotal:/ {print $2}' /proc/meminfo)
RAM_MB=$(( RAM_KB / 1024 ))
if [ "$RAM_MB" -lt 1800 ]; then
  echo "  ⚠ host has ${RAM_MB} MB RAM — release link phase typically needs"
  echo "    ~1500 MB. Add swap before continuing or expect OOM-kill:"
  echo "      sudo fallocate -l 2G /swapfile && sudo chmod 600 /swapfile"
  echo "      sudo mkswap /swapfile && sudo swapon /swapfile"
  echo
fi

# ── 3. toolchain check ──────────────────────────────────────────────────
if ! command -v cargo >/dev/null 2>&1; then
  echo "✗ cargo not found. Install rust:"
  echo "    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
  exit 1
fi

echo "  ◆ phantom-mesh — Linux release build"
echo "    target : $TARGET"
echo "    repo   : $REPO_ROOT"
echo "    ram    : ${RAM_MB} MB"
echo "    rustc  : $(rustc --version 2>/dev/null || echo '(missing)')"
echo

# ── 4. cargo build ──────────────────────────────────────────────────────
# Use the system toolchain (target=host); no rustup target install needed.
echo "  [1/3] cargo build --release --bin phantom"
cargo build --release --bin phantom

OUT_BIN="$CARGO_DIR/target/release/phantom"
if [ ! -x "$OUT_BIN" ]; then
  echo "  ✗ binary missing at $OUT_BIN"
  exit 1
fi
echo "    ✓ built $OUT_BIN ($(du -h "$OUT_BIN" | awk '{print $1}'))"

# ── 5. smoke-verify launch ──────────────────────────────────────────────
echo "  [2/3] launch smoke (5s timeout)"
if command -v timeout >/dev/null 2>&1; then
  RUN_OUT="$(timeout 5 "$OUT_BIN" --version 2>&1)" || RUN_RC=$?
else
  RUN_OUT="$("$OUT_BIN" --version 2>&1)" || RUN_RC=$?
fi
RUN_RC="${RUN_RC:-0}"
if [ "$RUN_RC" -ne 0 ]; then
  echo "  ✗ binary --version exited $RUN_RC: $RUN_OUT"
  exit 1
fi
echo "    ✓ $RUN_OUT"

# ── 6. mirror into dist/ ────────────────────────────────────────────────
echo "  [3/3] mirror → dist/phantom-$TARGET"
mkdir -p "$REPO_ROOT/dist"
cp "$OUT_BIN" "$REPO_ROOT/dist/phantom-$TARGET"
echo "    ✓ dist/phantom-$TARGET ($(du -h "$REPO_ROOT/dist/phantom-$TARGET" | awk '{print $1}'))"

echo
echo "  ✓ Linux release build complete."
echo "    Source : $OUT_BIN"
echo "    Dist   : $REPO_ROOT/dist/phantom-$TARGET"
echo
echo "  Next:"
echo "    ./scripts/setup-oci.sh                  # configure VM (Tailscale, systemd, firewall)"
echo "    cp dist/phantom-$TARGET ~/.local/bin/phantom   # adopt locally"
