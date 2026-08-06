// F101 · Single peer row.
//
// Renders one peer with status badge, "this device" pill, and the first
// few capability chips. Tap selects the row (selection is persisted in
// localStorage by the store — see clusterPeersStore.ts).

import type { PeerSummary } from '../../stores/clusterPeersStore';
import PeerBadge from './peerBadge';

const MAX_CAPS_VISIBLE = 4;

// Stable priority list so the most-important caps are kept when we trim
// to `MAX_CAPS_VISIBLE` (per F101 risk register).
const CAP_PRIORITY: readonly string[] = [
  'gpu',
  'camera',
  'vision',
  'gps',
  'audio',
  'mic',
];

function sortCaps(caps: readonly string[] | null | undefined): string[] {
  // peer.caps comes from unvalidated backend data — guard against null/undefined
  // so the spread can't throw ("... is not iterable") and crash the cluster screen.
  return [...(caps ?? [])].sort((a, b) => {
    const ai = CAP_PRIORITY.indexOf(a.toLowerCase());
    const bi = CAP_PRIORITY.indexOf(b.toLowerCase());
    if (ai === -1 && bi === -1) return a.localeCompare(b);
    if (ai === -1) return 1;
    if (bi === -1) return -1;
    return ai - bi;
  });
}

interface PeerCardProps {
  peer: PeerSummary;
  isThisDevice: boolean;
  isSelected: boolean;
  onSelect: (peerId: string) => void;
}

export default function PeerCard({
  peer,
  isThisDevice,
  isSelected,
  onSelect,
}: PeerCardProps) {
  const sorted = sortCaps(peer.caps);
  const visible = sorted.slice(0, MAX_CAPS_VISIBLE);
  const overflow = sorted.length - visible.length;

  return (
    <button
      type="button"
      onClick={() => onSelect(peer.peer_id)}
      data-peer-id={peer.peer_id}
      data-selected={isSelected ? 'true' : 'false'}
      className={`w-full text-left bg-spectyn-card border rounded-lg px-3 py-2.5 transition-colors ${
        isSelected
          ? 'border-spectyn-primary'
          : 'border-spectyn-border hover:border-spectyn-primary/40'
      }`}
      aria-pressed={isSelected}
      aria-label={`peer ${peer.display_name}`}
    >
      <div className="flex items-center justify-between gap-2">
        <div className="flex items-center gap-2 min-w-0">
          <span className="text-sm font-medium text-spectyn-text truncate">
            {peer.display_name}
          </span>
          {isThisDevice && (
            <span
              className="text-[10px] px-1.5 py-0.5 rounded-full bg-spectyn-primary/15 text-spectyn-primary border border-spectyn-primary/30"
              data-testid="this-device-pill"
            >
              this device
            </span>
          )}
        </div>
        <PeerBadge status={peer.status} />
      </div>

      {visible.length > 0 && (
        <div className="mt-1.5 flex flex-wrap gap-1">
          {visible.map((c) => (
            <span
              key={c}
              className="text-[10px] px-1.5 py-0.5 rounded bg-spectyn-bg border border-spectyn-border text-spectyn-muted"
            >
              {c}
            </span>
          ))}
          {overflow > 0 && (
            <span
              data-testid="caps-overflow"
              className="text-[10px] px-1.5 py-0.5 rounded bg-spectyn-bg border border-spectyn-border text-spectyn-muted"
            >
              +{overflow} more
            </span>
          )}
        </div>
      )}
    </button>
  );
}
