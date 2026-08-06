// Mobile Hands (工作流) panel — mobile-friendly port of desktop
// settings/HandsPanel.tsx (which is a 30-line raw JSON dump). Renders a
// clean list of the hands/pipelines registered on the hub (name + summary
// + enabled hint) by normalising the common shapes the hub /hands endpoint
// returns (array, { hands: [...] }, or an object-map), with a raw-JSON
// fallback for anything unexpected. UI-only — reuses the existing get_hands
// command (provider::get_hands, registered at lib.rs:648).

import { useEffect, useState } from 'react';
import { Workflow } from 'lucide-react';
import { safeInvoke as invoke } from '../../lib/tauri-compat';

interface HandRow {
  name: string;
  description?: string;
  enabled?: boolean;
}

// get_hands returns arbitrary JSON. Coerce the common shapes into a flat
// HandRow[]; return null if we can't recognise it (caller shows raw).
function normalizeHands(value: unknown): HandRow[] | null {
  const coerce = (h: unknown, fallbackName?: string): HandRow | null => {
    if (typeof h === 'string') return { name: h };
    if (h && typeof h === 'object') {
      const o = h as Record<string, unknown>;
      const name =
        (typeof o.name === 'string' && o.name) ||
        (typeof o.id === 'string' && o.id) ||
        fallbackName;
      if (!name) return null;
      return {
        name,
        description:
          typeof o.description === 'string'
            ? o.description
            : typeof o.summary === 'string'
              ? o.summary
              : undefined,
        enabled: typeof o.enabled === 'boolean' ? o.enabled : undefined,
      };
    }
    return null;
  };

  let list: unknown = value;
  if (value && typeof value === 'object' && !Array.isArray(value)) {
    const o = value as Record<string, unknown>;
    if (Array.isArray(o.hands)) list = o.hands;
    else if (Array.isArray(o.pipelines)) list = o.pipelines;
    else {
      const rows = Object.entries(o)
        .map(([k, v]) => coerce(v, k))
        .filter((r): r is HandRow => r !== null);
      return rows.length > 0 ? rows : null;
    }
  }
  if (Array.isArray(list)) {
    const rows = list.map((h) => coerce(h)).filter((r): r is HandRow => r !== null);
    return rows.length > 0 ? rows : null;
  }
  return null;
}

export default function MobileHands() {
  const [raw, setRaw] = useState<unknown>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let alive = true;
    (async () => {
      try {
        const v = await invoke('get_hands');
        if (alive) setRaw(v);
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

  if (loading) {
    return (
      <div data-testid="mobile-hands-loading" className="text-sm text-spectyn-muted py-4">
        載入工作流中…
      </div>
    );
  }

  if (error) {
    return (
      <div
        data-testid="mobile-hands-error"
        className="rounded border border-spectyn-danger/30 bg-spectyn-danger/10 px-3 py-2 text-xs text-spectyn-danger"
      >
        {error}
      </div>
    );
  }

  const rows = normalizeHands(raw);

  if (!rows) {
    return (
      <pre
        data-testid="mobile-hands-raw"
        className="bg-spectyn-card border border-spectyn-border rounded p-3 text-[11px] text-spectyn-text overflow-auto whitespace-pre-wrap break-words"
      >
        {JSON.stringify(raw, null, 2)}
      </pre>
    );
  }

  if (rows.length === 0) {
    return (
      <div data-testid="mobile-hands-empty" className="text-sm text-spectyn-muted py-4">
        沒有已註冊的工作流。
      </div>
    );
  }

  return (
    <ul data-testid="mobile-hands-list" className="flex flex-col gap-1.5">
      {rows.map((h) => (
        <li
          key={h.name}
          data-testid={`hand-${h.name}`}
          className="rounded-lg border border-spectyn-border bg-spectyn-card px-3 py-2.5"
        >
          <div className="flex items-center gap-2">
            <Workflow size={15} className="flex-shrink-0 text-spectyn-primary" />
            <span className="flex-1 min-w-0 text-sm text-spectyn-text truncate">{h.name}</span>
            {h.enabled === false && (
              <span className="text-[10px] uppercase tracking-wide text-spectyn-muted flex-shrink-0">
                disabled
              </span>
            )}
          </div>
          {h.description && (
            <p className="mt-1 text-[11px] text-spectyn-muted line-clamp-2">{h.description}</p>
          )}
        </li>
      ))}
    </ul>
  );
}
