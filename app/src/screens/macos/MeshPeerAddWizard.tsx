// SPEC-41 §10.12 — S12 mesh peer add wizard (對等節點加入精靈).
//
// Settings → Cluster → "+ Add peer" lands here. Walks the user through joining
// another device to their cluster: a 5s mDNS scan that surfaces nearby peers,
// with a QR-code fallback for devices that aren't on the same subnet.
//
// State machine (§10.13): scanning → list | qr → invited-waiting → joined,
// plus qr-expired (the 300s QR TTL). Edge case (§10.12): a 5s scan that finds
// 0 peers auto-switches to the QR view.
//
// Frontend-only: reuses the existing `useClusterPeers` hook to read discovered
// peers — no new backend command. The actual invite/QR-mint round-trips are not
// wired yet (no backing command exists), so the invite + QR are presentational
// placeholders driven by the local state machine; a TODO marks where the real
// `cluster_invite_peer` / `cluster_mint_join_qr` commands plug in.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { ArrowLeft, CheckCircle2, Loader2, QrCode, RefreshCw, Smartphone, WifiOff } from "lucide-react";
import { useClusterPeers } from "../../hooks/useClusterPeers";

type WizardState =
  | "scanning"
  | "list"
  | "qr"
  | "invited_waiting"
  | "joined"
  | "qr_expired";

/** mDNS scan window before we decide there's nothing nearby (§10.12). */
const SCAN_MS = 5000;
/** QR join code time-to-live (§10.12: 300s). */
const QR_TTL_S = 300;

function mmss(secs: number): string {
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  return `${m}:${String(s).padStart(2, "0")}`;
}

