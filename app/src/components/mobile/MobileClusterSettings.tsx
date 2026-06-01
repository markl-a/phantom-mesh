import { useState } from "react";
import { Network, CheckCircle, AlertCircle, RefreshCw } from "lucide-react";
import { useClusterModeStore } from "../../stores/clusterModeStore";
import { dispatchToCluster } from "../../lib/clusterDispatch";

export default function MobileClusterSettings() {
  const cluster = useClusterModeStore();
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<{ ok: boolean; msg: string } | null>(null);

  const test = async () => {
    setTesting(true);
    setTestResult(null);
    try {
      const r = await dispatchToCluster({
        coordinatorUrl: cluster.coordinatorUrl,
        secret: cluster.clusterSecret,
        agent: "master",
        prompt: "say one word: OK",
        maxWaitMs: 120000,
      });
      if (r.ok) {
        setTestResult({ ok: true, msg: `回應：${r.output} (${r.elapsedMs}ms, job ${r.jobId?.slice(0, 8)})` });
        // Auto-enable cluster mode after first successful test —
        // iOS local mode is non-functional, so once dispatch is verified
        // working there's no reason to leave the toggle off.
        if (!cluster.enabled) cluster.setEnabled(true);
      } else {
        setTestResult({ ok: false, msg: r.error || "unknown" });
      }
    } catch (e) {
      setTestResult({ ok: false, msg: String(e) });
    } finally {
      setTesting(false);
    }
  };

  return (
    <div className="space-y-4 max-w-md mx-auto">
      <div>
        <p className="text-sm text-phantom-text mb-1 flex items-center gap-2">
          <Network size={16} className="text-phantom-primary" />
          Cluster 派送模式
        </p>
        <p className="text-xs text-phantom-muted">
          開啟後，對話訊息會送到 coordinator <code className="bg-phantom-card px-1 rounded">/rpc/task/assign</code>
          ，由協調者選一個最閒的 worker 跑（可能是這台、可能是任一台 worker 節點…）。
        </p>
      </div>

      <div className="space-y-3">
        <label className="block">
          <span className="text-xs text-phantom-muted mb-1 block">Coordinator URL</span>
          <input
            type="text"
            value={cluster.coordinatorUrl}
            onChange={(e) => cluster.setCoordinatorUrl(e.target.value)}
            placeholder="http://100.x.x.x:7878"
            style={{ fontSize: "16px" }}
            className="w-full bg-phantom-card border border-phantom-border rounded-lg px-3 py-2.5 text-phantom-text placeholder-phantom-muted focus:outline-none focus:border-phantom-primary font-mono text-sm"
          />
          <span className="text-[10px] text-phantom-muted mt-1 block">
            coordinator 範例：<code>http://192.0.2.1:7878</code>
          </span>
        </label>

        <label className="block">
          <span className="text-xs text-phantom-muted mb-1 block">Cluster Secret</span>
          <input
            type="password"
            value={cluster.clusterSecret}
            onChange={(e) => cluster.setClusterSecret(e.target.value)}
            placeholder="phantom-cluster-..."
            style={{ fontSize: "16px" }}
            className="w-full bg-phantom-card border border-phantom-border rounded-lg px-3 py-2.5 text-phantom-text placeholder-phantom-muted focus:outline-none focus:border-phantom-primary font-mono text-sm"
          />
          <span className="text-[10px] text-phantom-muted mt-1 block">
            跟 coordinator agents.toml 內 <code>cluster_secret</code> 一致
          </span>
        </label>
      </div>

      {/* Test connection */}
      <button
        onClick={test}
        disabled={testing || !cluster.isConfigured()}
        className="w-full bg-phantom-primary text-phantom-bg py-2.5 rounded-lg font-medium flex items-center justify-center gap-2 disabled:opacity-40 transition"
      >
        {testing ? <RefreshCw size={16} className="animate-spin" /> : <Network size={16} />}
        {testing ? "測試中…" : "測試 dispatch"}
      </button>

      {testResult && (
        <div className={`rounded-lg p-3 flex items-start gap-2 ${
          testResult.ok ? "bg-phantom-success/15 border border-phantom-success/40" : "bg-phantom-danger/15 border border-phantom-danger/40"
        }`}>
          {testResult.ok
            ? <CheckCircle size={18} className="text-phantom-success flex-shrink-0 mt-0.5" />
            : <AlertCircle size={18} className="text-phantom-danger flex-shrink-0 mt-0.5" />}
          <div className="text-sm">
            <div className={`font-medium ${testResult.ok ? "text-phantom-success" : "text-phantom-danger"}`}>
              {testResult.ok ? "成功" : "失敗"}
            </div>
            <div className="text-phantom-muted text-xs mt-1 break-all">{testResult.msg}</div>
          </div>
        </div>
      )}

      {/* Toggle status */}
      <div className="bg-phantom-card border border-phantom-border rounded-lg px-3 py-3 flex items-center justify-between">
        <div>
          <div className="text-sm font-medium text-phantom-text">Cluster 模式</div>
          <div className="text-[11px] text-phantom-muted mt-0.5">
            {cluster.enabled ? "✓ 開啟（chat 走 coordinator）" : "關閉（chat 走本機）"}
          </div>
        </div>
        <button
          onClick={() => cluster.setEnabled(!cluster.enabled)}
          disabled={!cluster.isConfigured()}
          role="switch"
          aria-checked={cluster.enabled}
          aria-label="Cluster 模式"
          className={`relative w-12 h-6 rounded-full transition ${
            cluster.enabled ? "bg-phantom-success" : "bg-phantom-bg border border-phantom-border"
          } ${!cluster.isConfigured() ? "opacity-40" : ""}`}
        >
          <div className={`absolute top-0.5 w-5 h-5 rounded-full bg-white transition-transform ${
            cluster.enabled ? "translate-x-6" : "translate-x-0.5"
          }`} />
        </button>
      </div>
    </div>
  );
}
