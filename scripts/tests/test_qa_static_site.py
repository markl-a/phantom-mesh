"""Tests for scripts/qa_static_site.py.

These tests import the QA module directly and exercise its pure helpers plus the
``collect_errors`` aggregator. They guard the one piece of logic that gates CI
(the static-site QA) so future edits to the site or the checks fail loudly
instead of silently shipping a broken landing page.
"""

from __future__ import annotations

import importlib.util
import sys
from pathlib import Path

import pytest

# Load scripts/qa_static_site.py as a module without requiring it to be on the
# package path (the repo has no Python package layout).
_SCRIPTS_DIR = Path(__file__).resolve().parents[1]
_QA_PATH = _SCRIPTS_DIR / "qa_static_site.py"

_spec = importlib.util.spec_from_file_location("qa_static_site", _QA_PATH)
assert _spec is not None and _spec.loader is not None
qa = importlib.util.module_from_spec(_spec)
sys.modules["qa_static_site"] = qa
_spec.loader.exec_module(qa)


def test_repo_passes_static_site_qa():
    """The committed static site must pass QA with zero errors."""
    assert qa.collect_errors() == []


def test_main_returns_zero_on_clean_repo():
    assert qa.main() == 0


# --- local asset reference extraction -------------------------------------

def test_local_asset_refs_extracts_relative_paths():
    html = '<img src="logo.png"><a href="docs/page.html">x</a>'
    assert qa._local_asset_refs(html) == ["logo.png", "docs/page.html"]


def test_local_asset_refs_skips_external_and_special():
    html = (
        '<a href="https://example.com">x</a>'
        '<a href="//cdn.example.com/a.js">y</a>'
        '<a href="#section">z</a>'
        '<a href="mailto:me@example.com">m</a>'
        '<img src="data:image/png;base64,AAAA">'
    )
    assert qa._local_asset_refs(html) == []


def test_local_asset_refs_strips_query_and_fragment():
    html = '<link href="style.css?v=2#top">'
    assert qa._local_asset_refs(html) == ["style.css"]


# --- HTML health checks ----------------------------------------------------

def test_html_health_flags_missing_pieces(tmp_path, monkeypatch):
    site = tmp_path / "site"
    site.mkdir()
    (site / "index.html").write_text("<html><body>hi</body></html>", encoding="utf-8")
    monkeypatch.setattr(qa, "SITE_DIR", site)

    errors: list[str] = []
    qa._check_html_health(errors)

    joined = "\n".join(errors)
    assert "<!DOCTYPE html>" in joined
    assert "<title>" in joined
    assert "lang=" in joined
    assert "viewport" in joined
    assert "description" in joined


def test_html_health_passes_on_good_document(tmp_path, monkeypatch):
    site = tmp_path / "site"
    site.mkdir()
    (site / "index.html").write_text(
        "<!DOCTYPE html>\n"
        '<html lang="en">\n'
        "<head>\n"
        "<title>Hello</title>\n"
        '<meta name="description" content="x">\n'
        '<meta name="viewport" content="width=device-width">\n'
        "</head><body>hi</body></html>",
        encoding="utf-8",
    )
    monkeypatch.setattr(qa, "SITE_DIR", site)

    errors: list[str] = []
    qa._check_html_health(errors)
    assert errors == []


# --- local asset existence -------------------------------------------------

def test_check_local_assets_reports_missing(tmp_path, monkeypatch):
    site = tmp_path / "site"
    site.mkdir()
    (site / "index.html").write_text(
        '<img src="logo.png"><img src="missing.png">', encoding="utf-8"
    )
    (site / "logo.png").write_bytes(b"\x89PNG")
    monkeypatch.setattr(qa, "SITE_DIR", site)

    errors: list[str] = []
    qa._check_local_assets(errors)
    assert any("missing.png" in e for e in errors)
    assert not any("logo.png" in e for e in errors)


# --- stale phrases ---------------------------------------------------------

def test_stale_phrase_is_detected(tmp_path, monkeypatch):
    bad = tmp_path / "index.html"
    bad.write_text("hello Coming May 2026 world", encoding="utf-8")
    monkeypatch.setattr(qa, "TEXT_FILES", [bad])

    errors: list[str] = []
    qa._check_stale_phrases(errors)
    assert any("Coming May 2026" in e for e in errors)


# --- CNAME -----------------------------------------------------------------

def test_cname_wrong_domain_is_flagged(tmp_path, monkeypatch):
    site = tmp_path / "site"
    site.mkdir()
    (site / "CNAME").write_text("wrong.example.com\n", encoding="utf-8")
    monkeypatch.setattr(qa, "SITE_DIR", site)

    errors: list[str] = []
    qa._check_cname(errors)
    assert any("phantommesh.io" in e for e in errors)


def test_cname_correct_domain_passes(tmp_path, monkeypatch):
    site = tmp_path / "site"
    site.mkdir()
    (site / "CNAME").write_text("phantommesh.io\n", encoding="utf-8")
    monkeypatch.setattr(qa, "SITE_DIR", site)

    errors: list[str] = []
    qa._check_cname(errors)
    assert errors == []


if __name__ == "__main__":
    raise SystemExit(pytest.main([__file__, "-q"]))
