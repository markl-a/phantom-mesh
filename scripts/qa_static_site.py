from __future__ import annotations

from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]

TEXT_FILES = [
    ROOT / "README.md",
    ROOT / "site" / "index.html",
]

REQUIRED_FILES = [
    ROOT / "README.md",
    ROOT / "site" / "index.html",
    ROOT / "site" / "CNAME",
    ROOT / "site" / "logo.png",
]

LICENSE_FILES = [
    ROOT / "LICENSE",
    ROOT / "LICENSE-MIT",
    ROOT / "LICENSE-APACHE",
]

STALE_PHRASES = [
    "Coming May 2026",
    "2026 年 5 月開源",
    "2026 年 5 月正式開源",
    "五月開源",
    "#open-source-coming-may-2026",
]


def main() -> int:
    errors: list[str] = []

    for path in REQUIRED_FILES:
        if not path.exists():
            errors.append(f"missing required file: {path.relative_to(ROOT)}")

    if not any(path.exists() for path in LICENSE_FILES):
        errors.append("missing a root license file")

    cname = ROOT / "site" / "CNAME"
    if cname.exists() and cname.read_text(encoding="utf-8").strip() != "phantommesh.io":
        errors.append("site/CNAME must point to phantommesh.io")

    logo = ROOT / "site" / "logo.png"
    if logo.exists():
        size = logo.stat().st_size
        if size <= 0:
            errors.append("site/logo.png is empty")
        if size > 5_000_000:
            errors.append("site/logo.png is larger than 5 MB")

    for path in TEXT_FILES:
        if not path.exists():
            continue
        text = path.read_text(encoding="utf-8")
        for phrase in STALE_PHRASES:
            if phrase in text:
                errors.append(f"{path.relative_to(ROOT)} contains stale phrase: {phrase}")

    if errors:
        for error in errors:
            print(f"ERROR: {error}")
        return 1

    print("static site QA passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
