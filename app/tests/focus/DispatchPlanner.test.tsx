// SPEC-26 — DispatchPlanner render + capability picker + empty-peer gating.
// safeInvoke mocked so useClusterPeers/planDispatch don't hit a live daemon.

import { describe, expect, it, vi } from 'vitest';

vi.mock('../../src/lib/tauri-compat', () => ({
  safeInvoke: vi.fn(async (cmd: string) => (cmd === 'get_cluster_peers' ? [] : null)),
}));

import { render, screen, fireEvent } from '@testing-library/react';
import DispatchPlanner from '../../src/screens/macos/DispatchPlanner';

describe('<DispatchPlanner /> (SPEC-26)', () => {
  it('renders the planner + capability options', () => {
    render(<DispatchPlanner />);
    expect(screen.getByTestId('dispatch-planner')).toBeInTheDocument();
    expect(screen.getByText('派工規劃')).toBeInTheDocument();
    expect(screen.getByText('cargo')).toBeInTheDocument();
    expect(screen.getByText('webSearch')).toBeInTheDocument();
  });

  it('toggles a capability via aria-pressed', () => {
    render(<DispatchPlanner />);
    const cargo = screen.getByText('cargo');
    expect(cargo).toHaveAttribute('aria-pressed', 'false');
    fireEvent.click(cargo);
    expect(cargo).toHaveAttribute('aria-pressed', 'true');
  });

  it('disables planning + shows hint when no peers are connected', () => {
    render(<DispatchPlanner />);
    expect(screen.getByText('規劃派工').closest('button')).toBeDisabled();
    expect(screen.getByText(/尚無連線的 peer/)).toBeInTheDocument();
  });
});
