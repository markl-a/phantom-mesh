// SPEC-31 coach-review-list — route /coach/review/list; commands_used: daily_review_get (via lib/dailyReview)
//
// iOS sibling of app/src/screens/macos/CoachReviewList.tsx (教練回顧列表 / Coach
// review list). Probes the last 14 days through the offline `daily_review_load`
// command (reused via loadDailyReview) and lists the days that have a review —
// no new backend. Each row shows the date, a 已就緒/pending readiness indicator,
// an optional locked (已加密) badge, and a chevron. Tapping a row hands the date
// to the optional `onOpenDate` prop (router wiring lives in the host app, out of
// scope here). A refresh button re-probes. Honest loading / empty / error states.
//
// SPEC-31 HIG: safe-area insets, Dynamic Type (min-h not fixed h), reachability
// (primary CTA pinned to a sticky bottom footer), ≥44px touch targets, reduced
// motion, bilingual zh/en aria-labels, role=alert/status. spectyn-* palette only.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ChevronRight, ClipboardList, Lock, RefreshCw } from "lucide-react";
import { loadDailyReview, todayIso } from "../lib/dailyReview";
import { useHaptics } from "../lib/useHaptics";

/** How many days back to probe for reviews. 14 keeps the offline scan cheap. */
const SCAN_DAYS = 14;

interface CoachReviewListProps {
  onOpenDate?: (date: string) => void;
}

interface ReviewEntry {
  date: string;
  eventCount: number;
  locked: boolean;
}

/** ISO date `days` before `base` (days ≥ 0). Mirrors todayIso()'s local-day math.
 *  Takes a captured `base` so a full scan uses one consistent "now" (no mid-loop
 *  midnight tick can duplicate/skip a date). */
