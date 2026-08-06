// apex-④ · Phone approvals screen.
//
// The ④-safe-unattended differentiator's HUMAN end: when a governed run hits a
// high-risk action it PAUSES and raises an approval; this screen shows every
// pending approval and lets the operator tap Approve / Deny / Stop from the
// phone. Decisions go to the backend's existing /rpc/inbox (topic =
// approval_id), where the governor's escalator correlates the reply and
// resumes / aborts the run — see core/src/governed_run/escalation.rs.
//
// Wire conventions (matches the contract):
//   - pending list:   POST /rpc/approvals/list  {}              → { pending: ApprovalCard[] }
//   - decision:       POST /rpc/inbox  { topic: <approval_id>, text: "approve"|"deny"|"stop" }
//
// Networking is REUSED, not reinvented: every request goes through
// `clusterPost()` (HMAC-SHA256 X-Cluster-Auth via crypto.subtle, routed through
// the native iOS NSURLSession bridge / tauri-plugin-http). We never touch the
// HMAC/fetch primitives directly. Style + Tailwind mirror MobileDispatch.tsx.
//
// baseUrl / secret resolution:
//   - Default: read from `useClusterModeStore()` (coordinatorUrl + clusterSecret) —
//     this is what the desktop wrapper and the legacy mobile shell use.
//   - Override: callers can pass `baseUrl`/`secret` props. The live mobile shell
//     (AppTemplate) keeps its connection config in its own `useApp()` context +
//     localStorage (NOT the cluster store), so it passes those in as props.

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { ShieldCheck, Check, Ban, OctagonX, Clock, RefreshCw, AlertTriangle } from 'lucide-react';
import { clusterPost } from '../../lib/clusterDispatch';
import { useClusterModeStore } from '../../stores/clusterModeStore';
import {
  useApprovalsStore,
  type ApprovalCard,
} from '../../stores/approvalsStore';

const POLL_MS = 5_000;

type Decision = 'approve' | 'deny' | 'stop';

export interface MobileApprovalsProps {
  /** Optional override for the backend base URL. When omitted, falls back to
   *  `useClusterModeStore().coordinatorUrl`. AppTemplate passes its own. */
  baseUrl?: string;
  /** Optional override for the HMAC cluster secret. When omitted, falls back to
   *  `useClusterModeStore().clusterSecret`. */
  secret?: string;
}

// ── Helpers ──────────────────────────────────────────────────────────────

/** Map a risk-level string to a Tailwind text color. Tolerant of the various
 *  spellings the backend may emit (execute_high / high / medium / low). */
function riskColor(risk: string): string {
  const r = (risk || '').toLowerCase();
  if (r.includes('high') || r.includes('critical') || r.includes('danger')) {
    return 'text-spectyn-danger';
  }
  if (r.includes('med') || r.includes('warn') || r.includes('moderate')) {
    return 'text-spectyn-warning';
  }
  if (r.includes('low') || r.includes('safe') || r.includes('read')) {
    return 'text-green-400';
  }
  return 'text-spectyn-muted';
}

/** Border tint for the card, derived from the same risk classification. */
function riskBorder(risk: string): string {
  const r = (risk || '').toLowerCase();
  if (r.includes('high') || r.includes('critical') || r.includes('danger')) {
    return 'border-spectyn-danger/40';
  }
  if (r.includes('med') || r.includes('warn') || r.includes('moderate')) {
    return 'border-spectyn-warning/40';
  }
  return 'border-spectyn-border';
}

/** "3m ago" style relative age from a unix-millis timestamp. */
function ageLabel(createdMs: number, nowMs: number): string {
  if (!createdMs || createdMs <= 0) return '';
  const sec = Math.max(0, Math.round((nowMs - createdMs) / 1000));
  if (sec < 60) return `${sec}s ago`;
  const min = Math.round(sec / 60);
  if (min < 60) return `${min}m ago`;
  const hr = Math.round(min / 60);
  if (hr < 24) return `${hr}h ago`;
  const day = Math.round(hr / 24);
  return `${day}d ago`;
}

/** Parse a /rpc/approvals/list response into ApprovalCard[]. Tolerant of a
 *  missing / malformed `pending` field (returns []). */
function parsePending(json: unknown): ApprovalCard[] {
  const raw = (json as { pending?: unknown } | undefined)?.pending;
  if (!Array.isArray(raw)) return [];
  const out: ApprovalCard[] = [];
  for (const item of raw) {
    const o = (item ?? {}) as Record<string, unknown>;
    const approval_id = String(o.approval_id ?? '');
    if (!approval_id) continue;
    out.push({
      approval_id,
      task_id: String(o.task_id ?? ''),
      tool: String(o.tool ?? ''),
      risk: String(o.risk ?? ''),
      reason: String(o.reason ?? ''),
      created_ms:
        typeof o.created_ms === 'number'
          ? o.created_ms
          : Number(o.created_ms) || 0,
    });
  }
  return out;
}

