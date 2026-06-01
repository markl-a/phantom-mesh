// F101 · Mobile cluster screen.
//
// Lists known peers (via F100's `get_cluster_peers`) with status badges
// that update live (via F100's `cluster::peer_event` stream). Per the
// E002 acceptance criterion the selected peer is persisted in
// localStorage by `clusterPeersStore.ts`.
//
// Behaviour:
//   - skeleton while initial invoke is in flight (≤200ms target)
//   - empty state with link hint to Settings
//   - error state surfaces the typed error code from F100
//   - pull-to-refresh re-invokes `get_cluster_peers`
//
// We intentionally render the LIST in a separate sub-component
// (`PeerList`) so the test for "empty / skeleton / list" branches stays
// shallow.

import { RefreshCw } from 'lucide-react';
import { useClusterPeers } from '../../hooks/useClusterPeers';
import type { PeerSummary } from '../../stores/clusterPeersStore';
import PeerCard from './PeerCard';

function Skeleton() {
  // Lightweight Tailwind-only skeleton. We don't have @radix-ui/themes
  // in this app's deps (see package.json), so we hand-roll three shimmer
  // rows that match the PeerCard footprint.
  return (
    <div className="space-y-2" data-testid="peer-list-skeleton">
      {[0, 1, 2].map((i) => (
        <div
          key={i}
          className="bg-phantom-card border border-phantom-border rounded-lg px-3 py-2.5 animate-pulse"
        >
          <div className="h-3 w-24 bg-phantom-border rounded" />
          <div className="mt-2 h-2 w-40 bg-phantom-border/60 rounded" />
        </div>
      ))}
    </div>
  );
}

function EmptyState() {
  return (
    <div
      className="bg-phantom-card border border-phantom-border rounded-lg p-6 text-sm text-phantom-muted text-center"
      data-testid="peers-empty"
    >
      <p className="mb-1 text-phantom-text">No peers discovered yet</p>
      <p>
        No peers configured — see Settings → Cluster to sign in to a broker, or
        make sure your other devices are on the same network.
      </p>
    </div>
  );
}

function ErrorState({ message }: { message: string }) {
  // F100 returns typed error codes like `E_CLUSTER_HUB_UNCONFIGURED`.
  const friendly =
    message === 'E_CLUSTER_HUB_UNCONFIGURED'
      ? 'Sign in to a broker in Settings to start discovering peers.'
      : message;
  return (
    <div
      className="bg-phantom-card border border-phantom-danger/40 rounded-lg p-4 text-sm text-phantom-text"
      data-testid="peers-error"
      role="alert"
    >
      <p className="font-medium mb-1">Couldn't load peers</p>
      <p className="text-phantom-muted">{friendly}</p>
    </div>
  );
}

interface PeerListProps {
  peers: PeerSummary[];
  thisDeviceId: string | null;
  selectedPeerId: string | null;
  onSelect: (peerId: string) => void;
}

/** Left sidebar / list of peers — exported for unit tests. */
export function PeerList({
  peers,
  thisDeviceId,
  selectedPeerId,
  onSelect,
}: PeerListProps) {
  if (peers.length === 0) return <EmptyState />;
  return (
    <div className="space-y-2" data-testid="peer-list">
      {peers.map((p) => (
        <PeerCard
          key={p.peer_id}
          peer={p}
          isThisDevice={p.peer_id === thisDeviceId}
          isSelected={p.peer_id === selectedPeerId}
          onSelect={onSelect}
        />
      ))}
    </div>
  );
}

export default function MobileCluster() {
  const {
    peers,
    status,
    error,
    thisDeviceId,
    selectedPeerId,
    selectPeer,
    refresh,
  } = useClusterPeers();

  // Initial loading: status === 'loading' AND we have no peers yet.
  // After the first successful load we never show the full skeleton
  // again — manual refreshes show the spinner in the header instead.
  const showSkeleton = status === 'loading' && peers.length === 0;
  const showError = status === 'error' && peers.length === 0;
  const refreshing = status === 'loading' && peers.length > 0;

  // Case-insensitive: the Rust wire emits lowercase "online" (snake_case), while
  // the TS PeerStatus union is PascalCase — a raw === 'Online' silently counted 0.
  // See badgeStyleFor in peerBadge.tsx for the full contract-drift note.
  const onlineCount = peers.filter((p) => String(p.status).toLowerCase() === 'online').length;

  return (
    <div className="flex flex-col h-full overflow-y-auto">
      <div className="px-4 py-3 flex items-center justify-between border-b border-phantom-border">
        <div className="flex items-center gap-2 text-sm">
          <span className="text-phantom-text">Cluster</span>
          <span className="text-phantom-muted">
            {onlineCount}/{peers.length} online
          </span>
        </div>
        <button
          type="button"
          onClick={() => {
            void refresh();
          }}
          className="text-phantom-muted hover:text-phantom-text p-2 -m-2"
          aria-label="refresh peers"
        >
          <RefreshCw size={18} className={refreshing ? 'animate-spin' : ''} />
        </button>
      </div>

      <div className="p-3" data-testid="mobile-cluster-root">
        {showSkeleton ? (
          <Skeleton />
        ) : showError ? (
          <ErrorState message={error ?? 'unknown error'} />
        ) : (
          <PeerList
            peers={peers}
            thisDeviceId={thisDeviceId}
            selectedPeerId={selectedPeerId}
            onSelect={selectPeer}
          />
        )}
      </div>
    </div>
  );
}
