// F101 · Unit tests — cluster peers store reducer.
//
// Covers: setPeers, applyEvent (existing peer patch + unknown peer
// auto-create), persistence of selectedPeerId across re-reads of the
// localStorage key, and reset().

import { beforeEach, describe, expect, it } from 'vitest';
import {
  useClusterPeersStore,
  __INTERNAL,
  type PeerSummary,
} from '../../src/stores/clusterPeersStore';

function makePeer(over: Partial<PeerSummary> = {}): PeerSummary {
  return {
    peer_id: 'p1',
    display_name: 'Peer One',
    caps: ['gpu'],
    status: 'Online',
    last_seen_unix: 1234,
    ...over,
  };
}

beforeEach(() => {
  // Reset the store + clear localStorage between tests so selection
  // doesn't leak across cases.
  try {
    localStorage.removeItem(__INTERNAL.SELECTED_KEY);
  } catch {
    /* ignore */
  }
  useClusterPeersStore.getState().reset();
  useClusterPeersStore.setState({ selectedPeerId: null });
});

describe('clusterPeersStore', () => {
  it('setPeers replaces the list and clears error', () => {
    useClusterPeersStore.getState().setStatus('error', 'boom');
    const peers = [makePeer(), makePeer({ peer_id: 'p2', display_name: 'P2' })];
    useClusterPeersStore.getState().setPeers(peers);

    const s = useClusterPeersStore.getState();
    expect(s.peers).toHaveLength(2);
    expect(s.peers[1].peer_id).toBe('p2');
    expect(s.status).toBe('idle');
    expect(s.error).toBeUndefined();
    expect(s.lastSyncMs).toBeGreaterThan(0);
  });

  it('applyEvent patches an existing peer in place', () => {
    useClusterPeersStore.getState().setPeers([makePeer()]);
    useClusterPeersStore
      .getState()
      .applyEvent({ peer_id: 'p1', status: 'Unhealthy', last_seen_unix: 9999 });

    const updated = useClusterPeersStore.getState().peers[0];
    expect(updated.status).toBe('Unhealthy');
    expect(updated.last_seen_unix).toBe(9999);
    // Fields not in the event should be preserved.
    expect(updated.display_name).toBe('Peer One');
    expect(updated.caps).toEqual(['gpu']);
  });

  it('applyEvent synthesises a new row for an unknown peer_id', () => {
    useClusterPeersStore.getState().setPeers([makePeer()]);
    useClusterPeersStore
      .getState()
      .applyEvent({ peer_id: 'p2', display_name: 'P2', status: 'Online' });

    const peers = useClusterPeersStore.getState().peers;
    expect(peers).toHaveLength(2);
    expect(peers[1].peer_id).toBe('p2');
    expect(peers[1].status).toBe('Online');
    // Missing fields fall back to safe defaults.
    expect(peers[1].caps).toEqual([]);
    expect(peers[1].last_seen_unix).toBe(0);
  });

  it('selectPeer persists to localStorage', () => {
    useClusterPeersStore.getState().selectPeer('p42');
    expect(localStorage.getItem(__INTERNAL.SELECTED_KEY)).toBe('p42');

    useClusterPeersStore.getState().selectPeer(null);
    expect(localStorage.getItem(__INTERNAL.SELECTED_KEY)).toBeNull();
  });
});
