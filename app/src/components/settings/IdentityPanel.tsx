// Settings → 身分與隱私 (Identity & Privacy) — the app home for the P4
// cryptographic identity (BIG-GOAL P4). The app counterpart of the TUI
// `/identity` pane: shows the real on-device identity.key fingerprint +
// keystore backend (via the read-only identity_status command) and a concise
// honesty-rail summary of what is encrypted at rest vs plaintext.

import { useCallback, useEffect, useState } from "react";
import { KeyRound, RefreshCw, ShieldCheck, FileText, Download } from "lucide-react";
import { loadIdentityStatus, type IdentityStatus } from "../../lib/identity";
import { safeInvoke as invoke } from "../../lib/tauri-compat";

// Mirrors BIG-GOAL P4 / the TUI P4_SCOPE honesty rail at a summary level: the
// two age-encrypted paths vs the plaintext config/state. (The TUI pane pins the
// full 8-path table; this app tab states the user-facing essence.)
const ENCRYPTED = [
  "~/.phantom-mesh/events/ — Life Node 事件 (age v1)",
  "~/.phantom-mesh/identity.key — 裝置根金鑰",
];
const PLAINTEXT = [
  "~/.phantom-mesh/agents.toml — Provider 設定",
  "~/.phantom-mesh/memory.db — 記憶資料庫",
  "~/.phantom-mesh/sessions/ — 對話紀錄",
];

