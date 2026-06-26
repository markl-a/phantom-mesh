// The daemon's real task queue route `GET /tasks` (core/src/main.rs
// `tasks_list`) returns raw `pm_types::TaskRecord` rows — { task_id, prompt,
// agent_name, status } with statuses pending | awaiting_approval | running |
// completed | failed | cancelled. The TasksPanel renders TaskItem
// { id, title, agent, status: pending|running|done|failed|cancelled } and
// indexes STATUS_CONFIG by status, so an unmapped status would throw at
// render. This guards the TaskRecord -> TaskItem mapping.

import { describe, expect, it } from "vitest";

import { toTaskItems } from "@/components/dashboard/TasksPanel";

describe("toTaskItems", () => {
  it("maps daemon TaskRecord rows to the panel's TaskItem shape", () => {
    const rows = [
      {
        task_id: "aaaa-1111",
        prompt: "summarise the logs",
        agent_name: "master",
        status: "completed",
      },
      {
        task_id: "bbbb-2222",
        prompt: "long build",
        agent_name: "coder",
        status: "running",
      },
      {
        task_id: "cccc-3333",
        prompt: "needs sign-off",
        agent_name: "ops",
        status: "awaiting_approval",
      },
      {
        task_id: "dddd-4444",
        prompt: "user aborted",
        agent_name: "ops",
        status: "cancelled",
      },
    ];
    expect(toTaskItems(rows)).toEqual([
      { id: "aaaa-1111", title: "summarise the logs", agent: "master", status: "done" },
      { id: "bbbb-2222", title: "long build", agent: "coder", status: "running" },
      { id: "cccc-3333", title: "needs sign-off", agent: "ops", status: "pending" },
      { id: "dddd-4444", title: "user aborted", agent: "ops", status: "cancelled" },
    ]);
  });

  it("tolerates legacy {id,title,agent} rows", () => {
    expect(
      toTaskItems([{ id: "x", title: "old shape", agent: "a", status: "done" }])
    ).toEqual([{ id: "x", title: "old shape", agent: "a", status: "done" }]);
  });

  it("never produces a status outside STATUS_CONFIG, even for junk input", () => {
    const items = toTaskItems([
      { task_id: "e", prompt: "future variant", agent_name: "a", status: "paused" },
      {},
      null,
    ]);
    const allowed = ["pending", "running", "done", "failed", "cancelled"];
    for (const item of items) {
      expect(allowed).toContain(item.status);
    }
    expect(items).toHaveLength(3);
  });

  it("returns [] for non-array payloads", () => {
    expect(toTaskItems(undefined)).toEqual([]);
    expect(toTaskItems({ tasks: [] })).toEqual([]);
  });
});
