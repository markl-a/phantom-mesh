// F101 · Status badge for a cluster peer.
//
// Three colours: green=Online, amber=Unhealthy, gray=Unknown. No red —
// per the existing mobile design system, red is reserved for dispatch
// errors (see MobileDashboard.tsx).

import type { PeerStatus } from '../../stores/clusterPeersStore';

interface PeerBadgeProps {
  status: PeerStatus;
  className?: string;
}

interface BadgeStyle {
  dot: string;
  text: string;
  label: string;
}

/**
 * Total mapping to a badge style. Exported for unit tests.
 *
 * NOTE (contract drift): the Rust wire (`app/src-tauri/src/commands/cluster_peers.rs`,
 * `#[serde(rename_all = "snake_case")]` on `PeerStatusKind`) emits LOWERCASE
 * `"online"` / `"unhealthy"` / `"unknown"`, but the TS `PeerStatus` union is
 * PascalCase. A raw `switch (status)` over the union therefore never matched the
 * real payload and fell through to `undefined`, crashing the entire app at
 * `s.text` (white-screen on the 集群/cluster tab — the always-present local peer
 * has status `"online"`). We normalize case-insensitively and keep the mapping
 * TOTAL (default → Unknown) so a missing/unmapped status can never return
 * undefined. See also the onlineCount in MobileCluster.tsx. The proper systemic
 * fix is to reconcile the wire casing at the boundary (store/hook or Rust).
 */
export function badgeStyleFor(status: PeerStatus | string | null | undefined): BadgeStyle {
  switch (String(status ?? '').toLowerCase()) {
    case 'online':
      return {
        dot: 'bg-spectyn-success',
        text: 'text-spectyn-success',
        label: 'Online',
      };
    case 'unhealthy':
      return {
        dot: 'bg-spectyn-warning',
        text: 'text-spectyn-warning',
        label: 'Unhealthy',
      };
    default:
      return {
        dot: 'bg-spectyn-muted',
        text: 'text-spectyn-muted',
        label: 'Unknown',
      };
  }
}

export default function PeerBadge({ status, className }: PeerBadgeProps) {
  const s = badgeStyleFor(status);
  return (
    <span
      className={`inline-flex items-center gap-1.5 text-xs ${s.text} ${className ?? ''}`}
      role="status"
      aria-label={`peer status: ${s.label}`}
    >
      <span className={`w-2 h-2 rounded-full ${s.dot}`} aria-hidden="true" />
      {s.label}
    </span>
  );
}
