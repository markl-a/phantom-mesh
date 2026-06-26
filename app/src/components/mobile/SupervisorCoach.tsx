// P1-2 M3 — Coach Review supervisor tab. Fetches the BACKEND node's offline
// daily-review aggregate markdown over HMAC (/rpc/review via lib/supervisor)
// and renders it with the shared parseReview markdown parser from
// lib/dailyReview (pure, no Tauri). Reviews the supervised backend node, NOT
// this device (that's MobileDailyReview). Read-only; reuses AppTemplate context.
import { useEffect, useState, useCallback } from "react";
import { useApp } from "./AppTemplate";
import { fetchReview } from "../../lib/supervisor";
import { parseReview, type ReviewRow } from "../../lib/dailyReview";

const C = {
  surface: "#161a22",
  border: "#272d3a",
  text: "#e7eaf0",
  muted: "#8b93a6",
  accent: "#7c5cff",
  danger: "#f87171",
};

export default function SupervisorCoach() {
  const { baseUrl, secret, addLog } = useApp();
  const [rows, setRows] = useState<ReviewRow[]>([]);
  const [date, setDate] = useState("");
  const [err, setErr] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    setErr(null);
    try {
      const r = await fetchReview(baseUrl, secret);
      setDate(r.date);
      setRows(parseReview(r.markdown));
      addLog("ok", `review ${r.date}: ${r.markdown.length} chars`);
    } catch (e) {
      setErr(String(e).slice(0, 120));
    } finally {
      setLoading(false);
    }
  }, [baseUrl, secret, addLog]);

  useEffect(() => {
    load();
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
        <div style={{ fontSize: 22, fontWeight: 800 }}>回顧{date ? ` · ${date}` : ""}</div>
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
      {rows.length === 0 && !loading && !err && (
        <div style={{ color: C.muted }}>今天還沒有擷取事件。</div>
      )}
      {rows.map((row, i) => {
        if (row.kind === "title")
          return (
            <div key={i} style={{ fontSize: 18, fontWeight: 800, marginBottom: 8 }}>
              {row.text}
            </div>
          );
        if (row.kind === "count")
          return (
            <div key={i} style={{ color: C.muted, marginBottom: 12 }}>
              Events captured: {row.text}
            </div>
          );
        if (row.kind === "group")
          return (
            <div
              key={i}
              style={{ fontWeight: 700, color: C.accent, marginTop: 12, marginBottom: 6 }}
            >
              {row.tag} ({row.n})
            </div>
          );
        if (row.kind === "bullet")
          return (
            <div
              key={i}
              style={{
                background: C.surface,
                border: `1px solid ${C.border}`,
                borderRadius: 12,
                padding: "10px 13px",
                marginBottom: 8,
              }}
            >
              <div style={{ fontWeight: 700, fontSize: 13 }}>
                {row.eventKind} · <span style={{ color: C.muted }}>{row.time}</span>
              </div>
              <div style={{ fontSize: 14, marginTop: 4 }}>{row.summary}</div>
            </div>
          );
        return (
          <div key={i} style={{ color: C.muted, marginBottom: 6 }}>
            {row.text}
          </div>
        );
      })}
    </div>
  );
}
