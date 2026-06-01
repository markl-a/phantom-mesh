// SPEC-31 mesh-peer-list — route /settings/cluster; commands_used: get_cluster_peers (via useClusterPeers)
//
// Mobile cluster / peer list. Read-only mesh health view mirrored from the macOS
// S7 ClusterStatusDashboard, but reflowed for touch + safe-area + Dynamic Type
// per the SPEC-31 HIG. Reuses the F101 `useClusterPeers` hook — no new backend.
// Peer rows are rendered inline (status dot + id + caps + relative last-seen);
// PeerBadge is intentionally NOT imported (out of scope for this screen).
// RTT / RPC-count columns are not in the PeerSummary wire shape, so they are
// omitted rather than faked.

import { Network, RefreshCw, Plus } from "lucide-react";
import { useClusterPeers } from "../hooks/useClusterPeers";
import { useHaptics } from "../lib/useHaptics";

interface MeshPeerListProps {
  /** Optional callback for the sticky "Add peer" CTA. Router wiring is out of scope. */
  onAddPeer?: () => void;
}

/** Format a unix-seconds timestamp as a short bilingual relative time. */
function relTime(unixSecs: number): string {
  if (!unixSecs) return "—";
  const secs = Math.max(0, Math.floor(Date.now() / 1000 - unixSecs));
  if (secs < 60) return `${secs} 秒前`;
  if (secs < 3600) return `${Math.floor(secs / 60)} 分前`;
  if (secs < 86400) return `${Math.floor(secs / 3600)} 小時前`;
  return `${Math.floor(secs / 86400)} 天前`;
}

