// SPEC-41 §10.3 — ChipPopover (S2 chip quick-log) render + interactions.

import { describe, expect, it, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import ChipPopover from '../../src/screens/macos/ChipPopover';

describe('<ChipPopover /> (SPEC-41 §10.3)', () => {
  it('renders the title, 12 starter chips, and actions', () => {
    render(<ChipPopover />);
    expect(screen.getByTestId('chip-popover')).toBeInTheDocument();
    expect(screen.getByText('記一個習慣')).toBeInTheDocument();
    // 12-chip starter palette → 12 chip buttons + 1 free-text toggle + 2 footer
    expect(screen.getByLabelText('水')).toBeInTheDocument();
    expect(screen.getByLabelText('咖啡')).toBeInTheDocument();
    expect(screen.getByLabelText('早睡')).toBeInTheDocument();
    expect(screen.getByText('送出')).toBeInTheDocument();
  });

  it('submit is disabled until a chip is picked, then enabled', () => {
    render(<ChipPopover />);
    const submit = screen.getByText('送出').closest('button')!;
    expect(submit).toBeDisabled();
    fireEvent.click(screen.getByLabelText('水'));
    expect(screen.getByLabelText('水')).toHaveAttribute('aria-pressed', 'true');
    expect(submit).not.toBeDisabled();
  });

  it('free-text toggle swaps the chip grid for a single text input', () => {
    render(<ChipPopover />);
    fireEvent.click(screen.getByText('自由打字…'));
    expect(screen.getByPlaceholderText('自由打字記錄…')).toBeInTheDocument();
  });

  it('calls onCancel when 取消 is clicked', () => {
    const onCancel = vi.fn();
    render(<ChipPopover onCancel={onCancel} />);
    fireEvent.click(screen.getByText(/取消/));
    expect(onCancel).toHaveBeenCalled();
  });
});