// ── Component ────────────────────────────────────────────────────────────

export default function MobileApprovals(props: MobileApprovalsProps) {
  const cluster = useClusterModeStore();
  const baseUrl = props.baseUrl ?? cluster.coordinatorUrl;
  const secret = props.secret ?? cluster.clusterSecret;
  const configured = baseUrl.length > 0 && secret.length > 0;

  const items = useApprovalsStore((s) => s.items);
  const setItems = useApprovalsStore((s) => s.setItems);
  const removeItem = useApprovalsStore((s) => s.removeItem);

  const [loading, setLoading] = useState(false);
  const [listError, setListError] = useState<string | null>(null);
  // Per-card transient state: which decision is in flight, and any error.
  const [busy, setBusy] = useState<Record<string, Decision>>({});
  const [cardErrors, setCardErrors] = useState<Record<string, string>>({});
  const [toast, setToast] = useState<string | null>(null);
  // A clock tick so relative ages re-render without a fresh poll.
  const [nowMs, setNowMs] = useState(() => Date.now());

  // Keep the latest baseUrl/secret in a ref so the polling interval closure
  // always reads current values without re-arming the interval each render.
  const connRef = useRef({ baseUrl, secret });
  connRef.current = { baseUrl, secret };

  // ── poll /rpc/approvals/list on mount + every POLL_MS ──
  const refresh = useCallback(async () => {
    const { baseUrl: b, secret: s } = connRef.current;
    if (!b || !s) {
      setListError(null);
      return;
    }
    setLoading(true);
    try {
      const r = await clusterPost(b, s, '/rpc/approvals/list', {});
      if (!r.ok) {
        setListError(`列表載入失敗 (${r.status})`);
        return;
      }
      setListError(null);
      setItems(parsePending(r.json));
    } catch (e) {
      setListError(`列表載入發生例外:${String(e).slice(0, 120)}`);
    } finally {
      setLoading(false);
    }
  }, [setItems]);

  useEffect(() => {
    if (!configured) return;
    void refresh();
    const id = window.setInterval(() => {
      void refresh();
    }, POLL_MS);
    return () => window.clearInterval(id);
    // Re-arm when the configured connection identity changes.
  }, [configured, baseUrl, secret, refresh]);

  // A 1s clock so ages stay fresh between polls.
  useEffect(() => {
    const id = window.setInterval(() => setNowMs(Date.now()), 1_000);
    return () => window.clearInterval(id);
  }, []);

  // Auto-clear the confirmation toast.
  useEffect(() => {
    if (!toast) return;
    const id = window.setTimeout(() => setToast(null), 2_500);
    return () => window.clearTimeout(id);
  }, [toast]);

  // ── decision handler ──
  const decide = useCallback(
    async (card: ApprovalCard, decision: Decision) => {
      if (busy[card.approval_id]) return; // one decision per card at a time
      setBusy((m) => ({ ...m, [card.approval_id]: decision }));
      setCardErrors((m) => {
        if (!(card.approval_id in m)) return m;
        const next = { ...m };
        delete next[card.approval_id];
        return next;
      });
      const { baseUrl: b, secret: s } = connRef.current;
      try {
        const r = await clusterPost(b, s, '/rpc/inbox', {
          topic: card.approval_id,
          text: decision,
        });
        if (!r.ok) {
          // Keep the card; surface the error so the operator can retry.
          setCardErrors((m) => ({
            ...m,
            [card.approval_id]: `送出失敗 (${r.status})`,
          }));
          return;
        }
        // Optimistic: drop the card + confirm.
        removeItem(card.approval_id);
        setToast(
          decision === 'approve'
            ? '已批准 ✓'
            : decision === 'deny'
              ? '已拒絕 ✓'
              : '已停止 ✓',
        );
      } catch (e) {
        setCardErrors((m) => ({
          ...m,
          [card.approval_id]: `發生例外:${String(e).slice(0, 100)}`,
        }));
      } finally {
        setBusy((m) => {
          const next = { ...m };
          delete next[card.approval_id];
          return next;
        });
      }
    },
    [busy, removeItem],
  );

  // Newest-first list for rendering.
  const cards = useMemo(
    () => Object.values(items).sort((a, b) => b.created_ms - a.created_ms),
    [items],
  );

  // ── render ──
  return (
    <div
      className="flex flex-col h-full overflow-y-auto"
      data-testid="mobile-approvals-root"
    >
      <div className="px-4 py-3 border-b border-spectyn-border flex items-center justify-between">
        <div className="flex items-center gap-2 text-sm text-spectyn-text">
          <ShieldCheck size={16} className="text-spectyn-primary" />
          審核 · Approvals
        </div>
        <button
          type="button"
          onClick={() => void refresh()}
          disabled={!configured || loading}
          data-testid="approvals-refresh"
          className="text-spectyn-muted hover:text-spectyn-text disabled:opacity-40 p-1"
          aria-label="重新整理"
          title="重新整理"
        >
          <RefreshCw size={16} className={loading ? 'animate-spin' : ''} />
        </button>
      </div>

      <div className="p-3 space-y-3">
        {/* Not-configured banner */}
        {!configured && (
          <div
            className="bg-spectyn-card border border-spectyn-warning/40 rounded-lg p-3 text-sm text-spectyn-text"
            data-testid="approvals-not-configured"
          >
            <div className="flex items-center gap-2 text-spectyn-warning mb-1">
              <AlertTriangle size={15} />
              <span className="text-xs font-medium uppercase tracking-wider">
                尚未設定
              </span>
            </div>
            請先到「設定」設定後端連線(Base URL 與 Cluster Secret),才能載入待審核項目。
          </div>
        )}

        {/* List-level error */}
        {configured && listError && (
          <div
            className="text-xs text-spectyn-danger px-1"
            data-testid="approvals-list-error"
            role="alert"
          >
            {listError}
          </div>
        )}

        {/* Confirmation toast */}
        {toast && (
          <div
            className="bg-spectyn-card border border-green-500/40 rounded-lg px-3 py-2 text-sm text-green-400"
            data-testid="approvals-toast"
            role="status"
          >
            {toast}
          </div>
        )}

        {/* Empty state */}
        {configured && cards.length === 0 && !listError && (
          <div
            className="text-center text-spectyn-muted text-sm py-10"
            data-testid="approvals-empty"
          >
            <ShieldCheck size={28} className="mx-auto mb-2 opacity-50" />
            目前沒有待審核的項目。
          </div>
        )}

        {/* Cards */}
        {cards.map((card) => {
          const inFlight = busy[card.approval_id];
          const err = cardErrors[card.approval_id];
          return (
            <div
              key={card.approval_id}
              data-testid={`approval-card-${card.approval_id}`}
              className={`bg-spectyn-card border ${riskBorder(card.risk)} rounded-lg p-3 space-y-2`}
            >
              {/* Header: tool + risk + age */}
              <div className="flex items-center justify-between gap-2">
                <div className="text-sm font-medium text-spectyn-text truncate">
                  {card.tool || '(unknown tool)'}
                </div>
                <div
                  className={`text-[10px] font-semibold uppercase tracking-wider ${riskColor(card.risk)}`}
                  data-testid="approval-risk"
                >
                  {card.risk || 'unknown'}
                </div>
              </div>

              {/* Reason */}
              {card.reason && (
                <div className="text-xs text-spectyn-muted whitespace-pre-wrap break-words">
                  {card.reason}
                </div>
              )}

              {/* Meta: task id + age */}
              <div className="flex items-center gap-3 text-[10px] text-spectyn-muted/80">
                {card.task_id && (
                  <span className="font-mono truncate" title={card.task_id}>
                    {card.task_id.slice(0, 12)}
                  </span>
                )}
                {card.created_ms > 0 && (
                  <span className="flex items-center gap-1">
                    <Clock size={10} />
                    {ageLabel(card.created_ms, nowMs)}
                  </span>
                )}
              </div>

              {/* Per-card error */}
              {err && (
                <div
                  className="text-[11px] text-spectyn-danger"
                  data-testid="approval-card-error"
                  role="alert"
                >
                  {err}
                </div>
              )}

              {/* Actions */}
              <div className="flex items-center gap-2 pt-1">
                <button
                  type="button"
                  onClick={() => void decide(card, 'approve')}
                  disabled={!!inFlight}
                  data-testid="approval-approve"
                  className="flex-1 flex items-center justify-center gap-1.5 bg-spectyn-primary text-white rounded-lg py-2 text-sm font-medium disabled:opacity-40 disabled:cursor-not-allowed"
                >
                  <Check size={15} />
                  {inFlight === 'approve' ? '送出中…' : '批准'}
                </button>
                <button
                  type="button"
                  onClick={() => void decide(card, 'deny')}
                  disabled={!!inFlight}
                  data-testid="approval-deny"
                  className="flex-1 flex items-center justify-center gap-1.5 bg-spectyn-card border border-spectyn-border text-spectyn-text rounded-lg py-2 text-sm hover:bg-spectyn-bg disabled:opacity-40 disabled:cursor-not-allowed"
                >
                  <Ban size={15} />
                  {inFlight === 'deny' ? '送出中…' : '拒絕'}
                </button>
                <button
                  type="button"
                  onClick={() => void decide(card, 'stop')}
                  disabled={!!inFlight}
                  data-testid="approval-stop"
                  className="flex-1 flex items-center justify-center gap-1.5 bg-spectyn-card border border-spectyn-danger/50 text-spectyn-danger rounded-lg py-2 text-sm hover:bg-spectyn-danger/10 disabled:opacity-40 disabled:cursor-not-allowed"
                >
                  <OctagonX size={15} />
                  {inFlight === 'stop' ? '送出中…' : '停止'}
                </button>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
