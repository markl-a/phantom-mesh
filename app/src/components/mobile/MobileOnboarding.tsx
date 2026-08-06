import { useState } from "react";
import { safeInvoke as invoke } from "../../lib/tauri-compat";
import { Download, CheckCircle, AlertCircle } from "lucide-react";

type Status = "idle" | "fetching" | "writing" | "ok" | "error";

export default function MobileOnboarding() {
  const [host, setHost] = useState("");  // Mac Tailscale IP (e.g. 100.x.x.x) — left blank; user fills in
  const [token, setToken] = useState("");
  const [nodeName, setNodeName] = useState("android-phone");
  const [status, setStatus] = useState<Status>("idle");
  const [errMsg, setErrMsg] = useState("");

  const importConfig = async () => {
    setStatus("fetching");
    setErrMsg("");
    try {
      const url = `http://${host}:7878/onboarding/config?token=${encodeURIComponent(token)}&node_name=${encodeURIComponent(nodeName)}`;
      const resp = await fetch(url);
      if (!resp.ok) {
        const text = await resp.text();
        throw new Error(`HTTP ${resp.status}: ${text.slice(0, 200)}`);
      }
      const tomlText = await resp.text();
      if (!tomlText.includes("[providers")) {
        throw new Error("回傳不是 agents.toml 格式");
      }

      setStatus("writing");
      // Tauri command to write the config file
      await invoke("import_agents_toml", { content: tomlText });

      setStatus("ok");
    } catch (e) {
      setErrMsg(String(e));
      setStatus("error");
    }
  };

  return (
    <div className="space-y-4 max-w-md mx-auto">
      <div>
        <p className="text-sm text-spectyn-text mb-1">用 Mac 端的 token 把 cluster + provider 設定一次拉到手機。</p>
        <p className="text-xs text-spectyn-muted">在 Mac 跑 <code className="bg-spectyn-card px-1 rounded">spectyn onboarding-token</code> 取得 token，或從 spectyn-mesh app 設定頁複製。</p>
      </div>

      <div className="space-y-3">
        <label className="block">
          <span className="text-xs text-spectyn-muted mb-1 block">Mac Tailscale IP</span>
          <input
            type="text"
            value={host}
            onChange={(e) => setHost(e.target.value)}
            placeholder="100.x.x.x"
            style={{ fontSize: "16px" }}
            className="w-full bg-spectyn-card border border-spectyn-border rounded-lg px-3 py-2.5 text-spectyn-text placeholder-spectyn-muted focus:outline-none focus:border-spectyn-primary"
          />
        </label>

        <label className="block">
          <span className="text-xs text-spectyn-muted mb-1 block">Token</span>
          <input
            type="text"
            value={token}
            onChange={(e) => setToken(e.target.value)}
            placeholder="貼上 token..."
            style={{ fontSize: "16px" }}
            className="w-full bg-spectyn-card border border-spectyn-border rounded-lg px-3 py-2.5 text-spectyn-text placeholder-spectyn-muted focus:outline-none focus:border-spectyn-primary font-mono"
          />
        </label>

        <label className="block">
          <span className="text-xs text-spectyn-muted mb-1 block">這台節點名稱</span>
          <input
            type="text"
            value={nodeName}
            onChange={(e) => setNodeName(e.target.value)}
            placeholder="例: android-phone"
            style={{ fontSize: "16px" }}
            className="w-full bg-spectyn-card border border-spectyn-border rounded-lg px-3 py-2.5 text-spectyn-text placeholder-spectyn-muted focus:outline-none focus:border-spectyn-primary"
          />
        </label>
      </div>

      <button
        onClick={importConfig}
        disabled={status === "fetching" || status === "writing" || !token || !host}
        className="w-full bg-spectyn-primary text-spectyn-bg py-3 rounded-lg font-medium flex items-center justify-center gap-2 disabled:opacity-40 transition"
      >
        <Download size={18} />
        {status === "fetching" ? "下載中…"
          : status === "writing" ? "寫入中…"
          : "匯入並啟動"}
      </button>

      {status === "ok" && (
        <div className="bg-spectyn-success/15 border border-spectyn-success/40 rounded-lg p-3 flex items-start gap-2">
          <CheckCircle size={18} className="text-spectyn-success flex-shrink-0 mt-0.5" />
          <div className="text-sm">
            <div className="text-spectyn-success font-medium">匯入成功</div>
            <div className="text-spectyn-muted text-xs mt-1">關閉 app 重開即可生效</div>
          </div>
        </div>
      )}

      {status === "error" && (
        <div className="bg-spectyn-danger/15 border border-spectyn-danger/40 rounded-lg p-3 flex items-start gap-2">
          <AlertCircle size={18} className="text-spectyn-danger flex-shrink-0 mt-0.5" />
          <div className="text-sm">
            <div className="text-spectyn-danger font-medium">失敗</div>
            <div className="text-spectyn-muted text-xs mt-1 break-all">{errMsg}</div>
          </div>
        </div>
      )}
    </div>
  );
}
