import { useState, useEffect, useCallback } from "react";
import { safeInvoke as invoke } from "../../lib/tauri-compat";

interface Agent {
  id: string;
  name: string;
  role: string;
  description: string;
  online: boolean;
}

interface ClusterWorkerRecord {
  id?: string;
  name?: string;
  role?: string;
  description?: string;
  online?: boolean;
  status?: string;
  [key: string]: unknown;
}

const AGENTS: Agent[] = [
  {
    id: "master",
    name: "Master",
    role: "協調者",
    description: "負責任務分解、Agent 調度與結果整合。所有使用者指令的入口。",
    online: true,
  },
  {
    id: "coder",
    name: "Coder",
    role: "開發者",
    description: "執行程式碼生成、修改與重構。支援多語言與框架。",
    online: true,
  },
  {
    id: "browser",
    name: "Browser",
    role: "瀏覽器",
    description: "網頁瀏覽、資料擷取、表單填寫與截圖。Headless Chrome 驅動。",
    online: false,
  },
  {
    id: "reviewer",
    name: "Reviewer",
    role: "審查員",
    description: "程式碼審查、安全掃描與品質分析。確保輸出符合標準。",
    online: true,
  },
  {
    id: "analyst",
    name: "Analyst",
    role: "分析師",
    description: "資料分析、趨勢預測與報告生成。處理結構化與非結構化資料。",
    online: false,
  },
];

function parseWorkerToAgent(record: ClusterWorkerRecord, index: number): Agent {
  const id = typeof record.id === "string" ? record.id : `worker-${index}`;
  const name = typeof record.name === "string" ? record.name : id;
  const role = typeof record.role === "string" ? record.role : "Worker";
  const description =
    typeof record.description === "string" ? record.description : `Cluster worker: ${name}`;
  const online =
    typeof record.online === "boolean"
      ? record.online
      : typeof record.status === "string"
        ? record.status === "online" || record.status === "running"
        : false;
  return { id, name, role, description, online };
}

function mergeAgents(fallback: Agent[], remote: Agent[]): Agent[] {
  const merged = new Map<string, Agent>();
  for (const a of fallback) {
    merged.set(a.id, a);
  }
  for (const a of remote) {
    const existing = merged.get(a.id);
    if (existing) {
      merged.set(a.id, { ...existing, online: a.online });
    } else {
      merged.set(a.id, a);
    }
  }
  return Array.from(merged.values());
}

