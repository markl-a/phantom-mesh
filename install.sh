#!/usr/bin/env sh
# Build-from-source installer for the `spectyn` CLI (Spectyn Mesh) on macOS/Linux.
#
# Mirrors install.ps1 (the Windows installer that was run-verified on a Windows
# machine). This POSIX variant builds the optimized `spectyn` binary from source
# with cargo and installs it to a per-user bin dir (no sudo required). Idempotent:
# re-running overwrites the installed binary cleanly.
#
# This is the SOURCE-BUILD installer and therefore REQUIRES a Rust toolchain
# (cargo). The hosted prebuilt-download one-liner advertised in older docs
# (curl .../install.sh | sh) is a FUTURE release-pipeline item and is not wired
# up yet — use this script from a checkout instead.
#
# NOTE (honesty): unlike install.ps1, this script's full run-validation is
# DEFERRED to a POSIX/macOS machine. It has NOT yet been executed end-to-end
# here (the verification machine is Windows-only). Treat it as the mirror of the
# proven Windows logic, pending a POSIX run.
#
# Usage:
#   ./install.sh                     # installs to ~/.local
#   ./install.sh --prefix /tmp/pm    # installs the binary to /tmp/pm/bin
#   PREFIX=/tmp/pm ./install.sh      # same, via env var

set -eu

# ---- args ------------------------------------------------------------------
PREFIX="${PREFIX:-$HOME/.local}"
while [ $# -gt 0 ]; do
    case "$1" in
        --prefix)
            [ $# -ge 2 ] || { echo "error: --prefix needs a value" >&2; exit 2; }
            PREFIX="$2"; shift 2 ;;
        --prefix=*)
            PREFIX="${1#--prefix=}"; shift ;;
        -h|--help)
            sed -n '2,30p' "$0"; exit 0 ;;
        *)
            echo "error: unknown argument: $1" >&2; exit 2 ;;
    esac
done

step() { printf '==> %s\n' "$1"; }
ok()   { printf '    %s\n' "$1"; }

# Repo layout: this script lives at the repo root; the Rust crate is in core/.
SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
CRATE_DIR="$SCRIPT_DIR/core"
MANIFEST="$CRATE_DIR/Cargo.toml"

if [ ! -f "$MANIFEST" ]; then
    echo "error: cannot find core/Cargo.toml next to install.sh (looked in '$CRATE_DIR')." >&2
    echo "       Run this from a spectyn-mesh checkout." >&2
    exit 1
fi

# 1. Toolchain check ---------------------------------------------------------
step 'Checking for the Rust toolchain (cargo)...'
if ! command -v cargo >/dev/null 2>&1; then
    cat >&2 <<'EOF'

error: cargo (the Rust toolchain) was not found on PATH.

This is the build-from-source installer, so Rust is required. Install it via rustup:

    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

Then restart your shell (so cargo is on PATH) and re-run this script.
EOF
    exit 1
fi
ok "found cargo: $(command -v cargo)"

# 2. Build the optimized binary from source ----------------------------------
step 'Building spectyn (cargo build --release --bin spectyn) — this is slow on a cold build, please wait...'

# Best-effort: stamp the real commit into the binary so `spectyn --version`
# reports provenance instead of "nogit". Never fatal if git is unavailable.
if command -v git >/dev/null 2>&1 && \
   GIT_HASH="$(git -C "$SCRIPT_DIR" rev-parse --short HEAD 2>/dev/null)"; then
    export SPECTYN_GIT_HASH="$GIT_HASH"
fi

( cd "$CRATE_DIR" && cargo build --release --bin spectyn )

SRC_BIN="$CRATE_DIR/target/release/spectyn"
if [ ! -f "$SRC_BIN" ]; then
    echo "error: build reported success but '$SRC_BIN' is missing. Aborting." >&2
    exit 1
fi
ok "built: $SRC_BIN"

# 3. Install to the per-user bin dir (idempotent) ----------------------------
BIN_DIR="$PREFIX/bin"
step "Installing to $BIN_DIR ..."
mkdir -p "$BIN_DIR"
DEST_BIN="$BIN_DIR/spectyn"
install -m 0755 "$SRC_BIN" "$DEST_BIN" 2>/dev/null || { cp -f "$SRC_BIN" "$DEST_BIN"; chmod 0755 "$DEST_BIN"; }
ok "installed: $DEST_BIN"

# 4. Create the data dir if absent -------------------------------------------
DATA_DIR="$HOME/.spectyn-mesh"
step "Ensuring data dir $DATA_DIR ..."
mkdir -p "$DATA_DIR"
ok "data dir ready: $DATA_DIR"

# 5. Next steps --------------------------------------------------------------
printf '\nspectyn installed.\n\n'
printf 'Add the bin dir to your PATH (append to ~/.bashrc or ~/.zshrc):\n'
printf '    export PATH="%s:$PATH"\n\n' "$BIN_DIR"
printf 'Then verify and start the daemon:\n'
printf '    spectyn --version\n'
printf '    spectyn --help\n'
printf '    spectyn serve\n'
