// SPEC-41 §10.6 — CoachReviewList (S6) render + list / empty states.
// safeInvoke is mocked so the test never depends on a live daemon. daily_review_load
// returns a small set of days; the list keeps days with events (or locked) only.

import { describe, expect, it, vi } from 'vitest';

// Today's ISO, computed the same way the screen does, so exactly one probed
// date (today) reports events and the rest are empty.
function todayIso(): string {
  const d = new Date();
  const p = (n: number) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}
const TODAY = todayIso();

vi.mock('../../src/lib/tauri-compat', () => ({
  safeInvoke: vi.fn(async (cmd: string, args?: { date?: string | null }) => {
    if (cmd !== 'daily_review_load') return null;
    const date = args?.date ?? TODAY;
    return {
      date,
      markdown: '# Daily review',
      eventCount: date === TODAY ? 5 : 0,
      locked: false,
      flagged: false,
    };
  }),
}));

import { render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import CoachReviewList from '../../src/screens/macos/CoachReviewList';

function renderScreen() {
  return render(
    <MemoryRouter>
      <CoachReviewList />
    </MemoryRouter>,
  );
}

describe('<CoachReviewList /> (SPEC-41 §10.6)', () => {
  it('renders the header', () => {
    renderScreen();
    expect(screen.getByTestId('coach-review-list')).toBeInTheDocument();
    expect(screen.getByText('教練回顧')).toBeInTheDocument();
  });

  it('lists only days that have events, linking to the reader with ?date', async () => {
    renderScreen();
    await waitFor(() => {
      const rows = screen.getAllByTestId(/^review-row-/);
      expect(rows.length).toBe(1);
      expect(rows[0].getAttribute('href')).toMatch(/\/review\?date=/);
    });
  });
});
