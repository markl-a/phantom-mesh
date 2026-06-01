// SPEC-31 coach-review-detail — route /coach/review; commands_used: daily_review_get (via lib/dailyReview)
//
// iOS push-notification deep-link landing (SPEC-31 JS1): a coach notification taps
// through to this screen for a given date, defaulting to today. It renders the real,
// offline daily review (reuses the `phantom coach review` backend via
// `daily_review_load`) — the "明日的一個行動 / Tomorrow" card surfaced at top, then the
// captured Life Node events grouped by goal-tag. Mirrors the macOS sibling
// app/src/screens/macos/CoachReviewReader.tsx but tuned for iPhone touch / HIG.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { CalendarDays, ChevronLeft, RefreshCw, Lock, Sparkles, AlertTriangle } from "lucide-react";
import {
  loadDailyReview,
  extractTomorrowAction,
  parseReview,
  todayIso,
  KIND_EMOJI,
  type ReviewRow,
} from "../lib/dailyReview";
import type { DailyReviewView } from "../lib/generated/daily_review/DailyReviewView";
import { useHaptics } from "../lib/useHaptics";

export interface CoachReviewDetailProps {
  /** ISO date (YYYY-MM-DD) the review covers; defaults to local-today. */
  date?: string;
  /** Back affordance for the navigation host (router wiring is out of scope). */
  onBack?: () => void;
}