export default function AgentsPanel() {
  const [agents, setAgents] = useState<Agent[]>(AGENTS);
  const [fetchLoading, setFetchLoading] = useState(true);
  const [fetchError, setFetchError] = useState<string | null>(null);
  const [offline, setOffline] = useState(false);

  const [showModal, setShowModal] = useState(false);
  const [selectedAgent, setSelectedAgent] = useState<string>("master");
  const [prompt, setPrompt] = useState("");
  const [loading, setLoading] = useState(false);
  const [result, setResult] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const fetchWorkers = useCallback(async () => {
    setFetchLoading(true);
    setFetchError(null);
    try {
      const raw = await invoke<unknown>("get_cluster_workers");
      const arr: ClusterWorkerRecord[] = [];
      if (Array.isArray(raw)) {
        for (const item of raw) {
          if (item !== null && typeof item === "object") {
            arr.push(item as ClusterWorkerRecord);
          }
        }
      } else if (raw !== null && typeof raw === "object") {
        const obj = raw as Record<string, unknown>;
        const workers = obj["workers"] ?? obj["agents"] ?? obj["data"];
        if (Array.isArray(workers)) {
          for (const item of workers) {
            if (item !== null && typeof item === "object") {
              arr.push(item as ClusterWorkerRecord);
            }
          }
        }
      }
      const remoteAgents = arr.map((r, i) => parseWorkerToAgent(r, i));
      setAgents(mergeAgents(AGENTS, remoteAgents));
      setOffline(false);
    } catch (e) {
      setFetchError(String(e));
      setAgents(AGENTS);
      setOffline(true);
    } finally {
      setFetchLoading(false);
    }
  }, []);

  useEffect(() => {
    void fetchWorkers();
  }, [fetchWorkers]);

  const openModal = (agentId: string) => {
    setSelectedAgent(agentId);
    setPrompt("");
    setResult(null);
    setError(null);
    setShowModal(true);
  };

  const runAgent = async () => {
    if (!prompt.trim() || loading) return;
    setLoading(true);
    setError(null);
    setResult(null);
    try {
      const response = await invoke<string>("run_agent", {
        name: selectedAgent,
        input: prompt.trim(),
      });
      setResult(response);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  return (
    <div>
      <div className="flex items-center justify-between mb-6">
        <h1 className="text-2xl font-bold">
          Agent 監控{offline && <span className="text-sm font-normal text-phantom-muted ml-2">(離線模式)</span>}
        </h1>
        <div className="flex items-center gap-3 text-sm text-phantom-muted">
          <span className="flex items-center gap-1">
            <span className="w-2 h-2 rounded-full bg-phantom-success inline-block" />
            在線 {agents.filter((a) => a.online).length}
          </span>
          <span className="flex items-center gap-1">
            <span className="w-2 h-2 rounded-full bg-phantom-danger inline-block" />
            離線 {agents.filter((a) => !a.online).length}
          </span>
        </div>
      </div>

      {/* Error banner */}
      {fetchError && (
        <div className="bg-phantom-danger/20 border border-phantom-danger rounded p-3 mb-4 flex items-center justify-between text-sm">
          <span title={fetchError ?? undefined}>無法連接本機 daemon — 確認 phantom serve 已啟動（行動裝置可改用「集群派送」或從 Mac 匯入設定）</span>
          <button
            onClick={() => void fetchWorkers()}
            className="ml-4 px-3 py-1 rounded text-xs font-medium bg-phantom-danger/30 hover:bg-phantom-danger/50"
          >
            重試
          </button>
        </div>
      )}

      {/* Loading state */}
      {fetchLoading ? (
        <div className="flex items-center justify-center py-16">
          <div className="w-6 h-6 border-2 border-phantom-primary border-t-transparent rounded-full animate-spin" />
          <span className="ml-3 text-phantom-muted text-sm">載入 Agent 資訊...</span>
        </div>
      ) : (
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
          {agents.map((agent) => (
            <div
              key={agent.id}
              className="bg-phantom-card border border-phantom-border rounded-lg p-4 flex flex-col"
            >
              <div className="flex items-center justify-between mb-3">
                <div className="flex items-center gap-2">
                  <span
                    className={`w-2.5 h-2.5 rounded-full ${
                      agent.online ? "bg-phantom-success" : "bg-phantom-danger"
                    }`}
                  />
                  <h3 className="font-semibold text-lg">{agent.name}</h3>
                </div>
                <span className="text-xs bg-phantom-primary/20 text-phantom-primary px-2 py-0.5 rounded">
                  {agent.role}
                </span>
              </div>
              <p className="text-sm text-phantom-muted flex-1 mb-4">{agent.description}</p>
              <div className="flex items-center justify-between">
                <span
                  className={`text-xs font-medium ${
                    agent.online ? "text-phantom-success" : "text-phantom-danger"
                  }`}
                >
                  {agent.online ? "運行中" : "離線"}
                </span>
                <button
                  onClick={() => openModal(agent.id)}
                  disabled={!agent.online}
                  className="bg-phantom-primary text-phantom-bg px-3 py-1.5 rounded text-xs font-medium hover:opacity-90 disabled:opacity-30 disabled:cursor-not-allowed"
                >
                  執行 Agent
                </button>
              </div>
            </div>
          ))}
        </div>
      )}

      {/* Modal */}
      {showModal && (
        <div className="fixed inset-0 bg-black/60 flex items-center justify-center z-50 p-4">
          <div className="bg-phantom-card border border-phantom-border rounded-lg w-full max-w-lg p-6">
            <div className="flex items-center justify-between mb-4">
              <h2 className="text-lg font-bold">
                執行 Agent — {agents.find((a) => a.id === selectedAgent)?.name}
              </h2>
              <button
                onClick={() => setShowModal(false)}
                className="text-phantom-muted hover:text-phantom-text text-xl leading-none"
              >
                &times;
              </button>
            </div>

            <textarea
              value={prompt}
              onChange={(e) => setPrompt(e.target.value)}
              placeholder="輸入指令..."
              rows={4}
              className="w-full bg-phantom-bg border border-phantom-border rounded px-3 py-2 text-sm text-phantom-text placeholder-phantom-muted focus:outline-none focus:border-phantom-primary mb-3 resize-none"
            />

            {error && (
              <div className="bg-phantom-danger/20 border border-phantom-danger rounded p-3 mb-3 text-sm">
                {error}
              </div>
            )}

            {result && (
              <div className="bg-phantom-bg border border-phantom-border rounded p-3 mb-3 text-sm max-h-48 overflow-y-auto whitespace-pre-wrap">
                {result}
              </div>
            )}

            <div className="flex justify-end gap-2">
              <button
                onClick={() => setShowModal(false)}
                className="px-4 py-2 rounded text-sm text-phantom-muted hover:text-phantom-text border border-phantom-border"
              >
                關閉
              </button>
              <button
                onClick={() => void runAgent()}
                disabled={loading || !prompt.trim()}
                className="bg-phantom-primary text-phantom-bg px-4 py-2 rounded text-sm font-medium hover:opacity-90 disabled:opacity-50"
              >
                {loading ? "執行中..." : "執行"}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
