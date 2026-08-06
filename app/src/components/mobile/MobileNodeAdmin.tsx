// F105 · Mobile node-admin panel (E002 §"Settings screen" extension).
//
// Three controls, one Rust command per:
//   1. Broker token rotation — obfuscated preview + Rotate button →
//      `rotate_broker_token`. Success / failure pill renders below.
//   2. Manual peer add — URL input + Add button → `add_cluster_peer`.
//      JS validates URL shape; Rust re-validates via validate_daemon_url.
//      On success the cluster store is updated optimistically; on
//      failure (E_SETTINGS_PEER_*) we roll the optimistic row back.
//   3. Heartbeat-interval slider — range 5..=300s, current value read
//      via `get_heartbeat_interval`, commits on release via
//      `set_heartbeat_interval` (debounced 500ms).
//
// All three commands route through safeInvoke so the dev-browser build
// gets a no-op fallback and the mobile/Tauri build hits the real Rust
// surface. Error codes (E_SETTINGS_*) are surfaced verbatim to the
// user; the JS layer doesn't translate them — keeps the round-trip
// debuggable.

import { useEffect, useRef, useState } from 'react';
import { Key, Plus, Activity, Loader2, CheckCircle2, AlertCircle } from 'lucide-react';
import { safeInvoke } from '../../lib/tauri-compat';
import { useClusterStore } from '../../stores/clusterStore';

// ── Heartbeat slider bounds (mirror MIN/MAX_HEARTBEAT_SECS in
// commands/mobile_settings.rs — keep both ends in sync). ────────────────
const MIN_HEARTBEAT = 5;
const MAX_HEARTBEAT = 300;
const DEFAULT_HEARTBEAT = 30;

interface BrokerTokenPreview {
  token_preview: string;
  broker_url: string;
  expires_at_ms: number;
  configured: boolean;
}

interface RotateResult {
  token_preview: string;
  rotated_at_unix: number;
}

type Pill =
  | { kind: 'idle' }
  | { kind: 'loading'; label: string }
  | { kind: 'ok'; label: string }
  | { kind: 'err'; label: string };

/** Loose URL-shape gate so we can disable Add on obviously bad input
 *  without waiting for the round-trip. Rust is the source of truth. */
function urlShapeOk(s: string): boolean {
  const t = s.trim();
  if (t.length === 0) return false;
  return t.startsWith('http://') || t.startsWith('https://');
}

