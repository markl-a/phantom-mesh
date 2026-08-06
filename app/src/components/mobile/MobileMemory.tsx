// Mobile Memory panel — focused port of desktop settings/MemoryPanel.tsx.
//
// Desktop is a 469-line three-pane explorer (stats, working/episodic/cluster
// layers, AI assist). On a phone we strip to the highest-value bits: a small
// stats card, a recent-observations list (limit 50), and a search box that
// runs search_memory. Backed by the existing get_memory_stats /
// get_memory_observations / search_memory commands — no backend change.

import { useCallback, useEffect, useMemo, useState } from 'react';
import { Brain, Search, RefreshCw } from 'lucide-react';
import { safeInvoke as invoke } from '../../lib/tauri-compat';

interface MemoryEntry {
  id: string;
  content: string;
  type: string;
  timestamp: string;
  relevance: number;
}

interface MemoryStats {
  totalEntries: number | null;
  compressionRatio: string | null;
  lastSync: string | null;
}

function pickString(raw: Record<string, unknown>, ...keys: string[]): string | null {
  for (const k of keys) {
    const v = raw[k];
    if (typeof v === 'string' && v.length > 0) return v;
  }
  return null;
}

function pickNumber(raw: Record<string, unknown>, ...keys: string[]): number | null {
  for (const k of keys) {
    const v = raw[k];
    if (typeof v === 'number' && Number.isFinite(v)) return v;
  }
  return null;
}

function parseStats(raw: unknown): MemoryStats {
  if (!raw || typeof raw !== 'object') {
    return { totalEntries: null, compressionRatio: null, lastSync: null };
  }
  const r = raw as Record<string, unknown>;
  return {
    totalEntries: pickNumber(r, 'totalEntries', 'total_entries', 'count'),
    compressionRatio: pickString(r, 'compressionRatio', 'compression_ratio'),
    lastSync: pickString(r, 'lastSync', 'last_sync', 'updatedAt', 'updated_at'),
  };
}

function parseEntry(raw: unknown, idx: number): MemoryEntry | null {
  if (!raw || typeof raw !== 'object') return null;
  const r = raw as Record<string, unknown>;
  const content = pickString(r, 'content', 'text', 'message') ?? '';
  if (!content) return null;
  return {
    id: pickString(r, 'id', 'uuid') ?? `MEM-${String(idx).padStart(3, '0')}`,
    content,
    type: pickString(r, 'type', 'kind', 'layer') ?? 'unknown',
    timestamp: pickString(r, 'timestamp', 'created_at', 'createdAt') ?? '',
    relevance: pickNumber(r, 'relevance', 'score') ?? 0,
  };
}

function extractEntries(raw: unknown): MemoryEntry[] {
  let list: unknown = raw;
  if (raw && typeof raw === 'object' && !Array.isArray(raw)) {
    const r = raw as Record<string, unknown>;
    list = r.observations ?? r.entries ?? r.results ?? r.items ?? null;
  }
  if (!Array.isArray(list)) return [];
  return list
    .map((e, i) => parseEntry(e, i))
    .filter((e): e is MemoryEntry => e !== null);
}

