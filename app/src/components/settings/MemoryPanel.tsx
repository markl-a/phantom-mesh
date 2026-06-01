import { useState, useEffect, useCallback, useRef } from "react";
import { safeInvoke as invoke } from "../../lib/tauri-compat";

type NodeMemoryTab = "semantic" | "episodic" | "procedural" | "observational";

interface MemoryEntry {
  id: string;
  content: string;
  type: string;
  timestamp: string;
  relevance: number;
}

interface MemoryStats {
  totalEntries: number;
  compressionRatio: string;
  lastSync: string;
}

interface MemoryPageState {
  isOffline: boolean;
  loading: boolean;
  error: string | null;
  stats: MemoryStats;
  clusterEntries: MemoryEntry[];
  nodeEntries: Record<NodeMemoryTab, MemoryEntry[]>;
  subagentEntries: MemoryEntry[];
  searchLoading: boolean;
}

const NODE_TABS: { key: NodeMemoryTab; label: string; description: string }[] = [
  { key: "semantic", label: "語義記憶", description: "事實、知識與概念" },
  { key: "episodic", label: "情節記憶", description: "事件序列與經驗" },
  { key: "procedural", label: "程序記憶", description: "操作步驟與方法" },
  { key: "observational", label: "觀察記憶", description: "環境觀察與感知" },
];

const MOCK_CLUSTER_ENTRIES: MemoryEntry[] = [
  { id: "CM-001", content: "集群拓撲: node-a(Hub) + 7 Worker 節點", type: "topology", timestamp: "2026-03-21 10:00", relevance: 0.95 },
  { id: "CM-002", content: "4-Tier Provider 路由策略已生效", type: "config", timestamp: "2026-03-21 09:30", relevance: 0.88 },
  { id: "CM-003", content: "跨節點任務遷移協議 v2 啟用", type: "protocol", timestamp: "2026-03-20 22:15", relevance: 0.82 },
];

const MOCK_NODE_ENTRIES: Record<NodeMemoryTab, MemoryEntry[]> = {
  semantic: [
    { id: "NS-001", content: "Rust 工具鏈版本: 1.82.0, Tauri 2.x 框架", type: "knowledge", timestamp: "2026-03-21 08:00", relevance: 0.91 },
    { id: "NS-002", content: "使用者偏好: pluggable 架構, zh-TW 介面", type: "preference", timestamp: "2026-03-20 18:30", relevance: 0.87 },
    { id: "NS-003", content: "ClawtexOS 為分散式 AI Agent 集群作業系統", type: "definition", timestamp: "2026-03-19 14:00", relevance: 0.93 },
  ],
  episodic: [
    { id: "NE-001", content: "03/21 成功部署 phantom-desktop v0.1.0", type: "event", timestamp: "2026-03-21 09:00", relevance: 0.85 },
    { id: "NE-002", content: "03/20 完成 7 個 Tauri command 實作", type: "event", timestamp: "2026-03-20 23:45", relevance: 0.78 },
  ],
  procedural: [
    { id: "NP-001", content: "cargo tauri dev 啟動開發伺服器流程", type: "procedure", timestamp: "2026-03-21 07:00", relevance: 0.92 },
    { id: "NP-002", content: "git commit 前執行 cargo clippy + fmt", type: "procedure", timestamp: "2026-03-20 20:00", relevance: 0.89 },
  ],
  observational: [
    { id: "NO-001", content: "目前 CPU 使用率 23%, 記憶體 8.2GB/32GB", type: "system", timestamp: "2026-03-21 11:30", relevance: 0.76 },
    { id: "NO-002", content: "網路延遲: LAN 2ms, Iroh 45ms, Relay 120ms", type: "network", timestamp: "2026-03-21 11:28", relevance: 0.72 },
  ],
};

