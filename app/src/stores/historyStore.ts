// F104 · Dispatch history store (E002 mobile history screen).
//
// Last-50 ring of completed dispatches, persisted via tauri-plugin-store
// so the History screen survives app restart (E002 acceptance row:
// "History screen survives app restart … not memory-only").
//
// Architecture (per supervisor brief, "History wraps Dispatch"):
//   - `dispatchStore` remains the in-flight source of truth.
//   - `historyStore` subscribes to it and copies a snapshot into the
//     persistent ring on the first terminal-phase transition per
//     dispatch (done | failed | cancelled). Stray late frames after
//     the first terminal record don't double-write because we key the
//     recorded-set on dispatch id.
//
// Persistence is best-effort: in non-Tauri (vitest jsdom, browser dev)
// the plugin-store import either resolves to the supplied mock or
// throws — both paths fall back to in-memory and never crash the UI.

import { create } from 'zustand';
import { useDispatchStore, type DispatchState } from './dispatchStore';

/** Hard cap on the persisted ring. Older entries are FIFO-evicted. */
export const HISTORY_CAP = 50;

/** Prompt display redaction width — full prompt is preserved in the
 *  entry, only the list-row preview is truncated. */
const PROMPT_DISPLAY_LIMIT = 140;

const STORE_FILE = 'dispatch-history.json';
const ENTRIES_KEY = 'entries';

export type HistoryStatus = 'queued' | 'running' | 'done' | 'failed' | 'cancelled';

export interface HistoryEntry {
  id: string;
  /** Full prompt — list rows use `redactPromptForDisplay()` for the
   *  preview, but the expanded detail view shows the raw value. */
  prompt: string;
  caps: string[];
  provider?: string;
  status: HistoryStatus;
  /** Full token stream captured at terminal phase, for replay. */
  finalTokens: string[];
  result?: string;
  errorCode?: string;
  errorMessage?: string;
  startedAt: number;
  completedAt?: number;
}

export interface HistoryStoreState {
  entries: HistoryEntry[];
  hydrated: boolean;
  /** id of the entry currently expanded for replay; null = list view. */
  expandedId: string | null;

  append: (entry: HistoryEntry) => Promise<void>;
  hydrate: () => Promise<void>;
  clear: () => Promise<void>;
  expand: (id: string) => void;
  collapse: () => void;
}

/** Truncate long prompts for the list-row preview. Keeps the source
 *  string in the entry; the detail view renders it verbatim. */
export function redactPromptForDisplay(prompt: string): string {
  // Defensive: legacy/malformed store entries may lack a prompt; don't crash.
  if (typeof prompt !== 'string') return '';
  if (prompt.length <= PROMPT_DISPLAY_LIMIT) return prompt;
  return prompt.slice(0, PROMPT_DISPLAY_LIMIT - 1) + '…';
}

// ── Persistence helpers ─────────────────────────────────────────────────
//
// `@tauri-apps/plugin-store` is added to the React-side deps already
// (app/package.json). In jsdom / browser dev we either get the test
// mock or a runtime throw — both fall back to in-memory.

interface PluginStoreLike {
  get: (k: string) => Promise<unknown>;
  set: (k: string, v: unknown) => Promise<void>;
  save: () => Promise<void>;
}

async function loadPersistedStore(): Promise<PluginStoreLike | null> {
  try {
    const mod = (await import('@tauri-apps/plugin-store')) as unknown as {
      load: (name: string) => Promise<PluginStoreLike>;
    };
    if (typeof mod.load !== 'function') return null;
    return await mod.load(STORE_FILE);
  } catch {
    return null;
  }
}

async function persistEntries(entries: HistoryEntry[]): Promise<void> {
  const store = await loadPersistedStore();
  if (!store) return;
  try {
    await store.set(ENTRIES_KEY, entries);
    await store.save();
  } catch {
    /* best-effort — UI stays consistent in-memory */
  }
}

