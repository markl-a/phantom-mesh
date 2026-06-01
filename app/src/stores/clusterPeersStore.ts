// F101 · Cluster peers store (mobile)
//
// Separate from the desktop `clusterStore.ts` so the two surfaces don't
// entangle state shapes. This store mirrors the `PeerSummary` payload
// returned by F100's `get_cluster_peers` Tauri command and the
// `cluster::peer_event` stream emitted by `subscribe_cluster_events`.
//
// Contract (per docs/superpowers/features/F100-tauri-cluster-peers-commands.md §Scope):
//   PeerSummary {
//     peer_id: String,
//     display_name: String,
//     caps: Vec<String>,
//     status: PeerStatus (Online | Unhealthy | Unknown),
//     last_seen_unix: i64,
//   }
//
// The store is intentionally small: status reducer + event patch. UI
// components subscribe to slices.

import { create } from 'zustand';

export type PeerStatus = 'Online' | 'Unhealthy' | 'Unknown';

export interface PeerSummary {
  peer_id: string;
  display_name: string;
  caps: string[];
  status: PeerStatus;
  last_seen_unix: number;
}

/** Event payload shape emitted by F100's `cluster::peer_event` channel. */
export interface PeerEvent {
  // F100 emits per-peer patches; missing fields mean "leave as-is".
  peer_id: string;
  display_name?: string;
  caps?: string[];
  status?: PeerStatus;
  last_seen_unix?: number;
}

export type ClusterPeersStatus = 'idle' | 'loading' | 'error';

export interface ClusterPeersState {
  peers: PeerSummary[];
  lastSyncMs: number;
  status: ClusterPeersStatus;
  error?: string;
  /** Peer-id of the local device (set via F100 `set_this_device_label`). */
  thisDeviceId: string | null;
  /** Persisted selection — survives reload via localStorage. */
  selectedPeerId: string | null;

  /** Replace the entire peer list (used after `get_cluster_peers`). */
  setPeers: (peers: PeerSummary[]) => void;
  /** Apply a single SSE patch event to one peer (creates row if unknown). */
  applyEvent: (ev: PeerEvent) => void;
  setStatus: (status: ClusterPeersStatus, error?: string) => void;
  setThisDeviceId: (id: string | null) => void;
  selectPeer: (peerId: string | null) => void;
  reset: () => void;
}

const SELECTED_KEY = 'phantom_mesh_f101_selected_peer';

/** Normalize a wire status value to the PascalCase PeerStatus union. The
 *  backend derives status at runtime and emits lowercase (online / unhealthy
 *  / unknown); without this, strict comparisons like `p.status === 'Online'`
 *  (e.g. the online-peer count) silently never match, and the badge styling
 *  falls through to neutral. Normalizing at ingestion keeps the stored type
 *  honest for every consumer. */
export function normalizePeerStatus(s: unknown): PeerStatus {
  switch (String(s).toLowerCase()) {
    case 'online':
      return 'Online';
    case 'unhealthy':
      return 'Unhealthy';
    default:
      return 'Unknown';
  }
}

function loadSelected(): string | null {
  try {
    return localStorage.getItem(SELECTED_KEY);
  } catch {
    return null;
  }
}

function saveSelected(id: string | null): void {
  try {
    if (id === null) localStorage.removeItem(SELECTED_KEY);
    else localStorage.setItem(SELECTED_KEY, id);
  } catch {
    /* private mode / quota — non-fatal */
  }
}

export const useClusterPeersStore = create<ClusterPeersState>()((set) => ({
  peers: [],
  lastSyncMs: 0,
  status: 'idle',
  error: undefined,
  thisDeviceId: null,
  selectedPeerId: loadSelected(),

  setPeers: (peers) =>
    set({
      peers: peers.map((p) => ({ ...p, status: normalizePeerStatus(p.status) })),
      lastSyncMs: Date.now(),
      status: 'idle',
      error: undefined,
    }),

  applyEvent: (ev) =>
    set((s) => {
      const idx = s.peers.findIndex((p) => p.peer_id === ev.peer_id);
      if (idx === -1) {
        // Unknown peer — synthesise a new row from the partial event.
        const row: PeerSummary = {
          peer_id: ev.peer_id,
          display_name: ev.display_name ?? ev.peer_id,
          caps: ev.caps ?? [],
          status: normalizePeerStatus(ev.status),
          last_seen_unix: ev.last_seen_unix ?? 0,
        };
        return { peers: [...s.peers, row] };
      }
      const next = [...s.peers];
      const cur = next[idx];
      next[idx] = {
        ...cur,
        display_name: ev.display_name ?? cur.display_name,
        caps: ev.caps ?? cur.caps,
        status: ev.status !== undefined ? normalizePeerStatus(ev.status) : cur.status,
        last_seen_unix: ev.last_seen_unix ?? cur.last_seen_unix,
      };
      return { peers: next };
    }),

  setStatus: (status, error) => set({ status, error }),
  setThisDeviceId: (id) => set({ thisDeviceId: id }),
  selectPeer: (peerId) => {
    saveSelected(peerId);
    set({ selectedPeerId: peerId });
  },

  reset: () =>
    set({
      peers: [],
      lastSyncMs: 0,
      status: 'idle',
      error: undefined,
      thisDeviceId: null,
    }),
}));

// Exported for unit tests that need to poke localStorage directly.
export const __INTERNAL = { SELECTED_KEY };