export default function IdentityPanel() {
  const [status, setStatus] = useState<IdentityStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const s = await loadIdentityStatus();
      setStatus(s);
      if (!s) setError("身分後端暫時無法使用（需在桌面 app 中執行）");
    } catch (e) {
      setError(String(e ?? "未知錯誤"));
      setStatus(null);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { void refresh(); }, [refresh]);

  const [exportMsg, setExportMsg] = useState<string | null>(null);
  const [exporting, setExporting] = useState(false);
  const [exportKind, setExportKind] = useState<string>("");
  const [exportSince, setExportSince] = useState<string>("");
  const doExport = useCallback(async (format: "json" | "markdown") => {
    setExporting(true);
    setExportMsg(null);
    try {
      const out = await invoke<string>("data_export", {
        format,
        kind: exportKind || null,
        since: exportSince || null,
      });
      setExportMsg(out ? `✓ 已匯出：${out}` : "匯出後端暫時無法使用(需在桌面 app 中執行)");
    } catch (e) {
      setExportMsg(`匯出失敗:${String(e ?? "未知錯誤")}`);
    } finally {
      setExporting(false);
    }
  }, [exportKind, exportSince]);
  const openFolder = useCallback(async () => {
    try { await invoke<string>("open_exports_folder", {}); }
    catch (e) { setExportMsg(`開啟資料夾失敗:${String(e ?? "未知錯誤")}`); }
  }, []);

  return (
    <div className="max-w-2xl space-y-5" data-testid="identity-panel">
      <header className="flex items-center gap-3">
        <div className="w-10 h-10 rounded-lg bg-phantom-primary/15 flex items-center justify-center">
          <KeyRound size={20} className="text-phantom-primary" />
        </div>
        <div className="flex-1">
          <h1 className="text-xl font-bold text-phantom-text">身分與隱私</h1>
          <p className="text-xs text-phantom-muted">Identity & Privacy · BIG-GOAL P4</p>
        </div>
        <button onClick={() => void refresh()} className="text-phantom-muted hover:text-phantom-text p-1.5" title="重新整理" aria-label="重新整理">
          <RefreshCw size={16} className={loading ? "animate-spin" : ""} />
        </button>
      </header>

      {error && (
        <div className="bg-phantom-warning/10 border border-phantom-warning/40 rounded-lg p-3 text-sm text-phantom-warning">{error}</div>
      )}

      {status?.hasIdentity && (
        <div className="bg-phantom-card border border-phantom-border rounded-lg p-4 space-y-2">
          <div className="flex items-center gap-2 text-sm text-phantom-text">
            <span className="text-phantom-primary">🔑 本機身分</span>
            <code className="font-mono text-phantom-text">{status.fingerprint}</code>
          </div>
          <div className="text-xs text-phantom-muted">建立於 {status.createdAt} · 金鑰庫：{status.keystore}</div>
          {status.identityLine && (
            <div className="text-xs text-phantom-muted">{status.identityLine}</div>
          )}
        </div>
      )}

      {status && !status.hasIdentity && !error && (
        <div className="bg-phantom-card border border-phantom-border rounded-lg p-4 text-sm text-phantom-text">
          尚未產生本機身分金鑰。執行 <code className="text-phantom-primary">phantom init</code> 後重新整理。
        </div>
      )}

      <div className="bg-phantom-card border border-phantom-border rounded-lg p-4">
        <div className="flex items-center gap-2 mb-3">
          <ShieldCheck size={16} className="text-phantom-success" />
          <span className="text-sm font-semibold text-phantom-text">靜態加密範圍（誠實揭露）</span>
        </div>
        <p className="text-xs text-phantom-muted mb-2">以裝置金鑰 age 加密：</p>
        <ul className="space-y-1 mb-3">
          {ENCRYPTED.map((p) => (
            <li key={p} className="text-xs text-phantom-text flex items-start gap-2">
              <span className="text-phantom-success flex-shrink-0">🔒</span>
              <code className="font-mono break-all">{p}</code>
            </li>
          ))}
        </ul>
        <p className="text-xs text-phantom-muted mb-2 flex items-center gap-1"><FileText size={12} /> 明文（未加密）：</p>
        <ul className="space-y-1">
          {PLAINTEXT.map((p) => (
            <li key={p} className="text-xs text-phantom-muted flex items-start gap-2">
              <span className="flex-shrink-0">○</span>
              <code className="font-mono break-all">{p}</code>
            </li>
          ))}
        </ul>
        <p className="text-[11px] text-phantom-muted/70 mt-3">刪除所有事件資料：<code className="text-phantom-text">phantom data delete --all --yes</code></p>
      </div>

      <div className="bg-phantom-card border border-phantom-border rounded-lg p-4">
        <div className="flex items-center gap-2 mb-3">
          <Download size={16} className="text-phantom-primary" />
          <span className="text-sm font-semibold text-phantom-text">匯出我的資料</span>
        </div>
        <p className="text-xs text-phantom-muted mb-3">把 Life Node 事件匯出成檔案(寫到 ~/.phantom-mesh/exports/)。可選篩選類型與起始日期。</p>
        <div className="flex flex-wrap items-center gap-2 mb-3">
          <select value={exportKind} onChange={(e) => setExportKind(e.target.value)}
            className="bg-phantom-bg border border-phantom-border rounded-lg px-2 py-1.5 text-xs text-phantom-text focus:border-phantom-primary outline-none">
            <option value="">全部類型</option>
            <option value="food">🍽 飲食</option>
            <option value="focus">🎯 專注</option>
            <option value="habit">✅ 習慣</option>
            <option value="text">📝 文字</option>
          </select>
          <input type="date" value={exportSince} onChange={(e) => setExportSince(e.target.value)}
            className="bg-phantom-bg border border-phantom-border rounded-lg px-2 py-1.5 text-xs text-phantom-text focus:border-phantom-primary outline-none" title="起始日期(含)" />
          {(exportKind || exportSince) && (
            <button onClick={() => { setExportKind(""); setExportSince(""); }} className="text-xs text-phantom-muted hover:text-phantom-text">清除</button>
          )}
        </div>
        <div className="flex flex-wrap gap-2">
          <button disabled={exporting} onClick={() => void doExport("json")}
            className="px-3 py-1.5 rounded-lg bg-phantom-bg border border-phantom-border text-sm text-phantom-text hover:border-phantom-primary/40 disabled:opacity-50">匯出 JSON</button>
          <button disabled={exporting} onClick={() => void doExport("markdown")}
            className="px-3 py-1.5 rounded-lg bg-phantom-bg border border-phantom-border text-sm text-phantom-text hover:border-phantom-primary/40 disabled:opacity-50">匯出 Markdown</button>
          <button onClick={() => void openFolder()}
            className="px-3 py-1.5 rounded-lg bg-phantom-bg border border-phantom-border text-sm text-phantom-muted hover:text-phantom-text hover:border-phantom-primary/40">開啟資料夾</button>
        </div>
        {exportMsg && <p className="text-xs text-phantom-muted mt-2 break-all">{exportMsg}</p>}
      </div>
    </div>
  );
}
