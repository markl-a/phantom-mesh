// SPEC-41 macOS screen #3 — Coach Review Reader (每日回顧).
//
// Reads today's (or any date's) Life Node daily review via the real, offline
// `daily_review_load` command (reuses the `phantom coach review` backend). Three
// states mirror docs/superpowers/design/tui-daily-review.md: has-events (grouped
// by goal-tag), empty (neutral — an empty day is fine), locked (no identity key).
// Design lineage: BIG-GOAL P2 multimodal → Life Track → SPEC-23 coach.

import { useCallback, useEffect, useMemo, useState } from "react";
import { useSearchParams } from "react-router-dom";
import { CalendarDays, ChevronLeft, ChevronRight, Lock, RefreshCw, Sparkles } from "lucide-react";
import {
  loadDailyReview,
  generateReview,
  extractTomorrowAction,
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

/** A valid `?date=` value is an ISO YYYY-MM-DD; anything else falls back to today. */
function isIsoDate(s: string | null): s is string {
  return !!s && /^\d{4}-\d{2}-\d{2}$/.test(s);
}

export default function CoachReviewReader() {
  // Land on the date from the S6 list / `phantom://coach/review?date=…` deep-link
  // (SPEC-41 §10.6) when present; otherwise default to today.
  const [searchParams] = useSearchParams();
  const initialDate = searchParams.get("date");
  const [date, setDate] = useState<string>(() => (isIsoDate(initialDate) ? initialDate : todayIso()));
  const [view, setView] = useState<DailyReviewView | null>(null);
  const [loading, setLoading] = useState(true);
  const [generating, setGenerating] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async (iso: string) => {
    setLoading(true);
    setError(null);
    try {
      const v = await loadDailyReview(iso);
      setView(v);
      if (!v) setError("每日回顧後端暫時無法使用");
    } catch (e) {
      setError(String(e ?? "未知錯誤"));
      setView(null);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { void refresh(date); }, [date, refresh]);

  // Run the coach (Gemini "明日的一個行動" pass) for this date and persist it.
  const generate = useCallback(async () => {
    setGenerating(true);
    setError(null);
    try {
      const v = await generateReview(date, true);
      if (v) setView(v);
      else setError("教練回顧需要在桌面 app 中產生（瀏覽器模式不支援）");
    } catch (e) {
      setError(String(e ?? "未知錯誤"));
    } finally {
      setGenerating(false);
    }
  }, [date]);

  const rows: ReviewRow[] = useMemo(
    () => (view ? parseReview(view.markdown) : []),
    [view],
  );
  const tomorrow = useMemo(
    () => (view ? extractTomorrowAction(view.markdown) : null),
    [view],
  );
  const isToday = date === todayIso();
  const canGenerate = !!view && !view.locked && view.eventCount > 0;

  return (
    <div className="max-w-2xl mx-auto space-y-5" data-testid="coach-review-reader">
      <header className="flex items-center gap-3">
        <div className="w-10 h-10 rounded-lg bg-phantom-primary/15 flex items-center justify-center">
          <CalendarDays size={20} className="text-phantom-primary" />
        </div>
        <div className="flex-1">
          <h1 className="text-xl font-bold text-phantom-text">每日回顧</h1>
          <p className="text-xs text-phantom-muted">Daily review · {date}{view ? ` · ${view.eventCount} events` : ""}</p>
        </div>
        <div className="flex items-center gap-1">
          <button onClick={() => setDate(shiftDate(date, -1))} className="text-phantom-muted hover:text-phantom-text p-1.5 min-w-[44px] min-h-[44px] inline-flex items-center justify-center" title="前一天" aria-label="前一天">
            <ChevronLeft size={16} />
          </button>
          <button onClick={() => setDate(shiftDate(date, 1))} disabled={isToday} className="text-phantom-muted hover:text-phantom-text disabled:opacity-30 p-1.5 min-w-[44px] min-h-[44px] inline-flex items-center justify-center" title="後一天" aria-label="後一天">
            <ChevronRight size={16} />
          </button>
          <button onClick={() => void refresh(date)} className="text-phantom-muted hover:text-phantom-text p-1.5 min-w-[44px] min-h-[44px] inline-flex items-center justify-center" title="重新整理" aria-label="重新整理">
            <RefreshCw size={16} className={loading ? "animate-spin" : ""} />
          </button>
        </div>
      </header>

      {!isToday && (
        <button onClick={() => setDate(todayIso())} className="text-xs text-phantom-primary hover:underline">↩ 回到今天</button>
      )}

      {error && (
        <div className="bg-phantom-warning/10 border border-phantom-warning/40 rounded-lg p-3 text-sm text-phantom-warning flex items-center justify-between gap-3" role="alert">
          <span className="min-w-0 break-words">{error}</span>
          <button onClick={() => void refresh(date)} className="flex-shrink-0 text-xs px-2 py-1 rounded border border-phantom-warning/40 hover:bg-phantom-warning/15">重試</button>
        </div>
      )}

      {loading && !view && !error && (
        <div className="space-y-2" data-testid="review-skeleton" aria-busy="true">
          {[0, 1, 2].map((i) => (
            <div key={i} className="h-12 rounded bg-phantom-card border border-phantom-border animate-pulse" />
          ))}
        </div>
      )}

      {view?.flagged && (
        <div className="bg-phantom-warning/10 border border-phantom-warning/30 rounded-lg p-3 text-xs text-phantom-muted">
          部分內容被標記 — 顯示原始紀錄(shame-free 防護)。
        </div>
      )}

      {canGenerate && (
        <div className="flex items-center justify-between gap-3 bg-phantom-card border border-phantom-border rounded-lg p-3">
          <div className="min-w-0">
            <p className="text-sm text-phantom-text flex items-center gap-1.5">
              <Sparkles size={14} className="text-phantom-primary" /> 教練回顧
            </p>
            <p className="text-xs text-phantom-muted mt-0.5">
              {tomorrow ? "已產生明日行動 — 可重新產生" : "讓教練讀今天的紀錄，給一個最小的明日行動"}
            </p>
          </div>
          <button
            onClick={() => void generate()}
            disabled={generating}
            className="flex-shrink-0 px-3 py-1.5 rounded-lg text-sm bg-phantom-primary/15 border border-phantom-primary/40 text-phantom-primary hover:bg-phantom-primary/25 disabled:opacity-50"
          >
            {generating ? "產生中…" : tomorrow ? "重新產生" : "產生回顧"}
          </button>
        </div>
      )}

      {tomorrow && !tomorrow.skipped && (
        <div className="bg-phantom-primary/10 border border-phantom-primary/30 rounded-lg p-4" data-testid="tomorrow-action">
          <p className="text-xs font-semibold text-phantom-primary flex items-center gap-1.5 mb-1">
            <Sparkles size={13} /> 明日的一個行動
          </p>
          <p className="text-sm text-phantom-text whitespace-pre-wrap break-words">{tomorrow.text}</p>
        </div>
      )}
      {tomorrow?.skipped && (
        <div className="bg-phantom-card border border-phantom-border rounded-lg p-3 text-xs text-phantom-muted">
          明日行動已略過 — 需設定 <code className="text-phantom-primary">GEMINI_API_KEY</code> 才能產生教練建議。
        </div>
      )}

      {!error && view?.locked && (
        <div className="bg-phantom-card border border-phantom-border rounded-lg p-6 text-center">
          <Lock size={22} className="text-phantom-muted mx-auto mb-2" />
          <p className="text-sm text-phantom-text">事件已加密(age v1)</p>
          <p className="text-xs text-phantom-muted mt-1">尚未載入身分金鑰 — 執行 <code className="text-phantom-primary">phantom init</code> 後重新整理。</p>
        </div>
      )}

      {!error && view && !view.locked && view.eventCount === 0 && (
        <div className="bg-phantom-card border border-phantom-border rounded-lg p-6 text-center">
          <p className="text-sm text-phantom-text">{date} 沒有 Life Node 事件。</p>
          <p className="text-xs text-phantom-muted mt-1">用專注 / 習慣 / 飲食頁記錄一筆後,會在這裡按目標標籤分組。</p>
          <p className="text-[11px] text-phantom-muted/70 mt-2">空白的一天沒關係 — 這是紀錄,不是評分表。</p>
        </div>
      )}

      {!error && view && !view.locked && view.eventCount > 0 && (
        <div className="space-y-3">
          {rows.filter((r) => r.kind === "group" || r.kind === "bullet").map((r, i) => {
            if (r.kind === "group") {
              return (
                <div key={`g-${i}`} className="flex items-baseline gap-2 pt-2 first:pt-0">
                  <span className="text-sm font-semibold text-phantom-primary">{r.tag}</span>
                  <span className="text-[11px] text-phantom-muted">({r.n})</span>
                </div>
              );
            }
            return (
              <div key={`b-${i}`} className="flex items-start gap-3 px-3 py-2 rounded bg-phantom-card border border-phantom-border ml-2">
                <span className="text-base w-6 text-center flex-shrink-0">{KIND_EMOJI[r.eventKind] ?? "•"}</span>
                <div className="flex-1 min-w-0">
                  <span className="text-xs text-phantom-muted mr-2">{r.time}</span>
                  <span className="text-sm text-phantom-text break-words">{r.summary}</span>
                </div>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
