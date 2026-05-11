import { useState, useEffect, useCallback } from "react";
import { safeInvoke as invoke } from "../../lib/tauri-compat";
import type { TaskItem, TaskStatus } from "../../lib/types";

// ─── Helpers ──────────────────────────────────────────────────────────────────

const STATUS_CONFIG: Record<TaskStatus, { label: string; color: string }> = {
  pending: { label: "待處理", color: "bg-phantom-warning/20 text-phantom-warning" },
  running: { label: "執行中", color: "bg-phantom-primary/20 text-phantom-primary" },
  done: { label: "完成", color: "bg-phantom-success/20 text-phantom-success" },
  failed: { label: "失敗", color: "bg-phantom-danger/20 text-phantom-danger" },
};

// ─── Component ────────────────────────────────────────────────────────────────

export default function TasksPanel() {
  const [tasks, setTasks] = useState<TaskItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const fetchTasks = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const res = await invoke<{ tasks: TaskItem[] }>("get_tasks");
      setTasks(res.tasks);
    } catch (e) {
      setError(String(e));
      setTasks([]);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void fetchTasks();
  }, [fetchTasks]);

  const running = tasks.filter((t) => t.status === "running");
  const recent = tasks.filter((t) => t.status === "done" || t.status === "failed");

  if (loading) {
    return (
      <div className="flex items-center justify-center py-12">
        <div className="w-5 h-5 border-2 border-phantom-primary border-t-transparent rounded-full animate-spin" />
        <span className="ml-3 text-phantom-muted text-sm">載入任務資料...</span>
      </div>
    );
  }

  if (tasks.length === 0 && !error) {
    return (
      <div className="flex flex-col items-center justify-center py-16 text-center">
        <p className="text-phantom-muted text-sm">
          尚無任務 — 在對話頁發送指令開始
        </p>
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-4">
      {error && (
        <div className="bg-phantom-danger/20 border border-phantom-danger rounded p-3 text-sm flex items-center justify-between">
          <span>無法連接 Daemon：{error}</span>
          <button
            onClick={() => void fetchTasks()}
            className="ml-4 px-2 py-1 rounded text-xs bg-phantom-danger/30 hover:bg-phantom-danger/50"
          >
            重試
          </button>
        </div>
      )}

      {/* Running tasks */}
      {running.length > 0 && (
        <section>
          <h3 className="text-xs font-semibold uppercase tracking-wider text-phantom-muted mb-2">
            執行中
          </h3>
          <div className="flex flex-col gap-2">
            {running.map((task) => (
              <div
                key={task.id}
                className="bg-phantom-card border border-phantom-primary/30 rounded-lg p-3 flex items-center gap-3"
              >
                <div className="w-2 h-2 rounded-full bg-phantom-primary animate-pulse flex-shrink-0" />
                <div className="flex-1 min-w-0">
                  <p className="text-sm font-medium truncate">{task.title}</p>
                  <p className="text-xs text-phantom-muted">{task.agent}</p>
                </div>
                <div className="w-24 h-1.5 bg-phantom-border rounded-full overflow-hidden flex-shrink-0">
                  <div className="h-full bg-phantom-primary rounded-full animate-pulse w-1/2" />
                </div>
              </div>
            ))}
          </div>
        </section>
      )}

      {/* Recent completed / failed */}
      {recent.length > 0 && (
        <section>
          <h3 className="text-xs font-semibold uppercase tracking-wider text-phantom-muted mb-2">
            最近完成
          </h3>
          <div className="bg-phantom-card border border-phantom-border rounded-lg overflow-hidden">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b border-phantom-border">
                  <th className="text-left px-3 py-2 text-phantom-muted font-medium text-xs">標題</th>
                  <th className="text-left px-3 py-2 text-phantom-muted font-medium text-xs">Agent</th>
                  <th className="text-left px-3 py-2 text-phantom-muted font-medium text-xs">狀態</th>
                </tr>
              </thead>
              <tbody>
                {recent.slice(0, 10).map((task, i) => (
                  <tr
                    key={task.id}
                    className={`border-b border-phantom-border last:border-0 ${
                      i % 2 === 1 ? "bg-phantom-bg/50" : ""
                    }`}
                  >
                    <td className="px-3 py-2 truncate max-w-[200px]">{task.title}</td>
                    <td className="px-3 py-2 text-phantom-muted">{task.agent}</td>
                    <td className="px-3 py-2">
                      <span
                        className={`inline-block px-2 py-0.5 rounded text-xs font-medium ${
                          STATUS_CONFIG[task.status].color
                        }`}
                      >
                        {STATUS_CONFIG[task.status].label}
                      </span>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </section>
      )}

      {tasks.length > 0 && running.length === 0 && recent.length === 0 && (
        <div className="flex flex-col gap-2">
          {tasks.slice(0, 8).map((task, i) => (
            <div
              key={task.id}
              className={`bg-phantom-card border border-phantom-border rounded-lg px-3 py-2 flex items-center gap-3 ${
                i % 2 === 1 ? "opacity-80" : ""
              }`}
            >
              <span
                className={`inline-block px-2 py-0.5 rounded text-xs font-medium flex-shrink-0 ${
                  STATUS_CONFIG[task.status].color
                }`}
              >
                {STATUS_CONFIG[task.status].label}
              </span>
              <span className="text-sm truncate flex-1">{task.title}</span>
              <span className="text-xs text-phantom-muted flex-shrink-0">{task.agent}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
