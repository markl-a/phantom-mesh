// SPEC-34 Screen 3 — Coach Review (每日回顧), Android/mobile variant.
//
// The macOS reader is app/src/screens/macos/CoachReviewReader.tsx; this is
// the mobile-shell-fitted twin. Both read the same offline `daily_review_load`
// command via lib/dailyReview.ts (no LLM/network) and render the three states
// from docs/superpowers/design/tui-daily-review.md: has-events (grouped by
// goal-tag), empty (neutral — an empty day is fine), locked (no identity key).
// Design lineage: BIG-GOAL P2 multimodal → Life Track → SPEC-23 coach.
//
// Reached from Settings → 每日回顧 (and any future FCM coach-review deep-link
// can route to /review). Lives inside MobileShell so the bottom tab nav stays.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useNavigate, useSearchParams } from "react-router-dom";
import {
  CalendarDays,
  ChevronLeft,
  ChevronRight,
  Lock,
  RefreshCw,
} from "lucide-react";
import {
  loadDailyReview,
  parseReview,
  todayIso,
  KIND_EMOJI,
  type ReviewRow,
} from "../../lib/dailyReview";
import type { DailyReviewView } from "../../lib/generated/daily_review/DailyReviewView";

function shiftDate(iso: string, days: number): string {
  const d = new Date(iso + "T00:00:00");
  if (Number.isNaN(d.getTime())) return iso;
  d.setDate(d.getDate() + days);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

// Accept a `?date=YYYY-MM-DD` deep-link as the starting day (e.g. a Recall hit
// tapped through to its day, or a future coach-review notification). Reject
// malformed or future dates — fall back to today. In-screen prev/next nav is
// unaffected (it drives local state, not the URL).
function validDateParam(p: string | null): string | null {
  if (!p || !/^\d{4}-\d{2}-\d{2}$/.test(p)) return null;
  const d = new Date(p + "T00:00:00");
  if (Number.isNaN(d.getTime())) return null;
  return p <= todayIso() ? p : null;
}

export default function MobileDailyReview() {
  const navigate = useNavigate();
  const [searchParams] = useSearchParams();
  const [date, setDate] = useState<string>(
    () => validDateParam(searchParams.get("date")) ?? todayIso(),
  );
  const [view, setView] = useState<DailyReviewView | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Request sequence guard: prev/next/refresh fire overlapping loads, and a
  // slow older request must not clobber a newer one (would show the current
  // date with another day's events). Only the latest request applies state.
  const reqSeq = useRef(0);
  const refresh = useCallback(async (iso: string) => {
    const myReq = ++reqSeq.current;
    setLoading(true);
    setError(null);
    try {
      const v = await loadDailyReview(iso);
      if (myReq !== reqSeq.current) return;
      setView(v);
      if (!v) setError("每日回顧後端暫時無法使用");
    } catch (e) {
      if (myReq !== reqSeq.current) return;
      setError(String(e ?? "未知錯誤"));
      setView(null);
    } finally {
      if (myReq === reqSeq.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh(date);
  }, [date, refresh]);

  const rows: ReviewRow[] = useMemo(
    () => (view ? parseReview(view.markdown) : []),
    [view],
  );
  const isToday = date === todayIso();

  return (
    <div
      className="flex flex-col h-full overflow-y-auto"
      data-testid="mobile-daily-review"
    >
      {/* Header bar — matches the Settings sub-panel pattern (back + title) */}
      <div className="flex items-center px-2 py-2.5 border-b border-spectyn-border flex-shrink-0">
        <button
          onClick={() => navigate("/settings")}
          className="text-spectyn-text p-2 -m-2 flex items-center gap-1"
          aria-label="返回設定"
        >
          <ChevronLeft size={20} />
          <span className="text-sm">設定</span>
        </button>
        <span className="text-sm font-medium text-spectyn-text mx-auto pr-8">
          每日回顧
        </span>
      </div>

      <div className="p-3 space-y-3">
        {/* Date strip */}
        <div className="flex items-center gap-2">
          <div className="w-9 h-9 rounded-lg bg-spectyn-primary/15 flex items-center justify-center flex-shrink-0">
            <CalendarDays size={18} className="text-spectyn-primary" />
          </div>
          <div className="flex-1 min-w-0">
            <div className="text-sm font-semibold text-spectyn-text">{date}</div>
            <div className="text-[11px] text-spectyn-muted">
              Daily review{view ? ` · ${view.eventCount} 筆事件` : ""}
            </div>
          </div>
          <div className="flex items-center gap-0.5 flex-shrink-0">
            <button
              onClick={() => setDate(shiftDate(date, -1))}
              className="text-spectyn-muted hover:text-spectyn-text p-1.5"
              aria-label="前一天"
            >
              <ChevronLeft size={18} />
            </button>
            <button
              onClick={() => setDate(shiftDate(date, 1))}
              disabled={isToday}
              className="text-spectyn-muted hover:text-spectyn-text disabled:opacity-30 p-1.5"
              aria-label="後一天"
            >
              <ChevronRight size={18} />
            </button>
            <button
              onClick={() => void refresh(date)}
              className="text-spectyn-muted hover:text-spectyn-text p-1.5"
              aria-label="重新整理"
            >
              <RefreshCw size={18} className={loading ? "animate-spin" : ""} />
            </button>
          </div>
        </div>

        {!isToday && (
          <button
            onClick={() => setDate(todayIso())}
            className="text-xs text-spectyn-primary hover:underline"
          >
            ↩ 回到今天
          </button>
        )}

        {error && (
          <div className="bg-spectyn-warning/10 border border-spectyn-warning/40 rounded-lg p-3 text-sm text-spectyn-warning">
            {error}
          </div>
        )}

        {view?.flagged && (
          <div className="bg-spectyn-warning/10 border border-spectyn-warning/30 rounded-lg p-3 text-xs text-spectyn-muted">
            部分內容被標記 — 顯示原始紀錄(shame-free 防護)。
          </div>
        )}

        {!error && view?.locked && (
          <div className="bg-spectyn-card border border-spectyn-border rounded-lg p-6 text-center">
            <Lock size={22} className="text-spectyn-muted mx-auto mb-2" />
            <p className="text-sm text-spectyn-text">事件已加密(age v1)</p>
            <p className="text-xs text-spectyn-muted mt-1">
              尚未載入身分金鑰 — 執行{" "}
              <code className="text-spectyn-primary">spectyn init</code>{" "}
              後重新整理。
            </p>
          </div>
        )}

        {!error && view && !view.locked && view.eventCount === 0 && (
          <div className="bg-spectyn-card border border-spectyn-border rounded-lg p-6 text-center">
            <p className="text-sm text-spectyn-text">{date} 沒有 Life Node 事件。</p>
            <p className="text-xs text-spectyn-muted mt-1">
              用專注 / 習慣 / 飲食頁記錄一筆後,會在這裡按目標標籤分組。
            </p>
            <p className="text-[11px] text-spectyn-muted/70 mt-2">
              空白的一天沒關係 — 這是紀錄,不是評分表。
            </p>
          </div>
        )}

        {!error && view && !view.locked && view.eventCount > 0 && (
          <div className="space-y-2">
            {rows
              .filter((r) => r.kind === "group" || r.kind === "bullet")
              .map((r, i) => {
                if (r.kind === "group") {
                  return (
                    <div
                      key={`g-${i}`}
                      className="flex items-baseline gap-2 pt-2 first:pt-0"
                    >
                      <span className="text-sm font-semibold text-spectyn-primary">
                        {r.tag}
                      </span>
                      <span className="text-[11px] text-spectyn-muted">
                        ({r.n})
                      </span>
                    </div>
                  );
                }
                return (
                  <div
                    key={`b-${i}`}
                    className="flex items-start gap-3 px-3 py-2 rounded bg-spectyn-card border border-spectyn-border ml-1"
                  >
                    <span className="text-base w-6 text-center flex-shrink-0">
                      {KIND_EMOJI[r.eventKind] ?? "•"}
                    </span>
                    <div className="flex-1 min-w-0">
                      <span className="text-xs text-spectyn-muted mr-2">
                        {r.time}
                      </span>
                      <span className="text-sm text-spectyn-text break-words">
                        {r.summary}
                      </span>
                    </div>
                  </div>
                );
              })}
          </div>
        )}
      </div>
    </div>
  );
}
