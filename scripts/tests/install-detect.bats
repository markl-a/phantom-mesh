#!/usr/bin/env bats
#
# scripts/tests/install-detect.bats — unit tests for F500 install.sh
# OS/arch detection branches.
#
# We use SPECTYN_INSTALL_DRY_RUN=1 so the script never writes to disk or
# touches the network — it just emits its detection + the URL it WOULD
# fetch. That makes these tests hermetic.
#
# We stub `uname` by prepending a tmpdir to PATH that contains a fake
# uname script. This lets us drive every (OS, arch) branch from a single
# host machine.
#
# Run:
#   bats scripts/tests/install-detect.bats
#
# If bats isn't installed locally, CI installs it (see
# .github/workflows/install-smoke-linux.yml under F500 spec test matrix).

INSTALL_SH="${BATS_TEST_DIRNAME}/../install.sh"

setup() {
    STUB_DIR="$(mktemp -d)"
    export STUB_DIR
    export SPECTYN_INSTALL_DRY_RUN=1
    # Pin the base URL so output is deterministic across hosts.
    export SPECTYN_INSTALL_BASE='https://example.test'
}

teardown() {
    rm -rf "$STUB_DIR"
}

# Helper: stub `uname` with fixed -s and -m return values.
stub_uname() {
    local os="$1" arch="$2"
    cat > "$STUB_DIR/uname" <<EOF
#!/bin/sh
case "\$1" in
  -s) printf '%s\n' '$os' ;;
  -m) printf '%s\n' '$arch' ;;
  *)  /usr/bin/uname "\$@" ;;
esac
EOF
    chmod +x "$STUB_DIR/uname"
}

run_install() {
    PATH="$STUB_DIR:$PATH" sh "$INSTALL_SH"
}

@test "Linux x86_64 → spectyn-linux-x86_64" {
    stub_uname Linux x86_64
    run run_install
    [ "$status" -eq 0 ]
    [[ "$output" == *"detected OS:   linux"* ]]
    [[ "$output" == *"detected arch: x86_64"* ]]
    [[ "$output" == *"R2 object:     spectyn-linux-x86_64"* ]]
    [[ "$output" == *"https://example.test/dist/spectyn-linux-x86_64"* ]]
}

@test "Linux aarch64 → spectyn-aarch64-unknown-linux-gnu" {
    stub_uname Linux aarch64
    run run_install
    [ "$status" -eq 0 ]
    [[ "$output" == *"R2 object:     spectyn-aarch64-unknown-linux-gnu"* ]]
}

@test "Linux amd64 alias also maps to x86_64" {
    stub_uname Linux amd64
    run run_install
    [ "$status" -eq 0 ]
    [[ "$output" == *"detected arch: x86_64"* ]]
}

@test "Linux arm64 alias also maps to aarch64" {
    stub_uname Linux arm64
    run run_install
    [ "$status" -eq 0 ]
    [[ "$output" == *"detected arch: aarch64"* ]]
}

@test "Darwin arm64 → spectyn-aarch64-apple-darwin" {
    stub_uname Darwin arm64
    run run_install
    [ "$status" -eq 0 ]
    [[ "$output" == *"detected OS:   darwin"* ]]
    [[ "$output" == *"R2 object:     spectyn-aarch64-apple-darwin"* ]]
}

@test "Darwin x86_64 (Intel Mac) is rejected with actionable error" {
    stub_uname Darwin x86_64
    run run_install
    [ "$status" -ne 0 ]
    [[ "$output" == *"Intel Macs"* ]]
}

@test "Unknown OS is rejected with pointer to install.ps1" {
    stub_uname FreeBSD x86_64
    run run_install
    [ "$status" -ne 0 ]
    [[ "$output" == *"Unsupported OS"* ]]
}

@test "Unknown arch is rejected" {
    stub_uname Linux riscv64
    run run_install
    [ "$status" -ne 0 ]
    [[ "$output" == *"Unsupported arch"* ]]
}

@test "Dry run honors SPECTYN_INSTALL_BASE override" {
    stub_uname Linux x86_64
    SPECTYN_INSTALL_BASE='https://staging.phantommesh.io' run run_install
    [ "$status" -eq 0 ]
    [[ "$output" == *"https://staging.phantommesh.io/dist/spectyn-linux-x86_64"* ]]
}

@test "Dry run does not write the target binary" {
    stub_uname Linux x86_64
    rm -f "$HOME/.spectyn-mesh/bin/spectyn"
    run run_install
    [ "$status" -eq 0 ]
    [ ! -e "$HOME/.spectyn-mesh/bin/spectyn" ]
}