export default function MobileNodeAdmin() {
  // ── Broker token rotation ─────────────────────────────────────────
  const [tokenPreview, setTokenPreview] = useState<BrokerTokenPreview | null>(null);
  const [rotatePill, setRotatePill] = useState<Pill>({ kind: 'idle' });

  // ── Manual peer add ───────────────────────────────────────────────
  const [peerUrl, setPeerUrl] = useState('');
  const [peerPill, setPeerPill] = useState<Pill>({ kind: 'idle' });
  const cluster = useClusterStore();

  // ── Heartbeat slider ──────────────────────────────────────────────
  const [heartbeat, setHeartbeat] = useState<number>(DEFAULT_HEARTBEAT);
  const [heartbeatLoaded, setHeartbeatLoaded] = useState(false);
  const [heartbeatPill, setHeartbeatPill] = useState<Pill>({ kind: 'idle' });
  const commitTimer = useRef<number | null>(null);

  // ── Initial loads ─────────────────────────────────────────────────
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const preview = (await safeInvoke<BrokerTokenPreview>(
          'get_broker_token_preview',
        )) ?? null;
        if (!cancelled) setTokenPreview(preview);
      } catch {
        if (!cancelled) setTokenPreview(null);
      }
      try {
        const secs = (await safeInvoke<number>('get_heartbeat_interval')) ?? DEFAULT_HEARTBEAT;
        if (!cancelled) {
          setHeartbeat(Math.max(MIN_HEARTBEAT, Math.min(MAX_HEARTBEAT, Number(secs))));
          setHeartbeatLoaded(true);
        }
      } catch {
        if (!cancelled) setHeartbeatLoaded(true);
      }
    })();
    return () => {
      cancelled = true;
      if (commitTimer.current !== null) window.clearTimeout(commitTimer.current);
    };
  }, []);

  // ── Rotate token ──────────────────────────────────────────────────
  const handleRotate = async () => {
    setRotatePill({ kind: 'loading', label: '產生新 token…' });
    try {
      const out = await safeInvoke<RotateResult>('rotate_broker_token');
      // Reload preview so the UI reflects the rotated value.
      const preview = (await safeInvoke<BrokerTokenPreview>(
        'get_broker_token_preview',
      )) ?? null;
      setTokenPreview(preview);
      setRotatePill({
        kind: 'ok',
        label: `已輪換 → ${out.token_preview}`,
      });
    } catch (e) {
      setRotatePill({ kind: 'err', label: String(e) });
    }
  };

  // ── Add peer ──────────────────────────────────────────────────────
  const handleAddPeer = async () => {
    const url = peerUrl.trim();
    if (!urlShapeOk(url)) {
      setPeerPill({ kind: 'err', label: 'URL 必須以 http:// 或 https:// 開頭' });
      return;
    }
    setPeerPill({ kind: 'loading', label: '新增中…' });
    // Optimistic insert — append a placeholder node so the cluster
    // store reflects the add immediately. Rollback on error.
    const optimisticId = `pending-${Date.now()}`;
    cluster.add({
      nodeId: optimisticId,
      name: url,
      status: 'suspected',
      role: 'worker',
      capabilities: [],
      cpuLoad: 0,
      memoryPct: 0,
      activeTasks: 0,
      uptimeSecs: 0,
    });
    try {
      await safeInvoke<void>('add_cluster_peer', { peerUrl: url });
      setPeerPill({ kind: 'ok', label: `已加入 ${url}` });
      setPeerUrl('');
    } catch (e) {
      cluster.remove(optimisticId);
      setPeerPill({ kind: 'err', label: String(e) });
    }
  };

  // ── Heartbeat slider commit (debounced 500ms after release) ───────
  const scheduleHeartbeatCommit = (secs: number) => {
    if (commitTimer.current !== null) window.clearTimeout(commitTimer.current);
    commitTimer.current = window.setTimeout(async () => {
      setHeartbeatPill({ kind: 'loading', label: `儲存 ${secs}s…` });
      try {
        await safeInvoke<void>('set_heartbeat_interval', { secs });
        setHeartbeatPill({ kind: 'ok', label: `已儲存：${secs}s` });
      } catch (e) {
        setHeartbeatPill({ kind: 'err', label: String(e) });
      }
    }, 500);
  };

  return (
    <div className="space-y-6 max-w-md mx-auto">
      {/* ── Broker token rotation ───────────────────────────────────── */}
      <section data-testid="section-rotate" className="space-y-2">
        <div className="flex items-center gap-2">
          <Key size={16} className="text-spectyn-primary" />
          <h2 className="text-sm font-medium text-spectyn-text">Broker token</h2>
        </div>
        <p className="text-xs text-spectyn-muted">
          phantommesh.io 簽發給這台裝置的 token。按 Rotate 會在本機產生新 token；
          下次跟 broker 通訊時會用新值（可能需要重新登入）。
        </p>
        <div className="bg-spectyn-card border border-spectyn-border rounded-lg px-3 py-2.5">
          <div className="text-[10px] text-spectyn-muted">目前 token</div>
          <code
            data-testid="rotate-current-preview"
            className="text-sm text-spectyn-text font-mono"
          >
            {tokenPreview?.token_preview || '(尚未登入 broker)'}
          </code>
          {tokenPreview?.broker_url && (
            <div className="text-[10px] text-spectyn-muted mt-1 break-all">
              {tokenPreview.broker_url}
            </div>
          )}
        </div>
        <button
          data-testid="rotate-button"
          onClick={handleRotate}
          disabled={
            !tokenPreview?.configured ||
            rotatePill.kind === 'loading'
          }
          className="w-full bg-spectyn-primary text-spectyn-bg py-2.5 rounded-lg font-medium flex items-center justify-center gap-2 disabled:opacity-40 transition"
        >
          {rotatePill.kind === 'loading' ? (
            <Loader2 size={16} className="animate-spin" />
          ) : (
            <Key size={16} />
          )}
          {rotatePill.kind === 'loading' ? '處理中' : 'Rotate'}
        </button>
        <PillView data-testid="rotate-pill" pill={rotatePill} />
      </section>

      {/* ── Manual peer add ──────────────────────────────────────────── */}
      <section data-testid="section-peer" className="space-y-2">
        <div className="flex items-center gap-2">
          <Plus size={16} className="text-spectyn-primary" />
          <h2 className="text-sm font-medium text-spectyn-text">手動加 peer</h2>
        </div>
        <p className="text-xs text-spectyn-muted">
          把另一個 spectyn-mesh 節點寫進 <code className="bg-spectyn-card px-1 rounded">~/.spectyn-mesh/agents.toml</code>{' '}
          的 <code className="bg-spectyn-card px-1 rounded">[cluster] peers</code>。
        </p>
        <input
          data-testid="peer-input"
          type="text"
          inputMode="url"
          autoCapitalize="off"
          autoCorrect="off"
          value={peerUrl}
          onChange={(e) => setPeerUrl(e.target.value)}
          placeholder="https://oracle.tail.ts.net:7878"
          style={{ fontSize: '16px' }}
          className="w-full bg-spectyn-card border border-spectyn-border rounded-lg px-3 py-2.5 text-spectyn-text placeholder-spectyn-muted focus:outline-none focus:border-spectyn-primary font-mono text-sm"
        />
        <button
          data-testid="peer-add-button"
          onClick={handleAddPeer}
          disabled={!urlShapeOk(peerUrl) || peerPill.kind === 'loading'}
          className="w-full bg-spectyn-primary text-spectyn-bg py-2.5 rounded-lg font-medium flex items-center justify-center gap-2 disabled:opacity-40 transition"
        >
          {peerPill.kind === 'loading' ? (
            <Loader2 size={16} className="animate-spin" />
          ) : (
            <Plus size={16} />
          )}
          Add
        </button>
        <PillView data-testid="peer-pill" pill={peerPill} />
      </section>

      {/* ── Heartbeat slider ─────────────────────────────────────────── */}
      <section data-testid="section-heartbeat" className="space-y-2">
        <div className="flex items-center gap-2">
          <Activity size={16} className="text-spectyn-primary" />
          <h2 className="text-sm font-medium text-spectyn-text">Heartbeat 間隔</h2>
        </div>
        <p className="text-xs text-spectyn-muted">
          多久 ping 一次 cluster peers — 範圍 {MIN_HEARTBEAT}–{MAX_HEARTBEAT} 秒。
          數字越小越快發現掉線，越大越省電。
        </p>
        <div className="bg-spectyn-card border border-spectyn-border rounded-lg px-3 py-3 space-y-2">
          <div className="flex items-center justify-between">
            <span className="text-xs text-spectyn-muted">目前</span>
            <span
              data-testid="heartbeat-value"
              className="text-sm font-mono text-spectyn-text"
            >
              {heartbeat}s
            </span>
          </div>
          <input
            data-testid="heartbeat-slider"
            type="range"
            min={MIN_HEARTBEAT}
            max={MAX_HEARTBEAT}
            step={1}
            value={heartbeat}
            disabled={!heartbeatLoaded}
            onChange={(e) => setHeartbeat(Number(e.target.value))}
            onPointerUp={() => scheduleHeartbeatCommit(heartbeat)}
            onKeyUp={() => scheduleHeartbeatCommit(heartbeat)}
            onBlur={() => scheduleHeartbeatCommit(heartbeat)}
            className="w-full accent-spectyn-primary"
          />
          <div className="flex justify-between text-[10px] text-spectyn-muted">
            <span>{MIN_HEARTBEAT}s</span>
            <span>{MAX_HEARTBEAT}s</span>
          </div>
        </div>
        <PillView data-testid="heartbeat-pill" pill={heartbeatPill} />
      </section>
    </div>
  );
}

