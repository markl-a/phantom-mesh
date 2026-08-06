#!/usr/bin/env bash
# Build an x86_64 linux `spectyn` binary for Terminal-Bench containers.
#
# spectyn is normally built on macOS (arm64), but Terminal-Bench task containers
# are x86_64 Debian linux, so we cross-build inside a rust:bookworm container.
# Output: evals/terminal-bench/spectyn-x86_64-linux (glibc, dynamically linked —
# runs on the standard Debian-based task images).
#
# Usage:  ./build-linux-binary.sh        # needs Docker running
#
# After building, host the binary so containers can fetch it:
#   - quick/local:  python3 -m http.server 8000  (from this dir), then
#                   SPECTYN_TB_BINARY_URL=http://host.docker.internal:8000/spectyn-x86_64-linux
#   - real runs:    upload it as a GitHub release asset and point the URL there.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
out="$here/spectyn-x86_64-linux"

if ! docker info >/dev/null 2>&1; then
  echo "ERROR: Docker is not running. Start Docker Desktop first." >&2
  exit 1
fi

echo "Building spectyn for x86_64-unknown-linux-gnu inside rust:bookworm ..."
echo "(first run compiles the whole crate — expect several minutes)"

# Mount the repo read-write; cache the cargo registry + target dir on the host
# so repeat builds are incremental.
docker run --rm \
  --platform linux/amd64 \
  -v "$repo":/src \
  -v "$here/.cargo-registry":/usr/local/cargo/registry \
  -v "$here/.target-linux":/src/core/target \
  -w /src/core \
  rust:bookworm \
  bash -c "cargo build --release --locked --bin spectyn"

cp "$here/.target-linux/release/spectyn" "$out"
chmod +x "$out"
echo "OK -> $out"
file "$out" || true
ls -lh "$out"
