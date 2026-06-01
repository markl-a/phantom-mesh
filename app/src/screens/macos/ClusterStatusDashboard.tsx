// SPEC-41 §10.7 — S7 cluster status dashboard (embedded in settings → Cluster tab).
//
// Read-only health view of the mesh: self node + health, peer list (status /
// caps / last-seen), and last-sync time. Reuses the F101 useClusterPeers hook
// + PeerBadge — no new backend. The wireframe's RTT / RPC-count / 24h-conflict
// columns are not in the PeerSummary wire shape yet, so they're omitted rather
// than faked; caps + last-seen are shown instead. The [+ Add peer] /
// [Reset cluster_secret] actions belong to the S12 mesh-add wizard (deferred).

import { Network, RefreshCw } from "lucide-react";
import { useClusterPeers } from "../../hooks/useClusterPeers";
import PeerBadge from "../../components/mobile/peerBadge";

function relTime(unix: number): string {
  if (!unix) return "—";
  const secs = Math.max(0, Math.floor(Date.now() / 1000 - unix));
  if (secs < 60) return `${secs} 秒前`;
  if (secs < 3600) return `${Math.floor(secs / 60)} 分前`;
  if (secs < 86400) return `${Math.floor(secs / 3600)} 小時前`;
  return `${Math.floor(secs / 86400)} 天前`;
}

export default function ClusterStatusDashboard() {
  const { peers, status, error, lastSyncMs, thisDeviceId, refresh } = useClusterPeers();

  // Overall health: amber if any peer Unhealthy, green if peers present and ok,
  // gray when isolated (no peers) or still loading.
  const anyUnhealthy = peers.some((p) => p.status === "Unhealthy");
  const health = peers.length === 0 ? "gray" : anyUnhealthy ? "amber" : "green";
  const healthColor = health === "green" ? "bg-phantom-success"
    : health === "amber" ? "bg-phantom-warning" : "bg-phantom-muted";
  const healthLabel = health === "green" ? "Green" : health === "amber" ? "Degraded" : "—";

  return (
    <div data-testid="cluster-status-dashboard" className="max-w-2xl space-y-5">
      <header className="flex items-center gap-3">
        <div className="w-9 h-9 rounded-lg bg-phantom-primary/15 flex items-center justify-center">
          <Network size={18} className="text-phantom-primary" />
        </div>
        <div className="flex-1">
          <h1 className="text-lg font-bold text-phantom-text">叢集狀態</h1>
          <p className="text-xs text-phantom-muted">
            本機：{thisDeviceId || "（未命名）"}
            <span className="mx-2">·</span>
            <span className={`inline-block w-2 h-2 rounded-full ${healthColor} align-middle mr-1`} />
            Health: {healthLabel}
          </p>
        </div>
        <button
          onClick={() => void refresh()}
          className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-sm bg-phantom-card border border-phantom-border text-phantom-text hover:border-phantom-primary/40 transition"
        >
          <RefreshCw size={14} className={status === "loading" ? "animate-spin" : ""} />
          重新整理
        </button>
      </header>

      {error && (
        <div className="bg-phantom-danger/10 border border-phantom-danger/40 rounded-lg p-3 text-sm text-phantom-danger">
          無法取得叢集狀態：{error}
        </div>
      )}

      {/* Peers / isolated edge case (§10.7) */}
      {peers.length === 0 ? (
        <div className="bg-phantom-card border border-phantom-border rounded-lg p-6 text-center">
          <p className="text-sm text-phantom-text">孤立模式（單機跑）</p>
          <p className="text-xs text-phantom-muted mt-1">
            尚未連接任何 peer。透過設定 → API 金鑰 / 同網段裝置加入叢集。
          </p>
        </div>
      ) : (
        <div className="bg-phantom-card border border-phantom-border rounded-lg p-4">
          <h3 className="text-sm font-medium text-phantom-text mb-3">
            Peers（{peers.length}）
          </h3>
          <div className="space-y-2">
            {peers.map((p) => (
              <div key={p.peer_id} className="flex items-center gap-3 px-3 py-2 rounded bg-phantom-bg border border-phantom-border">
                <PeerBadge status={p.status} />
                <span className="text-sm text-phantom-text flex-1 truncate">{p.display_name}</span>
                {(p.caps ?? []).slice(0, 3).map((c) => (
                  <span key={c} className="text-[10px] px-1.5 py-0.5 rounded bg-phantom-primary/10 text-phantom-primary">
                    {c}
                  </span>
                ))}
                <span className="text-[11px] text-phantom-muted w-16 text-right flex-shrink-0">
                  {relTime(p.last_seen_unix)}
                </span>
              </div>
            ))}
          </div>
        </div>
      )}

      <p className="text-[11px] text-phantom-muted">
        {lastSyncMs > 0 ? `最後同步：${relTime(Math.floor(lastSyncMs / 1000))}` : "尚未同步"}
      </p>
    </div>
  );
}
