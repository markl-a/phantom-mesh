// F104 · Component tests — MobileHistory screen (E002 mobile history).
//
// Mirrors the F103 MobileDispatch.test.tsx setup: mock the
// @tauri-apps/plugin-store import (so hydrate() round-trips an in-
// memory backing object), drive the historyStore directly, then assert
// on rendered DOM.
//
// Covered states:
//   - empty: empty-state copy is visible, no list rows
//   - list: rows render in newest-first order with status badge +
//     redacted prompt preview
//   - expanded: tap a row → detail panel shows full prompt + joined
//     token transcript + final result (or error block on `failed`)

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

// ── plugin-store mock (identical to historyStore.test.ts) ────────────────

interface MockStore {
  _data: Record<string, unknown>;
  get: (k: string) => Promise<unknown>;
  set: (k: string, v: unknown) => Promise<void>;
  save: () => Promise<void>;
  delete: (k: string) => Promise<void>;
  entries: () => Promise<[string, unknown][]>;
}
const mockStores = new Map<string, MockStore>();
function makeMockStore(name: string): MockStore {
  if (mockStores.has(name)) return mockStores.get(name)!;
  const data: Record<string, unknown> = {};
  const store: MockStore = {
    _data: data,
    get: async (k) => data[k],
    set: async (k, v) => {
      data[k] = v;
    },
    save: async () => {},
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

import {
  useHistoryStore,
  type HistoryEntry,
} from '../../src/stores/historyStore';

beforeEach(() => {
  mockStores.clear();
  useHistoryStore.setState({
    entries: [],
    hydrated: false,
    expandedId: null,
  });
});

afterEach(() => {
  useHistoryStore.setState({
    entries: [],
    hydrated: false,
    expandedId: null,
  });
});

function makeEntry(over: Partial<HistoryEntry> = {}): HistoryEntry {
  return {
    id: 'd-1',
    prompt: 'tell me a joke',
    caps: ['gpu'],
    provider: undefined,
    status: 'done',
    finalTokens: ['why ', 'did ', 'the chicken'],
    result: 'why did the chicken',
    errorCode: undefined,
    errorMessage: undefined,
    startedAt: 1_700_000_000_000,
    completedAt: 1_700_000_001_500,
    ...over,
  };
}

async function renderScreen() {
  const { default: MobileHistory } = await import(
    '../../src/components/mobile/MobileHistory'
  );
  return render(<MobileHistory />);
}

describe('<MobileHistory />', () => {
  it('renders + expands without crashing on legacy data missing caps/finalTokens', async () => {
    useHistoryStore.setState({
      entries: [
        makeEntry({
          id: 'malformed',
          caps: undefined as unknown as string[],
          finalTokens: undefined as unknown as string[],
        }),
      ],
      hydrated: true,
      expandedId: null,
    });
    await renderScreen();
    // List render: entry.caps.length / redactPromptForDisplay used to crash.
    const row = await screen.findByText('tell me a joke');
    // Expand: transcript join(finalTokens) + finalTokens.length used to crash.
    await userEvent.click(row);
    await waitFor(() =>
      expect(screen.getByText(/transcript/)).toBeInTheDocument(),
    );
  });

  it('renders the empty state when no history is recorded', async () => {
    await renderScreen();
    await waitFor(() => {
      expect(screen.getByTestId('history-empty')).toBeInTheDocument();
    });
    expect(screen.queryByTestId('history-list')).not.toBeInTheDocument();
  });

  it('renders rows for each persisted entry, newest first', async () => {
    // Seed store directly — bypass the dispatch-subscription path.
    useHistoryStore.setState({
      entries: [
        makeEntry({ id: 'd-c', prompt: 'newest', startedAt: 3 }),
        makeEntry({ id: 'd-b', prompt: 'middle', startedAt: 2 }),
        makeEntry({
          id: 'd-a',
          prompt: 'oldest',
          status: 'failed',
          errorCode: 'E_NET',
          errorMessage: 'down',
          finalTokens: [],
          result: undefined,
          startedAt: 1,
        }),
      ],
      hydrated: true,
    });
    await renderScreen();
    const rows = await screen.findAllByTestId(/^history-row-/);
    expect(rows).toHaveLength(3);
    // Newest first.
    expect(rows[0]).toHaveAttribute('data-testid', 'history-row-d-c');
    expect(rows[2]).toHaveAttribute('data-testid', 'history-row-d-a');
    // Status badge for each row.
    expect(rows[2].textContent).toContain('failed');
  });

  it('redacts long prompts in the list-row preview', async () => {
    const long = 'lorem '.repeat(60); // > 140 chars
    useHistoryStore.setState({
      entries: [makeEntry({ id: 'd-long', prompt: long })],
      hydrated: true,
    });
    await renderScreen();
    const row = await screen.findByTestId('history-row-d-long');
    // Truncation marker present.
    expect(row.textContent).toContain('…');
    // Full text NOT rendered in the row.
    expect(row.textContent?.includes(long)).toBe(false);
  });

  it('tap-to-expand shows the full prompt and joined transcript', async () => {
    useHistoryStore.setState({
      entries: [
        makeEntry({
          id: 'd-exp',
          prompt: 'tell me a joke',
          finalTokens: ['why ', 'did ', 'the chicken'],
          result: 'why did the chicken',
        }),
      ],
      hydrated: true,
    });
    await renderScreen();
    const row = await screen.findByTestId('history-row-d-exp');
    await userEvent.click(row);

    const detail = await screen.findByTestId('history-detail');
    expect(detail.textContent).toContain('tell me a joke');
    // Joined transcript is rendered as a single string.
    const transcript = screen.getByTestId('history-transcript');
    expect(transcript.textContent).toContain('why did the chicken');
    // Result block.
    expect(screen.getByTestId('history-result').textContent).toContain(
      'why did the chicken',
    );
  });

  it('expanded failed entry renders an error block', async () => {
    useHistoryStore.setState({
      entries: [
        makeEntry({
          id: 'd-fail',
          status: 'failed',
          errorCode: 'E_DISPATCH_NETWORK',
          errorMessage: 'broker unreachable',
          result: undefined,
          finalTokens: ['partial'],
        }),
      ],
      hydrated: true,
    });
    await renderScreen();
    await userEvent.click(await screen.findByTestId('history-row-d-fail'));
    const err = await screen.findByTestId('history-error');
    expect(err.textContent).toContain('E_DISPATCH_NETWORK');
    expect(err.textContent).toContain('broker unreachable');
  });

  it('tapping the open row collapses it', async () => {
    useHistoryStore.setState({
      entries: [makeEntry({ id: 'd-tog' })],
      hydrated: true,
    });
    await renderScreen();
    const row = await screen.findByTestId('history-row-d-tog');
    await userEvent.click(row);
    expect(await screen.findByTestId('history-detail')).toBeInTheDocument();
    await userEvent.click(row);
    await waitFor(() => {
      expect(screen.queryByTestId('history-detail')).not.toBeInTheDocument();
    });
  });
});
