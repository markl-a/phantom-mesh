import { describe, it, expect } from "vitest";
import { parseTasks, parseCaptures, parseReview } from "../../src/lib/supervisor";

// GOLDEN: these objects are byte-for-byte the shapes the Rust handlers emit
// (see core/src/serve.rs rpc_tasks_list / rpc_captures_recent / rpc_review +
// their squad_dispatch_tests). If the backend renames a field, update BOTH
// sides — this test is the alarm.
describe("P1-2 wire contract (backend ↔ app)", () => {
  it("tasks/list golden", () => {
    // Mirrors task_record_to_wire (snake_case) + the pending-card projection.
    const r = parseTasks({
      tasks: [
        {
          task_id: "t1",
          agent_name: "coder",
          prompt: "p",
          status: "running",
          created_at: 1,
          started_at: null,
          finished_at: null,
          cost_usd: 0,
          turns: 0,
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
          reason: "r",
          created_ms: 2,
        },
      ],
    });
    expect(r.tasks[0].agent).toBe("coder");
    expect(r.tasks[0].status).toBe("running");
    expect(r.pending[0].approvalId).toBe("a1");
    expect(r.pending[0].risk).toBe("execute_high");
  });

  it("captures golden (snake_case event_meta_to_wire, kind = EventKind snake_case)", () => {
    const caps = parseCaptures({
      captures: [
        { event_id: "e1", timestamp: "2026-06-17T00:00:00Z", kind: "food", tags: ["fat_loss"] },
      ],
    });
    expect(caps[0].id).toBe("e1");
    expect(caps[0].kind).toBe("food");
    expect(caps[0].tags).toEqual(["fat_loss"]);
  });

  it("review golden", () => {
    expect(parseReview({ date: "2026-06-17", markdown: "# Daily review" }).date).toBe("2026-06-17");
  });
});
