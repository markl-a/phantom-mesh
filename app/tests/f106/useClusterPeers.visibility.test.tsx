// F106 · Cluster SSE backgrounding + foregrounding reconnect.
//
// E002 acceptance #4: "Cluster screen subscription survives app
// backgrounding + foregrounding (reconnects within 5s of resume)".
//
// We drive `document.visibilityState` + `visibilitychange` events through
// the hook under test (which is mounted via a thin host component so we
// exercise React 19's effect lifecycle) and assert:
//
//   1. hidden → existing unlisten fn is called exactly once,
//   2. visible → fresh `subscribe_cluster_events` invoke + fresh `listen`,
//   3. multiple rapid hidden→visible cycles do not multi-subscribe,
//   4. an event delivered after re-subscription still reaches the store.
//
// The harness mirrors `tests/f101/MobileCluster.test.tsx` — same invoke
// mock + `__setListenImpl` test seam.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, render, waitFor } from '@testing-library/react';
import {
  useClusterPeersStore,
  type PeerEvent,
  type PeerSummary,
} from '../../src/stores/clusterPeersStore';
import {
  __setListenImpl,
  useClusterPeers,
} from '../../src/hooks/useClusterPeers';

// ── invoke() mock ────────────────────────────────────────────────────────

const invokeMock = vi.fn();
vi.mock('../../src/lib/tauri-compat', () => ({
  isTauri: () => false,
  safeInvoke: (...args: unknown[]) => invokeMock(...args),
}));

// ── listen() test seam ───────────────────────────────────────────────────

type EmittedHandler = (ev: { payload: PeerEvent }) => void;

interface AttachedListener {
  handler: EmittedHandler;
  unlisten: ReturnType<typeof vi.fn>;
  unlistened: boolean;
}

let attached: AttachedListener[] = [];

function installListenImpl(): void {
  __setListenImpl(async (_name, handler) => {
    const entry: AttachedListener = {
      handler: handler as EmittedHandler,
      unlisten: vi.fn(),
      unlistened: false,
    };
    entry.unlisten.mockImplementation(() => {
      entry.unlistened = true;
    });
    attached.push(entry);
    return entry.unlisten;
  });
}

function samplePeers(): PeerSummary[] {
  return [
    {
      peer_id: 'macbook',
      display_name: 'MacBook',
      caps: ['gpu'],
      status: 'Online',
      last_seen_unix: 100,
    },
  ];
}

/** Drive `document.visibilityState` + fire the `visibilitychange` event. */
function setVisibility(value: 'visible' | 'hidden'): void {
  Object.defineProperty(document, 'visibilityState', {
    value,
    configurable: true,
  });
  Object.defineProperty(document, 'hidden', {
    value: value === 'hidden',
    configurable: true,
  });
  document.dispatchEvent(new Event('visibilitychange'));
}

/** Thin host so we can mount the hook directly without coupling to a screen. */
function HookHost(): null {
  useClusterPeers();
  return null;
}

beforeEach(() => {
  invokeMock.mockReset();
  attached = [];
  useClusterPeersStore.getState().reset();
  useClusterPeersStore.setState({ selectedPeerId: null });
  installListenImpl();
  // Start visible — JSDOM defaults to 'visible', but be explicit so a
  // previous test that left us hidden can't bleed in.
  setVisibility('visible');

  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === 'get_cluster_peers') return Promise.resolve(samplePeers());
    if (cmd === 'get_this_device_label') return Promise.resolve(null);
    if (cmd === 'subscribe_cluster_events') return Promise.resolve(null);
    return Promise.resolve(null);
  });
});

afterEach(() => {
  __setListenImpl(null);
});

function subscribeInvokeCount(): number {
  return invokeMock.mock.calls.filter(
    (c) => c[0] === 'subscribe_cluster_events',
  ).length;
}

