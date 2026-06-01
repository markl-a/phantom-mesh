// F101 · Integration tests — MobileCluster screen.
//
// We mock the `tauri-compat` invoke wrapper so the hook resolves with
// canned `get_cluster_peers` results, and we drive the SSE stream via
// the test seam `__setListenImpl` exposed by `useClusterPeers`.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, render, screen, waitFor } from '@testing-library/react';
import {
  useClusterPeersStore,
  type PeerEvent,
  type PeerSummary,
} from '../../src/stores/clusterPeersStore';
import { __setListenImpl } from '../../src/hooks/useClusterPeers';

// ── invoke() mock ────────────────────────────────────────────────────────
//
// `tauri-compat` calls `safeInvoke` → our wrapper. We replace it at the
// module level so every call from the hook is observable.

const invokeMock = vi.fn();
vi.mock('../../src/lib/tauri-compat', () => ({
  isTauri: () => false,
  safeInvoke: (...args: unknown[]) => invokeMock(...args),
}));

// ── listen() test seam ───────────────────────────────────────────────────

type EmittedHandler = (ev: { payload: PeerEvent }) => void;
let lastHandler: EmittedHandler | null = null;
let unlistenCalls = 0;

beforeEach(() => {
  invokeMock.mockReset();
  lastHandler = null;
  unlistenCalls = 0;
  useClusterPeersStore.getState().reset();
  useClusterPeersStore.setState({ selectedPeerId: null });

  __setListenImpl(async (_name, handler) => {
    lastHandler = handler as EmittedHandler;
    return () => {
      unlistenCalls += 1;
    };
  });
});

afterEach(() => {
  __setListenImpl(null);
});

function samplePeers(): PeerSummary[] {
  return [
    {
      peer_id: 'macbook',
      display_name: 'MacBook',
      caps: ['gpu', 'camera'],
      status: 'Online',
      last_seen_unix: 100,
    },
    {
      peer_id: 'phone',
      display_name: 'Phone',
      caps: ['camera', 'gps'],
      status: 'Unhealthy',
      last_seen_unix: 99,
    },
  ];
}

async function renderScreen() {
  // Dynamic import after mocks are set up.
  const { default: MobileCluster } = await import(
    '../../src/components/mobile/MobileCluster'
  );
  return render(<MobileCluster />);
}

describe('<MobileCluster />', () => {
  it('shows skeleton then populates list from get_cluster_peers', async () => {
    let resolvePeers!: (v: PeerSummary[]) => void;
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_cluster_peers') {
        return new Promise<PeerSummary[]>((r) => {
          resolvePeers = r;
        });
      }
      if (cmd === 'get_this_device_label') return Promise.resolve(null);
      if (cmd === 'subscribe_cluster_events') return Promise.resolve(null);
      return Promise.resolve(null);
    });

    await renderScreen();
    expect(screen.getByTestId('peer-list-skeleton')).toBeInTheDocument();

    await act(async () => {
      resolvePeers(samplePeers());
    });

    await waitFor(() => {
      expect(screen.getByTestId('peer-list')).toBeInTheDocument();
    });
    expect(screen.getByText('MacBook')).toBeInTheDocument();
    expect(screen.getByText('Phone')).toBeInTheDocument();
  });

  it('renders the empty state when no peers are returned', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_cluster_peers') return Promise.resolve([]);
      return Promise.resolve(null);
    });
    await renderScreen();
    await waitFor(() => {
      expect(screen.getByTestId('peers-empty')).toBeInTheDocument();
    });
  });

  it('flips a peer badge live when a cluster::peer_event arrives', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_cluster_peers') return Promise.resolve(samplePeers());
      return Promise.resolve(null);
    });

    await renderScreen();
    await waitFor(() => {
      expect(screen.getByText('MacBook')).toBeInTheDocument();
    });

    // Find MacBook's row → badge should currently say "Online".
    const macRow = screen.getByText('MacBook').closest('button');
    expect(macRow).not.toBeNull();
    expect(macRow!.textContent).toContain('Online');

    // Push an event flipping MacBook to Unhealthy.
    await waitFor(() => expect(lastHandler).not.toBeNull());
    await act(async () => {
      lastHandler!({ payload: { peer_id: 'macbook', status: 'Unhealthy' } });
    });

    await waitFor(() => {
      expect(macRow!.textContent).toContain('Unhealthy');
    });
  });

  it('shows the error state when get_cluster_peers rejects with a typed code', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_cluster_peers') {
        return Promise.reject(new Error('E_CLUSTER_HUB_UNCONFIGURED'));
      }
      return Promise.resolve(null);
    });

    await renderScreen();
    await waitFor(() => {
      const err = screen.getByTestId('peers-error');
      expect(err).toBeInTheDocument();
      expect(err.textContent).toContain('Sign in to a broker');
    });
  });

  it('unsubscribes from cluster::peer_event on unmount (no leak)', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_cluster_peers') return Promise.resolve(samplePeers());
      return Promise.resolve(null);
    });
    const { unmount } = await renderScreen();
    await waitFor(() => {
      expect(screen.getByText('MacBook')).toBeInTheDocument();
    });
    // Wait for the listener to attach.
    await waitFor(() => expect(lastHandler).not.toBeNull());

    unmount();
    // The unlisten fn returned by our test impl bumps the counter.
    await waitFor(() => expect(unlistenCalls).toBeGreaterThanOrEqual(1));
  });
});
