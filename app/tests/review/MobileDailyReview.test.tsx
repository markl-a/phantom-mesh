// SPEC-34 Screen 3 (Coach Review / 每日回顧) — mobile variant tests.
//
// Drives the three view states the screen must render (locked / empty /
// has-events) by mocking the `safeInvoke('daily_review_load')` seam that
// lib/dailyReview.ts calls. Mirrors the F103 MobileDispatch.test mock style.
// Wraps in MemoryRouter because the screen uses useNavigate for its back btn.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { MemoryRouter } from 'react-router-dom';

const invokeMock = vi.fn();
vi.mock('../../src/lib/tauri-compat', () => ({
  isTauri: () => false,
  safeInvoke: (...args: unknown[]) => invokeMock(...args),
}));

beforeEach(() => {
  invokeMock.mockReset();
});

afterEach(() => {
  vi.clearAllMocks();
});

async function renderScreen() {
  const { default: MobileDailyReview } = await import(
    '../../src/components/mobile/MobileDailyReview'
  );
  return render(
    <MemoryRouter initialEntries={['/review']}>
      <MobileDailyReview />
    </MemoryRouter>,
  );
}

describe('<MobileDailyReview />', () => {
  it('renders the locked state when no identity key is present', async () => {
    invokeMock.mockResolvedValue({
      date: '2026-05-28',
      markdown: '# Daily review — 2026-05-28\n',
      eventCount: 0,
      locked: true,
      flagged: false,
    });
    await renderScreen();
    await waitFor(() => {
      expect(screen.getByText(/事件已加密/)).toBeTruthy();
    });
  });

  it('renders the neutral empty state for a day with no events', async () => {
    invokeMock.mockResolvedValue({
      date: '2026-05-28',
      markdown: '# Daily review — 2026-05-28\n**Events captured:** 0\n',
      eventCount: 0,
      locked: false,
      flagged: false,
    });
    await renderScreen();
    await waitFor(() => {
      expect(screen.getByText(/沒有 Life Node 事件/)).toBeTruthy();
      // Shame-free reassurance copy is present.
      expect(screen.getByText(/空白的一天沒關係/)).toBeTruthy();
    });
  });

  it('renders grouped events with kind emoji + summaries', async () => {
    const markdown = [
      '# Daily review — 2026-05-28',
      '**Events captured:** 2',
      '## 健康 (1)',
      '- **focus** (2026-05-28T09:30:00+08:00): 寫程式 25 分鐘',
      '## 飲食 (1)',
      '- **food** (2026-05-28T12:00:00+08:00): 午餐沙拉',
    ].join('\n');
    invokeMock.mockResolvedValue({
      date: '2026-05-28',
      markdown,
      eventCount: 2,
      locked: false,
      flagged: false,
    });
    await renderScreen();
    await waitFor(() => {
      expect(screen.getByText('健康')).toBeTruthy();
      expect(screen.getByText('飲食')).toBeTruthy();
      expect(screen.getByText('寫程式 25 分鐘')).toBeTruthy();
      expect(screen.getByText('午餐沙拉')).toBeTruthy();
      // Local HH:MM extracted from the rfc3339 timestamp.
      expect(screen.getByText('09:30')).toBeTruthy();
    });
  });

  it('shows the flagged shame-free banner when the aggregate is flagged', async () => {
    invokeMock.mockResolvedValue({
      date: '2026-05-28',
      markdown: '# Daily review — 2026-05-28\n**Events captured:** 1\n## 其他 (1)\n- **text** (2026-05-28T08:00:00+08:00): note\n',
      eventCount: 1,
      locked: false,
      flagged: true,
    });
    await renderScreen();
    await waitFor(() => {
      expect(screen.getByText(/部分內容被標記/)).toBeTruthy();
    });
  });

  it('ignores a stale slow response that resolves after a newer date load', async () => {
    // Capture the per-call resolvers so we control resolution order.
    const calls: Array<{ date: unknown; resolve: (v: unknown) => void }> = [];
    invokeMock.mockImplementation(
      (_cmd: string, args: { date?: string }) =>
        new Promise((resolve) => {
          calls.push({ date: args?.date, resolve });
        }),
    );
    const view = (tag: string, summary: string) => ({
      date: '2026-05-28',
      markdown: `# Daily review — 2026-05-28\n**Events captured:** 1\n## ${tag} (1)\n- **focus** (2026-05-28T09:00:00+08:00): ${summary}\n`,
      eventCount: 1,
      locked: false,
      flagged: false,
    });

    await renderScreen();
    // Mount fired refresh(today) → calls[0] pending.
    await waitFor(() => expect(calls.length).toBe(1));

    // Navigate to the previous day → refresh(prev) → calls[1] pending.
    await userEvent.click(screen.getByLabelText('前一天'));
    await waitFor(() => expect(calls.length).toBe(2));

    // Resolve the NEWER request (calls[1]) first.
    await act(async () => {
      calls[1].resolve(view('新', '昨天的事件'));
    });
    await waitFor(() => expect(screen.getByText('昨天的事件')).toBeTruthy());

    // Now the STALE older request (calls[0]) resolves late — must be ignored.
    await act(async () => {
      calls[0].resolve(view('舊', '今天的事件-過期'));
    });
    // The screen must still show the newer day's data, not the stale one.
    expect(screen.queryByText('今天的事件-過期')).toBeNull();
    expect(screen.getByText('昨天的事件')).toBeTruthy();
  });

  it('surfaces a soft error when the backend is unavailable (null view)', async () => {
    // tauri-compat httpFallback returns {} in web mode → loadDailyReview maps
    // an unparseable view to null; the screen shows a soft banner not a crash.
    invokeMock.mockResolvedValue({});
    await renderScreen();
    await waitFor(() => {
      expect(screen.getByText(/每日回顧後端暫時無法使用/)).toBeTruthy();
    });
  });

  it('starts on the ?date= deep-link day (e.g. a Recall hit tapped through)', async () => {
    invokeMock.mockResolvedValue({
      date: '2020-01-15',
      markdown: '# Daily review — 2020-01-15\n',
      eventCount: 0,
      locked: false,
      flagged: false,
    });
    const { default: MobileDailyReview } = await import(
      '../../src/components/mobile/MobileDailyReview'
    );
    render(
      <MemoryRouter initialEntries={['/review?date=2020-01-15']}>
        <MobileDailyReview />
      </MemoryRouter>,
    );
    // The date strip reflects the deep-linked day, and the load used it.
    await waitFor(() => expect(screen.getByText('2020-01-15')).toBeTruthy());
    expect(invokeMock).toHaveBeenCalledWith(
      'daily_review_load',
      expect.objectContaining({ date: '2020-01-15' }),
    );
  });

  it('ignores a future / malformed ?date= param (falls back to today)', async () => {
    invokeMock.mockResolvedValue({
      date: '2099-01-01',
      markdown: '#\n',
      eventCount: 0,
      locked: false,
      flagged: false,
    });
    const { default: MobileDailyReview } = await import(
      '../../src/components/mobile/MobileDailyReview'
    );
    render(
      <MemoryRouter initialEntries={['/review?date=2099-01-01']}>
        <MobileDailyReview />
      </MemoryRouter>,
    );
    // A future date is rejected → the strip shows today, never the bad param.
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());
    expect(screen.queryByText('2099-01-01')).toBeNull();
  });
});
