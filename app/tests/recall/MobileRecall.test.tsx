// BIG-GOAL P2 Life Track — MobileRecall (回想) mobile screen.
//
// The recall bridge (lib/recall) is mocked so we drive the search states
// (initial recent load, results, empty, kind filter, error) without a Tauri
// runtime. MobileRecall uses useNavigate, so it renders inside a MemoryRouter.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor, fireEvent, cleanup } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';

const recallSearch = vi.fn();
vi.mock('../../src/lib/recall', () => ({
  recallSearch: (...a: unknown[]) => recallSearch(...a),
  RECALL_KIND_META: {
    food: { label: '飲食', emoji: '🍽' },
    focus: { label: '專注', emoji: '🎯' },
    habit: { label: '習慣', emoji: '✅' },
    text: { label: '文字', emoji: '📝' },
  },
}));

// Spy on navigation (the hit → /review?date= deep-link) while keeping the real
// MemoryRouter so useSearchParams/Router context still work.
const navigateMock = vi.fn();
vi.mock('react-router-dom', async (importOriginal) => {
  const actual = await importOriginal<typeof import('react-router-dom')>();
  return { ...actual, useNavigate: () => navigateMock };
});

beforeEach(() => {
  recallSearch.mockReset();
  navigateMock.mockReset();
});
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

async function renderRecall() {
  const { default: MobileRecall } = await import(
    '../../src/components/mobile/MobileRecall'
  );
  render(
    <MemoryRouter>
      <MobileRecall />
    </MemoryRouter>,
  );
}

describe('<MobileRecall />', () => {
  it('loads recent events on mount (empty query) and renders hits', async () => {
    recallSearch.mockResolvedValue([
      { eventId: 'e1', timestamp: '2026-05-29T08:00:00Z', kind: 'focus', summary: 'deep work 90m' },
    ]);
    await renderRecall();
    await waitFor(() => expect(screen.getByText('deep work 90m')).toBeTruthy());
    expect(recallSearch).toHaveBeenCalledWith({ query: '', kind: null, since: null, limit: 100 });
  });

  it('shows the empty state when no events and query is blank', async () => {
    recallSearch.mockResolvedValue([]);
    await renderRecall();
    await waitFor(() =>
      expect(screen.getByText(/尚無事件/)).toBeTruthy(),
    );
  });

  it('searches within a kind when a filter chip is tapped', async () => {
    recallSearch.mockResolvedValue([]);
    await renderRecall();
    await waitFor(() => expect(recallSearch).toHaveBeenCalledTimes(1));
    fireEvent.click(screen.getByRole('button', { name: /專注/ }));
    await waitFor(() =>
      expect(recallSearch).toHaveBeenLastCalledWith({
        query: '',
        kind: 'focus',
        since: null,
        limit: 100,
      }),
    );
  });

  it('applies a time-range cutoff via the `since` filter', async () => {
    recallSearch.mockResolvedValue([]);
    await renderRecall();
    await waitFor(() => expect(recallSearch).toHaveBeenCalledTimes(1));
    fireEvent.click(screen.getByRole('button', { name: '近 7 天' }));
    await waitFor(() =>
      expect(recallSearch).toHaveBeenLastCalledWith(
        expect.objectContaining({
          // a YYYY-MM-DD cutoff (7 days ago) — TZ-robust shape assert
          since: expect.stringMatching(/^\d{4}-\d{2}-\d{2}$/),
          query: '',
          kind: null,
        }),
      ),
    );
    // "全部時間" clears the cutoff back to null
    fireEvent.click(screen.getByRole('button', { name: '全部時間' }));
    await waitFor(() =>
      expect(recallSearch).toHaveBeenLastCalledWith(
        expect.objectContaining({ since: null }),
      ),
    );
  });

  it('surfaces a search error without crashing', async () => {
    // Mount succeeds, then a user-triggered search hits the error — isolates the
    // rejection to an explicit action (mount-time rejection trips vitest's
    // unhandled-rejection detector even though the component catches it).
    recallSearch.mockResolvedValueOnce([]);
    await renderRecall();
    await waitFor(() => expect(recallSearch).toHaveBeenCalledTimes(1));
    recallSearch.mockImplementationOnce(async () => {
      throw new Error('store locked');
    });
    fireEvent.click(screen.getByRole('button', { name: '搜尋' }));
    await waitFor(() => expect(screen.getByText(/store locked/)).toBeTruthy());
  });

  it('unknown kinds fall back to a bullet glyph (no crash on legacy data)', async () => {
    recallSearch.mockResolvedValue([
      { eventId: 'e2', timestamp: 'not-a-date', kind: 'mystery', summary: 'legacy row' },
    ]);
    await renderRecall();
    await waitFor(() => expect(screen.getByText('legacy row')).toBeTruthy());
    // bad timestamp renders the raw-ish fallback, not "Invalid Date"
    expect(screen.queryByText(/Invalid Date/)).toBeNull();
    // …and an unparseable timestamp makes the row NON-tappable (no day to link).
    expect(screen.queryByRole('button', { name: /每日回顧/ })).toBeNull();
  });

  it('taps a hit through to that day’s review (/review?date=…)', async () => {
    recallSearch.mockResolvedValue([
      { eventId: 'e1', timestamp: '2026-05-29T08:00:00Z', kind: 'focus', summary: 'deep work 90m' },
    ]);
    await renderRecall();
    await waitFor(() => expect(screen.getByText('deep work 90m')).toBeTruthy());
    fireEvent.click(screen.getByRole('button', { name: /每日回顧/ }));
    // TZ-robust: assert the route shape, not an exact (timezone-dependent) date.
    expect(navigateMock).toHaveBeenCalledWith(
      expect.stringMatching(/^\/review\?date=\d{4}-\d{2}-\d{2}$/),
    );
  });
});