// ── Pill component ───────────────────────────────────────────────────────

function PillView({
  pill,
  ...rest
}: {
  pill: Pill;
  [k: string]: unknown;
}) {
  if (pill.kind === 'idle') return null;
  const base = 'mt-1 px-3 py-2 rounded-lg flex items-start gap-2 text-xs';
  if (pill.kind === 'loading') {
    return (
      <div {...rest} className={`${base} bg-spectyn-card border border-spectyn-border text-spectyn-muted`}>
        <Loader2 size={14} className="animate-spin flex-shrink-0 mt-0.5" />
        <span>{pill.label}</span>
      </div>
    );
  }
  if (pill.kind === 'ok') {
    return (
      <div
        {...rest}
        className={`${base} bg-spectyn-success/15 border border-spectyn-success/40 text-spectyn-success`}
      >
        <CheckCircle2 size={14} className="flex-shrink-0 mt-0.5" />
        <span>{pill.label}</span>
      </div>
    );
  }
  return (
    <div
      {...rest}
      className={`${base} bg-spectyn-danger/15 border border-spectyn-danger/40 text-spectyn-danger`}
    >
      <AlertCircle size={14} className="flex-shrink-0 mt-0.5" />
      <span className="break-all">{pill.label}</span>
    </div>
  );
}
