// SPEC-41 §10.7 — ClusterStatusDashboard (S7) render + isolated-mode edge case.
// safeInvoke is mocked so the test never depends on a live daemon's cluster state.

import { describe, expect, it, vi } from 'vitest';

vi.mock('../../src/lib/tauri-compat', () => ({
  safeInvoke: vi.fn(async (cmd: string) => (cmd === 'get_cluster_peers' ? [] : null)),
}));

import { render, screen, waitFor } from '@testing-library/react';
import ClusterStatusDashboard from '../../src/screens/macos/ClusterStatusDashboard';

describe('<ClusterStatusDashboard /> (SPEC-41 §10.7)', () => {
  it('renders the dashboard header + refresh action', () => {
    render(<ClusterStatusDashboard />);
    expect(screen.getByTestId('cluster-status-dashboard')).toBeInTheDocument();
    expect(screen.getByText('叢集狀態')).toBeInTheDocument();
    expect(screen.getByText('重新整理')).toBeInTheDocument();
  });

  it('shows the isolated-mode message when there are no peers (§10.7 edge case)', async () => {
    render(<ClusterStatusDashboard />);
    await waitFor(() =>
      expect(screen.getByText('孤立模式（單機跑）')).toBeInTheDocument(),
    );
  });
});
