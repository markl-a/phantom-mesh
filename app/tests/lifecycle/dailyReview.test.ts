// Lifecycle · Unit tests — daily-review markdown parsing (coach review reader,
// the end-of-day "look back over what I captured" journey).
//
// The coach/daily-review screen receives a Markdown aggregate from the backend
// and turns it into typed rows for rendering, plus pulls out the optional
// "Tomorrow's one action" coaching line. parseReview / extractTomorrowAction /
// todayIso are pure functions in src/lib/dailyReview.ts (no Tauri runtime).
// Mirrors the assertion style of tests/providers/describeError.test.ts.

import { describe, it, expect } from "vitest";
import {
  todayIso,
  parseReview,
  extractTomorrowAction,
} from "../../src/lib/dailyReview";

describe("todayIso", () => {
  it("formats local today as zero-padded YYYY-MM-DD", () => {
    const iso = todayIso();
    expect(iso).toMatch(/^\d{4}-\d{2}-\d{2}$/);
    // Each component is the padded local date.
    const d = new Date();
    const p = (n: number) => String(n).padStart(2, "0");
    expect(iso).toBe(`${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`);
  });
});

describe("parseReview — typed rows from the aggregate markdown", () => {
  it("parses the canonical sections the backend emits", () => {
    const md = [
      "# Daily review — 2026-05-31",
      "**Events captured:** 3",
      "## work (2)",
      "- **focus** (2026-05-31T09:15:00+08:00): 25m pomodoro",
      "- **habit** (2026-05-31T10:00:00+08:00): water x1",
      "## meals (1)",
      "- **food** (2026-05-31T12:30:00+08:00): lunch",
    ].join("\n");

    const rows = parseReview(md);

    expect(rows[0]).toEqual({ kind: "title", text: "Daily review — 2026-05-31" });
    expect(rows[1]).toEqual({ kind: "count", text: "3" });
    expect(rows[2]).toEqual({ kind: "group", tag: "work", n: 2 });
    expect(rows[3]).toEqual({
      kind: "bullet",
      eventKind: "focus",
      time: "09:15",
      summary: "25m pomodoro",
    });
    expect(rows[5]).toEqual({ kind: "group", tag: "meals", n: 1 });
  });

  it("extracts HH:MM directly from a local-offset timestamp (no UTC re-shift)", () => {
    const md = "- **focus** (2026-05-31T23:45:00+08:00): late session";
    const [row] = parseReview(md);
    expect(row).toMatchObject({ kind: "bullet", time: "23:45" });
  });

  it("skips blank lines and treats unknown lines as notes", () => {
    const md = ["", "Just some prose.", "", "**bold but not a section**"].join("\n");
    const rows = parseReview(md);
    expect(rows).toHaveLength(2);
    expect(rows[0]).toEqual({ kind: "note", text: "Just some prose." });
    expect(rows[1].kind).toBe("note");
  });

  it("returns an empty array for empty markdown", () => {
    expect(parseReview("")).toEqual([]);
  });
});

describe("extractTomorrowAction", () => {
  it("pulls the action body out of the generated section", () => {
    const md = [
      "# Daily review — 2026-05-31",
      "## Tomorrow's one action",
      "Start the day with a 25-minute focus block.",
    ].join("\n");
    expect(extractTomorrowAction(md)).toEqual({
      text: "Start the day with a 25-minute focus block.",
      skipped: false,
    });
  });

  it("flags a skipped LLM pass (no API key) so the UI can mute it", () => {
    const md = "## Tomorrow's one action\n(skipped: no GEMINI_API_KEY)";
    expect(extractTomorrowAction(md)).toEqual({
      text: "(skipped: no GEMINI_API_KEY)",
      skipped: true,
    });
  });

  it("returns null for an aggregate-only review (load path, no action)", () => {
    const md = "# Daily review — 2026-05-31\n**Events captured:** 0";
    expect(extractTomorrowAction(md)).toBeNull();
  });
});
