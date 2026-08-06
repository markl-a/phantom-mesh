import { useState, useEffect } from "react";
import UptimeBadge from "./UptimeBadge";

// ─── Types ────────────────────────────────────────────────────────────────────

interface RpcPeer {
  name: string;
  host: string;
  online: boolean;
  active_tasks: number;
}

interface RpcSelf {
  name: string;
  version?: string;
}

interface RpcPeersResponse {
  peers?: RpcPeer[];
  self?: RpcSelf;
  // legacy fallbacks
  this_node?: { name?: string; uptime?: string | number };
  name?: string;
  uptime?: string | number;
}

// ─── Component ────────────────────────────────────────────────────────────────

export default function NodeInfoPanel() {
  // Mesh peers from /rpc/peers
  const [peers, setPeers] = useState<RpcPeer[]>([]);
  const [peersLoading, setPeersLoading] = useState(true);
  const [selfNode, setSelfNode] = useState<RpcSelf | null>(null);

  // Poll /rpc/peers every 5 seconds
  useEffect(() => {
    const fetchPeers = async () => {
      try {
        const res = await fetch("http://localhost:7878/rpc/peers");
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
        const data = await res.json() as RpcPeersResponse;

        // Support both { peers: [...] } and flat array responses
        const peerList: RpcPeer[] = Array.isArray(data)
          ? (data as RpcPeer[])
          : (data.peers ?? []);
        setPeers(peerList);

        // Resolve "self" info from new or legacy shapes
        if (data.self) {
          setSelfNode(data.self);
        } else if (data.this_node?.name) {
          setSelfNode({ name: data.this_node.name });
        } else if (data.name) {
          setSelfNode({ name: data.name });
        }
      } catch {
        // Silently fail — daemon may not be running
      } finally {
        setPeersLoading(false);
      }
    };

    void fetchPeers();
    const interval = setInterval(() => void fetchPeers(), 5_000);
    return () => clearInterval(interval);
  }, []);

  // ── Render ──────────────────────────────────────────────────────────────────

  return (
    <div className="flex flex-col gap-4">
      {/* ── Self node (always first) ── */}
      <div>
        <p className="text-xs font-semibold text-spectyn-muted uppercase tracking-wider mb-2">
          本節點
        </p>
        {selfNode ? (
          <div className="bg-spectyn-bg border border-spectyn-border rounded-lg p-3">
            <div className="flex items-center justify-between">
              <span className="text-sm font-medium text-spectyn-text">{selfNode.name}</span>
              <span className="flex items-center gap-1 text-xs text-spectyn-success">
                <span className="w-1.5 h-1.5 rounded-full bg-spectyn-success" />
                本機
              </span>
            </div>
            <div className="flex items-center justify-between mt-1">
              {selfNode.version && (
                <p className="text-xs text-spectyn-muted">v{selfNode.version}</p>
              )}
              <UptimeBadge />
            </div>
          </div>
        ) : peersLoading ? (
          <div className="flex items-center gap-2 py-2">
            <div className="w-3 h-3 border-2 border-spectyn-primary border-t-transparent rounded-full animate-spin" />
            <span className="text-spectyn-muted text-xs">連線中...</span>
          </div>
        ) : (
          <p className="text-spectyn-muted text-xs py-1">— Daemon 未連線</p>
        )}
      </div>

      {/* ── Peer nodes ── */}
      <div>
        <p className="text-xs font-semibold text-spectyn-muted uppercase tracking-wider mb-2">
          Mesh 節點
        </p>
        {peersLoading ? (
          <div className="flex items-center gap-2 py-2">
            <div className="w-3 h-3 border-2 border-spectyn-primary border-t-transparent rounded-full animate-spin" />
            <span className="text-spectyn-muted text-xs">連線中...</span>
          </div>
        ) : peers.length === 0 ? (
          <p className="text-spectyn-muted text-xs text-center py-3">
            無對等節點
          </p>
        ) : (
          <div className="flex flex-col gap-2">
            {peers.map((peer, i) => (
              <div
                key={i}
                className="bg-spectyn-bg border border-spectyn-border rounded-lg p-3"
              >
                <div className="flex items-center justify-between mb-1">
                  <span className="text-sm font-medium text-spectyn-text truncate">{peer.name}</span>
                  <span
                    className={`flex items-center gap-1 text-xs flex-shrink-0 ${
                      peer.online ? "text-spectyn-success" : "text-red-400"
                    }`}
                  >
                    <span
                      className={`w-1.5 h-1.5 rounded-full ${
                        peer.online ? "bg-spectyn-success" : "bg-red-400"
                      }`}
                    />
                    {peer.online ? "online" : "offline"}
                  </span>
                </div>
                <div className="flex items-center justify-between text-xs text-spectyn-muted">
                  <span className="truncate">{peer.host}</span>
                  <span
                    className={`flex-shrink-0 ml-2 ${
                      peer.active_tasks > 0 ? "text-spectyn-primary" : "text-spectyn-muted"
                    }`}
                  >
                    {peer.active_tasks} 任務
                  </span>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
