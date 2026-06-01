// F104 · Unit tests — history store (E002 mobile history screen).
//
// Covers (mirrors F104 spec acceptance):
//   - append() inserts entries; cap-50 FIFO evicts oldest
//   - prompt redaction: stored prompt is preserved verbatim, display
//     helper truncates to a fixed width without mutating the source
//   - hydrate() pulls from the mocked tauri-plugin-store on mount
//   - persist() writes via the mocked plugin-store after every mutation
//   - subscribeDispatch() wires the dispatch store: on done/failed/
//     cancelled phases it copies a snapshot into history exactly once
//     per dispatch (no double-record on stray late frames)
//   - select() finds an entry by id (for tap-to-expand)
//
// Tests run in jsdom; we mock @tauri-apps/plugin-store with an in-memory
// backing object so we exercise the load → set → save round-trip without
// a real Tauri runtime. The mock is the same shape used in
// app/src/components/onboarding/StepComplete.tsx.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// ── Mock @tauri-apps/plugin-store ───────────────────────────────────────
//
// Backing: a single in-memory record keyed by store-name → entries.
// `load(name)` returns a stub that get/set/save/keys/delete-against it.

interface MockStore {
  _data: Record<string, unknown>;
  get: (k: string) => Promise<unknown>;
  set: (k: string, v: unknown) => Promise<void>;
  save: () => Promise<void>;
  delete: (k: string) => Promise<void>;
  entries: () => Promise<[string, unknown][]>;
}

const mockStores = new Map<string, MockStore>();
let saveCalls = 0;

function makeMockStore(name: string): MockStore {
  if (mockStores.has(name)) return mockStores.get(name)!;
  const data: Record<string, unknown> = {};
  const store: MockStore = {
    _data: data,
    get: async (k) => data[k],
    set: async (k, v) => {
      data[k] = v;
    },
    save: async () => {
      saveCalls += 1;
    },
    delete: async (k) => {
      delete data[k];
    },
    entries: async () => Object.entries(data),
  };
  mockStores.set(name, store);
  return store;
}

vi.mock('@tauri-apps/plugin-store', () => ({
  load: async (name: string) => makeMockStore(name),
}));

// ── Imports under test (after the mock is registered) ──────────────────

import {
  useHistoryStore,
  redactPromptForDisplay,
  HISTORY_CAP,
  __INTERNAL,
  type HistoryEntry,
} from '../../src/stores/historyStore';
import { useDispatchStore } from '../../src/stores/dispatchStore';

function makeEntry(over: Partial<HistoryEntry> = {}): HistoryEntry {
  return {
    id: 'd-1',
    prompt: 'hello world',
    caps: ['gpu'],
    provider: undefined,
    status: 'done',
    finalTokens: ['hello ', 'world'],
    result: 'hello world',
    errorCode: undefined,
    errorMessage: undefined,
    startedAt: 1_700_000_000_000,
    completedAt: 1_700_000_001_000,
    ...over,
  };
}

beforeEach(() => {
  mockStores.clear();
  saveCalls = 0;
  useHistoryStore.setState({
    entries: [],
    hydrated: false,
    expandedId: null,
  });
  useDispatchStore.getState().reset();
});

afterEach(() => {
  __INTERNAL.detachDispatchSubscription();
});

describe('snapshotFromDispatch — malformed dispatch state', () => {
  it('does not throw and yields empty arrays when caps/tokens are missing', () => {
    // Spreading [...undefined] would crash and lose the history entry.
    const malformed = {
      id: 'x', prompt: 'p', phase: 'done', provider: undefined,
      result: 'r', errorCode: undefined, errorMessage: undefined,
      startedAt: 1, completedAt: 2,
      // caps + tokens intentionally absent
    } as unknown as Parameters<typeof __INTERNAL.snapshotFromDispatch>[0];

    const snap = __INTERNAL.snapshotFromDispatch(malformed);
    expect(snap).not.toBeNull();
    expect(snap!.caps).toEqual([]);
    expect(snap!.finalTokens).toEqual([]);
  });

  it('redactPromptForDisplay tolerates a missing prompt', () => {
    expect(redactPromptForDisplay(undefined as unknown as string)).toBe('');
  });
});