export default function MeshPeerList({ onAddPeer }: MeshPeerListProps) {
  const { peers, status, error, lastSyncMs, thisDeviceId, refresh } = useClusterPeers();
  const { impact } = useHaptics();

  // Overall mesh health (PeerStatus = 'Online' | 'Unhealthy' | 'Unknown'):
  //   green — at least one Online peer and none Unhealthy
  //   amber — any Unhealthy, OR peers present but none confirmed Online (all Unknown)
  //   gray  — isolated (no peers)
  const anyUnhealthy = peers.some((p) => p.status === "Unhealthy");
  const anyOnline = peers.some((p) => p.status === "Online");
  const health =
    peers.length === 0
      ? "gray"
      : anyUnhealthy || !anyOnline
        ? "amber"
        : "green";
  const healthDot =
    health === "green"
      ? "bg-phantom-success"
      : health === "amber"
        ? "bg-phantom-warning"
        : "bg-phantom-muted";
  const healthLabel =
    health === "green"
      ? "健康 Healthy"
      : health === "amber"
        ? "降級 Degraded"
        : "孤立 Isolated";

  const isLoading = status === "loading";

  const handleRefresh = () => {
    void refresh();
  };

  const handleAddPeer = () => {
    impact("medium");
    onAddPeer?.();
  };

  // PeerStatus union has no "Healthy": Online→green, Unhealthy→red, Unknown→muted.
  const dotColor = (peerStatus: string) =>
    peerStatus === "Online"
      ? "bg-phantom-success"
      : peerStatus === "Unhealthy"
        ? "bg-phantom-danger"
        : "bg-phantom-muted";

  return (
    <div
      data-testid="mesh-peer-list"
      className="min-h-screen flex flex-col bg-phantom-bg text-phantom-text
        pt-[env(safe-area-inset-top)]
        pl-[env(safe-area-inset-left)] pr-[env(safe-area-inset-right)]"
    >
      {/* Header: health summary + refresh */}
      <header className="flex items-center gap-3 px-4 pt-4 pb-3 border-b border-phantom-border">
        <div className="w-10 h-10 rounded-lg bg-phantom-primary/15 flex items-center justify-center flex-shrink-0">
          <Network size={20} className="text-phantom-primary" aria-hidden="true" />
        </div>
        <div className="flex-1 min-w-0">
          <h1 className="text-lg font-bold text-phantom-text">叢集對等節點 Mesh peers</h1>
          <p className="text-sm text-phantom-muted flex items-center gap-1.5 mt-0.5">
            <span
              className={`inline-block w-2.5 h-2.5 rounded-full ${healthDot} flex-shrink-0`}
              aria-hidden="true"
            />
            <span className="truncate">
              {healthLabel}
              <span className="mx-1.5 text-phantom-muted">·</span>
              {thisDeviceId || "本機（未命名）"}
            </span>
          </p>
        </div>
        <button
          type="button"
          onClick={handleRefresh}
          disabled={isLoading}
          aria-label="重新整理對等節點清單 Refresh peer list"
          className="flex items-center gap-1.5 min-h-[44px] px-3 rounded-lg text-base
            bg-phantom-card border border-phantom-border text-phantom-text
            hover:border-phantom-primary/40 transition motion-reduce:transition-none
            disabled:opacity-60 flex-shrink-0"
        >
          <RefreshCw
            size={16}
            aria-hidden="true"
            className={isLoading ? "animate-spin motion-reduce:animate-none" : ""}
          />
          <span className="hidden sm:inline">重新整理</span>
        </button>
      </header>

      {/* Scrollable body */}
      <main className="flex-1 overflow-y-auto px-4 py-4 space-y-3">
        {error && (
          <div
            role="alert"
            className="bg-phantom-danger/10 border border-phantom-danger/40 rounded-lg p-3 text-base text-phantom-danger"
          >
            無法取得對等節點：{error}
            <span className="block text-sm opacity-80 mt-1">Failed to load peers.</span>
          </div>
        )}

        {/* Loading (first load, no data yet) */}
        {isLoading && peers.length === 0 && !error && (
          <div
            role="status"
            className="flex items-center justify-center gap-2 min-h-[44px] text-base text-phantom-muted py-8"
          >
            <RefreshCw
              size={18}
              aria-hidden="true"
              className="animate-spin motion-reduce:animate-none"
            />
            載入中… Loading…
          </div>
        )}

        {/* Empty / isolated */}
        {!isLoading && !error && peers.length === 0 && (
          <div className="bg-phantom-card border border-phantom-border rounded-lg p-6 text-center">
            <p className="text-base text-phantom-text">孤立模式 Isolated</p>
            <p className="text-sm text-phantom-muted mt-1.5">
              尚未連接任何對等節點。透過下方按鈕加入叢集。
              <span className="block mt-0.5">No peers connected yet. Add one below.</span>
            </p>
          </div>
        )}

        {/* Peer rows */}
        {peers.length > 0 && (
          <ul className="space-y-2" aria-label="對等節點清單 Peer list">
            {peers.map((p) => (
              <li
                key={p.peer_id}
                className="flex items-center gap-3 min-h-[44px] px-3 py-2.5 rounded-lg
                  bg-phantom-card border border-phantom-border"
              >
                <span
                  className={`inline-block w-3 h-3 rounded-full ${dotColor(p.status)} flex-shrink-0`}
                  aria-hidden="true"
                />
                <div className="flex-1 min-w-0">
                  <p className="text-base text-phantom-text truncate">
                    {p.display_name || p.peer_id}
                  </p>
                  {(p.caps ?? []).length > 0 && (
                    <p className="flex flex-wrap gap-1 mt-1">
                      {(p.caps ?? []).slice(0, 4).map((c) => (
                        <span
                          key={c}
                          className="text-xs px-1.5 py-0.5 rounded bg-phantom-primary/10 text-phantom-primary"
                        >
                          {c}
                        </span>
                      ))}
                    </p>
                  )}
                </div>
                <span className="text-sm text-phantom-muted flex-shrink-0 text-right">
                  {relTime(p.last_seen_unix)}
                </span>
              </li>
            ))}
          </ul>
        )}

        {/* Last-sync footer line */}
        <p className="text-sm text-phantom-muted pt-1">
          {lastSyncMs > 0
            ? `最後同步 Last sync：${relTime(Math.floor(lastSyncMs / 1000))}`
            : "尚未同步 Not synced yet"}
        </p>
      </main>

      {/* Sticky bottom CTA (reachability) */}
      <footer className="sticky bottom-0 px-4 pt-3 pb-[max(env(safe-area-inset-bottom),0.75rem)] border-t border-phantom-border bg-phantom-bg">
        <button
          type="button"
          onClick={handleAddPeer}
          aria-label="新增對等節點 Add peer"
          className="w-full min-h-[48px] flex items-center justify-center gap-2 rounded-xl
            bg-phantom-primary text-phantom-bg text-base font-semibold
            hover:opacity-90 transition motion-reduce:transition-none"
        >
          <Plus size={18} aria-hidden="true" />
          新增對等節點 / Add peer
        </button>
      </footer>
    </div>
  );
}