describe('useClusterPeers · visibilitychange reconnect (F106)', () => {
  it('calls the active unlisten exactly once when the document hides', async () => {
    render(<HookHost />);

    // Wait for initial subscribe to attach.
    await waitFor(() => expect(attached.length).toBe(1));
    expect(attached[0].unlistened).toBe(false);

    await act(async () => {
      setVisibility('hidden');
    });

    await waitFor(() => expect(attached[0].unlisten).toHaveBeenCalledTimes(1));
    expect(attached[0].unlistened).toBe(true);
    // Still only the original listener — no re-subscribe on hide.
    expect(attached.length).toBe(1);
  });

  it('re-subscribes on visible after hidden (fresh invoke + fresh listen)', async () => {
    render(<HookHost />);

    await waitFor(() => expect(attached.length).toBe(1));
    const initialSubscribeCount = subscribeInvokeCount();
    expect(initialSubscribeCount).toBeGreaterThanOrEqual(1);

    await act(async () => {
      setVisibility('hidden');
    });
    await waitFor(() => expect(attached[0].unlistened).toBe(true));

    await act(async () => {
      setVisibility('visible');
    });

    // A second listener entry should attach.
    await waitFor(() => expect(attached.length).toBe(2));
    // And `subscribe_cluster_events` should have been invoked again.
    expect(subscribeInvokeCount()).toBe(initialSubscribeCount + 1);
    // The fresh listener must not be torn down.
    expect(attached[1].unlistened).toBe(false);
  });

  it('is idempotent — two hidden→visible cycles produce 3 listeners, not 4+', async () => {
    render(<HookHost />);
    await waitFor(() => expect(attached.length).toBe(1));

    // Cycle 1.
    await act(async () => {
      setVisibility('hidden');
    });
    await waitFor(() => expect(attached[0].unlistened).toBe(true));
    await act(async () => {
      setVisibility('visible');
    });
    await waitFor(() => expect(attached.length).toBe(2));

    // Cycle 2.
    await act(async () => {
      setVisibility('hidden');
    });
    await waitFor(() => expect(attached[1].unlistened).toBe(true));
    await act(async () => {
      setVisibility('visible');
    });
    await waitFor(() => expect(attached.length).toBe(3));

    // No bonus subscribers were spun up by repeated 'visible' events.
    // Mount + 2 resumes = exactly 3.
    expect(attached.length).toBe(3);
  });

  it('ignores duplicate visible events while already foregrounded', async () => {
    render(<HookHost />);
    await waitFor(() => expect(attached.length).toBe(1));

    // We're already visible; firing another 'visible' event must NOT
    // attach a second listener.
    await act(async () => {
      setVisibility('visible');
      setVisibility('visible');
    });
    // Give any spurious async work a tick to settle.
    await new Promise((r) => setTimeout(r, 10));

    expect(attached.length).toBe(1);
  });

  it('ignores duplicate hidden events without double-calling unlisten', async () => {
    render(<HookHost />);
    await waitFor(() => expect(attached.length).toBe(1));

    await act(async () => {
      setVisibility('hidden');
      setVisibility('hidden');
    });
    await waitFor(() => expect(attached[0].unlistened).toBe(true));
    expect(attached[0].unlisten).toHaveBeenCalledTimes(1);
  });

  it('delivers events from the re-subscribed listener into the store', async () => {
    render(<HookHost />);
    await waitFor(() => expect(attached.length).toBe(1));
    await waitFor(() =>
      expect(useClusterPeersStore.getState().peers.length).toBe(1),
    );

    // Background, then foreground.
    await act(async () => {
      setVisibility('hidden');
    });
    await waitFor(() => expect(attached[0].unlistened).toBe(true));
    await act(async () => {
      setVisibility('visible');
    });
    await waitFor(() => expect(attached.length).toBe(2));

    // Push an event via the *new* listener — store must reflect it.
    await act(async () => {
      attached[1].handler({
        payload: { peer_id: 'macbook', status: 'Unhealthy' },
      });
    });

    await waitFor(() => {
      const mac = useClusterPeersStore
        .getState()
        .peers.find((p) => p.peer_id === 'macbook');
      expect(mac?.status).toBe('Unhealthy');
    });
  });

  it('tears down both the visibility handler and the listener on unmount', async () => {
    const { unmount } = render(<HookHost />);
    await waitFor(() => expect(attached.length).toBe(1));

    unmount();
    await waitFor(() => expect(attached[0].unlistened).toBe(true));

    // After unmount, a stray visibilitychange must not re-subscribe.
    const beforeCount = attached.length;
    await act(async () => {
      setVisibility('hidden');
      setVisibility('visible');
    });
    await new Promise((r) => setTimeout(r, 10));
    expect(attached.length).toBe(beforeCount);
  });
});
