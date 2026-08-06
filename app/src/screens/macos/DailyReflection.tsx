// macOS screen — 每日對齊反思 (partner MVP §B, NORTH-STAR Q2 proactive half).
//
// Surfaces the once-a-day gentle alignment reflection the partner core produces
// ("what I said I wanted vs what actually happened", ally-tone). There's no HTTP
// endpoint for it — the reflection is fired daily by the coach daemon
// (`ai.spectynmesh.coach`, 21:00 via launchd → `spectyn coach review` →
// partner::daily_reflection) and appended to ~/.spectyn-mesh/partner-signals.jsonl.
// This panel reads the latest such record (offline) and explains the daily cadence.

import { useCallback, useEffect, useState } from "react";
import { HeartHandshake, RefreshCw, Clock } from "lucide-react";
import { loadLatestReflection, type LatestReflection } from "../../lib/partnerBrain";

/** Human-friendly relative-ish timestamp for a unix-seconds value. */
function formatTs(ts: number): string {
  if (!ts) return "";
  const d = new Date(ts * 1000);
  if (Number.isNaN(d.getTime())) return "";
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

export default function DailyReflection() {
  const [reflection, setReflection] = useState<LatestReflection | null>(null);
  const [loading, setLoading] = useState(true);

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      setReflection(await loadLatestReflection());
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { void refresh(); }, [refresh]);

  return (
    <div className="max-w-2xl mx-auto space-y-5" data-testid="daily-reflection">
      <header className="flex items-center gap-3">
        <div className="w-10 h-10 rounded-lg bg-spectyn-primary/15 flex items-center justify-center">
          <HeartHandshake size={20} className="text-spectyn-primary" />
        </div>
        <div className="flex-1">
          <h1 className="text-xl font-bold text-spectyn-text">每日對齊反思</h1>
          <p className="text-xs text-spectyn-muted">Daily alignment · 夥伴每天的一段溫和回顧</p>
        </div>
        <button
          onClick={() => void refresh()}
          className="text-spectyn-muted hover:text-spectyn-text p-1.5 min-w-[44px] min-h-[44px] inline-flex items-center justify-center"
          title="重新整理"
          aria-label="重新整理"
        >
          <RefreshCw size={16} className={loading ? "animate-spin" : ""} />
        </button>
      </header>

      {/* How it works — the daily cadence (no HTTP endpoint; coach daemon fires it). */}
      <div className="bg-spectyn-card border border-spectyn-border rounded-lg p-3 flex items-start gap-2.5">
        <Clock size={15} className="text-spectyn-muted mt-0.5 flex-shrink-0" />
        <p className="text-xs text-spectyn-muted leading-relaxed">
          反思由教練守護程式每天 <span className="text-spectyn-text">21:00</span> 自動產生
          （<code className="text-spectyn-primary">ai.spectynmesh.coach</code>），
          讀過去 24 小時的訊號與你的目標，給一段對齊「我說我想要的」與「今天實際發生的」的溫和觀察 —
          當盟友，不當保姆。這裡顯示最近一次的反思。
        </p>
      </div>

      {loading && !reflection && (
        <div className="space-y-2" aria-busy="true">
          {[0, 1].map((i) => (
            <div key={i} className="h-12 rounded bg-spectyn-card border border-spectyn-border animate-pulse" />
          ))}
        </div>
      )}

      {!loading && !reflection && (
        <div className="bg-spectyn-card border border-spectyn-border rounded-lg p-6 text-center">
          <p className="text-sm text-spectyn-text">還沒有反思紀錄。</p>
          <p className="text-xs text-spectyn-muted mt-1">
            教練守護程式第一次跑（每天 21:00）後，最近一次的反思會顯示在這裡。
          </p>
          <p className="text-[11px] text-spectyn-muted/70 mt-2">
            想立刻產生一筆：在終端機跑 <code className="text-spectyn-primary">spectyn coach review</code>。
          </p>
        </div>
      )}

      {reflection && (
        <div className="bg-spectyn-primary/10 border border-spectyn-primary/30 rounded-lg p-4 space-y-2">
          {reflection.ts > 0 && (
            <p className="text-[11px] text-spectyn-muted">{formatTs(reflection.ts)}</p>
          )}
          <p className="text-sm text-spectyn-text whitespace-pre-wrap break-words leading-relaxed">
            {reflection.text}
          </p>
          {reflection.summary && (
            <p className="text-xs text-spectyn-muted border-t border-spectyn-primary/20 pt-2 mt-1">
              <span className="text-spectyn-muted/80">當天摘要：</span>{reflection.summary}
            </p>
          )}
        </div>
      )}
    </div>
  );
}
