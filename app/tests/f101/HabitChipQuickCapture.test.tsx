// Component tests — HabitChipQuickCapture (J2 in-app habit capture).
// Covers the qty quick-pick branch, the free-text fallback, and the
// non-quantifiable direct-commit path. captureHabit is mocked so no Tauri
// backend is needed; HabitQtyPicker renders for real against the mocked
// QUANTIFIABLE map.

import { describe, expect, it, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import HabitChipQuickCapture from '../../src/components/mobile/HabitChipQuickCapture';
import { ensureCheckin } from '../../src/lib/captureHabit';

vi.mock('../../src/lib/captureHabit', () => ({
  STARTER_PALETTE: [
    { slug: 'water', label: '水', emoji: '💧' },
    { slug: 'quit_smoke', label: '戒菸', emoji: '🚭' },
  ],
  QUANTIFIABLE: { water: { unit: 'ml', quick: [250, 500] } },
  isQuantifiable: (s: string) => s === 'water',
  ensureCheckin: vi.fn(async () => ({
    habitSlug: 'x', currentStreak: 3, longestStreak: 3, lastCheckinAt: null,
  })),
  describeHabitError: (e: unknown) => String(e),
}));

const mockEnsure = vi.mocked(ensureCheckin);

beforeEach(() => mockEnsure.mockClear());

describe('<HabitChipQuickCapture />', () => {
  it('non-quantifiable chip commits directly (no qty picker)', async () => {
    render(<HabitChipQuickCapture />);
    fireEvent.click(screen.getByRole('button', { name: '記錄習慣 戒菸' }));
    await waitFor(() =>
      expect(mockEnsure).toHaveBeenCalledWith('quit_smoke', '戒菸', { note: null }),
    );
    // no qty sheet for a non-quantifiable chip
    expect(screen.queryByText('水 — 數量')).not.toBeInTheDocument();
  });

  it('quantifiable chip opens the qty picker and commits the chosen quantity', async () => {
    render(<HabitChipQuickCapture />);
    fireEvent.click(screen.getByRole('button', { name: '記錄習慣 水' }));
    // picker opened, nothing committed yet
    expect(screen.getByText('水 — 數量')).toBeInTheDocument();
    expect(mockEnsure).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: '250 ml' }));
    await waitFor(() =>
      expect(mockEnsure).toHaveBeenCalledWith('water', '水', { note: '250ml' }),
    );
  });

  it('free-text submit commits to the freetext bucket; empty submit is a no-op', async () => {
    render(<HabitChipQuickCapture />);
    const input = screen.getByLabelText('自由記錄習慣');
    // empty submit → no commit
    fireEvent.click(screen.getByRole('button', { name: '送出' }));
    expect(mockEnsure).not.toHaveBeenCalled();
    // real text → commits to "freetext"
    fireEvent.change(input, { target: { value: '戒菸 87 天' } });
    fireEvent.click(screen.getByRole('button', { name: '送出' }));
    await waitFor(() =>
      expect(mockEnsure).toHaveBeenCalledWith('freetext', '自由記錄', { note: '戒菸 87 天' }),
    );
  });
});
