// task-2026052706 · FocusPage idle-render assertion (SPEC-21 capture_focus).
// Frontend counterpart to the Rust v4 e2e smoke — the React FSM page can only
// be rendered here, not from the CLI test harness.

import { describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';
import FocusPage from '../../src/components/focus/FocusPage';

describe('<FocusPage /> idle state', () => {
  it('renders the idle mode picker on first mount', () => {
    render(<FocusPage />);
    expect(screen.getByTestId('focus-page')).toBeInTheDocument();
    expect(screen.getByTestId('focus-idle')).toBeInTheDocument();
  });

  it('shows all four SPEC-21 §5 preset modes', () => {
    render(<FocusPage />);
    expect(screen.getByText('番茄鐘 25 分')).toBeInTheDocument();
    expect(screen.getByText('深度工作 50 分')).toBeInTheDocument();
    expect(screen.getByText('短衝 10 分')).toBeInTheDocument();
    expect(screen.getByText('自訂')).toBeInTheDocument();
  });

  it('defaults to pomodoro25 (aria-pressed) and offers a start action', () => {
    render(<FocusPage />);
    expect(screen.getByText('番茄鐘 25 分')).toHaveAttribute('aria-pressed', 'true');
    expect(screen.getByText(/開始/)).toBeInTheDocument();
  });
});