async function readEntries(): Promise<HistoryEntry[]> {
  const store = await loadPersistedStore();
  if (!store) return [];
  try {
    const raw = await store.get(ENTRIES_KEY);
    if (!Array.isArray(raw)) return [];
    return raw as HistoryEntry[];
  } catch {
    return [];
  }
}

// ── Dispatch → history bridge ───────────────────────────────────────────
//
// Subscribes to dispatchStore. On the first terminal phase per id we
// copy a snapshot into history. The `recorded` set prevents the
// late-frame race documented in F103's risk register (a second
// terminal frame must be a no-op for history).

const recorded = new Set<string>();
let dispatchUnsub: (() => void) | null = null;

function isTerminalStatus(phase: string): phase is HistoryStatus {
  return phase === 'done' || phase === 'failed' || phase === 'cancelled';
}

function snapshotFromDispatch(d: DispatchState): HistoryEntry | null {
  if (!isTerminalStatus(d.phase)) return null;
  return {
    id: d.id,
    prompt: d.prompt,
    // Guard the spreads — d.caps / d.tokens may be absent on malformed or
    // legacy dispatch state; [...undefined] would throw and lose the entry.
    caps: [...(d.caps ?? [])],
    provider: d.provider,
    status: d.phase,
    finalTokens: [...(d.tokens ?? [])],
    result: d.result,
    errorCode: d.errorCode,
    errorMessage: d.errorMessage,
    startedAt: d.startedAt,
    completedAt: d.completedAt,
  };
}

function attachDispatchSubscription(): void {
  if (dispatchUnsub) return;
  dispatchUnsub = useDispatchStore.subscribe((state, prev) => {
    // Walk dispatch ids that just transitioned into a terminal phase.
    for (const [id, cur] of Object.entries(state.byId)) {
      if (recorded.has(id)) continue;
      const before = prev.byId[id];
      const beforeTerminal = before
        ? isTerminalStatus(before.phase)
        : false;
      if (!beforeTerminal && isTerminalStatus(cur.phase)) {
        const snap = snapshotFromDispatch(cur);
        if (snap) {
          recorded.add(id);
          // Fire-and-forget the persistent write; UI state updates
          // synchronously inside append().
          void useHistoryStore.getState().append(snap);
        }
      }
    }
  });
}

function detachDispatchSubscription(): void {
  if (dispatchUnsub) {
    dispatchUnsub();
    dispatchUnsub = null;
  }
  recorded.clear();
}

// ── Store ───────────────────────────────────────────────────────────────

export const useHistoryStore = create<HistoryStoreState>()((set, get) => ({
  entries: [],
  hydrated: false,
  expandedId: null,

  append: async (entry) => {
    // Newest-first; cap at HISTORY_CAP via slice. If an entry with the
    // same id already exists (shouldn't happen with the `recorded`
    // gate, but guard anyway), the new one wins and the old is dropped.
    const cur = get().entries.filter((e) => e.id !== entry.id);
    const next = [entry, ...cur].slice(0, HISTORY_CAP);
    set({ entries: next });
    await persistEntries(next);
  },

  hydrate: async () => {
    if (get().hydrated) return;
    const stored = await readEntries();
    // Defensive cap on read — disk may have been edited by hand.
    set({ entries: stored.slice(0, HISTORY_CAP), hydrated: true });
  },

  clear: async () => {
    set({ entries: [], expandedId: null });
    await persistEntries([]);
  },

  expand: (id) => set({ expandedId: id }),
  collapse: () => set({ expandedId: null }),
}));

// Auto-attach the dispatch subscription on first module load so the
// history screen captures terminal phases even if it's never mounted.
// Tests opt-out via __INTERNAL.detachDispatchSubscription().
attachDispatchSubscription();

/** Test seam — internal symbols exposed for vitest. */
export const __INTERNAL = {
  STORE_FILE,
  ENTRIES_KEY,
  attachDispatchSubscription,
  detachDispatchSubscription,
  snapshotFromDispatch,
};