const MOCK_SUBAGENT_ENTRIES: MemoryEntry[] = [
  { id: "SA-001", content: "[Coder] TSK-002 重構進度: 3/5 模組完成", type: "task", timestamp: "2026-03-21 11:15", relevance: 0.94 },
  { id: "SA-002", content: "[Browser] TSK-001 擷取 42 筆定價資料", type: "task", timestamp: "2026-03-21 09:45", relevance: 0.80 },
  { id: "SA-003", content: "[Reviewer] PR #47 發現 2 個中風險問題", type: "task", timestamp: "2026-03-21 10:50", relevance: 0.88 },
];

const MOCK_STATS: MemoryStats = {
  totalEntries:
    MOCK_CLUSTER_ENTRIES.length +
    Object.values(MOCK_NODE_ENTRIES).flat().length +
    MOCK_SUBAGENT_ENTRIES.length,
  compressionRatio: "3.2:1",
  lastSync: "2026-03-21 11:30",
};

function parseMemoryEntry(raw: Record<string, unknown>, index: number): MemoryEntry {
  return {
    id: typeof raw["id"] === "string" ? raw["id"] : `MEM-${String(index).padStart(3, "0")}`,
    content: typeof raw["content"] === "string" ? raw["content"] : String(raw["content"] ?? ""),
    type: typeof raw["type"] === "string" ? raw["type"] : "unknown",
    timestamp: typeof raw["timestamp"] === "string" ? raw["timestamp"] : "",
    relevance: typeof raw["relevance"] === "number" ? raw["relevance"] : 0.5,
  };
}

function parseMemoryStats(raw: Record<string, unknown>): MemoryStats {
  return {
    totalEntries: typeof raw["totalEntries"] === "number"
      ? raw["totalEntries"]
      : typeof raw["total_entries"] === "number"
        ? raw["total_entries"]
        : MOCK_STATS.totalEntries,
    compressionRatio: typeof raw["compressionRatio"] === "string"
      ? raw["compressionRatio"]
      : typeof raw["compression_ratio"] === "string"
        ? raw["compression_ratio"]
        : MOCK_STATS.compressionRatio,
    lastSync: typeof raw["lastSync"] === "string"
      ? raw["lastSync"]
      : typeof raw["last_sync"] === "string"
        ? raw["last_sync"]
        : MOCK_STATS.lastSync,
  };
}

function MemoryTable({ entries }: { entries: MemoryEntry[] }) {
  return (
    <div className="bg-phantom-card border border-phantom-border rounded-lg overflow-hidden">
      <table className="w-full text-sm">
        <thead>
          <tr className="border-b border-phantom-border">
            <th className="text-left px-4 py-2 text-phantom-muted font-medium text-xs">ID</th>
            <th className="text-left px-4 py-2 text-phantom-muted font-medium text-xs">內容</th>
            <th className="text-left px-4 py-2 text-phantom-muted font-medium text-xs">類型</th>
            <th className="text-left px-4 py-2 text-phantom-muted font-medium text-xs">相關度</th>
            <th className="text-left px-4 py-2 text-phantom-muted font-medium text-xs">時間</th>
          </tr>
        </thead>
        <tbody>
          {entries.map((entry, i) => (
            <tr
              key={entry.id}
              className={`border-b border-phantom-border last:border-0 ${
                i % 2 === 1 ? "bg-phantom-bg/50" : ""
              }`}
            >
              <td className="px-4 py-2 font-mono text-phantom-muted text-xs">{entry.id}</td>
              <td className="px-4 py-2">{entry.content}</td>
              <td className="px-4 py-2">
                <span className="text-xs bg-phantom-primary/10 text-phantom-primary px-2 py-0.5 rounded">
                  {entry.type}
                </span>
              </td>
              <td className="px-4 py-2">
                <div className="flex items-center gap-2">
                  <div className="w-12 h-1.5 bg-phantom-border rounded-full overflow-hidden">
                    <div
                      className="h-full bg-phantom-primary rounded-full"
                      style={{ width: `${entry.relevance * 100}%` }}
                    />
                  </div>
                  <span className="text-xs text-phantom-muted">{(entry.relevance * 100).toFixed(0)}%</span>
                </div>
              </td>
              <td className="px-4 py-2 text-phantom-muted text-xs">{entry.timestamp}</td>
            </tr>
          ))}
          {entries.length === 0 && (
            <tr>
              <td colSpan={5} className="px-4 py-6 text-center text-phantom-muted">
                沒有符合條件的記憶條目
              </td>
            </tr>
          )}
        </tbody>
      </table>
    </div>
  );
}

