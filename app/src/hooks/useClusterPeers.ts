// F101 · Cluster peers hook.
// F106 · Backgrounding + foregrounding SSE reconnect.
//
// Combines an initial `get_cluster_peers` invoke + a long-lived
// `subscribe_cluster_events` Tauri event subscription. Returns the store
// slice the UI needs.
//
// Per F101 acceptance: must not double-subscribe under StrictMode.
// Strategy: track listener via a module-level ref-count keyed by the
// hook instance, only call F100's `subscribe_cluster_events` once per
// mount cycle, and tear down via the unlisten fn returned by Tauri's
// `listen()` API.
//
// Per F106 (E002 acceptance #4): the cluster screen subscription must
// survive app backgrounding + foregrounding and reconnect within 5s of
// resume. We hook into `document.visibilitychange`:
//
//   - on `'hidden'`: release the active `unlisten` and abort the
//     in-flight refresh. We do not keep an SSE channel alive while the
//     app is in the background — saves battery on mobile, and the
//     broker has no reverse keep-alive cost to retiring the client end.
//   - on `'visible'` (only if we currently hold no listener): re-bootstrap
//     via `refresh()` + a fresh `subscribe_cluster_events` invoke + a
//     fresh `listen()` registration.
//
// Idempotency is preserved by guarding on `unlistenRef.current == null`
// before re-subscribing and by ignoring 'hidden' events when we already
// have no listener.

import { useCallback, useEffect, useRef } from 'react';
import { safeInvoke } from '../lib/tauri-compat';
import {
  useClusterPeersStore,
  type PeerEvent,
  type PeerSummary,
} from '../stores/clusterPeersStore';

/** The Tauri event name F100 emits per peer change. */
export const PEER_EVENT_NAME = 'cluster::peer_event';

/** Override-able for tests — defaults to dynamic import of @tauri-apps/api/event. */
export type ListenFn = <T>(
  event: string,
  handler: (ev: { payload: T }) => void,
) => Promise<() => void>;

let _listenImpl: ListenFn | null = null;

/** Test-seam: swap the underlying `listen` implementation. */
export function __setListenImpl(impl: ListenFn | null): void {
  _listenImpl = impl;
}

async function getListen(): Promise<ListenFn> {
  if (_listenImpl) return _listenImpl;
  // In non-Tauri environments (browser dev / vitest), return a no-op
  // listener. Tests that exercise the SSE path should call
  // `__setListenImpl` to inject a mock.
  if (typeof window === 'undefined') {
    return async () => () => {};
  }
  const w = window as unknown as { __TAURI_INTERNALS__?: unknown; __TAURI__?: unknown };
  if (!w.__TAURI_INTERNALS__ && !w.__TAURI__) {
    return async () => () => {};
  }
  const mod = (await import('@tauri-apps/api/event')) as unknown as { listen: ListenFn };
  return mod.listen;
}

interface UseClusterPeersResult {
  peers: PeerSummary[];
  status: 'idle' | 'loading' | 'error';
  error?: string;
  lastSyncMs: number;
  thisDeviceId: string | null;
  selectedPeerId: string | null;
  selectPeer: (peerId: string | null) => void;
  refresh: () => Promise<void>;
}

