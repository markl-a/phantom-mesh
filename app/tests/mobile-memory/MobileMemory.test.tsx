// Component tests — MobileMemory (mobile port of settings/MemoryPanel.tsx).
// Covers stats rendering, observations list, snake/camelCase normalisation,
// and the search flow. Error/reject path skipped to avoid the vitest
// mock-results-tracked unhandled-rejection gotcha.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

const invokeMock = vi.fn();
vi.mock('../../src/lib/tauri-compat', () => ({
  isTauri: () => false,
  safeInvoke: (...args: unknown[]) => invokeMock(...args),
}));

beforeEach(() => invokeMock.mockReset());
afterEach(() => vi.useRealTimers());

async function renderPanel() {
  const { default: MobileMemory } = await import(
    '../../src/components/mobile/MobileMemory'
  );
  return render(<MobileMemory />);
}

const sampleStats = {
  total_entries: 128,
  compression_ratio: '4.2:1',
  last_sync: '2026-05-30 14:32',
};

const sampleObservations = [
  {
    id: 'OBS-001',
    content: '用戶詢問如何使用 broker token',
    type: 'episodic',
    timestamp: '2026-05-30 14:30',
    relevance: 0.85,
  },
  {
    id: 'OBS-002',
    content: 'agent 成功執行 web_search',
    type: 'working',
    timestamp: '2026-05-30 14:28',
    relevance: 0.62,
  },
];

describe('<MobileMemory />', () => {
  it('renders stats (snake_case normalised) and observations on mount', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_memory_stats') return Promise.resolve(sampleStats);
      if (cmd === 'get_memory_observations') return Promise.resolve(sampleObservations);
      return Promise.resolve(null);
    });
    await renderPanel();
    await waitFor(() => expect(screen.getByTestId('mobile-memory-root')).toBeTruthy());
    expect(screen.getByTestId('memory-stats').textContent).toContain('128');
    expect(screen.getByTestId('memory-stats').textContent).toContain('4.2:1');
    expect(screen.getByTestId('memory-OBS-001')).toBeTruthy();
    expect(screen.getByText('用戶詢問如何使用 broker token')).toBeTruthy();
  });

  it('unwraps {observations: [...]} envelope', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_memory_stats') return Promise.resolve({});
      if (cmd === 'get_memory_observations')
        return Promise.resolve({ observations: [sampleObservations[0]] });
      return Promise.resolve(null);
    });
    await renderPanel();
    await waitFor(() => expect(screen.getByTestId('memory-OBS-001')).toBeTruthy());
  });

  it('shows empty state when there are no observations', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_memory_stats') return Promise.resolve({});
      if (cmd === 'get_memory_observations') return Promise.resolve([]);
      return Promise.resolve(null);
    });
    await renderPanel();
    await waitFor(() => expect(screen.getByTestId('memory-empty')).toBeTruthy());
  });

  it('search invokes search_memory and replaces the list', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_memory_stats') return Promise.resolve(sampleStats);
      if (cmd === 'get_memory_observations') return Promise.resolve(sampleObservations);
      if (cmd === 'search_memory')
        return Promise.resolve([
          {
            id: 'OBS-007',
            content: 'broker token rotation 完成',
            type: 'episodic',
            relevance: 0.91,
          },
        ]);
      return Promise.resolve(null);
    });
    await renderPanel();
    await waitFor(() => screen.getByTestId('memory-OBS-001'));

    await userEvent.type(screen.getByTestId('memory-search-input'), 'broker');
    await userEvent.click(screen.getByTestId('memory-search-go'));

    await waitFor(() => {
      expect(
        invokeMock.mock.calls.some(
          (c) =>
            c[0] === 'search_memory' &&
            (c[1] as { query: string }).query === 'broker',
        ),
      ).toBe(true);
    });
    await waitFor(() => expect(screen.getByTestId('memory-OBS-007')).toBeTruthy());
    // Clear button appears once a search has run.
    expect(screen.getByTestId('memory-search-clear')).toBeTruthy();
  });
});
