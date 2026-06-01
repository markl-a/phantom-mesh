// SPEC-41 §10.4 — FocusStartSheet (S3 focus start sheet) render + edge cases.

import { describe, expect, it, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import FocusStartSheet from '../../src/screens/macos/FocusStartSheet';

describe('<FocusStartSheet /> (SPEC-41 §10.4)', () => {
  it('renders the wireframe surface: title, presets, recording, note, actions', () => {
    render(<FocusStartSheet />);
    expect(screen.getByTestId('focus-start-sheet')).toBeInTheDocument();
    expect(screen.getByText('開始焦點 session')).toBeInTheDocument();
    expect(screen.getByText('25 min（Pomodoro 標準）')).toBeInTheDocument();
    expect(screen.getByText('50 min（Pomodoro 長）')).toBeInTheDocument();
    expect(screen.getByText(/同步錄音/)).toBeInTheDocument();
    expect(screen.getByText(/Cloud ASR fallback/)).toBeInTheDocument();
    expect(screen.getByText('開始')).toBeInTheDocument();
    expect(screen.getByText('取消')).toBeInTheDocument();
  });

  it('defaults to the 50-min preset (wireframe ● marker)', () => {
    render(<FocusStartSheet />);
    const radios = screen.getAllByRole('radio');
    // order: 25, 50, custom — 50 is checked by default
    expect((radios[1] as HTMLInputElement).checked).toBe(true);
    expect((radios[0] as HTMLInputElement).checked).toBe(false);
  });

  it('disables on-device ASR + shows the permission hint when mic is not granted', () => {
    render(<FocusStartSheet micGranted={false} />);
    const sync = screen.getByText(/同步錄音/).closest('label')!.querySelector('input')!;
    expect(sync).toBeDisabled();
    expect(screen.getByText(/需開麥克風權限/)).toBeInTheDocument();
  });

  it('calls onCancel when 取消 is clicked', () => {
    const onCancel = vi.fn();
    render(<FocusStartSheet onCancel={onCancel} />);
    fireEvent.click(screen.getByText('取消'));
    expect(onCancel).toHaveBeenCalled();
  });
});