export function useClusterPeers(): UseClusterPeersResult {
  const peers = useClusterPeersStore((s) => s.peers);
  const status = useClusterPeersStore((s) => s.status);
  const error = useClusterPeersStore((s) => s.error);
  const lastSyncMs = useClusterPeersStore((s) => s.lastSyncMs);
  const thisDeviceId = useClusterPeersStore((s) => s.thisDeviceId);
  const selectedPeerId = useClusterPeersStore((s) => s.selectedPeerId);
  const selectPeer = useClusterPeersStore((s) => s.selectPeer);
  const setPeers = useClusterPeersStore((s) => s.setPeers);
  const setStatus = useClusterPeersStore((s) => s.setStatus);
  const applyEvent = useClusterPeersStore((s) => s.applyEvent);
  const setThisDeviceId = useClusterPeersStore((s) => s.setThisDeviceId);

  const cancelledRef = useRef(false);

  const refresh = useCallback(async () => {
    setStatus('loading');
    try {
      // F100 contract: `get_cluster_peers` → Vec<PeerSummary>.
      // Tauri's invoke returns the bare array; we accept either an array
      // or `{ peers: [...] }` for forward-compat with the broker shape.
      // TODO(F100): once F100 lands and we can confirm the wire shape,
      // tighten this to whichever the Rust side actually returns.
      const raw = await safeInvoke<PeerSummary[] | { peers?: PeerSummary[] }>(
        'get_cluster_peers',
      );
      if (cancelledRef.current) return;
      const list = Array.isArray(raw) ? raw : raw?.peers ?? [];
      setPeers(list);

      // Best-effort: ask F100 which peer is "this device". Command name
      // is `set_this_device_label` (write) — F100 doesn't yet expose a
      // dedicated getter, so we look it up via a sibling command if
      // present and fall back to null otherwise. TODO(F100): swap to
      // `get_this_device_label` once the read API exists.
      try {
        const label = await safeInvoke<string | null>('get_this_device_label');
        if (!cancelledRef.current) setThisDeviceId(label ?? null);
      } catch {
        /* command may not exist yet — non-fatal */
      }
    } catch (err) {
      if (cancelledRef.current) return;
      const msg = err instanceof Error ? err.message : String(err);
      setStatus('error', msg);
    }
  }, [setPeers, setStatus, setThisDeviceId]);

  // F106 · Hold the active unlisten in a ref so the visibility handler
  // (a sibling effect) can release + reacquire without going through
  // the bootstrap effect's local closure.
  const unlistenRef = useRef<(() => void) | null>(null);
  // Track whether a `subscribe()` call is currently in flight, so two
  // visibility events arriving in the same tick don't double-attach.
  const subscribingRef = useRef(false);
  // Tracks whether the hook has been unmounted; visibility handlers
  // fired after unmount must not re-subscribe.
  const mountedRef = useRef(true);

  // Pulled out so both the mount-bootstrap effect and the
  // visibilitychange effect call the same code path.
  const subscribe = useCallback(async () => {
    if (subscribingRef.current) return;
    if (unlistenRef.current !== null) return;
    if (!mountedRef.current) return;
    subscribingRef.current = true;
    try {
      cancelledRef.current = false;
      // 1) Bootstrap (or refresh) the peer list.
      await refresh();
      if (!mountedRef.current) return;
      // 2) Register the SSE-style event stream from F100.
      // F100 contract: `subscribe_cluster_events(window)` registers the
      // emitter; the JS side listens on `cluster::peer_event`. We invoke
      // first so Rust knows the window wants the stream, then attach the
      // listener. Order matters: if the listener attaches second we may
      // miss the first frame on slow daemons (TODO(F100): confirm).
      await safeInvoke('subscribe_cluster_events').catch(() => {
        /* command may not exist yet — non-fatal */
      });
      if (!mountedRef.current) return;
      const listen = await getListen();
      const off = await listen<PeerEvent>(PEER_EVENT_NAME, (ev) => {
        applyEvent(ev.payload);
      });
      if (!mountedRef.current) {
        // Race: we unmounted while `listen()` was in flight. Release
        // the freshly-acquired listener so we don't leak.
        try {
          off();
        } catch {
          /* non-fatal */
        }
        return;
      }
      unlistenRef.current = off;
    } catch {
      /* listen wiring is best-effort — UI still works via refresh */
    } finally {
      subscribingRef.current = false;
    }
  }, [refresh, applyEvent]);

  // Mount-time bootstrap. We keep this effect minimal — the heavy
  // lifting moved into `subscribe()` so the visibility handler can
  // reuse it verbatim.
  useEffect(() => {
    mountedRef.current = true;
    void subscribe();
    return () => {
      mountedRef.current = false;
      cancelledRef.current = true;
      const off = unlistenRef.current;
      unlistenRef.current = null;
      if (off) {
        try {
          off();
        } catch {
          /* non-fatal */
        }
      }
    };
  }, [subscribe]);

  // F106 · Visibility-driven release/reacquire.
  useEffect(() => {
    if (typeof document === 'undefined') return;

    const onChange = (): void => {
      const state = document.visibilityState;
      if (state === 'hidden') {
        // Release the live listener + cancel any in-flight refresh.
        // Idempotent: if we already have no listener, this is a no-op.
        const off = unlistenRef.current;
        if (off) {
          unlistenRef.current = null;
          try {
            off();
          } catch {
            /* non-fatal */
          }
        }
        cancelledRef.current = true;
        return;
      }
      if (state === 'visible') {
        // Re-acquire iff we currently hold no listener. Without this
        // guard a stray 'visible' event while already foregrounded
        // would multi-subscribe.
        if (unlistenRef.current === null && mountedRef.current) {
          void subscribe();
        }
      }
    };

    document.addEventListener('visibilitychange', onChange);
    return () => {
      document.removeEventListener('visibilitychange', onChange);
    };
  }, [subscribe]);

  return {
    peers,
    status,
    error,
    lastSyncMs,
    thisDeviceId,
    selectedPeerId,
    selectPeer,
    refresh,
  };
}
