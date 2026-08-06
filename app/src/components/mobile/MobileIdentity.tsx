// Mobile Identity & Privacy panel — mobile port of desktop
// settings/IdentityPanel.tsx (BIG-GOAL P4 cryptographic identity).
//
// Three cards stacked for phone: (1) the on-device identity status
// (fingerprint + keystore), (2) the static encryption honesty rail (what's
// age-encrypted at rest vs plaintext), and (3) a compact Life Node data
// export (type filter, since date, JSON/Markdown, open exports folder).
// Backed by existing identity_status / data_export / open_exports_folder
// commands — no backend change. Reuses the platform-agnostic
// loadIdentityStatus() helper.

import { useCallback, useEffect, useState } from 'react';
import { KeyRound, RefreshCw, ShieldCheck, FileText, Download, FolderOpen } from 'lucide-react';
import { loadIdentityStatus, type IdentityStatus } from '../../lib/identity';
import { safeInvoke as invoke } from '../../lib/tauri-compat';

// Mirrors the desktop honesty rail (BIG-GOAL P4 / TUI /identity §P4_SCOPE).
const ENCRYPTED = [
  '~/.spectyn-mesh/events/ — Life Node 事件 (age v1)',
  '~/.spectyn-mesh/identity.key — 裝置根金鑰',
];
const PLAINTEXT = [
  '~/.spectyn-mesh/agents.toml — Provider 設定',
  '~/.spectyn-mesh/memory.db — 記憶資料庫',
  '~/.spectyn-mesh/sessions/ — 對話紀錄',
];

const KIND_OPTIONS: { value: string; label: string }[] = [
  { value: '', label: '全部類型' },
  { value: 'food', label: '🍽 飲食' },
  { value: 'focus', label: '🎯 專注' },
  { value: 'habit', label: '✅ 習慣' },
  { value: 'text', label: '📝 文字' },
];

