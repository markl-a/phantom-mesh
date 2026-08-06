#!/usr/bin/env python3
"""Summarize a Terminal-Bench run for the spectyn agent.

Reads a run's results.json and prints accuracy + per-task pass/fail, and—because
free-tier quota errors (413/429/503) masquerade as task failures—scans each
task's post-agent terminal pane for provider errors so you can tell "agent
couldn't reason" apart from "the LLM endpoint refused to answer".

Usage:
    ./score.py                      # newest run under ./runs
    ./score.py runs/2026-06-03__16-24-47
"""

import json
import re
import sys
from pathlib import Path

RUNS = Path(__file__).parent / "runs"

_QUOTA = re.compile(r"HTTP (4\d\d|5\d\d)|rate limit|Request too large|unavailable", re.I)


def newest_run() -> Path | None:
    if not RUNS.is_dir():
        return None
    runs = [p for p in RUNS.iterdir() if (p / "results.json").is_file()]
    return max(runs, key=lambda p: p.stat().st_mtime) if runs else None


def quota_hits(run_dir: Path, task_id: str) -> set[str]:
    hits = set()
    for pane in run_dir.glob(f"{task_id}/*/panes/post-agent.txt"):
        text = pane.read_text(errors="ignore")
        for m in _QUOTA.finditer(text):
            hits.add(m.group(0).lower())
    return hits


def main() -> int:
    run_dir = Path(sys.argv[1]) if len(sys.argv) > 1 else newest_run()
    if not run_dir or not (run_dir / "results.json").is_file():
        print("no run found (pass a runs/<ts> dir, or run the harness first)")
        return 1

    data = json.loads((run_dir / "results.json").read_text())
    results = data.get("results", [])
    print(f"run: {run_dir.name}")
    print(f"accuracy: {data.get('accuracy', 0.0) * 100:.1f}%  "
          f"({data.get('n_resolved', 0)}/{len(results)} resolved)\n")

    quota_blocked = 0
    for r in sorted(results, key=lambda r: r.get("task_id", "")):
        tid = r.get("task_id", "?")
        ok = r.get("is_resolved")
        mark = "✅" if ok else "❌"
        note = ""
        if not ok:
            hits = quota_hits(run_dir, tid)
            if hits:
                quota_blocked += 1
                note = f"  ⚠ quota/endpoint: {', '.join(sorted(hits))}"
            elif r.get("failure_mode") not in (None, "unset", "none"):
                note = f"  (failure_mode={r['failure_mode']})"
        print(f"  {mark} {tid}{note}")

    if quota_blocked:
        print(f"\n⚠ {quota_blocked}/{len(results)} failed on provider quota/endpoint "
              "errors, not agent capability — not a real capability signal.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