function isoDaysAgo(days: number, base: Date = new Date()): string {
  const d = new Date(base);
  d.setDate(d.getDate() - days);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

/** Localised weekday + a 今天/昨天 hint for the most recent rows. */
function dayLabel(iso: string): string {
  if (iso === todayIso()) return "今天 Today";
  if (iso === isoDaysAgo(1)) return "昨天 Yesterday";
  const d = new Date(iso + "T00:00:00");
  if (Number.isNaN(d.getTime())) return "";
  return ["週日", "週一", "週二", "週三", "週四", "週五", "週六"][d.getDay()] ?? "";
}

export default function CoachReviewList({ onOpenDate }: CoachReviewListProps) {
  const { impact } = useHaptics();
  const [entries, setEntries] = useState<ReviewEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Sequence guard: only the latest refresh may commit state, and nothing
  // commits after unmount — prevents a slow earlier probe overwriting a newer
  // one (stale-overwrite race) and setState-after-unmount warnings.
  const reqSeq = useRef(0);
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);

  const refresh = useCallback(async () => {
    const seq = ++reqSeq.current;
    const commit = (fn: () => void) => {
      if (alive.current && seq === reqSeq.current) fn();
    };
    commit(() => {
      setLoading(true);
      setError(null);
    });
    const base = new Date();
    const dates = Array.from({ length: SCAN_DAYS }, (_, i) => isoDaysAgo(i, base));
    // allSettled lets us tell apart two cases loadDailyReview's null return
    // alone cannot: a *rejected* probe = real backend failure; a *fulfilled*
    // null = a legitimately empty day. Only "every probe rejected" is an error;
    // all-null-but-fulfilled is an honest empty window, not "backend down".
    const results = await Promise.allSettled(dates.map((iso) => loadDailyReview(iso)));
    const allFailed = results.every((r) => r.status === "rejected");
    if (allFailed) {
      commit(() => {
        setError("每日回顧後端暫時無法使用 (daily review backend unavailable)");
        setEntries([]);
        setLoading(false);
      });
      return;
    }
    const next: ReviewEntry[] = [];
    results.forEach((r, i) => {
      if (r.status !== "fulfilled" || !r.value) return;
      const v = r.value;
      if (v.locked || v.eventCount > 0) {
        next.push({ date: dates[i], eventCount: v.eventCount, locked: v.locked });
      }
    });
    commit(() => {
      setEntries(next);
      setLoading(false);
    });
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const anyLocked = useMemo(() => entries.some((e) => e.locked), [entries]);

  const onRefresh = useCallback(() => {
    impact("medium");
    void refresh();
  }, [impact, refresh]);

  return (
    <div
      className="min-h-screen flex flex-col bg-spectyn-bg text-spectyn-text
        pt-[env(safe-area-inset-top)]
        pl-[env(safe-area-inset-left)] pr-[env(safe-area-inset-right)]"
      data-testid="coach-review-list"
    >
      {/* ── Scrollable content ── */}
      <main className="flex-1 overflow-y-auto px-4 pt-4 pb-4 space-y-5">
        <header className="flex items-center gap-3">
          <div
            className="w-11 h-11 min-h-[44px] rounded-lg bg-spectyn-primary/15 flex items-center justify-center flex-shrink-0"
            aria-hidden="true"
          >
            <ClipboardList size={22} className="text-spectyn-primary" />
          </div>
          <div className="flex-1 min-w-0">
            <h1 className="text-lg font-bold text-spectyn-text">教練回顧 Coach reviews</h1>
            <p className="text-base text-spectyn-muted">最近 {SCAN_DAYS} 天有紀錄的日子</p>
          </div>
        </header>

        {error && (
          <div
            className="bg-spectyn-warning/10 border border-spectyn-warning/40 rounded-lg p-3 text-base text-spectyn-warning flex flex-col gap-2"
            role="alert"
          >
            <span className="min-w-0 break-words">{error}</span>
            <button
              type="button"
              onClick={onRefresh}
              className="self-start min-h-[44px] text-base px-3 py-2 rounded-lg border border-spectyn-warning/40 hover:bg-spectyn-warning/15 transition motion-reduce:transition-none"
              aria-label="重試載入 Retry loading reviews"
            >
              重試 Retry
            </button>
          </div>
        )}

        {loading && entries.length === 0 && !error && (
          <div className="space-y-2" data-testid="review-list-skeleton" role="status" aria-busy="true">
            <span className="sr-only">載入中 Loading reviews…</span>
            {[0, 1, 2, 3].map((i) => (
              <div
                key={i}
                className="min-h-[56px] rounded-lg bg-spectyn-card border border-spectyn-border animate-pulse motion-reduce:animate-none"
                aria-hidden="true"
              />
            ))}
          </div>
        )}

        {!loading && !error && entries.length === 0 && (
          <div
            className="bg-spectyn-card border border-spectyn-border rounded-lg p-6 text-center"
            data-testid="review-list-empty"
            role="status"
          >
            <p className="text-base text-spectyn-text">最近 {SCAN_DAYS} 天還沒有可回顧的紀錄。</p>
            <p className="text-base text-spectyn-muted mt-1">
              No reviews yet — 用專注 / 習慣 / 飲食頁記錄一筆後，當天就會出現在這裡。
            </p>
            <p className="text-base text-spectyn-muted/70 mt-2">
              空白的日子沒關係 — 這是紀錄，不是評分表。
            </p>
          </div>
        )}

        {anyLocked && (
          <div className="bg-spectyn-card border border-spectyn-border rounded-lg p-3 text-base text-spectyn-muted flex items-start gap-2">
            <Lock size={16} className="flex-shrink-0 mt-0.5" aria-hidden="true" />
            <span>
              部分日子的事件已加密 — 載入身分金鑰（<code className="text-spectyn-primary">spectyn init</code>）後才能讀內容。
            </span>
          </div>
        )}

        {entries.length > 0 && (
          <ul className="space-y-2" aria-label="回顧列表 Review list">
            {entries.map((e) => (
              <li key={e.date}>
                <button
                  type="button"
                  onClick={() => {
                    impact("light");
                    onOpenDate?.(e.date);
                  }}
                  className="w-full min-h-[44px] flex items-center gap-3 px-4 py-3 rounded-lg bg-spectyn-card border border-spectyn-border hover:border-spectyn-primary/40 transition motion-reduce:transition-none text-left"
                  data-testid={`review-row-${e.date}`}
                  aria-label={`開啟 ${e.date} 的回顧 Open review for ${e.date}${e.locked ? "，已加密 locked" : `，已就緒 ready, ${e.eventCount} 筆 events`}`}
                >
                  <div className="flex-1 min-w-0">
                    <p className="text-base text-spectyn-text font-medium">{e.date}</p>
                    <p className="text-base text-spectyn-muted">{dayLabel(e.date)}</p>
                  </div>
                  {e.locked ? (
                    <span className="text-base px-2 py-0.5 rounded-full bg-spectyn-border/40 text-spectyn-muted flex items-center gap-1 flex-shrink-0">
                      <Lock size={13} aria-hidden="true" /> 已加密 Locked
                    </span>
                  ) : (
                    <span className="text-base px-2 py-0.5 rounded-full bg-spectyn-success/15 text-spectyn-success flex-shrink-0">
                      已就緒 Ready · {e.eventCount}
                    </span>
                  )}
                  <ChevronRight
                    size={18}
                    className="text-spectyn-muted flex-shrink-0"
                    aria-hidden="true"
                  />
                </button>
              </li>
            ))}
          </ul>
        )}
      </main>

      {/* ── Reachability: primary CTA pinned to a sticky bottom footer ── */}
      <footer
        className="sticky bottom-0 bg-spectyn-bg/95 backdrop-blur border-t border-spectyn-border
          px-4 pt-3 pb-[max(0.75rem,env(safe-area-inset-bottom))]"
      >
        <button
          type="button"
          onClick={onRefresh}
          disabled={loading}
          className="w-full min-h-[48px] flex items-center justify-center gap-2 rounded-lg bg-spectyn-primary text-spectyn-bg text-base font-semibold disabled:opacity-60 transition motion-reduce:transition-none"
          aria-label="重新整理回顧列表 Refresh review list"
        >
          <RefreshCw
            size={18}
            className={loading ? "animate-spin motion-reduce:animate-none" : ""}
            aria-hidden="true"
          />
          {loading ? "重新整理中… Refreshing…" : "重新整理 Refresh"}
        </button>
      </footer>
    </div>
  );
}
