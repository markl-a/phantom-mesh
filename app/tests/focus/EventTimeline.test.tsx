// SPEC-16 — EventTimeline (life timeline) render + empty/data + kind filter.

import { describe, expect, it, vi } from 'vitest';

const events = vi.fn();
vi.mock('../../src/lib/eventStore', async (orig) => {
  const actual = await orig() as Record<string, unknown>;
  return { ...actual, queryEvents: () => events() };
});

import { render, screen, waitFor, fireEvent } from '@testing-library/react';
import EventTimeline from '../../src/screens/macos/EventTimeline';

describe('<EventTimeline /> (SPEC-16)', () => {
  it('renders header + kind filters', async () => {
    events.mockResolvedValue([]);
    render(<EventTimeline />);
    expect(screen.getByTestId('event-timeline')).toBeInTheDocument();
    expect(screen.getByText('生活時間軸')).toBeInTheDocument();
    expect(screen.getByText('全部')).toBeInTheDocument();
    expect(screen.getByText(/飲食/)).toBeInTheDocument();
  });

  it('shows empty state when no events', async () => {
    events.mockResolvedValue([]);
    render(<EventTimeline />);
    await waitFor(() => expect(screen.getByText('尚無事件紀錄')).toBeInTheDocument());
  });

  it('renders an event row from a real EventRecord', async () => {
    events.mockResolvedValue([
      { meta: { eventId: 'e1', timestamp: '2026-05-28T10:00:00Z', kind: 'focus', tags: ['deep'] }, encryptedBodyPath: '/x', analysis: null },
    ]);
    render(<EventTimeline />);
    await waitFor(() => expect(screen.getByText('專注')).toBeInTheDocument());
  });

  it('clicking a kind filter sets aria-pressed', async () => {
    events.mockResolvedValue([]);
    render(<EventTimeline />);
    const all = screen.getByText('全部');
    expect(all).toHaveAttribute('aria-pressed', 'true');
    fireEvent.click(screen.getByText(/習慣/));
    expect(screen.getByText(/習慣/)).toHaveAttribute('aria-pressed', 'true');
  });
});