export default function MeshPeerAddWizard() {
  const navigate = useNavigate();
  const { peers, refresh } = useClusterPeers();

  const [state, setState] = useState<WizardState>("scanning");
  // Which discovered peer we invited (display name), for the waiting/joined copy.
  const [invitedName, setInvitedName] = useState<string | null>(null);
  // QR countdown (seconds remaining); only meaningful in the "qr" state.
  const [qrLeft, setQrLeft] = useState(QR_TTL_S);

  const scanTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  // Latest peer list, read inside the scan-timeout without re-arming the timer.
  const peersRef = useRef(peers);
  peersRef.current = peers;

  // Begin (or restart) the mDNS scan: refresh the peer list, then after SCAN_MS
  // resolve to the list (peers found) or the QR fallback (nothing found, §10.12).
  const startScan = useCallback(() => {
    setState("scanning");
    void refresh();
    if (scanTimer.current) clearTimeout(scanTimer.current);
    scanTimer.current = setTimeout(() => {
      setState((prev) => {
        if (prev !== "scanning") return prev;
        return peersRef.current.length > 0 ? "list" : "qr";
      });
    }, SCAN_MS);
  }, [refresh]);

  useEffect(() => {
    startScan();
    return () => {
      if (scanTimer.current) clearTimeout(scanTimer.current);
    };
  }, [startScan]);

  // QR countdown → expiry.
  useEffect(() => {
    if (state !== "qr") return;
    setQrLeft(QR_TTL_S);
    const id = setInterval(() => {
      setQrLeft((s) => {
        if (s <= 1) {
          clearInterval(id);
          setState("qr_expired");
          return 0;
        }
        return s - 1;
      });
    }, 1000);
    return () => clearInterval(id);
  }, [state]);

  const invite = useCallback((displayName: string) => {
    setInvitedName(displayName);
    setState("invited_waiting");
    // TODO(cluster): replace with real `cluster_invite_peer` round-trip + a
    // peer-joined event. For now the local state machine advances to "joined"
    // once the invite is acknowledged out-of-band.
    setTimeout(() => setState("joined"), 1800);
  }, []);

  const close = useCallback(() => navigate("/settings/cluster"), [navigate]);

  const title = useMemo(() => {
    switch (state) {
      case "joined":
        return "已加入叢集";
      case "qr":
      case "qr_expired":
        return "用 QR 加入";
      default:
        return "新增對等節點";
    }
  }, [state]);

  return (
    <div className="max-w-xl mx-auto space-y-5" data-testid="mesh-peer-add-wizard" data-state={state}>
      <header className="flex items-center gap-3">
        <button
          onClick={close}
          className="text-phantom-muted hover:text-phantom-text p-1.5"
          title="返回叢集"
          aria-label="返回叢集"
        >
          <ArrowLeft size={18} />
        </button>
        <div className="flex-1">
          <h1 className="text-lg font-bold text-phantom-text">{title}</h1>
          <p className="text-xs text-phantom-muted">Add a peer to your cluster</p>
        </div>
      </header>

      {state === "scanning" && (
        <div className="bg-phantom-card border border-phantom-border rounded-lg p-6 text-center" data-testid="wizard-scanning">
          <Loader2 size={26} className="text-phantom-primary mx-auto mb-3 animate-spin" />
          <p className="text-sm text-phantom-text">正在掃描附近裝置（mDNS）…</p>
          <p className="text-xs text-phantom-muted mt-1">在另一台裝置上開啟 Phantom Mesh，讓它出現在同網段。</p>
        </div>
      )}

      {state === "list" && (
        <div className="space-y-3" data-testid="wizard-list">
          <div className="bg-phantom-card border border-phantom-border rounded-lg p-4">
            <h3 className="text-sm font-medium text-phantom-text mb-3">掃到的裝置（{peers.length}）</h3>
            <div className="space-y-2">
              {peers.map((p) => (
                <div
                  key={p.peer_id}
                  className="flex items-center gap-3 px-3 py-2 rounded bg-phantom-bg border border-phantom-border"
                >
                  <Smartphone size={16} className="text-phantom-primary flex-shrink-0" />
                  <span className="text-sm text-phantom-text flex-1 truncate">{p.display_name}</span>
                  <button
                    onClick={() => invite(p.display_name)}
                    className="text-xs px-3 py-1 rounded-lg bg-phantom-primary/15 border border-phantom-primary/40 text-phantom-primary hover:bg-phantom-primary/25"
                  >
                    邀請加入
                  </button>
                </div>
              ))}
            </div>
          </div>

          <div className="flex items-center gap-3 text-[11px] text-phantom-muted">
            <span className="flex-1 h-px bg-phantom-border" />
            或
            <span className="flex-1 h-px bg-phantom-border" />
          </div>

          <button
            onClick={() => setState("qr")}
            className="w-full flex items-center justify-center gap-2 px-3 py-2.5 rounded-lg bg-phantom-card border border-phantom-border text-sm text-phantom-text hover:border-phantom-primary/40 transition"
          >
            <QrCode size={15} /> 不在同網段？改用 QR
          </button>
        </div>
      )}

      {state === "qr" && (
        <div className="bg-phantom-card border border-phantom-border rounded-lg p-6 text-center" data-testid="wizard-qr">
          <div className="w-40 h-40 mx-auto rounded-lg bg-phantom-bg border border-phantom-border flex items-center justify-center mb-3">
            <QrCode size={88} className="text-phantom-text" aria-label="加入 QR 碼" />
          </div>
          <p className="text-sm text-phantom-text">在另一台裝置上掃描這個 QR 碼來加入。</p>
          <p className="text-xs text-phantom-muted mt-1">
            有效時間 <span className="text-phantom-primary font-mono">{mmss(qrLeft)}</span>
          </p>
          <button
            onClick={startScan}
            className="mt-4 text-xs text-phantom-primary hover:underline"
          >
            ↩ 回到掃描附近裝置
          </button>
        </div>
      )}

      {state === "qr_expired" && (
        <div className="bg-phantom-card border border-phantom-border rounded-lg p-6 text-center" data-testid="wizard-qr-expired">
          <WifiOff size={24} className="text-phantom-warning mx-auto mb-2" />
          <p className="text-sm text-phantom-text">QR 碼已過期。</p>
          <p className="text-xs text-phantom-muted mt-1">為了安全，加入碼只有 5 分鐘有效。</p>
          <button
            onClick={() => setState("qr")}
            className="mt-4 inline-flex items-center gap-1.5 text-xs px-3 py-1.5 rounded-lg bg-phantom-primary/15 border border-phantom-primary/40 text-phantom-primary hover:bg-phantom-primary/25"
          >
            <RefreshCw size={13} /> 重新產生 QR
          </button>
        </div>
      )}

      {state === "invited_waiting" && (
        <div className="bg-phantom-card border border-phantom-border rounded-lg p-6 text-center" data-testid="wizard-waiting">
          <Loader2 size={24} className="text-phantom-primary mx-auto mb-2 animate-spin" />
          <p className="text-sm text-phantom-text">已邀請 {invitedName ?? "裝置"}，等待對方確認…</p>
          <p className="text-xs text-phantom-muted mt-1">請在另一台裝置上接受加入請求。</p>
        </div>
      )}

      {state === "joined" && (
        <div className="bg-phantom-card border border-phantom-border rounded-lg p-6 text-center" data-testid="wizard-joined">
          <CheckCircle2 size={26} className="text-phantom-success mx-auto mb-2" />
          <p className="text-sm text-phantom-text">{invitedName ?? "新裝置"} 已加入叢集。</p>
          <p className="text-xs text-phantom-muted mt-1">事件會在裝置間自動同步（端對端加密）。</p>
          <button
            onClick={close}
            className="mt-4 text-xs px-4 py-1.5 rounded-lg bg-phantom-primary/15 border border-phantom-primary/40 text-phantom-primary hover:bg-phantom-primary/25"
          >
            完成
          </button>
        </div>
      )}

      {(state === "scanning" || state === "list" || state === "qr") && (
        <div className="flex justify-end">
          <button onClick={close} className="text-xs px-4 py-1.5 rounded-lg bg-phantom-card border border-phantom-border text-phantom-muted hover:text-phantom-text">
            取消
          </button>
        </div>
      )}
    </div>
  );
}
