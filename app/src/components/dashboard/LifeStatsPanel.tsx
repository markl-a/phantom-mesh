// Dashboard life-log stats card — app counterpart of the TUI `/stats` + CLI
// `spectyn data stats` (BIG-GOAL P2 Life Track). Rolls up all captured Life
// Node events: total · date span · last-7d · by-kind. Read-only.

import { useEffect, useState } from "react";
import { safeInvoke as invoke } from "../../lib/tauri-compat";

interface KindCount { kind: string; count: number }
interface LifeStats {
  total: number;
  byKind: KindCount[];
  earliest: string | null;
  latest: string | null;
  last7d: number;
}

const KIND_EMOJI: Record<string, string> = {
  food: "🍽", focus: "🎯", habit: "✅", text: "📝", image: "🖼", audio: "🎤",
};

export default function LifeStatsPanel() {
  const [stats, setStats] = useState<LifeStats | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const s = await invoke<LifeStats>("life_stats", {});
        if (cancelled) return;
        if (!s || typeof (s as LifeStats).total !== "number") { setError("生活紀錄統計暫時無法使用"); return; }
        setStats(s as LifeStats);
      } catch (e) {
        if (!cancelled) setError(String(e ?? "未知錯誤"));
      }
    })();
    return () => { cancelled = true; };
  }, []);

  if (error) return <p className="text-xs text-spectyn-muted">{error}</p>;
  if (!stats) return <p className="text-xs text-spectyn-muted">載入中…</p>;

  return (
    <div className="space-y-3" data-testid="life-stats-panel">
      <div className="flex items-baseline gap-4">
        <div>
          <div className="text-2xl font-bold text-spectyn-text">{stats.total}</div>
          <div className="text-[11px] text-spectyn-muted">總事件</div>
        </div>
        <div>
          <div className="text-2xl font-bold text-spectyn-primary">{stats.last7d}</div>
          <div className="text-[11px] text-spectyn-muted">近 7 天</div>
        </div>
      </div>
      {stats.byKind.length > 0 && (
        <div className="flex flex-wrap gap-1.5">
          {stats.byKind.map((k) => (
            <span key={k.kind} className="text-xs px-2 py-0.5 rounded-full bg-spectyn-bg border border-spectyn-border text-spectyn-text">
              {KIND_EMOJI[k.kind] ?? "•"} {k.kind} {k.count}
            </span>
          ))}
        </div>
      )}
      {stats.earliest && stats.latest && (
        <div className="text-[11px] text-spectyn-muted">{stats.earliest} → {stats.latest}</div>
      )}
      {stats.total === 0 && (
        <p className="text-xs text-spectyn-muted">尚無事件 — 用專注 / 習慣 / 飲食頁開始記錄。</p>
      )}
    </div>
  );
}