export default function MobileIdentity() {
  const [status, setStatus] = useState<IdentityStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const [exportKind, setExportKind] = useState('');
  const [exportSince, setExportSince] = useState('');
  const [exporting, setExporting] = useState(false);
  const [exportMsg, setExportMsg] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const s = await loadIdentityStatus();
      setStatus(s);
      if (!s) setError('身分後端暫時無法使用（需在桌面 app 中執行）');
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setStatus(null);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    let alive = true;
    (async () => {
      try {
        const s = await loadIdentityStatus();
        if (!alive) return;
        setStatus(s);
        if (!s) setError('身分後端暫時無法使用（需在桌面 app 中執行）');
      } catch (e) {
        if (alive) {
          setError(e instanceof Error ? e.message : String(e));
          setStatus(null);
        }
      } finally {
        if (alive) setLoading(false);
      }
    })();
    return () => {
      alive = false;
    };
  }, []);

  const doExport = useCallback(
    async (format: 'json' | 'markdown') => {
      if (exporting) return;
      setExporting(true);
      setExportMsg(null);
      try {
        const out = await invoke<string>('data_export', {
          format,
          kind: exportKind || null,
          since: exportSince || null,
        });
        setExportMsg(out ? `✓ 已匯出：${out}` : '匯出後端暫時無法使用');
      } catch (e) {
        setExportMsg(`匯出失敗：${e instanceof Error ? e.message : String(e)}`);
      } finally {
        setExporting(false);
      }
    },
    [exportKind, exportSince, exporting],
  );

  const openFolder = useCallback(async () => {
    try {
      await invoke('open_exports_folder', {});
    } catch (e) {
      setExportMsg(`開啟資料夾失敗：${e instanceof Error ? e.message : String(e)}`);
    }
  }, []);

  return (
    <div data-testid="mobile-identity-root" className="flex flex-col gap-3">
      {/* ─── Header ─────────────────────────────────────────────────────── */}
      <div className="flex items-center gap-2">
        <div className="w-8 h-8 rounded-lg bg-spectyn-primary/15 flex items-center justify-center flex-shrink-0">
          <KeyRound size={16} className="text-spectyn-primary" />
        </div>
        <div className="flex-1 min-w-0">
          <h2 className="text-sm font-semibold text-spectyn-text leading-tight">身分與隱私</h2>
          <p className="text-[10px] text-spectyn-muted">Identity & Privacy · BIG-GOAL P4</p>
        </div>
        <button
          type="button"
          data-testid="identity-refresh"
          onClick={() => void refresh()}
          aria-label="重新整理"
          className="text-spectyn-muted hover:text-spectyn-text p-1.5"
        >
          <RefreshCw size={15} className={loading ? 'animate-spin' : ''} />
        </button>
      </div>

      {error && (
        <div
          data-testid="identity-error"
          className="rounded-lg border border-spectyn-warning/40 bg-spectyn-warning/10 px-3 py-2 text-xs text-spectyn-warning"
        >
          {error}
        </div>
      )}

      {/* ─── Status card ────────────────────────────────────────────────── */}
      {status?.hasIdentity && (
        <div
          data-testid="identity-status-card"
          className="rounded-lg border border-spectyn-border bg-spectyn-card px-3 py-2.5 space-y-1.5"
        >
          <div className="flex items-center gap-2 text-xs text-spectyn-text">
            <span className="text-spectyn-primary">🔑 本機身分</span>
            <code className="font-mono text-[11px] break-all">{status.fingerprint}</code>
          </div>
          <div className="text-[10px] text-spectyn-muted">
            建立於 {status.createdAt} · 金鑰庫：{status.keystore}
          </div>
          {status.identityLine && (
            <div className="text-[10px] text-spectyn-muted break-all">{status.identityLine}</div>
          )}
        </div>
      )}

      {status && !status.hasIdentity && !error && (
        <div
          data-testid="identity-empty"
          className="rounded-lg border border-spectyn-border bg-spectyn-card px-3 py-2.5 text-xs text-spectyn-text"
        >
          尚未產生本機身分金鑰。執行 <code className="text-spectyn-primary">spectyn init</code> 後重新整理。
        </div>
      )}

      {/* ─── Encryption honesty rail ───────────────────────────────────── */}
      <div data-testid="identity-honesty-rail" className="rounded-lg border border-spectyn-border bg-spectyn-card px-3 py-2.5">
        <div className="flex items-center gap-2 mb-2">
          <ShieldCheck size={14} className="text-spectyn-success" />
          <span className="text-xs font-semibold text-spectyn-text">靜態加密範圍</span>
        </div>
        <p className="text-[10px] text-spectyn-muted mb-1.5">以裝置金鑰 age 加密：</p>
        <ul className="space-y-0.5 mb-2">
          {ENCRYPTED.map((p) => (
            <li key={p} className="text-[11px] text-spectyn-text flex items-start gap-1.5">
              <span className="text-spectyn-success flex-shrink-0">🔒</span>
              <code className="font-mono break-all">{p}</code>
            </li>
          ))}
        </ul>
        <p className="text-[10px] text-spectyn-muted mb-1.5 flex items-center gap-1">
          <FileText size={11} /> 明文（未加密）：
        </p>
        <ul className="space-y-0.5">
          {PLAINTEXT.map((p) => (
            <li key={p} className="text-[11px] text-spectyn-muted flex items-start gap-1.5">
              <span className="flex-shrink-0">○</span>
              <code className="font-mono break-all">{p}</code>
            </li>
          ))}
        </ul>
      </div>

      {/* ─── Export ─────────────────────────────────────────────────────── */}
      <div data-testid="identity-export" className="rounded-lg border border-spectyn-border bg-spectyn-card px-3 py-2.5">
        <div className="flex items-center gap-2 mb-2">
          <Download size={14} className="text-spectyn-primary" />
          <span className="text-xs font-semibold text-spectyn-text">匯出我的資料</span>
        </div>
        <p className="text-[10px] text-spectyn-muted mb-2">
          Life Node 事件 → 寫入 ~/.spectyn-mesh/exports/
        </p>
        <div className="flex flex-wrap items-center gap-2 mb-2">
          <select
            data-testid="export-kind"
            value={exportKind}
            onChange={(e) => setExportKind(e.target.value)}
            className="bg-spectyn-bg border border-spectyn-border rounded px-2 py-1 text-xs text-spectyn-text focus:border-spectyn-primary outline-none"
          >
            {KIND_OPTIONS.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </select>
          <input
            type="date"
            data-testid="export-since"
            value={exportSince}
            onChange={(e) => setExportSince(e.target.value)}
            title="起始日期 (含)"
            className="bg-spectyn-bg border border-spectyn-border rounded px-2 py-1 text-xs text-spectyn-text focus:border-spectyn-primary outline-none"
          />
          {(exportKind || exportSince) && (
            <button
              type="button"
              data-testid="export-clear"
              onClick={() => {
                setExportKind('');
                setExportSince('');
              }}
              className="text-[11px] text-spectyn-muted hover:text-spectyn-text"
            >
              清除
            </button>
          )}
        </div>
        <div className="flex flex-wrap gap-2">
          <button
            type="button"
            data-testid="export-json"
            disabled={exporting}
            onClick={() => void doExport('json')}
            className="px-2.5 py-1 rounded border border-spectyn-border bg-spectyn-bg text-xs text-spectyn-text hover:border-spectyn-primary/50 disabled:opacity-50"
          >
            JSON
          </button>
          <button
            type="button"
            data-testid="export-markdown"
            disabled={exporting}
            onClick={() => void doExport('markdown')}
            className="px-2.5 py-1 rounded border border-spectyn-border bg-spectyn-bg text-xs text-spectyn-text hover:border-spectyn-primary/50 disabled:opacity-50"
          >
            Markdown
          </button>
          <button
            type="button"
            data-testid="export-open-folder"
            onClick={() => void openFolder()}
            className="px-2.5 py-1 rounded border border-spectyn-border bg-spectyn-bg text-xs text-spectyn-muted hover:text-spectyn-text flex items-center gap-1"
          >
            <FolderOpen size={11} />
            資料夾
          </button>
        </div>
        {exportMsg && (
          <p data-testid="export-msg" className="text-[11px] text-spectyn-muted mt-2 break-all">
            {exportMsg}
          </p>
        )}
      </div>
    </div>
  );
}
