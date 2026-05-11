import { useState, useEffect, useCallback } from "react";
import { safeInvoke as invoke } from "../../lib/tauri-compat";

interface Provider {
  name: string;
  tier: number;
  tierLabel: string;
  status: "online" | "offline";
  models: number;
  description: string;
  apiKey: string | null;
}

interface ProviderHealthRecord {
  name?: string;
  provider?: string;
  provider_name?: string;
  tier?: number;
  tierLabel?: string;
  tier_label?: string;
  status?: string;
  healthy?: boolean;
  online?: boolean;
  is_available?: boolean;
  models?: number;
  model_count?: number;
  description?: string;
  apiKey?: string | null;
  api_key?: string | null;
  has_key?: boolean;
  [key: string]: unknown;
}

const MOCK_PROVIDERS: Provider[] = [
  {
    name: "Ollama",
    tier: 1,
    tierLabel: "Tier 1 — 本地",
    status: "online",
    models: 3,
    description: "本地推理，零延遲，完全隱私。支援 Llama 3、Mistral、Gemma 等。",
    apiKey: null,
  },
  {
    name: "Groq",
    tier: 2,
    tierLabel: "Tier 2 — 免費",
    status: "online",
    models: 4,
    description: "免費雲端推理，超低延遲。每日配額限制，適合輕量任務。",
    apiKey: "gsk_****************************a3Fm",
  },
  {
    name: "Claude",
    tier: 3,
    tierLabel: "Tier 3 — 訂閱",
    status: "online",
    models: 3,
    description: "Anthropic Claude 系列。訂閱制，高品質推理與長上下文。",
    apiKey: "sk-ant-****************************xQ7p",
  },
  {
    name: "OpenAI",
    tier: 4,
    tierLabel: "Tier 4 — 按量",
    status: "offline",
    models: 5,
    description: "GPT-4o、o1 系列。按量計費，成本最高但模型最多。",
    apiKey: "sk-****************************Tz9k",
  },
];

const TIER_LABELS: Record<number, string> = {
  1: "Tier 1 — 本地",
  2: "Tier 2 — 免費",
  3: "Tier 3 — 訂閱",
  4: "Tier 4 — 按量",
};

function parseHealthRecord(record: ProviderHealthRecord, index: number): Provider {
  const name =
    typeof record.name === "string"
      ? record.name
      : typeof record.provider === "string"
        ? record.provider
        : typeof record.provider_name === "string"
          ? record.provider_name
          : `Provider-${index + 1}`;

  const tier = typeof record.tier === "number" ? record.tier : index + 1;

  const tierLabel =
    typeof record.tierLabel === "string"
      ? record.tierLabel
      : typeof record.tier_label === "string"
        ? record.tier_label
        : TIER_LABELS[tier] ?? `Tier ${tier}`;

  let status: "online" | "offline";
  if (typeof record.status === "string") {
    status =
      record.status === "online" || record.status === "healthy" || record.status === "up"
        ? "online"
        : "offline";
  } else if (typeof record.healthy === "boolean") {
    status = record.healthy ? "online" : "offline";
  } else if (typeof record.online === "boolean") {
    status = record.online ? "online" : "offline";
  } else if (typeof record.is_available === "boolean") {
    status = record.is_available ? "online" : "offline";
  } else {
    status = "offline";
  }

  const models =
    typeof record.models === "number"
      ? record.models
      : typeof record.model_count === "number"
        ? record.model_count
        : 0;

  const description =
    typeof record.description === "string" ? record.description : `Provider: ${name}`;

  let apiKey: string | null = null;
  if (typeof record.apiKey === "string") {
    apiKey = record.apiKey;
  } else if (typeof record.api_key === "string") {
    apiKey = record.api_key;
  } else if (typeof record.has_key === "boolean" && record.has_key) {
    apiKey = "••••••••••••••••••••";
  }

  return { name, tier, tierLabel, status, models, description, apiKey };
}

