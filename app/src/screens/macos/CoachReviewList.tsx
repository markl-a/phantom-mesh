// SPEC-41 §10.6 — S6 coach review list (教練回顧列表).
//
// Settings → Privacy → "View coach reviews" lands here. Lists the most recent
// days' Life Node daily reviews newest-first; each row shows the date, weekday,
// and event count, and links to the S5 reader (CoachReviewReader) for that date
// via the `?date=YYYY-MM-DD` query param.
//
// Frontend-only: reuses the existing offline `daily_review_load` command through
// `loadDailyReview` (lib/dailyReview) — no new backend. We probe the last 14 days
// and keep only days that actually have events (an empty day is skipped from the
// list, but the empty-state still appears when *no* day has any). State machine
// mirrors §10.13: default / loading (skeleton) / error / empty.

import { useCallback, useEffect, useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { ChevronRight, ClipboardList, Lock, RefreshCw } from "lucide-react";
import { loadDailyReview, todayIso } from "../../lib/dailyReview";

/** How many days back to probe for reviews. 14 keeps the offline scan cheap. */
const SCAN_DAYS = 14;

interface ReviewEntry {
  date: string;
  eventCount: number;
  locked: boolean;
}

/** ISO date `days` before today (days ≥ 0). */
function isoDaysAgo(days: number): string {
  const d = new Date();
  d.setDate(d.getDate() - days);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

/** Localised weekday + a "today" / "yesterday" hint for the most recent rows. */
function dayLabel(iso: string): string {
  const today = todayIso();
  if (iso === today) return "今天";
  if (iso === isoDaysAgo(1)) return "昨天";
  const d = new Date(iso + "T00:00:00");
  if (Number.isNaN(d.getTime())) return "";
  return ["週日", "週一", "週二", "週三", "週四", "週五", "週六"][d.getDay()] ?? "";
}

export default function CoachReviewList() {
  const [entries, setEntries] = useState<ReviewEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const dates = Array.from({ length: SCAN_DAYS }, (_, i) => isoDaysAgo(i));
      const views = await Promise.all(dates.map((iso) => loadDailyReview(iso)));
      // A null view means the offline command is unavailable (web mode) for
      // *every* date — surface that as an error rather than a silent empty list.
      if (views.every((v) => v === null)) {
        setError("每日回顧後端暫時無法使用");
        setEntries([]);
        return;
      }
      const next: ReviewEntry[] = [];
      views.forEach((v, i) => {
        if (!v) return;
        if (v.locked || v.eventCount > 0) {
          next.push({ date: dates[i], eventCount: v.eventCount, locked: v.locked });
        }
      });
      setEntries(next);
    } catch (e) {
      setError(String(e ?? "未知錯誤"));
      setEntries([]);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { void refresh(); }, [refresh]);

  const anyLocked = useMemo(() => entries.some((e) => e.locked), [entries]);

  return (
    <div className="max-w-2xl mx-auto space-y-5" data-testid="coach-review-list">
      <header className="flex items-center gap-3">
        <div className="w-10 h-10 rounded-lg bg-spectyn-primary/15 flex items-center justify-center">
          <ClipboardList size={20} className="text-spectyn-primary" />
        </div>
        <div className="flex-1">
          <h1 className="text-xl font-bold text-spectyn-text">教練回顧</h1>
          <p className="text-xs text-spectyn-muted">Coach reviews · 最近 {SCAN_DAYS} 天有紀錄的日子</p>
        </div>
        <button
          onClick={() => void refresh()}
          className="text-spectyn-muted hover:text-spectyn-text p-1.5"
          title="重新整理"
          aria-label="重新整理"
        >
          <RefreshCw size={16} className={loading ? "animate-spin" : ""} />
        </button>
      </header>

      {error && (
        <div
          className="bg-spectyn-warning/10 border border-spectyn-warning/40 rounded-lg p-3 text-sm text-spectyn-warning flex items-center justify-between gap-3"
          role="alert"
        >
          <span className="min-w-0 break-words">{error}</span>
          <button
            onClick={() => void refresh()}
            className="flex-shrink-0 text-xs px-2 py-1 rounded border border-spectyn-warning/40 hover:bg-spectyn-warning/15"
          >
            重試
          </button>
        </div>
      )}

      {loading && entries.length === 0 && !error && (
        <div className="space-y-2" data-testid="review-list-skeleton" aria-busy="true">
          {[0, 1, 2, 3].map((i) => (
            <div key={i} className="h-14 rounded-lg bg-spectyn-card border border-spectyn-border animate-pulse" />
          ))}
        </div>
      )}

      {!loading && !error && entries.length === 0 && (
        <div className="bg-spectyn-card border border-spectyn-border rounded-lg p-6 text-center" data-testid="review-list-empty">
          <p className="text-sm text-spectyn-text">最近 {SCAN_DAYS} 天還沒有可回顧的紀錄。</p>
          <p className="text-xs text-spectyn-muted mt-1">
            用專注 / 習慣 / 飲食頁記錄一筆後，當天就會出現在這裡。
          </p>
          <p className="text-[11px] text-spectyn-muted/70 mt-2">空白的日子沒關係 — 這是紀錄，不是評分表。</p>
        </div>
      )}

      {anyLocked && (
        <div className="bg-spectyn-card border border-spectyn-border rounded-lg p-3 text-xs text-spectyn-muted flex items-center gap-2">
          <Lock size={13} className="flex-shrink-0" />
          部分日子的事件已加密 — 載入身分金鑰（<code className="text-spectyn-primary">spectyn init</code>）後才能讀內容。
        </div>
      )}

      {entries.length > 0 && (
        <div className="space-y-1.5">
          {entries.map((e) => (
            <Link
              key={e.date}
              to={`/review?date=${e.date}`}
              className="flex items-center gap-3 px-4 py-3 rounded-lg bg-spectyn-card border border-spectyn-border hover:border-spectyn-primary/40 transition group"
              data-testid={`review-row-${e.date}`}
            >
              <div className="flex-1 min-w-0">
                <p className="text-sm text-spectyn-text font-medium">{e.date}</p>
                <p className="text-xs text-spectyn-muted">{dayLabel(e.date)}</p>
              </div>
              {e.locked ? (
                <span className="text-[11px] text-spectyn-muted flex items-center gap-1">
                  <Lock size={11} /> 已加密
                </span>
              ) : (
                <span className="text-[11px] px-2 py-0.5 rounded-full bg-spectyn-primary/10 text-spectyn-primary">
                  {e.eventCount} events
                </span>
              )}
              <ChevronRight size={16} className="text-spectyn-muted group-hover:text-spectyn-primary transition flex-shrink-0" />
            </Link>
          ))}
        </div>
      )}
    </div>
  );
}
