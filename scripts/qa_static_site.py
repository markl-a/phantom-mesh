"""Static-site QA gate for the published phantommesh.io landing page.

Run as ``python scripts/qa_static_site.py``; prints ``ERROR: ...`` lines and
exits non-zero when the ``site/`` deliverable is broken, otherwise prints
``static site QA passed`` and exits 0. The checks live in small helpers and are
aggregated by :func:`collect_errors`, which is also exercised by
``scripts/tests/test_qa_static_site.py``.
"""

from __future__ import annotations

import re
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]

SITE_DIR = ROOT / "site"

TEXT_FILES = [
    ROOT / "README.md",
    SITE_DIR / "index.html",
]

REQUIRED_FILES = [
    ROOT / "README.md",
    SITE_DIR / "index.html",
    SITE_DIR / "CNAME",
    SITE_DIR / "logo.png",
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

# Required bits of HTML hygiene for the published landing page. Each entry is a
# (regex, human-readable description) pair; a missing match is reported as an
# error so the deployed page stays valid and indexable.
HTML_HEALTH_CHECKS = [
    (re.compile(r"<!DOCTYPE html>", re.IGNORECASE), "missing <!DOCTYPE html>"),
    (re.compile(r"<html[^>]*\blang=", re.IGNORECASE), "missing <html lang=...>"),
    (re.compile(r"<title>[^<]+</title>", re.IGNORECASE), "missing non-empty <title>"),
    (
        re.compile(r'<meta[^>]*name=["\']description["\']', re.IGNORECASE),
        "missing <meta name=\"description\">",
    ),
    (
        re.compile(r'<meta[^>]*name=["\']viewport["\']', re.IGNORECASE),
        "missing <meta name=\"viewport\">",
    ),
]

# Attribute values that are not local files and should be skipped when checking
# that relative asset references resolve on disk.
_EXTERNAL_REF = re.compile(r"^(?:[a-zA-Z][a-zA-Z0-9+.-]*:|//|#|mailto:|tel:|data:)")

_LOCAL_REF = re.compile(r'(?:src|href)\s*=\s*["\']([^"\']+)["\']', re.IGNORECASE)


def _display(path: Path) -> str:
    """Path relative to ROOT when possible, else the full path (test-safe)."""
    try:
        return str(path.relative_to(ROOT))
    except ValueError:
        return str(path)


def _local_asset_refs(html: str) -> list[str]:
    """Return relative src/href values that should resolve to a local file."""
    refs: list[str] = []
    for raw in _LOCAL_REF.findall(html):
        value = raw.strip()
        if not value or _EXTERNAL_REF.match(value):
            continue
        # Drop any query string / fragment before resolving on disk.
        value = value.split("?", 1)[0].split("#", 1)[0]
        if value:
            refs.append(value)
    return refs


def _check_required_files(errors: list[str]) -> None:
    for path in REQUIRED_FILES:
        if not path.exists():
            errors.append(f"missing required file: {_display(path)}")


def _check_license(errors: list[str]) -> None:
    if not any(path.exists() for path in LICENSE_FILES):
        errors.append("missing a root license file")


def _check_cname(errors: list[str]) -> None:
    cname = SITE_DIR / "CNAME"
    if cname.exists() and cname.read_text(encoding="utf-8").strip() != "phantommesh.io":
        errors.append("site/CNAME must point to phantommesh.io")


def _check_logo(errors: list[str]) -> None:
    logo = SITE_DIR / "logo.png"
    if logo.exists():
        size = logo.stat().st_size
        if size <= 0:
            errors.append("site/logo.png is empty")
        if size > 5_000_000:
            errors.append("site/logo.png is larger than 5 MB")


def _check_stale_phrases(errors: list[str]) -> None:
    for path in TEXT_FILES:
        if not path.exists():
            continue
        text = path.read_text(encoding="utf-8")
        for phrase in STALE_PHRASES:
            if phrase in text:
                errors.append(f"{_display(path)} contains stale phrase: {phrase}")


def _check_html_health(errors: list[str]) -> None:
    index = SITE_DIR / "index.html"
    if not index.exists():
        return
    html = index.read_text(encoding="utf-8")
    for pattern, description in HTML_HEALTH_CHECKS:
        if not pattern.search(html):
            errors.append(f"site/index.html {description}")


def _check_local_assets(errors: list[str]) -> None:
    index = SITE_DIR / "index.html"
    if not index.exists():
        return
    html = index.read_text(encoding="utf-8")
    for ref in _local_asset_refs(html):
        target = (SITE_DIR / ref).resolve()
        if not target.exists():
            errors.append(f"site/index.html references missing local asset: {ref}")


def collect_errors() -> list[str]:
    """Run every static-site QA check and return a list of error strings.

    An empty list means the static site passed QA. This is the testable core
    used by both ``main`` and the test suite.
    """
    errors: list[str] = []
    _check_required_files(errors)
    _check_license(errors)
    _check_cname(errors)
    _check_logo(errors)
    _check_stale_phrases(errors)
    _check_html_health(errors)
    _check_local_assets(errors)
    return errors


def main() -> int:
    errors = collect_errors()
    if errors:
        for error in errors:
            print(f"ERROR: {error}")
        return 1

    print("static site QA passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
