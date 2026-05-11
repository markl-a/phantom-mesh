import type { ClusterNode } from '../stores/clusterStore';

const STATUS_CONFIG = {
  online: { color: 'bg-green-500', icon: '●', label: 'Online' },
  offline: { color: 'bg-gray-500', icon: '○', label: 'Offline' },
  suspected: { color: 'bg-yellow-500', icon: '◑', label: 'Suspected' },
} as const;

export function DeviceCard({ node }: { node: ClusterNode }) {
  const status = STATUS_CONFIG[node.status] || STATUS_CONFIG.offline;

  return (
    <div className="rounded-lg border border-white/10 bg-white/5 p-4 hover:bg-white/8 transition-colors">
      <div className="flex items-center gap-2 mb-2">
        <span className={`w-2.5 h-2.5 rounded-full ${status.color}`} />
        <span className="text-white font-medium text-sm">{node.name}</span>
        {node.role === 'coordinator' && (
          <span className="text-xs bg-indigo-600/30 text-indigo-300 px-1.5 py-0.5 rounded">
            Coordinator
          </span>
        )}
      </div>
      <div className="text-white/50 text-xs space-y-1">
        <div>CPU: {(node.cpuLoad * 100).toFixed(0)}% | Memory: {node.memoryPct.toFixed(0)}%</div>
        <div>Tasks: {node.activeTasks} | Uptime: {formatUptime(node.uptimeSecs)}</div>
        {node.capabilities.length > 0 && (
          <div className="flex gap-1 flex-wrap mt-1">
            {node.capabilities.slice(0, 4).map((cap) => (
              <span key={cap} className="bg-white/10 px-1.5 py-0.5 rounded text-[10px]">
                {cap}
              </span>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

function formatUptime(secs: number): string {
  const d = Math.floor(secs / 86400);
  const h = Math.floor((secs % 86400) / 3600);
  if (d > 0) return `${d}d ${h}h`;
  const m = Math.floor((secs % 3600) / 60);
  return h > 0 ? `${h}h ${m}m` : `${m}m`;
}

/** Global status bar */
export function StatusBar({ lastEvent }: { lastEvent?: string }) {
  return (
    <div className="h-6 bg-[#0a0a14] border-t border-white/10 flex items-center px-3 text-[11px] text-white/40">
      {lastEvent || 'Ready'}
    </div>
  );
}