export default function MobileMemory() {
  const [stats, setStats] = useState<MemoryStats>({
    totalEntries: null,
    compressionRatio: null,
    lastSync: null,
  });
  const [entries, setEntries] = useState<MemoryEntry[]>([]);
  const [query, setQuery] = useState('');
  const [searchResults, setSearchResults] = useState<MemoryEntry[] | null>(null);
  const [loading, setLoading] = useState(true);
  const [searching, setSearching] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setError(null);
    try {
      const [statsRaw, obsRaw] = await Promise.all([
        invoke('get_memory_stats'),
        invoke('get_memory_observations', { query: null, limit: 50 }),
      ]);
      setStats(parseStats(statsRaw));
      setEntries(extractEntries(obsRaw));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    let alive = true;
    (async () => {
      try {
        const [statsRaw, obsRaw] = await Promise.all([
          invoke('get_memory_stats'),
          invoke('get_memory_observations', { query: null, limit: 50 }),
        ]);
        if (!alive) return;
        setStats(parseStats(statsRaw));
        setEntries(extractEntries(obsRaw));
      } catch (e) {
        if (alive) setError(e instanceof Error ? e.message : String(e));
      } finally {
        if (alive) setLoading(false);
      }
    })();
    return () => {
      alive = false;
    };
  }, []);

  const onSearch = useCallback(async () => {
    const q = query.trim();
    if (!q || searching) return;
    setSearching(true);
    setError(null);
    try {
      const raw = await invoke('search_memory', { query: q });
      setSearchResults(extractEntries(raw));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSearching(false);
    }
  }, [query, searching]);

  const clearSearch = useCallback(() => {
    setSearchResults(null);
    setQuery('');
  }, []);

  const visibleEntries = useMemo(() => searchResults ?? entries, [searchResults, entries]);

  if (loading) {
    return (
      <div
        data-testid="mobile-memory-loading"
        className="flex flex-1 flex-col items-center justify-center text-spectyn-muted py-8"
      >
        <Brain size={28} className="mb-2 animate-pulse opacity-60" />
        <div className="text-sm">載入記憶中…</div>
      </div>
    );
  }

  return (
    <div data-testid="mobile-memory-root" className="flex flex-col gap-3">
      {/* Stats card */}
      <div
        data-testid="memory-stats"
        className="grid grid-cols-3 gap-2 rounded-lg border border-spectyn-border bg-spectyn-card px-3 py-2.5"
      >
        <div>
          <div className="text-[10px] uppercase tracking-wide text-spectyn-muted">總筆數</div>
          <div className="text-sm text-spectyn-text">
            {stats.totalEntries ?? '—'}
          </div>
        </div>
        <div>
          <div className="text-[10px] uppercase tracking-wide text-spectyn-muted">壓縮比</div>
          <div className="text-sm text-spectyn-text">{stats.compressionRatio ?? '—'}</div>
        </div>
        <div>
          <div className="text-[10px] uppercase tracking-wide text-spectyn-muted">最後同步</div>
          <div className="text-sm text-spectyn-text truncate">{stats.lastSync ?? '—'}</div>
        </div>
      </div>

      {/* Search box */}
      <div className="flex items-center gap-2">
        <Search size={15} className="flex-shrink-0 text-spectyn-muted" />
        <input
          type="text"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => e.key === 'Enter' && void onSearch()}
          placeholder="搜尋記憶…"
          data-testid="memory-search-input"
          className="flex-1 bg-spectyn-card border border-spectyn-border rounded px-3 py-1.5 text-sm text-spectyn-text placeholder-spectyn-muted focus:outline-none focus:border-spectyn-primary"
        />
        <button
          type="button"
          data-testid="memory-search-go"
          onClick={() => void onSearch()}
          disabled={!query.trim() || searching}
          className="bg-spectyn-primary text-spectyn-bg px-3 py-1.5 rounded text-sm font-medium hover:brightness-110 disabled:opacity-40"
        >
          {searching ? '搜尋中…' : '搜尋'}
        </button>
        {searchResults && (
          <button
            type="button"
            data-testid="memory-search-clear"
            onClick={clearSearch}
            className="text-xs text-spectyn-muted hover:text-spectyn-text"
          >
            清除
          </button>
        )}
        <button
          type="button"
          data-testid="memory-refresh"
          onClick={() => void refresh()}
          className="text-spectyn-muted hover:text-spectyn-text"
          aria-label="重新整理"
        >
          <RefreshCw size={15} />
        </button>
      </div>

      {error && (
        <div
          data-testid="memory-error"
          className="rounded border border-spectyn-danger/30 bg-spectyn-danger/10 px-3 py-2 text-xs text-spectyn-danger"
        >
          {error}
        </div>
      )}

      {/* Entries list */}
      {visibleEntries.length === 0 ? (
        <div data-testid="memory-empty" className="text-sm text-spectyn-muted py-4 text-center">
          {searchResults ? '沒有相符的記憶。' : '還沒有記憶資料。'}
        </div>
      ) : (
        <ul data-testid="memory-list" className="flex flex-col gap-1.5">
          {visibleEntries.map((m) => (
            <li
              key={m.id}
              data-testid={`memory-${m.id}`}
              className="rounded-lg border border-spectyn-border bg-spectyn-card px-3 py-2"
            >
              <div className="flex items-center gap-2 mb-1">
                <span className="text-[10px] uppercase tracking-wide text-spectyn-primary flex-shrink-0">
                  {m.type}
                </span>
                {m.timestamp && (
                  <span className="text-[10px] text-spectyn-muted truncate">{m.timestamp}</span>
                )}
              </div>
              <p className="text-[12px] text-spectyn-text line-clamp-3 whitespace-pre-wrap break-words">
                {m.content}
              </p>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
