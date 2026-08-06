#!/usr/bin/env python3
"""
Build a Tauri updater manifest (latest.json) from a GitHub Release's assets.

Reads release assets via `gh release view`, finds Tauri bundle artifacts and
their accompanying `.sig` signature files, then prints a JSON manifest in the
shape Tauri's plugin-updater expects:

    {
      "version": "0.1.0",
      "notes": "...",
      "pub_date": "2026-05-01T12:00:00Z",
      "platforms": {
        "darwin-aarch64": { "signature": "...", "url": "https://..." },
        ...
      }
    }

Required env vars:
  TAG     — git tag, e.g. "v0.1.0"
  REPO    — owner/repo, e.g. "markl-a/spectyn-mesh"
  GH_TOKEN — passed to `gh` CLI (set by GitHub Actions automatically)

Optional:
  NOTES   — release notes string (default: link to release page)

The script tolerates missing platforms — if a `.sig` for a target isn't
present (build failed or platform skipped), that platform is omitted from
the manifest rather than failing the whole job.
"""
from __future__ import annotations

import datetime as _dt
import json
import os
import re
import subprocess
import sys
from typing import Optional


# Map updater platform key → regex matching the bundle artifact name.
# Tauri 2 bundle naming conventions:
#   macOS:          *_<version>_<arch>.app.tar.gz   (.sig sibling)
#   Windows NSIS:   *_<version>_<arch>-setup.nsis.zip
#   Windows MSI:    *_<version>_<arch>_en-US.msi.zip
#   Linux AppImage: *_<version>_amd64.AppImage.tar.gz
PLATFORM_PATTERNS: dict[str, list[str]] = {
    "darwin-aarch64": [r"_aarch64\.app\.tar\.gz$"],
    "darwin-x86_64":  [r"_x64\.app\.tar\.gz$", r"_x86_64\.app\.tar\.gz$"],
    "windows-x86_64": [
        r"_x64-setup\.nsis\.zip$",
        r"_x64_en-US\.msi\.zip$",
    ],
    "linux-x86_64":   [r"_amd64\.AppImage\.tar\.gz$"],
}


def gh(*args: str) -> str:
    return subprocess.check_output(["gh", *args], text=True)


def list_assets(tag: str) -> list[str]:
    out = gh("release", "view", tag, "--json", "assets", "--jq", ".assets[].name")
    return [line.strip() for line in out.splitlines() if line.strip()]


def find_bundle(assets: list[str], patterns: list[str]) -> Optional[str]:
    for p in patterns:
        for a in assets:
            if a.endswith(".sig"):
                continue
            if re.search(p, a):
                return a
    return None


def find_sig(assets: list[str], bundle_name: str) -> Optional[str]:
    target = bundle_name + ".sig"
    return target if target in assets else None


def read_sig(tag: str, sig_name: str) -> str:
    """Download the .sig file contents and return as string."""
    out = subprocess.check_output(
        ["gh", "release", "download", tag, "-p", sig_name, "-O", "-"],
        text=True,
    )
    return out.strip()


def encode_url_path(name: str) -> str:
    # Bundle filenames may contain spaces ("Spectyn Mesh_..."), encode them.
    return name.replace(" ", "%20")


def main() -> int:
    tag = os.environ["TAG"]
    repo = os.environ["REPO"]
    notes = os.environ.get(
        "NOTES",
        f"See https://github.com/{repo}/releases/tag/{tag}",
    )
    version = tag.lstrip("v")

    assets = list_assets(tag)
    print(f"# Found {len(assets)} assets in release {tag}", file=sys.stderr)
    for a in assets:
        print(f"#   - {a}", file=sys.stderr)

    platforms: dict[str, dict[str, str]] = {}
    for key, patterns in PLATFORM_PATTERNS.items():
        bundle = find_bundle(assets, patterns)
        if not bundle:
            print(f"# [skip] {key}: no bundle matching {patterns}", file=sys.stderr)
            continue
        sig = find_sig(assets, bundle)
        if not sig:
            print(f"# [skip] {key}: no .sig for {bundle}", file=sys.stderr)
            continue
        try:
            signature = read_sig(tag, sig)
        except subprocess.CalledProcessError as e:
            print(f"# [skip] {key}: failed to read sig ({e})", file=sys.stderr)
            continue
        url = f"https://github.com/{repo}/releases/download/{tag}/{encode_url_path(bundle)}"
        platforms[key] = {"signature": signature, "url": url}
        print(f"# [ok]   {key}: {bundle}", file=sys.stderr)

    if not platforms:
        print("ERROR: no platforms resolved — aborting (no latest.json written)",
              file=sys.stderr)
        return 1

    manifest = {
        "version": version,
        "notes": notes,
        "pub_date": _dt.datetime.now(_dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "platforms": platforms,
    }
    json.dump(manifest, sys.stdout, indent=2)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