export default function ProvidersPanel() {
  const [providers, setProviders] = useState<Provider[]>(MOCK_PROVIDERS);
  const [fetchLoading, setFetchLoading] = useState(true);
  const [fetchError, setFetchError] = useState<string | null>(null);
  const [offline, setOffline] = useState(false);
  const [visibleKeys, setVisibleKeys] = useState<Set<string>>(new Set());

  const fetchProviderHealth = useCallback(async () => {
    setFetchLoading(true);
    setFetchError(null);
    try {
      const raw = await invoke<unknown>("get_provider_health");
      const arr: ProviderHealthRecord[] = [];
      if (Array.isArray(raw)) {
        for (const item of raw) {
          if (item !== null && typeof item === "object") {
            arr.push(item as ProviderHealthRecord);
          }
        }
      } else if (raw !== null && typeof raw === "object") {
        const obj = raw as Record<string, unknown>;
        const inner = obj["providers"] ?? obj["health"] ?? obj["data"];
        if (Array.isArray(inner)) {
          for (const item of inner) {
            if (item !== null && typeof item === "object") {
              arr.push(item as ProviderHealthRecord);
            }
          }
        } else {
          // Object-keyed format: { "ollama": { status: "online", ... }, ... }
          for (const [providerName, value] of Object.entries(obj)) {
            if (value !== null && typeof value === "object") {
              arr.push({ name: providerName, ...(value as Record<string, unknown>) } as ProviderHealthRecord);
            }
          }
        }
      }
      const remoteProviders = arr.map((r, i) => parseHealthRecord(r, i));
      setProviders(remoteProviders.length > 0 ? remoteProviders : []);
      setOffline(false);
    } catch (e) {
      setFetchError(String(e));
      setProviders(MOCK_PROVIDERS);
      setOffline(true);
    } finally {
      setFetchLoading(false);
    }
  }, []);

  useEffect(() => {
    void fetchProviderHealth();
  }, [fetchProviderHealth]);

  const toggleKeyVisibility = (name: string) => {
    setVisibleKeys((prev) => {
      const next = new Set(prev);
      if (next.has(name)) {
        next.delete(name);
      } else {
        next.add(name);
      }
      return next;
    });
  };

  const tierColors: Record<number, string> = {
    1: "bg-phantom-success/20 text-phantom-success",
    2: "bg-phantom-primary/20 text-phantom-primary",
    3: "bg-purple-500/20 text-purple-400",
    4: "bg-phantom-warning/20 text-phantom-warning",
  };

  return (
    <div>
      <div className="flex items-center justify-between mb-6">
        <h1 className="text-2xl font-bold">
          API 金鑰管理{offline && <span className="text-sm font-normal text-phantom-muted ml-2">(離線模式)</span>}
        </h1>
        <div className="flex items-center gap-2">
          <button
            onClick={() => void fetchProviderHealth()}
            disabled={fetchLoading}
            className="border border-phantom-border text-phantom-muted px-3 py-2 rounded text-sm font-medium hover:text-phantom-text hover:border-phantom-primary/50 disabled:opacity-50"
          >
            {fetchLoading ? "重新整理中..." : "重新整理"}
          </button>
          <button className="bg-phantom-primary text-phantom-bg px-4 py-2 rounded text-sm font-medium hover:opacity-90">
            新增 Provider
          </button>
        </div>
      </div>

      {/* Error banner */}
      {fetchError && (
        <div className="bg-phantom-danger/20 border border-phantom-danger rounded p-3 mb-4 flex items-center justify-between text-sm">
          <span>無法連接 Daemon：{fetchError}</span>
          <button
            onClick={() => void fetchProviderHealth()}
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
          <span className="ml-3 text-phantom-muted text-sm">載入 Provider 資訊...</span>
        </div>
      ) : (
        <>
          {/* Provider Cards */}
          <div className="grid grid-cols-1 md:grid-cols-2 gap-4 mb-8">
            {providers.map((provider) => (
              <div
                key={provider.name}
                className="bg-phantom-card border border-phantom-border rounded-lg p-4"
              >
                <div className="flex items-center justify-between mb-3">
                  <div className="flex items-center gap-3">
                    <span
                      className={`w-2.5 h-2.5 rounded-full ${
                        provider.status === "online" ? "bg-phantom-success" : "bg-phantom-danger"
                      }`}
                    />
                    <h3 className="font-semibold text-lg">{provider.name}</h3>
                  </div>
                  <span className={`text-xs px-2 py-0.5 rounded font-medium ${tierColors[provider.tier] ?? ""}`}>
                    {provider.tierLabel}
                  </span>
                </div>

                <p className="text-sm text-phantom-muted mb-3">{provider.description}</p>

                <div className="flex items-center justify-between text-sm">
                  <span className="text-phantom-muted">
                    模型數: <span className="text-phantom-text font-medium">{provider.models}</span>
                  </span>
                  <span
                    className={`text-xs font-medium ${
                      provider.status === "online" ? "text-phantom-success" : "text-phantom-danger"
                    }`}
                  >
                    {provider.status === "online" ? "在線" : "離線"}
                  </span>
                </div>
              </div>
            ))}
          </div>

          {/* API Key Management */}
          <h2 className="text-lg font-bold mb-4">金鑰管理</h2>
          <div className="bg-phantom-card border border-phantom-border rounded-lg overflow-hidden">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b border-phantom-border">
                  <th className="text-left px-4 py-3 text-phantom-muted font-medium">Provider</th>
                  <th className="text-left px-4 py-3 text-phantom-muted font-medium">Tier</th>
                  <th className="text-left px-4 py-3 text-phantom-muted font-medium">API Key</th>
                  <th className="text-left px-4 py-3 text-phantom-muted font-medium">操作</th>
                </tr>
              </thead>
              <tbody>
                {providers.map((provider, i) => (
                  <tr
                    key={provider.name}
                    className={`border-b border-phantom-border last:border-0 ${
                      i % 2 === 1 ? "bg-phantom-bg/50" : ""
                    }`}
                  >
                    <td className="px-4 py-3 font-medium">{provider.name}</td>
                    <td className="px-4 py-3">
                      <span className={`text-xs px-2 py-0.5 rounded font-medium ${tierColors[provider.tier] ?? ""}`}>
                        Tier {provider.tier}
                      </span>
                    </td>
                    <td className="px-4 py-3 font-mono text-phantom-muted">
                      {provider.apiKey ? (
                        visibleKeys.has(provider.name) ? (
                          provider.apiKey
                        ) : (
                          "••••••••••••••••••••"
                        )
                      ) : (
                        <span className="text-phantom-muted/50 italic">不需要（本地）</span>
                      )}
                    </td>
                    <td className="px-4 py-3">
                      {provider.apiKey && (
                        <div className="flex gap-2">
                          <button
                            onClick={() => toggleKeyVisibility(provider.name)}
                            className="text-xs text-phantom-primary hover:underline"
                          >
                            {visibleKeys.has(provider.name) ? "隱藏" : "顯示"}
                          </button>
                          <button className="text-xs text-phantom-muted hover:text-phantom-text">
                            編輯
                          </button>
                        </div>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>

          {/* Tier Priority Info */}
          <div className="mt-6 bg-phantom-card border border-phantom-border rounded-lg p-4">
            <h3 className="text-sm font-medium mb-2">路由優先順序</h3>
            <p className="text-xs text-phantom-muted leading-relaxed">
              請求依 Tier 優先路由：Tier 1（本地） → Tier 2（免費雲端） → Tier 3（訂閱制） → Tier 4（按量計費）。
              僅在上層 Provider 不可用或模型不支援時，才會降級至下一層。
            </p>
          </div>
        </>
      )}
    </div>
  );
}