export default function CoachReviewDetail({ date: dateProp, onBack }: CoachReviewDetailProps) {
  // Derive from the prop (NOT a frozen useState initializer) so a re-used
  // component receiving a new deep-link date re-loads instead of showing stale.
  const date = dateProp ?? todayIso();
  const { impact } = useHaptics();

  // Loaded result is STAMPED with the iso it belongs to. We derive the visible
  // view/error/loading from whether the stamp matches the current `date`, so a
  // deep-link date change shows the skeleton synchronously (same render) — there
  // is no post-commit-effect frame where the new date renders old data.
  const [loaded, setLoaded] = useState<
    { iso: string; view: DailyReviewView | null; error: string | null } | null
  >(null);

  const current = loaded && loaded.iso === date ? loaded : null;
  const view = current?.view ?? null;
  const error = current?.error ?? null;
  const loading = current === null; // no result for THIS date yet → skeleton

  // Sequence guard + unmount guard: only the latest load commits state.
  const reqSeq = useRef(0);
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);

  const refresh = useCallback(async (iso: string) => {
    const seq = ++reqSeq.current;
    const commit = (fn: () => void) => {
      if (alive.current && seq === reqSeq.current) fn();
    };
    try {
      // A thrown error = real backend failure → error state. A null result =
      // legitimately no review for this date → empty state in render (not faked
      // as an error). Either way the result is stamped with `iso`.
      const v = await loadDailyReview(iso);
      commit(() => setLoaded({ iso, view: v, error: null }));
    } catch (e) {
      commit(() =>
        setLoaded({ iso, view: null, error: String(e ?? "未知錯誤 / Unknown error") }),
      );
    }
  }, []);

  useEffect(() => { void refresh(date); }, [date, refresh]);

  // Fire a light success haptic once a review with content lands (deep-link arrival).
  useEffect(() => {
    if (view && !view.locked && view.eventCount > 0) impact("medium");
  }, [view, impact]);

  const rows: ReviewRow[] = useMemo(
    () => (view ? parseReview(view.markdown) : []),
    [view],
  );
  const tomorrow = useMemo(
    () => (view ? extractTomorrowAction(view.markdown) : null),
    [view],
  );

  const hasEvents = !!view && !view.locked && view.eventCount > 0;

  return (
    <div data-testid="coach-review-detail" className="min-h-screen flex flex-col bg-phantom-bg text-phantom-text pt-[env(safe-area-inset-top)] pb-[env(safe-area-inset-bottom)] pl-[env(safe-area-inset-left)] pr-[env(safe-area-inset-right)]">
      {/* Header — DOM order matches visual order. */}
      <header className="flex items-center gap-3 px-4 py-3 border-b border-phantom-border">
        {onBack && (
          <button
            onClick={() => { impact("light"); onBack(); }}
            className="min-h-[44px] min-w-[44px] -ml-2 flex items-center justify-center rounded-lg text-phantom-muted hover:text-phantom-text transition-colors motion-reduce:transition-none"
            aria-label="返回 / Back"
          >
            <ChevronLeft size={22} aria-hidden="true" />
          </button>
        )}
        <div className="w-10 h-10 rounded-lg bg-phantom-primary/15 flex items-center justify-center flex-shrink-0">
          <CalendarDays size={20} className="text-phantom-primary" aria-hidden="true" />
        </div>
        <div className="flex-1 min-w-0">
          <h1 className="text-lg font-bold text-phantom-text truncate">每日回顧</h1>
          <p className="text-sm text-phantom-muted truncate">
            Daily review · {date}{view ? ` · ${view.eventCount} events` : ""}
          </p>
        </div>
      </header>

      {/* Scrollable body. */}
      <main className="flex-1 overflow-y-auto px-4 py-4 space-y-4">
        {/* Error — honest state. */}
        {error && (
          <div
            className="bg-phantom-danger/10 border border-phantom-danger/40 rounded-lg p-4 text-base text-phantom-danger flex items-start gap-3"
            role="alert"
          >
            <AlertTriangle size={18} className="flex-shrink-0 mt-0.5" aria-hidden="true" />
            <span className="min-w-0 break-words">{error}</span>
          </div>
        )}

        {/* Loading — honest state, reduced-motion aware. */}
        {loading && !view && !error && (
          <div className="space-y-3" role="status" aria-busy="true" aria-label="載入中 / Loading">
            <div className="flex items-center gap-2 text-phantom-muted text-base">
              <RefreshCw size={16} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />
              <span>載入回顧中… / Loading review…</span>
            </div>
            {[0, 1, 2].map((i) => (
              <div
                key={i}
                className="min-h-[48px] rounded-lg bg-phantom-card border border-phantom-border animate-pulse motion-reduce:animate-none"
                aria-hidden="true"
              />
            ))}
          </div>
        )}

        {/* Tomorrow action — highlighted card surfaced at top of the brief. */}
        {tomorrow && !tomorrow.skipped && (
          <section
            className="bg-phantom-primary/10 border border-phantom-primary/40 rounded-lg p-4"
            aria-label="明日的一個行動 / Tomorrow's one action"
          >
            <p className="text-sm font-semibold text-phantom-primary flex items-center gap-1.5 mb-1.5">
              <Sparkles size={14} aria-hidden="true" /> 明天行動 / Tomorrow
            </p>
            <p className="text-lg text-phantom-text whitespace-pre-wrap break-words leading-relaxed">
              {tomorrow.text}
            </p>
          </section>
        )}
        {tomorrow?.skipped && (
          <div className="bg-phantom-card border border-phantom-border rounded-lg p-4 text-sm text-phantom-muted">
            明日行動已略過 — 需設定 <code className="text-phantom-primary">GEMINI_API_KEY</code> 才能產生教練建議。
            <span className="block mt-1">Tomorrow's action skipped — set GEMINI_API_KEY for coaching.</span>
          </div>
        )}

        {/* Shame-free flag banner. */}
        {hasEvents && view?.flagged && (
          <div className="bg-phantom-warning/10 border border-phantom-warning/40 rounded-lg p-3 text-sm text-phantom-muted">
            部分內容被標記 — 顯示原始紀錄(shame-free 防護)。
            <span className="block mt-1">Some entries flagged — showing raw log (shame-free).</span>
          </div>
        )}

        {/* Locked — honest state (no identity key). */}
        {!error && view?.locked && (
          <div className="bg-phantom-card border border-phantom-border rounded-lg p-6 text-center">
            <Lock size={24} className="text-phantom-muted mx-auto mb-2" aria-hidden="true" />
            <p className="text-base text-phantom-text">事件已加密(age v1) / Events encrypted</p>
            <p className="text-sm text-phantom-muted mt-1">
              尚未載入身分金鑰 — 執行 <code className="text-phantom-primary">phantom init</code> 後重試。
            </p>
          </div>
        )}

        {/* No review for this date — null result (offline/web mode or never generated). */}
        {!error && !loading && view === null && (
          <div className="bg-phantom-card border border-phantom-border rounded-lg p-6 text-center">
            <p className="text-base text-phantom-text">{date} 尚無回顧。</p>
            <p className="text-sm text-phantom-muted mt-1">No review for this date yet.</p>
          </div>
        )}

        {/* Empty day — neutral, not-found-but-fine state. */}
        {!error && view && !view.locked && view.eventCount === 0 && (
          <div className="bg-phantom-card border border-phantom-border rounded-lg p-6 text-center">
            <p className="text-base text-phantom-text">{date} 沒有 Life Node 事件。</p>
            <p className="text-sm text-phantom-muted mt-1">No Life Node events for this date.</p>
            <p className="text-sm text-phantom-muted/70 mt-2">
              空白的一天沒關係 — 這是紀錄,不是評分表。 / An empty day is fine.
            </p>
          </div>
        )}

        {/* Brief — events grouped by goal-tag. */}
        {hasEvents && (
          <section className="space-y-3" aria-label="回顧內容 / Review entries">
            {rows
              .filter((r) => r.kind === "group" || r.kind === "bullet")
              .map((r, i) => {
                if (r.kind === "group") {
                  return (
                    <div key={`g-${i}`} className="flex items-baseline gap-2 pt-2 first:pt-0">
                      <span className="text-base font-semibold text-phantom-primary">{r.tag}</span>
                      <span className="text-sm text-phantom-muted">({r.n})</span>
                    </div>
                  );
                }
                return (
                  <div
                    key={`b-${i}`}
                    className="flex items-start gap-3 px-3 py-2.5 min-h-[44px] rounded-lg bg-phantom-card border border-phantom-border ml-2"
                  >
                    <span className="text-lg w-6 text-center flex-shrink-0" aria-hidden="true">
                      {KIND_EMOJI[r.eventKind] ?? "•"}
                    </span>
                    <div className="flex-1 min-w-0">
                      <span className="text-sm text-phantom-muted mr-2">{r.time}</span>
                      <span className="text-base text-phantom-text break-words">{r.summary}</span>
                    </div>
                  </div>
                );
              })}
          </section>
        )}
      </main>

      {/* Sticky bottom footer — primary CTA within thumb reach (reachability). */}
      <footer className="sticky bottom-0 border-t border-phantom-border bg-phantom-bg px-4 py-3 pb-[max(0.75rem,env(safe-area-inset-bottom))]">
        <button
          onClick={() => { impact("light"); void refresh(date); }}
          disabled={loading}
          className="w-full min-h-[48px] flex items-center justify-center gap-2 rounded-lg bg-phantom-primary text-phantom-bg font-semibold text-base disabled:opacity-50 transition-opacity motion-reduce:transition-none"
          aria-label="重新整理回顧 / Refresh review"
        >
          <RefreshCw
            size={18}
            className={loading ? "animate-spin motion-reduce:animate-none" : ""}
            aria-hidden="true"
          />
          {loading ? "載入中… / Loading…" : "重新整理 / Refresh"}
        </button>
      </footer>
    </div>
  );
}
