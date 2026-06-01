// SPEC-20 capture-food — FoodCapturePanel render + interaction.

import { describe, expect, it, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import FoodCapturePanel from '../../src/components/food/FoodCapturePanel';

vi.mock('../../src/lib/tauri-compat', () => ({
  safeInvoke: vi.fn(async () => { throw new Error('food.not_yet_wired: deferred'); }),
}));

describe('<FoodCapturePanel /> (SPEC-20)', () => {
  it('renders the food-log panel', () => {
    render(<FoodCapturePanel />);
    expect(screen.getByTestId('food-capture-panel')).toBeInTheDocument();
    expect(screen.getByText('飲食記錄')).toBeInTheDocument();
    expect(screen.getByPlaceholderText(/今天吃了什麼/)).toBeInTheDocument();
    expect(screen.getByText('分析')).toBeInTheDocument();
  });

  it('disables 分析 until the user types a meal', () => {
    render(<FoodCapturePanel />);
    const btn = screen.getByText('分析').closest('button')!;
    expect(btn).toBeDisabled();
    fireEvent.change(screen.getByPlaceholderText(/今天吃了什麼/), { target: { value: '鮭魚便當' } });
    expect(btn).not.toBeDisabled();
  });
});
