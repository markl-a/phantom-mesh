import { useState, useEffect, useCallback } from "react";
import { RefreshCw } from "lucide-react";

// ─── Types ────────────────────────────────────────────────────────────────────

interface CostData {
  total_usd: number;
  requests: number;
  prompt_tokens: number;
  completion_tokens: number;
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

function formatUsd(value: number): string {
  // Show enough decimal places to be meaningful for small amounts
  if (value === 0) return "$0.0000";
  if (value < 0.001) return `$${value.toFixed(6)}`;
  return `$${value.toFixed(4)}`;
}

function formatTokens(value: number): string {
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(2)}M`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}K`;
  return String(value);
}

// ─── Component ────────────────────────────────────────────────────────────────

export default function CostPanel() {
  const [data, setData] = useState<CostData | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);

  const fetchCosts = useCallback(async (isManual = false) => {
    if (isManual) setRefreshing(true);
    try {
      const res = await fetch("http://localhost:7878/costs");
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const json = await res.json() as CostData;
      setData(json);
      setError(null);
    } catch (e: unknown) {
      setError(String(e));
    } finally {
      setLoading(false);
      setRefreshing(false);
    }
  }, []);

  // Initial fetch + poll every 10 seconds
  useEffect(() => {
    void fetchCosts();
    const interval = setInterval(() => void fetchCosts(), 10_000);
    return () => clearInterval(interval);
  }, [fetchCosts]);

  if (loading) {
    return (
      <div className="flex items-center gap-2 py-4">
        <div className="w-4 h-4 border-2 border-spectyn-primary border-t-transparent rounded-full animate-spin" />
        <span className="text-spectyn-muted text-xs">載入成本資料...</span>
      </div>
    );
  }

  if (error && !data) {
    return (
      <div className="bg-spectyn-danger/20 border border-spectyn-danger rounded p-3 text-sm text-spectyn-danger flex items-center justify-between">
        <span>無法載入成本：{error}</span>
        <button
          onClick={() => void fetchCosts(true)}
          className="ml-2 p-1 rounded hover:bg-spectyn-danger/20 transition-colors"
          title="重試"
        >
          <RefreshCw size={12} />
        </button>
      </div>
    );
  }

  const totalTokens = data ? data.prompt_tokens + data.completion_tokens : 0;

  return (
    <div className="flex flex-col gap-3">
      {/* Cost Today card */}
      <div className="bg-spectyn-bg border border-spectyn-border rounded-lg p-3">
        <div className="flex items-center justify-between mb-2">
          <p className="text-[10px] uppercase tracking-wider text-spectyn-muted">今日花費</p>
          <button
            onClick={() => void fetchCosts(true)}
            className={`p-1 rounded text-spectyn-muted hover:text-spectyn-text hover:bg-spectyn-border/50 transition-colors ${
              refreshing ? "animate-spin" : ""
            }`}
            title="重新整理成本"
            disabled={refreshing}
          >
            <RefreshCw size={11} />
          </button>
        </div>
        <p className="text-2xl font-bold text-spectyn-text font-mono">
          {data ? formatUsd(data.total_usd) : "$0.0000"}
        </p>
        <p className="text-xs text-spectyn-muted mt-1">
          {data ? data.requests : 0} 次請求
        </p>
      </div>

      {/* Token breakdown */}
      <div className="grid grid-cols-2 gap-2">
        <div className="bg-spectyn-bg border border-spectyn-border rounded-lg p-3">
          <p className="text-[10px] uppercase tracking-wider text-spectyn-muted mb-1">提示 Token</p>
          <p className="text-base font-bold text-spectyn-text">
            {data ? formatTokens(data.prompt_tokens) : "0"}
          </p>
        </div>
        <div className="bg-spectyn-bg border border-spectyn-border rounded-lg p-3">
          <p className="text-[10px] uppercase tracking-wider text-spectyn-muted mb-1">完成 Token</p>
          <p className="text-base font-bold text-spectyn-text">
            {data ? formatTokens(data.completion_tokens) : "0"}
          </p>
        </div>
      </div>

      {/* Total tokens row */}
      <div className="bg-spectyn-bg border border-spectyn-border rounded-lg px-3 py-2 flex items-center justify-between text-xs">
        <span className="text-spectyn-muted">總 Token</span>
        <span className="font-mono font-medium text-spectyn-text">{formatTokens(totalTokens)}</span>
      </div>

      {!data && (
        <p className="text-spectyn-muted text-xs text-center py-2">尚無成本記錄</p>
      )}
    </div>
  );
}
