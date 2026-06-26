import { describe, it, expect } from "vitest";
import { parseTasks, parseCaptures, parseReview } from "../../src/lib/supervisor";

describe("supervisor wire parsers (P1-2)", () => {
  it("parseTasks maps the /rpc/tasks/list wire shape to rows", () => {
    const wire = {
      tasks: [
        {
          task_id: "t1",
          agent_name: "coder",
          prompt: "fix bug",
          status: "running",
          created_at: 1718600000000,
          cost_usd: 0.01,
          turns: 2,
          error: null,
          output: null,
        },
      ],
      pending: [
        {
          approval_id: "a1",
          task_id: "t1",
          tool: "Bash",
          risk: "execute_high",
          reason: "pre-action approval",
          created_ms: 1718600001000,
        },
      ],
    };
    const r = parseTasks(wire);
    expect(r.tasks).toHaveLength(1);
    expect(r.tasks[0]).toMatchObject({ id: "t1", agent: "coder", status: "running" });
    expect(r.pending).toHaveLength(1);
    expect(r.pending[0]).toMatchObject({ approvalId: "a1", tool: "Bash", risk: "execute_high" });
  });

  it("parseTasks tolerates junk (never throws, returns empty)", () => {
    expect(parseTasks(undefined).tasks).toEqual([]);
    expect(parseTasks({ tasks: "nope" }).tasks).toEqual([]);
    expect(parseTasks(null).pending).toEqual([]);
  });

  it("parseCaptures maps event metas (snake_case backend wire)", () => {
    const wire = {
      captures: [
        { event_id: "e1", timestamp: "2026-06-17T01:02:03Z", kind: "food", tags: ["fat_loss"] },
      ],
    };
    const r = parseCaptures(wire);
    expect(r).toHaveLength(1);
    expect(r[0]).toMatchObject({ id: "e1", kind: "food", tags: ["fat_loss"] });
  });

  it("parseCaptures tolerates a goal_tags alias and junk", () => {
    const r = parseCaptures({
      captures: [{ event_id: "e2", timestamp: "x", kind: "focus", goal_tags: ["work"] }],
    });
    expect(r[0].tags).toEqual(["work"]);
    expect(parseCaptures(undefined)).toEqual([]);
  });

  it("parseReview returns markdown + date, empty string when absent", () => {
    expect(parseReview({ date: "2026-06-17", markdown: "# Daily review" })).toEqual({
      date: "2026-06-17",
      markdown: "# Daily review",
    });
    expect(parseReview(undefined)).toEqual({ date: "", markdown: "" });
  });
});
