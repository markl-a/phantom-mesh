// SPEC-41 §10.12 — MeshPeerAddWizard (S12) render + scanning state.
// safeInvoke mocked so useClusterPeers doesn't hit a live daemon (no peers found).

import { describe, expect, it, vi } from 'vitest';

vi.mock('../../src/lib/tauri-compat', () => ({
  safeInvoke: vi.fn(async (cmd: string) => (cmd === 'get_cluster_peers' ? [] : null)),
}));

import { render, screen } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import MeshPeerAddWizard from '../../src/screens/macos/MeshPeerAddWizard';

function renderScreen() {
  return render(
    <MemoryRouter>
      <MeshPeerAddWizard />
    </MemoryRouter>,
  );
}

describe('<MeshPeerAddWizard /> (SPEC-41 §10.12)', () => {
  it('opens in the scanning state', () => {
    renderScreen();
    expect(screen.getByTestId('mesh-peer-add-wizard')).toHaveAttribute('data-state', 'scanning');
    expect(screen.getByTestId('wizard-scanning')).toBeInTheDocument();
    expect(screen.getByText(/正在掃描附近裝置/)).toBeInTheDocument();
  });

  it('shows the cancel + header chrome', () => {
    renderScreen();
    expect(screen.getByText('新增對等節點')).toBeInTheDocument();
    expect(screen.getByText('取消')).toBeInTheDocument();
  });
});
