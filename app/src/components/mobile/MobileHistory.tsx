// F104 · Mobile history screen.
//
// Spec: docs/superpowers/specs/_current/E002-mobile-cluster-dispatch-ui.md
//   §"History screen". Feature spec:
//   docs/superpowers/features/F104-history-store-and-screen.md.
//
// Renders:
//   1. Empty state when no dispatches have been recorded yet.
//   2. Reverse-chronological list of last-50 entries with status pill +
//      redacted prompt preview + elapsed-time hint.
//   3. Tap a row → expand the same row in place to show the full
//      prompt + joined token transcript + final result / error block.
//
// History entries are produced by `historyStore`'s subscription to
// `dispatchStore`. The screen never invokes a Tauri command directly —
// it's a pure render over the zustand store, hydrated from
// tauri-plugin-store on mount.

import { useEffect, useMemo } from 'react';
import { Clock } from 'lucide-react';
import {
  useHistoryStore,
  redactPromptForDisplay,
  type HistoryEntry,
  type HistoryStatus,
} from '../../stores/historyStore';

function statusBadgeClass(status: HistoryStatus): string {
  switch (status) {
    case 'done':
      return 'bg-green-500/15 text-green-400 border border-green-500/30';
    case 'failed':
      return 'bg-phantom-danger/15 text-phantom-danger border border-phantom-danger/30';
    case 'cancelled':
      return 'bg-phantom-muted/15 text-phantom-muted border border-phantom-muted/30';
    case 'running':
    case 'queued':
    default:
      return 'bg-phantom-primary/15 text-phantom-primary border border-phantom-primary/30';
  }
}

function relativeTime(unixMs: number): string {
  const delta = Date.now() - unixMs;
  if (delta < 60_000) return 'just now';
  if (delta < 3_600_000) return `${Math.floor(delta / 60_000)}m ago`;
  if (delta < 86_400_000) return `${Math.floor(delta / 3_600_000)}h ago`;
  return `${Math.floor(delta / 86_400_000)}d ago`;
}

function HistoryRow({
  entry,
  expanded,
  onToggle,
}: {
  entry: HistoryEntry;
  expanded: boolean;
  onToggle: () => void;
}) {
  const transcript = useMemo(
    () => (entry.finalTokens ?? []).join(''),
    [entry.finalTokens],
  );
  const elapsed =
    entry.completedAt && entry.startedAt
      ? `${((entry.completedAt - entry.startedAt) / 1000).toFixed(1)}s`
      : null;

  return (
    <li className="border-b border-phantom-border last:border-b-0">
      <button
        type="button"
        onClick={onToggle}
        data-testid={`history-row-${entry.id}`}
        className="w-full text-left px-4 py-3 hover:bg-phantom-card/50 transition-colors"
        aria-expanded={expanded}
      >
        <div className="flex items-center gap-2 mb-1">
          <span
            className={`text-[10px] uppercase tracking-wider px-1.5 py-0.5 rounded ${statusBadgeClass(
              entry.status,
            )}`}
          >
            {entry.status}
          </span>
          <span className="text-[10px] text-phantom-muted">
            {relativeTime(entry.startedAt)}
          </span>
          {elapsed && (
            <span className="text-[10px] text-phantom-muted">· {elapsed}</span>
          )}
          {entry.provider && (
            <span className="text-[10px] text-phantom-muted">
              · {entry.provider}
            </span>
          )}
        </div>
        <div className="text-sm text-phantom-text break-words">
          {redactPromptForDisplay(entry.prompt)}
        </div>
        {Array.isArray(entry.caps) && entry.caps.length > 0 && (
          <div className="flex flex-wrap gap-1 mt-1.5">
            {entry.caps.map((c) => (
              <span
                key={c}
                className="text-[10px] text-phantom-muted bg-phantom-card border border-phantom-border rounded px-1.5"
              >
                {c}
              </span>
            ))}
          </div>
        )}
      </button>

      {expanded && (
        <div
          data-testid="history-detail"
          className="px-4 pb-3 space-y-2"
        >
          <div>
            <div className="text-[10px] uppercase tracking-wider text-phantom-muted mb-1">
              prompt
            </div>
            <pre
              data-testid="history-prompt-full"
              className="bg-phantom-card border border-phantom-border rounded-lg p-2 text-xs text-phantom-text whitespace-pre-wrap break-words"
            >
              {entry.prompt}
            </pre>
          </div>

          <div>
            <div className="text-[10px] uppercase tracking-wider text-phantom-muted mb-1">
              transcript ({entry.finalTokens?.length ?? 0} tokens)
            </div>
            <pre
              data-testid="history-transcript"
              className="bg-phantom-card border border-phantom-border rounded-lg p-2 text-xs text-phantom-text whitespace-pre-wrap break-words max-h-[40vh] overflow-y-auto"
            >
              {transcript || '​'}
            </pre>
          </div>

          {entry.status === 'done' && entry.result && (
            <div
              data-testid="history-result"
              className="bg-phantom-card border border-green-500/40 rounded-lg p-2 text-sm text-phantom-text"
            >
              <div className="text-[10px] text-green-400 uppercase tracking-wider mb-1">
                result
              </div>
              {entry.result}
            </div>
          )}

          {entry.status === 'failed' && (
            <div
              data-testid="history-error"
              className="bg-phantom-card border border-phantom-danger/40 rounded-lg p-2 text-sm text-phantom-text"
              role="alert"
            >
              <div className="text-[10px] text-phantom-danger uppercase tracking-wider mb-1">
                {entry.errorCode || 'error'}
              </div>
              {entry.errorMessage || 'Dispatch failed'}
            </div>
          )}
        </div>
      )}
    </li>
  );
}

export default function MobileHistory() {
  const entries = useHistoryStore((s) => s.entries);
  const hydrated = useHistoryStore((s) => s.hydrated);
  const expandedId = useHistoryStore((s) => s.expandedId);
  const hydrate = useHistoryStore((s) => s.hydrate);
  const expand = useHistoryStore((s) => s.expand);
  const collapse = useHistoryStore((s) => s.collapse);

  useEffect(() => {
    if (!hydrated) void hydrate();
  }, [hydrated, hydrate]);

  const onToggleRow = (id: string) => {
    if (expandedId === id) collapse();
    else expand(id);
  };

  if (entries.length === 0) {
    return (
      <div
        data-testid="mobile-history-root"
        className="flex flex-col h-full overflow-y-auto"
      >
        <div
          data-testid="history-empty"
          className="flex flex-1 flex-col items-center justify-center text-center px-6 py-10 text-phantom-muted"
        >
          <Clock size={32} className="mb-2 opacity-60" />
          <div className="text-sm">No dispatches yet</div>
          <div className="text-xs mt-1">
            Send a task from the Dispatch tab — it'll show up here.
          </div>
        </div>
      </div>
    );
  }

  return (
    <div
      data-testid="mobile-history-root"
      className="flex flex-col h-full overflow-y-auto"
    >
      <ul data-testid="history-list" className="flex-1">
        {entries.map((e) => (
          <HistoryRow
            key={e.id}
            entry={e}
            expanded={expandedId === e.id}
            onToggle={() => onToggleRow(e.id)}
          />
        ))}
      </ul>
    </div>
  );
}
