// BIG-GOAL P2 Life Track — Recall (回想), Android/mobile variant.
//
// The macOS screen is app/src/screens/macos/RecallSearch.tsx; this is the
// mobile-shell-fitted twin. Both read the same offline `recall_search` command
// via lib/recall.ts (no LLM/network) — content-search past Life Node events
// (food / focus / habit / text), newest-first. /review browses one day; this
// finds by query across all captured events.
//
// Reached from Settings → 回想. Lives inside MobileShell so the bottom tab nav
// stays; renders its own back+title header (MobileShell hides its header for
// /recall, mirroring /review).

import { useCallback, useEffect, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { ChevronLeft, ChevronRight, RefreshCw, Search } from "lucide-react";
import {
  recallSearch,
  RECALL_KIND_META,
  type RecallHit,
  type RecallKind,
} from "../../lib/recall";

const KINDS: RecallKind[] = ["food", "focus", "habit", "text"];

function fmtTime(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? (iso.slice(0, 16) || iso) : d.toLocaleString();
}

// The YYYY-MM-DD an event is filed under, for deep-linking to that day's review.
// Take the LITERAL date prefix of the stored rfc3339 timestamp (which carries
// the capture-time offset) — this matches how the review backend buckets events
// (`meta.timestamp.starts_with(date)` in daily_review.rs). Re-parsing via
// `new Date(iso)` would re-project the instant into the *viewing device's*
// current offset and could land a day off after timezone travel / a DST change;
// dailyReview.ts's hhmm() reads literals for the same reason. Null when there's
// no date prefix (legacy/garbage rows) — the row then isn't linkable.
function dayOf(iso: string): string | null {
  const m = /^(\d{4}-\d{2}-\d{2})/.exec(iso);
  return m ? m[1] : null;
}

// The `since` filter is a YYYY-MM-DD cutoff (same shape as CLI
// `phantom recall --since DATE`): events on/after this local day. null = no
// lower bound (all time).
function sinceIso(days: number | null): string | null {
  if (days == null) return null;
  const d = new Date();
  d.setDate(d.getDate() - days);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

const SINCE_RANGES: { label: string; days: number | null }[] = [
  { label: "全部時間", days: null },
  { label: "近 7 天", days: 7 },
  { label: "近 30 天", days: 30 },
];

export default function MobileRecall() {
  const navigate = useNavigate();
  const [query, setQuery] = useState("");
  const [kind, setKind] = useState<RecallKind | null>(null);
  // null = all time; otherwise the cutoff is N days ago (the wired `since`
  // filter on recall_search). Mirrors the CLI `phantom recall --since DATE`.
  const [sinceDays, setSinceDays] = useState<number | null>(null);
  const [hits, setHits] = useState<RecallHit[]>([]);
  const [loading, setLoading] = useState(false);
  const [searched, setSearched] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Request-sequence guard: typing + tapping filter chips fire overlapping
  // searches, and a slow older response must not clobber a newer one (would
  // show stale hits under the current filter). Only the latest request applies.
  const reqSeq = useRef(0);
  const run = useCallback(
    async (q: string, k: RecallKind | null, sinceD: number | null) => {
      const myReq = ++reqSeq.current;
      setLoading(true);
      setError(null);
      try {
        const res = await recallSearch({
          query: q,
          kind: k,
          since: sinceIso(sinceD),
          limit: 100,
        });
        if (myReq !== reqSeq.current) return;
        setHits(res);
        setSearched(true);
      } catch (e) {
        if (myReq !== reqSeq.current) return;
        setError(String(e ?? "未知錯誤"));
        setHits([]);
      } finally {
        if (myReq === reqSeq.current) setLoading(false);
      }
    },
    [],
  );

  // Initial load: recent events (empty query, all kinds, all time).
  useEffect(() => {
    void run("", null, null);
  }, [run]);

  const pickKind = (k: RecallKind | null) => {
    setKind(k);
    void run(query, k, sinceDays);
  };
  const pickSince = (d: number | null) => {
    setSinceDays(d);
    void run(query, kind, d);
  };

  return (
    <div className="flex flex-col h-full overflow-y-auto" data-testid="mobile-recall">
      {/* Header bar — back to Settings + title (matches MobileDailyReview). */}
      <div className="flex items-center px-2 py-2.5 border-b border-phantom-border flex-shrink-0">
        <button
          onClick={() => navigate("/settings")}
          className="text-phantom-text p-2 -m-2 flex items-center gap-1"
          aria-label="返回設定"
        >
          <ChevronLeft size={20} />
          <span className="text-sm">設定</span>
        </button>
        <span className="text-sm font-medium text-phantom-text mx-auto pr-8">回想</span>
      </div>

      <div className="p-3 space-y-3">
        <div className="flex items-center gap-2">
          <div className="w-9 h-9 rounded-lg bg-phantom-primary/15 flex items-center justify-center flex-shrink-0">
            <Search size={18} className="text-phantom-primary" />
          </div>
          <div className="flex-1 min-w-0">
            <div className="text-sm font-semibold text-phantom-text">Recall</div>
            <div className="text-[11px] text-phantom-muted">搜尋過往 Life Node 事件</div>
          </div>
          <button
            onClick={() => void run(query, kind, sinceDays)}
            className="text-phantom-muted hover:text-phantom-text p-1.5 flex-shrink-0"
            aria-label="重新搜尋"
          >
            <RefreshCw size={18} className={loading ? "animate-spin" : ""} />
          </button>
        </div>

        <form
          onSubmit={(e) => {
            e.preventDefault();
            void run(query, kind, sinceDays);
          }}
          className="flex gap-2"
        >
          <input
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="搜尋內容(例如:沙拉、deep work、散步)…"
            aria-label="搜尋內容"
            className="flex-1 min-w-0 bg-phantom-bg border border-phantom-border rounded-lg px-3 py-2 text-sm text-phantom-text placeholder:text-phantom-muted focus:border-phantom-primary outline-none"
          />
          <button
            type="submit"
            className="px-4 py-2 rounded-lg bg-phantom-primary/15 border border-phantom-primary/40 text-phantom-primary text-sm flex-shrink-0 hover:bg-phantom-primary/25"
          >
            搜尋
          </button>
        </form>

        <div className="flex flex-wrap gap-2">
          <button
            onClick={() => pickKind(null)}
            aria-pressed={kind === null}
            className={`px-3 py-1 rounded-full text-xs border transition ${
              kind === null
                ? "bg-phantom-primary/15 border-phantom-primary/40 text-phantom-primary"
                : "bg-phantom-bg border-phantom-border text-phantom-text hover:border-phantom-primary/30"
            }`}
          >
            全部
          </button>
          {KINDS.map((k) => (
            <button
              key={k}
              onClick={() => pickKind(k)}
              aria-pressed={kind === k}
              className={`px-3 py-1 rounded-full text-xs border transition ${
                kind === k
                  ? "bg-phantom-primary/15 border-phantom-primary/40 text-phantom-primary"
                  : "bg-phantom-bg border-phantom-border text-phantom-text hover:border-phantom-primary/30"
              }`}
            >
              {RECALL_KIND_META[k].emoji} {RECALL_KIND_META[k].label}
            </button>
          ))}
        </div>

        {/* Time-range filter (wired `since` cutoff). */}
        <div className="flex flex-wrap gap-2">
          {SINCE_RANGES.map((r) => (
            <button
              key={r.label}
              onClick={() => pickSince(r.days)}
              aria-pressed={sinceDays === r.days}
              className={`px-3 py-1 rounded-full text-xs border transition ${
                sinceDays === r.days
                  ? "bg-phantom-primary/15 border-phantom-primary/40 text-phantom-primary"
                  : "bg-phantom-bg border-phantom-border text-phantom-text hover:border-phantom-primary/30"
              }`}
            >
              {r.label}
            </button>
          ))}
        </div>

        {error && (
          <div className="bg-phantom-warning/10 border border-phantom-warning/40 rounded-lg p-3 text-sm text-phantom-warning">
            {error}
          </div>
        )}

        {!error && searched && hits.length === 0 && !loading && (
          <div className="bg-phantom-card border border-phantom-border rounded-lg p-6 text-center text-sm text-phantom-muted">
            {query.trim()
              ? `沒有符合「${query.trim()}」的事件。`
              : "尚無事件 — 用專注 / 習慣 / 飲食頁記錄後會出現在這裡。"}
          </div>
        )}

        <div className="space-y-1.5">
          {hits.map((h, i) => {
            const meta = RECALL_KIND_META[h.kind] ?? { label: h.kind, emoji: "•" };
            const day = dayOf(h.timestamp);
            const inner = (
              <>
                <span className="text-base w-6 text-center flex-shrink-0">{meta.emoji}</span>
                <div className="flex-1 min-w-0">
                  <span className="text-sm text-phantom-text break-words">{h.summary}</span>
                </div>
                <span className="text-[11px] text-phantom-muted flex-shrink-0">
                  {fmtTime(h.timestamp)}
                </span>
              </>
            );
            // Tappable → that day's review (see the event in its full-day
            // context). Rows with an unparseable timestamp aren't linkable.
            return day ? (
              <button
                key={h.eventId || `hit-${i}`}
                onClick={() => navigate(`/review?date=${day}`)}
                aria-label={`查看 ${day} 的每日回顧`}
                className="w-full text-left flex items-start gap-3 px-3 py-2 rounded bg-phantom-card border border-phantom-border hover:border-phantom-primary/40 transition"
              >
                {inner}
                <ChevronRight size={14} className="text-phantom-muted flex-shrink-0 mt-0.5" />
              </button>
            ) : (
              <div
                key={h.eventId || `hit-${i}`}
                className="flex items-start gap-3 px-3 py-2 rounded bg-phantom-card border border-phantom-border"
              >
                {inner}
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}