export default function MemoryPanel() {
  const [nodeTab, setNodeTab] = useState<NodeMemoryTab>("semantic");
  const [searchQuery, setSearchQuery] = useState("");
  const [state, setState] = useState<MemoryPageState>({
    isOffline: false,
    loading: true,
    error: null,
    stats: MOCK_STATS,
    clusterEntries: MOCK_CLUSTER_ENTRIES,
    nodeEntries: MOCK_NODE_ENTRIES,
    subagentEntries: MOCK_SUBAGENT_ENTRIES,
    searchLoading: false,
  });
  const searchTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const fetchData = useCallback(async () => {
    setState((prev) => ({ ...prev, loading: true, error: null }));
    try {
      const [statsRaw, observationsRaw] = await Promise.all([
        invoke("get_memory_stats") as Promise<unknown>,
        invoke("get_memory_observations", { query: null, limit: 50 }) as Promise<unknown>,
      ]);

      const stats = parseMemoryStats(statsRaw as Record<string, unknown>);

      // Parse observations — could be flat array or structured object
      let clusterEntries = MOCK_CLUSTER_ENTRIES;
      let nodeEntries = MOCK_NODE_ENTRIES;
      let subagentEntries = MOCK_SUBAGENT_ENTRIES;

      if (Array.isArray(observationsRaw)) {
        const all = (observationsRaw as Record<string, unknown>[]).map(parseMemoryEntry);
        // Best-effort categorization: use 'layer' or 'category' field if present
        const cluster: MemoryEntry[] = [];
        const node: MemoryEntry[] = [];
        const subagent: MemoryEntry[] = [];
        for (let idx = 0; idx < (observationsRaw as Record<string, unknown>[]).length; idx++) {
          const raw = (observationsRaw as Record<string, unknown>[])[idx];
          const entry = parseMemoryEntry(raw, idx);
          const layer = String(raw["layer"] ?? raw["category"] ?? "node").toLowerCase();
          if (layer === "cluster") cluster.push(entry);
          else if (layer === "subagent" || layer === "sub_agent") subagent.push(entry);
          else node.push(entry);
        }
        if (cluster.length > 0) clusterEntries = cluster;
        if (node.length > 0) {
          // Distribute into node tabs by type if possible
          nodeEntries = { semantic: [], episodic: [], procedural: [], observational: [] };
          for (const e of node) {
            const t = e.type.toLowerCase();
            if (t.includes("event") || t.includes("episod")) nodeEntries.episodic.push(e);
            else if (t.includes("procedur") || t.includes("step")) nodeEntries.procedural.push(e);
            else if (t.includes("observ") || t.includes("system") || t.includes("network")) nodeEntries.observational.push(e);
            else nodeEntries.semantic.push(e);
          }
          // If all ended up in one bucket, put them in semantic as default
          const hasEntries = Object.values(nodeEntries).some((arr) => arr.length > 0);
          if (!hasEntries) {
            nodeEntries = MOCK_NODE_ENTRIES;
          }
        }
        if (subagent.length > 0) subagentEntries = subagent;
        // If the flat list had no categorization at all, just show all as cluster
        if (cluster.length === 0 && subagent.length === 0 && all.length > 0) {
          clusterEntries = all;
        }
      } else if (observationsRaw && typeof observationsRaw === "object") {
        const obj = observationsRaw as Record<string, unknown>;
        if (Array.isArray(obj["cluster"])) {
          clusterEntries = (obj["cluster"] as Record<string, unknown>[]).map(parseMemoryEntry);
        }
        if (Array.isArray(obj["subagent"])) {
          subagentEntries = (obj["subagent"] as Record<string, unknown>[]).map(parseMemoryEntry);
        }
        if (obj["node"] && typeof obj["node"] === "object" && !Array.isArray(obj["node"])) {
          const nodeObj = obj["node"] as Record<string, unknown>;
          nodeEntries = { semantic: [], episodic: [], procedural: [], observational: [] };
          for (const key of Object.keys(nodeObj)) {
            const tabKey = key as NodeMemoryTab;
            if (tabKey in nodeEntries && Array.isArray(nodeObj[key])) {
              nodeEntries[tabKey] = (nodeObj[key] as Record<string, unknown>[]).map(parseMemoryEntry);
            }
          }
        }
      }

      setState((prev) => ({
        ...prev,
        loading: false,
        isOffline: false,
        error: null,
        stats,
        clusterEntries,
        nodeEntries,
        subagentEntries,
      }));
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setState((prev) => ({
        ...prev,
        loading: false,
        isOffline: true,
        error: message,
        stats: MOCK_STATS,
        clusterEntries: MOCK_CLUSTER_ENTRIES,
        nodeEntries: MOCK_NODE_ENTRIES,
        subagentEntries: MOCK_SUBAGENT_ENTRIES,
      }));
    }
  }, []);

  useEffect(() => {
    fetchData();
  }, [fetchData]);

  // Debounced search via API
  const handleSearch = useCallback(
    (query: string) => {
      setSearchQuery(query);
      if (searchTimerRef.current) clearTimeout(searchTimerRef.current);

      if (!query.trim() || state.isOffline) {
        // Restore baseline data when query is cleared
        if (!query.trim() && !state.isOffline) {
          fetchData();
        }
        return;
      }

      searchTimerRef.current = setTimeout(async () => {
        setState((prev) => ({ ...prev, searchLoading: true }));
        try {
          const result = await invoke("search_memory", { query }) as unknown;
          if (Array.isArray(result)) {
            const entries = (result as Record<string, unknown>[]).map(parseMemoryEntry);
            // Put search results in cluster layer for simplicity
            setState((prev) => ({
              ...prev,
              searchLoading: false,
              clusterEntries: entries.length > 0 ? entries : prev.clusterEntries,
            }));
          } else {
            setState((prev) => ({ ...prev, searchLoading: false }));
          }
        } catch {
          // Silently fall back to client-side filtering on search error
          setState((prev) => ({ ...prev, searchLoading: false }));
        }
      }, 400);
    },
    [state.isOffline]
  );

  // Client-side filter (used in offline mode or as supplementary)
  const filterEntries = (entries: MemoryEntry[]) => {
    if (!searchQuery.trim()) return entries;
    if (!state.isOffline) return entries; // API handles search when online
    const q = searchQuery.toLowerCase();
    return entries.filter(
      (e) => e.content.toLowerCase().includes(q) || e.type.toLowerCase().includes(q)
    );
  };

  return (
    <div>
      <div className="flex items-center justify-between mb-6">
        <div className="flex items-center gap-3">
          <h1 className="text-2xl font-bold">記憶系統</h1>
          {state.isOffline && (
            <span className="text-xs px-2 py-0.5 rounded bg-phantom-warning/20 text-phantom-warning">
              (離線模式)
            </span>
          )}
        </div>
        {!state.loading && (
          <button
            onClick={fetchData}
            className="text-xs text-phantom-muted hover:text-phantom-text border border-phantom-border rounded px-2 py-1"
          >
            重新載入
          </button>
        )}
      </div>

      {/* Error Banner */}
      {state.error && (
        <div className="mb-4 bg-phantom-danger/10 border border-phantom-danger/30 rounded-lg px-4 py-3 flex items-center justify-between">
          <span className="text-sm text-phantom-danger">
            無法連線至記憶系統: {state.error}
          </span>
          <button
            onClick={fetchData}
            className="text-xs text-phantom-danger border border-phantom-danger/30 rounded px-2 py-1 hover:bg-phantom-danger/10"
          >
            重試
          </button>
        </div>
      )}

      {/* Loading State */}
      {state.loading && (
        <div className="flex items-center justify-center py-12">
          <div className="flex items-center gap-3 text-phantom-muted">
            <svg className="animate-spin h-5 w-5" viewBox="0 0 24 24" fill="none">
              <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
              <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
            </svg>
            <span className="text-sm">載入記憶資料中...</span>
          </div>
        </div>
      )}

      {!state.loading && (
        <>
          {/* Stats */}
          <div className="grid grid-cols-3 gap-4 mb-6">
            <div className="bg-phantom-card border border-phantom-border rounded-lg p-4">
              <p className="text-phantom-muted text-xs">總記憶條目</p>
              <p className="text-2xl font-bold mt-1">{state.stats.totalEntries}</p>
            </div>
            <div className="bg-phantom-card border border-phantom-border rounded-lg p-4">
              <p className="text-phantom-muted text-xs">壓縮比</p>
              <p className="text-2xl font-bold mt-1">{state.stats.compressionRatio}</p>
            </div>
            <div className="bg-phantom-card border border-phantom-border rounded-lg p-4">
              <p className="text-phantom-muted text-xs">最後同步</p>
              <p className="text-2xl font-bold mt-1 text-base">{state.stats.lastSync}</p>
            </div>
          </div>

          {/* Search */}
          <div className="mb-6 relative">
            <input
              type="text"
              value={searchQuery}
              onChange={(e) => handleSearch(e.target.value)}
              placeholder="搜尋記憶..."
              className="w-full bg-phantom-card border border-phantom-border rounded px-4 py-2.5 text-sm text-phantom-text placeholder-phantom-muted focus:outline-none focus:border-phantom-primary"
            />
            {state.searchLoading && (
              <div className="absolute right-3 top-1/2 -translate-y-1/2">
                <svg className="animate-spin h-4 w-4 text-phantom-muted" viewBox="0 0 24 24" fill="none">
                  <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
                  <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
                </svg>
              </div>
            )}
          </div>

          {/* Layer 1: Cluster Memory */}
          <section className="mb-6">
            <div className="flex items-center gap-2 mb-3">
              <span className="w-3 h-3 rounded-full bg-phantom-primary" />
              <h2 className="text-lg font-bold">Cluster Memory</h2>
              <span className="text-xs text-phantom-muted">— 全集群共享</span>
            </div>
            <MemoryTable entries={filterEntries(state.clusterEntries)} />
          </section>

          {/* Layer 2: Node Memory */}
          <section className="mb-6">
            <div className="flex items-center gap-2 mb-3">
              <span className="w-3 h-3 rounded-full bg-phantom-success" />
              <h2 className="text-lg font-bold">Node Memory</h2>
              <span className="text-xs text-phantom-muted">— 本機</span>
            </div>

            {/* Tabs */}
            <div className="flex gap-1 mb-3">
              {NODE_TABS.map((tab) => (
                <button
                  key={tab.key}
                  onClick={() => setNodeTab(tab.key)}
                  className={`px-3 py-1.5 rounded text-xs font-medium transition-colors ${
                    nodeTab === tab.key
                      ? "bg-phantom-primary text-phantom-bg"
                      : "bg-phantom-card border border-phantom-border text-phantom-muted hover:text-phantom-text"
                  }`}
                  title={tab.description}
                >
                  {tab.label}
                </button>
              ))}
            </div>

            <p className="text-xs text-phantom-muted mb-2">
              {NODE_TABS.find((t) => t.key === nodeTab)?.description}
            </p>

            <MemoryTable entries={filterEntries(state.nodeEntries[nodeTab])} />
          </section>

          {/* Layer 3: SubAgent Memory */}
          <section>
            <div className="flex items-center gap-2 mb-3">
              <span className="w-3 h-3 rounded-full bg-phantom-warning" />
              <h2 className="text-lg font-bold">SubAgent Memory</h2>
              <span className="text-xs text-phantom-muted">— 任務記憶</span>
            </div>
            <MemoryTable entries={filterEntries(state.subagentEntries)} />
          </section>
        </>
      )}
    </div>
  );
}
