#!/usr/bin/env bash
source "$PHANTOM_TEST_LIB/common.sh"

scenario "TUI snapshot — desktop screen capture (Windows-only via PowerShell)"

# This relies on PowerShell + System.Drawing — Windows native. Skip on
# non-Windows hosts since X11/Wayland capture is a different code path.
case "$(uname -s 2>/dev/null || echo unknown)" in
  MINGW*|MSYS*|CYGWIN*|Windows*) ;;  # ok
  *)
    warn "skipping — not a Windows host"
    exit 77 ;;
esac

require_cmd powershell

step "invoking lib/snapshot.ps1 (full virtual desktop)"
snap_path=$(powershell -NoProfile -ExecutionPolicy Bypass \
  -File "$(cygpath -w "$PHANTOM_TEST_LIB/snapshot.ps1" 2>/dev/null || echo "$PHANTOM_TEST_LIB/snapshot.ps1")" 2>&1 | tr -d '\r' | tail -1)

if [ -z "$snap_path" ]; then
  fail "snapshot.ps1 returned no path"
  exit 1
fi
pass "snapshot path: $snap_path"

# Convert Windows path to MSYS path for stat
snap_unix=$(echo "$snap_path" | sed -E 's|^([A-Za-z]):|/\L\1|; s|\\|/|g')

if [ ! -f "$snap_unix" ]; then
  fail "snapshot file not found at $snap_unix"
  exit 1
fi

size=$(stat -c %s "$snap_unix" 2>/dev/null)
if [ "$size" -gt 50000 ]; then
  pass "PNG size $size bytes (>50 KB, looks valid)"
else
  fail "PNG suspiciously small: $size bytes"
fi

# Header check: PNG magic bytes are 0x89 0x50 0x4E 0x47.
hdr=$(xxd -l 4 -p "$snap_unix" 2>/dev/null)
ASSERT_EQ "$hdr" "89504e47" "PNG magic bytes"

[ "$PHANTOM_TEST_FAILED" -eq 0 ]
