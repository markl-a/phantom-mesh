// SPEC P2 Life Track — Recall (回想): content-search past Life Node events.
// App counterpart of the TUI `/recall` + CLI `spectyn recall`. /review browses
// by day; this finds by query across all captured events, newest-first.
// Read-only; encrypted events decrypt only with the key (skipped otherwise).

import { useCallback, useEffect, useState } from "react";
import { Search, RefreshCw } from "lucide-react";
import { recallSearch, RECALL_KIND_META, type RecallHit, type RecallKind } from "../../lib/recall";

const KINDS: RecallKind[] = ["food", "focus", "habit", "text"];

function fmtTime(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? (iso.slice(0, 16) || iso) : d.toLocaleString();
}

export default function RecallSearch() {
  const [query, setQuery] = useState("");
  const [kind, setKind] = useState<RecallKind | null>(null);
  const [hits, setHits] = useState<RecallHit[]>([]);
  const [loading, setLoading] = useState(false);
  const [searched, setSearched] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const run = useCallback(async (q: string, k: RecallKind | null) => {
    setLoading(true);
    setError(null);
    try {
      const res = await recallSearch({ query: q, kind: k, limit: 100 });
      setHits(res);
      setSearched(true);
    } catch (e) {
      setError(String(e ?? "未知錯誤"));
      setHits([]);
    } finally {
      setLoading(false);
    }
  }, []);

  // Initial load: recent events (empty query).
  useEffect(() => { void run("", null); }, [run]);

  return (
    <div className="max-w-2xl mx-auto space-y-4" data-testid="recall-search">
      <header className="flex items-center gap-3">
        <div className="w-10 h-10 rounded-lg bg-spectyn-primary/15 flex items-center justify-center">
          <Search size={20} className="text-spectyn-primary" />
        </div>
        <div className="flex-1">
          <h1 className="text-xl font-bold text-spectyn-text">回想</h1>
          <p className="text-xs text-spectyn-muted">Recall · 搜尋過往 Life Node 事件</p>
        </div>
        <button onClick={() => void run(query, kind)} className="text-spectyn-muted hover:text-spectyn-text p-1.5" title="重新搜尋" aria-label="重新搜尋">
          <RefreshCw size={16} className={loading ? "animate-spin" : ""} />
        </button>
      </header>

      <form
        onSubmit={(e) => { e.preventDefault(); void run(query, kind); }}
        className="flex gap-2"
      >
        <input
          type="text"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="搜尋內容(例如:沙拉、deep work、散步)…"
          className="flex-1 bg-spectyn-bg border border-spectyn-border rounded-lg px-3 py-2 text-sm text-spectyn-text placeholder:text-spectyn-muted focus:border-spectyn-primary outline-none"
        />
        <button type="submit" className="px-4 py-2 rounded-lg bg-spectyn-primary/15 border border-spectyn-primary/40 text-spectyn-primary text-sm hover:bg-spectyn-primary/25">搜尋</button>
      </form>

      <div className="flex flex-wrap gap-2">
        <button onClick={() => { setKind(null); void run(query, null); }} aria-pressed={kind === null}
          className={`px-3 py-1 rounded-full text-xs border transition ${kind === null ? "bg-spectyn-primary/15 border-spectyn-primary/40 text-spectyn-primary" : "bg-spectyn-bg border-spectyn-border text-spectyn-text hover:border-spectyn-primary/30"}`}>全部</button>
        {KINDS.map((k) => (
          <button key={k} onClick={() => { setKind(k); void run(query, k); }} aria-pressed={kind === k}
            className={`px-3 py-1 rounded-full text-xs border transition ${kind === k ? "bg-spectyn-primary/15 border-spectyn-primary/40 text-spectyn-primary" : "bg-spectyn-bg border-spectyn-border text-spectyn-text hover:border-spectyn-primary/30"}`}>
            {RECALL_KIND_META[k].emoji} {RECALL_KIND_META[k].label}
          </button>
        ))}
      </div>

      {error && (
        <div className="bg-spectyn-warning/10 border border-spectyn-warning/40 rounded-lg p-3 text-sm text-spectyn-warning">{error}</div>
      )}

      {!error && searched && hits.length === 0 && !loading && (
        <div className="bg-spectyn-card border border-spectyn-border rounded-lg p-6 text-center text-sm text-spectyn-muted">
          {query.trim() ? `沒有符合「${query.trim()}」的事件。` : "尚無事件 — 用專注 / 習慣 / 飲食頁記錄後會出現在這裡。"}
        </div>
      )}

      <div className="space-y-1.5">
        {hits.map((h, i) => {
          const meta = RECALL_KIND_META[h.kind] ?? { label: h.kind, emoji: "•" };
          return (
            <div key={h.eventId || `hit-${i}`} className="flex items-start gap-3 px-3 py-2 rounded bg-spectyn-card border border-spectyn-border">
              <span className="text-base w-6 text-center flex-shrink-0">{meta.emoji}</span>
              <div className="flex-1 min-w-0">
                <span className="text-sm text-spectyn-text break-words">{h.summary}</span>
              </div>
              <span className="text-[11px] text-spectyn-muted flex-shrink-0">{fmtTime(h.timestamp)}</span>
            </div>
          );
        })}
      </div>
    </div>
  );
}
