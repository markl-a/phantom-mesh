// P1-2 M1 — Task State supervisor tab. Polls the BACKEND node's durable task
// queue over HMAC (/rpc/tasks/list via lib/supervisor) and renders the recent
// tasks + the pending high-risk approvals awaiting the operator. Read-only;
// reuses the AppTemplate connection context (no parallel state manager).
import { useEffect, useState, useCallback } from "react";
import { useApp } from "./AppTemplate";
import { fetchTasks, type SupTask, type SupPending } from "../../lib/supervisor";

const C = {
  surface: "#161a22",
  border: "#272d3a",
  text: "#e7eaf0",
  muted: "#8b93a6",
  success: "#34d399",
  warn: "#fbbf24",
  danger: "#f87171",
};
const statusColor = (s: string) =>
  s === "running"
    ? C.warn
    : s === "done" || s === "completed"
      ? C.success
      : s === "error" || s === "failed"
        ? C.danger
        : C.muted;

export default function SupervisorTasks() {
  const { baseUrl, secret, addLog } = useApp();
  const [tasks, setTasks] = useState<SupTask[]>([]);
  const [pending, setPending] = useState<SupPending[]>([]);
  const [err, setErr] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    setErr(null);
    try {
      const r = await fetchTasks(baseUrl, secret);
      setTasks(r.tasks);
      setPending(r.pending);
      addLog("ok", `tasks: ${r.tasks.length} · pending ${r.pending.length}`);
    } catch (e) {
      setErr(String(e).slice(0, 120));
      addLog("err", `tasks ${String(e).slice(0, 80)}`);
    } finally {
      setLoading(false);
    }
  }, [baseUrl, secret, addLog]);

  useEffect(() => {
    load();
  }, [load]);

  // Light auto-refresh so a phone parked on this tab stays live without a
  // manual tap. 15s is cheap (one HMAC POST) and stops on unmount.
  useEffect(() => {
    const id = setInterval(() => {
      load();
    }, 15_000);
    return () => clearInterval(id);
  }, [load]);

  return (
    <div style={{ flex: 1, overflowY: "auto", padding: "18px 16px 110px" }}>
      <div
        style={{
          display: "flex",
          justifyContent: "space-between",
          alignItems: "center",
          marginBottom: 14,
        }}
      >
        <div style={{ fontSize: 22, fontWeight: 800 }}>任務</div>
        <button
          onClick={load}
          disabled={loading}
          style={{
            background: C.surface,
            border: `1px solid ${C.border}`,
            color: C.text,
            borderRadius: 999,
            padding: "8px 14px",
            fontWeight: 600,
          }}
        >
          {loading ? "更新中…" : "↻"}
        </button>
      </div>

      {err && <div style={{ color: C.danger, marginBottom: 12 }}>讀取失敗:{err}</div>}

      {pending.length > 0 && (
        <div style={{ marginBottom: 16 }}>
          <div style={{ fontSize: 13, fontWeight: 700, color: C.warn, marginBottom: 8 }}>
            待審核 ({pending.length})
          </div>
          {pending.map((p) => (
            <div
              key={p.approvalId}
              style={{
                background: C.surface,
                border: `1px solid ${C.warn}`,
                borderRadius: 14,
                padding: "12px 14px",
                marginBottom: 10,
              }}
            >
              <div style={{ fontWeight: 700 }}>
                {p.tool} · <span style={{ color: C.warn }}>{p.risk}</span>
              </div>
              <div style={{ fontSize: 13, color: C.muted, marginTop: 4 }}>{p.reason}</div>
            </div>
          ))}
        </div>
      )}

      {tasks.length === 0 && !loading && !err && (
        <div style={{ color: C.muted }}>目前沒有任務。</div>
      )}
      {tasks.map((t) => (
        <div
          key={t.id}
          style={{
            background: C.surface,
            border: `1px solid ${C.border}`,
            borderRadius: 14,
            padding: "13px 15px",
            marginBottom: 10,
          }}
        >
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
            <span style={{ fontWeight: 700 }}>{t.agent}</span>
            <span
              style={{
                fontSize: 12,
                fontWeight: 800,
                color: statusColor(t.status),
                textTransform: "uppercase",
              }}
            >
              {t.status}
            </span>
          </div>
          <div
            style={{
              fontSize: 14,
              color: C.text,
              marginTop: 6,
              whiteSpace: "pre-wrap",
              wordBreak: "break-word",
            }}
          >
            {t.prompt.slice(0, 160)}
            {t.prompt.length > 160 ? "…" : ""}
          </div>
          <div style={{ fontSize: 11.5, color: C.muted, marginTop: 6 }}>
            {t.turns} turns · ${t.costUsd.toFixed(3)}
            {t.error ? ` · err: ${t.error.slice(0, 60)}` : ""}
          </div>
        </div>
      ))}
    </div>
  );
}
