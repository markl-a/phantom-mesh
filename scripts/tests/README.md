# scripts/tests

Python tests for the repo's build/QA helper scripts.

## What's covered

- `test_qa_static_site.py` — tests for `scripts/qa_static_site.py`, the
  static-site QA gate that CI runs against the published `site/` landing page.
  It checks required files, the `CNAME` domain, logo size, stale marketing
  phrases, basic HTML health (`<!DOCTYPE>`, `lang`, `<title>`, `description`
  and `viewport` meta), and that relative `src`/`href` assets resolve on disk.

## Run locally

```bash
python -m pytest scripts/tests/test_qa_static_site.py -q
```

The same suite runs in CI (`.github/workflows/ci.yml`) immediately after the
`python scripts/qa_static_site.py` gate, so a regression in either the site or
the QA logic fails the build.
