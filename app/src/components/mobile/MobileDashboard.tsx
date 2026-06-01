import { useEffect, useState } from "react";
import { safeInvoke as invoke } from "../../lib/tauri-compat";
import { Wifi, WifiOff, RefreshCw, Activity } from "lucide-react";
import HabitChipQuickCapture from "./HabitChipQuickCapture";

interface Peer {
  name: string;
  url: string;
  online: boolean;
  uptime_secs?: number;
  capabilities?: string[];
  active_tasks?: number;
}

interface Provider {
  name: string;
  display_name?: string;
  is_available: boolean;
  health: string;
}

function fmtUptime(s?: number): string {
  if (!s || s < 0) return "—";
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m`;
  return `${s}s`;
}

export default function MobileDashboard() {
  const [peers, setPeers] = useState<Peer[]>([]);
  const [providers, setProviders] = useState<Provider[]>([]);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);

  const loadAll = async () => {
    try {
      const [peersResp, provResp] = await Promise.allSettled([
        invoke<{ peers?: Peer[] }>("get_peers"),
        invoke<{ providers?: Provider[] }>("get_provider_health"),
      ]);
      if (peersResp.status === "fulfilled") {
        setPeers(peersResp.value?.peers || []);
      }
      if (provResp.status === "fulfilled") {
        setProviders(provResp.value?.providers || []);
      }
    } finally {
      setLoading(false);
      setRefreshing(false);
    }
  };

  useEffect(() => { loadAll(); }, []);

  const refresh = async () => {
    setRefreshing(true);
    await loadAll();
  };

  const onlineCount = peers.filter(p => p.online).length;

  return (
    <div className="flex flex-col h-full overflow-y-auto">
      {/* Refresh banner */}
      <div className="px-4 py-3 flex items-center justify-between border-b border-phantom-border">
        <div className="flex items-center gap-2 text-sm">
          <Activity size={16} className="text-phantom-primary" />
          <span className="text-phantom-text">{onlineCount}/{peers.length} 節點上線</span>
        </div>
        <button
          onClick={refresh}
          className="text-phantom-muted hover:text-phantom-text p-2 -m-2"
          aria-label="重新整理"
        >
          <RefreshCw size={18} className={refreshing ? "animate-spin" : ""} />
        </button>
      </div>

      <div className="p-3 space-y-4">
        {/* SPEC-22 in-app habit quick-capture */}
        <HabitChipQuickCapture />

        {/* Providers */}
        <section>
          <h2 className="text-xs font-semibold text-phantom-muted uppercase tracking-wide px-1 mb-2">
            LLM Providers
          </h2>
          {providers.length === 0 ? (
            <div className="bg-phantom-card border border-phantom-border rounded-lg p-4 text-sm text-phantom-muted text-center">
              尚未配置 provider
            </div>
          ) : (
            <div className="space-y-2">
              {providers.map((p) => (
                <div key={p.name}
                  className="flex items-center justify-between bg-phantom-card border border-phantom-border rounded-lg px-3 py-2.5"
                >
                  <div className="flex items-center gap-2">
                    <div className={`w-2 h-2 rounded-full ${
                      p.is_available ? "bg-phantom-success" : "bg-phantom-danger"
                    }`} />
                    <span className="text-sm font-medium text-phantom-text capitalize">
                      {p.display_name || p.name}
                    </span>
                  </div>
                  <span className={`text-xs ${
                    p.is_available ? "text-phantom-success" : "text-phantom-muted"
                  }`}>
                    {p.health}
                  </span>
                </div>
              ))}
            </div>
          )}
        </section>

        {/* Peers */}
        <section>
          <h2 className="text-xs font-semibold text-phantom-muted uppercase tracking-wide px-1 mb-2">
            Cluster Nodes
          </h2>
          {loading ? (
            <div className="bg-phantom-card border border-phantom-border rounded-lg p-4 text-sm text-phantom-muted text-center">
              載入中…
            </div>
          ) : peers.length === 0 ? (
            <div className="bg-phantom-card border border-phantom-border rounded-lg p-4 text-sm text-phantom-muted text-center">
              還沒連到其他節點
            </div>
          ) : (
            <div className="space-y-2">
              {peers.map((p) => (
                <div key={p.url}
                  className="bg-phantom-card border border-phantom-border rounded-lg px-3 py-2.5"
                >
                  <div className="flex items-center justify-between">
                    <div className="flex items-center gap-2">
                      {p.online ? (
                        <Wifi size={16} className="text-phantom-success" />
                      ) : (
                        <WifiOff size={16} className="text-phantom-muted" />
                      )}
                      <span className="text-sm font-medium text-phantom-text">{p.name}</span>
                    </div>
                    <span className="text-xs text-phantom-muted">{fmtUptime(p.uptime_secs)}</span>
                  </div>
                  <div className="mt-1 text-[11px] text-phantom-muted truncate">{p.url}</div>
                  {p.capabilities && p.capabilities.length > 0 && (
                    <div className="mt-1.5 flex flex-wrap gap-1">
                      {p.capabilities.slice(0, 4).map((c) => (
                        <span key={c}
                          className="text-[10px] px-1.5 py-0.5 rounded bg-phantom-bg border border-phantom-border text-phantom-muted"
                        >{c}</span>
                      ))}
                    </div>
                  )}
                </div>
              ))}
            </div>
          )}
        </section>
      </div>
    </div>
  );
}