describe('historyStore', () => {
  it('append() prepends an entry (newest first)', async () => {
    await useHistoryStore.getState().append(makeEntry({ id: 'a' }));
    await useHistoryStore.getState().append(makeEntry({ id: 'b' }));
    const list = useHistoryStore.getState().entries;
    expect(list.map((e) => e.id)).toEqual(['b', 'a']);
  });

  it(`caps at ${HISTORY_CAP} and FIFO-evicts oldest`, async () => {
    for (let i = 0; i < HISTORY_CAP + 5; i++) {
      await useHistoryStore.getState().append(
        makeEntry({ id: `d-${i}`, startedAt: 1_700_000_000_000 + i }),
      );
    }
    const list = useHistoryStore.getState().entries;
    expect(list).toHaveLength(HISTORY_CAP);
    // Newest first: last appended id is at index 0.
    expect(list[0].id).toBe(`d-${HISTORY_CAP + 4}`);
    // Oldest 5 (`d-0`..`d-4`) must be gone.
    expect(list.find((e) => e.id === 'd-0')).toBeUndefined();
    expect(list.find((e) => e.id === 'd-4')).toBeUndefined();
    // First retained entry is `d-5`.
    expect(list[list.length - 1].id).toBe('d-5');
  });

  it('append() writes through to plugin-store (save() called)', async () => {
    saveCalls = 0;
    await useHistoryStore.getState().append(makeEntry({ id: 'persist-1' }));
    expect(saveCalls).toBeGreaterThanOrEqual(1);
    // The backing record reflects the entry.
    const store = mockStores.get(__INTERNAL.STORE_FILE)!;
    const stored = (await store.get(__INTERNAL.ENTRIES_KEY)) as HistoryEntry[];
    expect(stored).toHaveLength(1);
    expect(stored[0].id).toBe('persist-1');
  });

  it('hydrate() round-trips entries written by a prior session', async () => {
    // Seed the mock store as if a previous session had saved 3 entries.
    const seeded: HistoryEntry[] = [
      makeEntry({ id: 'seed-c', startedAt: 3 }),
      makeEntry({ id: 'seed-b', startedAt: 2 }),
      makeEntry({ id: 'seed-a', startedAt: 1 }),
    ];
    const store = makeMockStore(__INTERNAL.STORE_FILE);
    await store.set(__INTERNAL.ENTRIES_KEY, seeded);
    await store.save();

    // Now hydrate the in-memory store from disk.
    await useHistoryStore.getState().hydrate();

    const list = useHistoryStore.getState().entries;
    expect(list.map((e) => e.id)).toEqual(['seed-c', 'seed-b', 'seed-a']);
    expect(useHistoryStore.getState().hydrated).toBe(true);
  });

  it('hydrate() is a no-op when the backing file is empty', async () => {
    await useHistoryStore.getState().hydrate();
    expect(useHistoryStore.getState().entries).toEqual([]);
    expect(useHistoryStore.getState().hydrated).toBe(true);
  });

  it('redactPromptForDisplay() truncates without mutating the source', () => {
    const long = 'a'.repeat(500);
    const out = redactPromptForDisplay(long);
    expect(out.length).toBeLessThan(long.length);
    expect(out.endsWith('…')).toBe(true);
    // Source unchanged — the entry retains the full prompt.
    expect(long.length).toBe(500);
    // Short prompts pass through.
    expect(redactPromptForDisplay('short')).toBe('short');
  });

  it('expand(id) sets expandedId; collapse() clears it', () => {
    useHistoryStore.getState().expand('xyz');
    expect(useHistoryStore.getState().expandedId).toBe('xyz');
    useHistoryStore.getState().collapse();
    expect(useHistoryStore.getState().expandedId).toBeNull();
  });

  it('subscribeDispatch() records exactly one entry on terminal phase', async () => {
    __INTERNAL.attachDispatchSubscription();
    // Drive a dispatch through the canonical flow.
    useDispatchStore.getState().startDispatch({
      id: 'd-flow-1',
      prompt: 'tell me a joke',
      caps: ['gpu', 'vision'],
      provider: undefined,
      startedAt: 1_700_000_000_000,
    });
    useDispatchStore
      .getState()
      .applyFrame('d-flow-1', { type: 'token', text: 'why ' });
    useDispatchStore
      .getState()
      .applyFrame('d-flow-1', { type: 'token', text: 'did' });
    useDispatchStore
      .getState()
      .applyFrame('d-flow-1', { type: 'done', result: 'why did' });

    // Allow the microtask from the subscriber to flush.
    await Promise.resolve();
    await Promise.resolve();

    const list = useHistoryStore.getState().entries;
    expect(list).toHaveLength(1);
    expect(list[0].id).toBe('d-flow-1');
    expect(list[0].status).toBe('done');
    expect(list[0].finalTokens).toEqual(['why ', 'did']);
    expect(list[0].result).toBe('why did');

    // A stray late frame must NOT double-record.
    useDispatchStore
      .getState()
      .applyFrame('d-flow-1', { type: 'status', phase: 'running' });
    await Promise.resolve();
    expect(useHistoryStore.getState().entries).toHaveLength(1);
  });

  it('subscribeDispatch() records failed status with error code+message', async () => {
    __INTERNAL.attachDispatchSubscription();
    useDispatchStore.getState().startDispatch({
      id: 'd-fail-1',
      prompt: 'boom',
      caps: [],
      provider: undefined,
      startedAt: 1_700_000_000_000,
    });
    useDispatchStore.getState().applyFrame('d-fail-1', {
      type: 'error',
      code: 'E_NET',
      message: 'broker unreachable',
    });
    await Promise.resolve();
    await Promise.resolve();
    const list = useHistoryStore.getState().entries;
    expect(list).toHaveLength(1);
    expect(list[0].status).toBe('failed');
    expect(list[0].errorCode).toBe('E_NET');
    expect(list[0].errorMessage).toBe('broker unreachable');
  });

  it('subscribeDispatch() records cancelled status when user cancels', async () => {
    __INTERNAL.attachDispatchSubscription();
    useDispatchStore.getState().startDispatch({
      id: 'd-cancel-1',
      prompt: 'stop me',
      caps: [],
      provider: undefined,
      startedAt: 1_700_000_000_000,
    });
    useDispatchStore.getState().markCancelled('d-cancel-1');
    await Promise.resolve();
    await Promise.resolve();
    expect(useHistoryStore.getState().entries).toHaveLength(1);
    expect(useHistoryStore.getState().entries[0].status).toBe('cancelled');
  });

  it('clear() removes all entries and persists the empty state', async () => {
    await useHistoryStore.getState().append(makeEntry({ id: 'x' }));
    await useHistoryStore.getState().append(makeEntry({ id: 'y' }));
    saveCalls = 0;
    await useHistoryStore.getState().clear();
    expect(useHistoryStore.getState().entries).toEqual([]);
    expect(saveCalls).toBeGreaterThanOrEqual(1);
  });
});
