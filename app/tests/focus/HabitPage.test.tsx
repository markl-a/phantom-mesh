// SPEC-22 — HabitPage (habit streaks dashboard) render + empty/data states.

import { describe, expect, it, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';

const listHabits = vi.fn();
vi.mock('../../src/lib/captureHabit', async (orig) => {
  const actual = await orig() as Record<string, unknown>;
  return { ...actual, listHabits: () => listHabits() };
});

import HabitPage from '../../src/screens/macos/HabitPage';

beforeEach(() => listHabits.mockReset());

describe('<HabitPage /> (SPEC-22)', () => {
  it('renders header + 記錄 action', async () => {
    listHabits.mockResolvedValue([]);
    render(<HabitPage />);
    expect(screen.getByTestId('habit-page')).toBeInTheDocument();
    expect(screen.getByText('習慣')).toBeInTheDocument();
    expect(screen.getByText('記錄')).toBeInTheDocument();
  });

  it('shows empty state when no habits are logged', async () => {
    listHabits.mockResolvedValue([]);
    render(<HabitPage />);
    await waitFor(() => expect(screen.getByText('還沒有習慣紀錄')).toBeInTheDocument());
  });

  it('renders a streak card from a real HabitSummary', async () => {
    listHabits.mockResolvedValue([
      {
        habitSlug: 'water', last7dCount: 5, last30dCount: 20, lastCheckinAt: null,
        streak: { habitSlug: 'water', currentStreak: 3, longestStreak: 9, lastCheckinAt: null },
      },
    ]);
    render(<HabitPage />);
    await waitFor(() => expect(screen.getByText('水')).toBeInTheDocument());
    expect(screen.getByText('7 天 5')).toBeInTheDocument();
    expect(screen.getByText('30 天 20')).toBeInTheDocument();
  });
});
