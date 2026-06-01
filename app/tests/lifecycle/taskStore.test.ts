// Lifecycle · Unit tests — task store (the "dispatch a task → watch it run →
// see it finish/fail" journey on the dashboard TasksPanel).
//
// The store keeps a newest-first task list. The UI mutates it directly
// (add/update/remove/set) and the daemon's event stream drives it via
// handleEvent (TaskCreated / TaskCompleted / TaskFailed). Pure zustand
// reducer in src/stores/taskStore.ts — no Tauri. Mirrors the reset/store
// conventions in tests/f101/clusterPeersStore.test.ts.

import { beforeEach, describe, expect, it } from "vitest";
import { useTaskStore, type TaskEntry } from "../../src/stores/taskStore";

function makeTask(over: Partial<TaskEntry> = {}): TaskEntry {
  return {
    id: "t1",
    title: "Build the thing",
    status: "pending",
    createdAt: "2026-01-01T00:00:00.000Z",
    updatedAt: "2026-01-01T00:00:00.000Z",
    retryCount: 0,
    ...over,
  };
}

beforeEach(() => {
  useTaskStore.setState({ tasks: [] });
});

describe("taskStore — direct mutations", () => {
  it("add() prepends so the newest task is first", () => {
    useTaskStore.getState().add(makeTask({ id: "a" }));
    useTaskStore.getState().add(makeTask({ id: "b" }));
    expect(useTaskStore.getState().tasks.map((t) => t.id)).toEqual(["b", "a"]);
  });

  it("update() patches only the matching task, preserving other fields", () => {
    useTaskStore.getState().set([makeTask({ id: "a" }), makeTask({ id: "b" })]);
    useTaskStore.getState().update("b", { status: "running", result: "wip" });
    const b = useTaskStore.getState().tasks.find((t) => t.id === "b")!;
    const a = useTaskStore.getState().tasks.find((t) => t.id === "a")!;
    expect(b.status).toBe("running");
    expect(b.result).toBe("wip");
    expect(b.title).toBe("Build the thing"); // untouched
    expect(a.status).toBe("pending"); // sibling untouched
  });

  it("update() on an unknown id is a no-op", () => {
    useTaskStore.getState().set([makeTask({ id: "a" })]);
    useTaskStore.getState().update("nope", { status: "done" });
    expect(useTaskStore.getState().tasks[0].status).toBe("pending");
  });

  it("remove() drops the matching task only", () => {
    useTaskStore.getState().set([makeTask({ id: "a" }), makeTask({ id: "b" })]);
    useTaskStore.getState().remove("a");
    expect(useTaskStore.getState().tasks.map((t) => t.id)).toEqual(["b"]);
  });

  it("set() replaces the whole list", () => {
    useTaskStore.getState().add(makeTask({ id: "old" }));
    useTaskStore.getState().set([makeTask({ id: "new" })]);
    expect(useTaskStore.getState().tasks.map((t) => t.id)).toEqual(["new"]);
  });
});

describe("taskStore — handleEvent (daemon stream)", () => {
  it("TaskCreated inserts a new pending task at the front", () => {
    useTaskStore.getState().handleEvent({
      type: "TaskCreated",
      data: { task_id: "x", title: "Index repo", timestamp: "2026-02-02T03:04:05.000Z" },
    });
    const t = useTaskStore.getState().tasks[0];
    expect(t.id).toBe("x");
    expect(t.title).toBe("Index repo");
    expect(t.status).toBe("pending");
    expect(t.createdAt).toBe("2026-02-02T03:04:05.000Z");
    expect(t.retryCount).toBe(0);
  });

  it("TaskCompleted flips the task to done and records the result", () => {
    useTaskStore.getState().set([makeTask({ id: "x", status: "running" })]);
    useTaskStore.getState().handleEvent({
      type: "TaskCompleted",
      data: { task_id: "x", result: "all green" },
    });
    const t = useTaskStore.getState().tasks[0];
    expect(t.status).toBe("done");
    expect(t.result).toBe("all green");
  });

  it("TaskFailed flips the task to failed", () => {
    useTaskStore.getState().set([makeTask({ id: "x", status: "running" })]);
    useTaskStore.getState().handleEvent({
      type: "TaskFailed",
      data: { task_id: "x", result: "boom" },
    });
    expect(useTaskStore.getState().tasks[0].status).toBe("failed");
  });

  it("full lifecycle: created → completed end-to-end via events", () => {
    useTaskStore.getState().handleEvent({
      type: "TaskCreated",
      data: { task_id: "j", title: "Job" },
    });
    expect(useTaskStore.getState().tasks[0].status).toBe("pending");
    useTaskStore.getState().handleEvent({
      type: "TaskCompleted",
      data: { task_id: "j", result: "ok" },
    });
    const t = useTaskStore.getState().tasks[0];
    expect(t.status).toBe("done");
    expect(t.result).toBe("ok");
  });
});
