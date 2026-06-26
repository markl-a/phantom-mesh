// P1-2 M2 — Capture History supervisor tab. Polls the BACKEND node's recent
// captured life-node events over HMAC (/rpc/captures/recent via lib/supervisor)
// and renders them newest-first with the kind emoji + local time + goal tags.
// Read-only; reuses the AppTemplate connection context.
import { useEffect, useState, useCallback } from "react";
import { useApp } from "./AppTemplate";
import { fetchCaptures, type SupCapture } from "../../lib/supervisor";
import { KIND_EMOJI } from "../../lib/dailyReview";

const C = {
  surface: "#161a22",
  border: "#272d3a",
  text: "#e7eaf0",
  muted: "#8b93a6",
  danger: "#f87171",
};

export default function SupervisorCaptures() {
  const { baseUrl, secret, addLog } = useApp();
  const [caps, setCaps] = useState<SupCapture[]>([]);
  const [err, setErr] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    setErr(null);
    try {
      const r = await fetchCaptures(baseUrl, secret);
      setCaps(r);
      addLog("ok", `captures: ${r.length}`);
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
        <div style={{ fontSize: 22, fontWeight: 800 }}>擷取</div>
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
      {caps.length === 0 && !loading && !err && (
        <div style={{ color: C.muted }}>尚無擷取事件。</div>
      )}
      {caps.map((c) => (
        <div
          key={c.id}
          style={{
            background: C.surface,
            border: `1px solid ${C.border}`,
            borderRadius: 14,
            padding: "12px 15px",
            marginBottom: 10,
          }}
        >
          <div style={{ fontWeight: 700 }}>
            {KIND_EMOJI[c.kind] ?? "•"} {c.kind}
          </div>
          <div style={{ fontSize: 12, color: C.muted, marginTop: 4 }}>
            {new Date(c.timestamp).toLocaleString()}
            {c.tags.length ? ` · ${c.tags.join(", ")}` : ""}
          </div>
        </div>
      ))}
    </div>
  );
}
