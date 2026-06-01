// SPEC-31 onboarding-join — route /onboarding/join; commands_used: get_cluster_peers (via useClusterPeers) + onboarding_advance (via lib/onboardingFsm advanceOnboarding); join transition is soft-success when onboarding backend unwired.
//
// Onboarding "join a cluster" step. Lists discovered peers from the F101
// `useClusterPeers` hook (same Online→green / Unhealthy→red / Unknown→muted
// mapping as mesh-peer-list), lets the user pick one and join — or skip for
// now. Joining records the picked peer in the onboarding context
// (patchContext) and advances the FSM (advanceOnboarding); when SPEC-28
// Stage 3 is still deferred the wire layer surfaces it as a soft-success and
// we say so honestly rather than implying a real join ran.
//
// Reuse-only: FORWARD_ORDER / advanceOnboarding / patchContext
// from lib/onboardingFsm (single source of truth — step order is NOT
// redeclared here). useClusterPeers owns its own async/seq/visibility safety;
// the local seq + alive guard here protects only the join transition.

import { useCallback, useEffect, useRef, useState } from "react";
import {
  ArrowLeft,
  ArrowRight,
  Check,
  Loader2,
  Network,
  RefreshCw,
} from "lucide-react";
import { useClusterPeers } from "../hooks/useClusterPeers";
import {
  advanceOnboarding,
  FORWARD_ORDER,
  patchContext,
} from "../lib/onboardingFsm";
import { useHaptics } from "../lib/useHaptics";

interface OnboardingJoinProps {
  onContinue?: () => void;
  onBack?: () => void;
}

/** This onboarding step's position in the FSM forward order (SPEC-28 §7.1).
 *  Derived from the single source of truth — never a redeclared list. Used
 *  only for the "step N of M" indicator. */
const JOIN_STEP_INDEX = FORWARD_ORDER.indexOf("joined_cluster");
const TOTAL_STEPS = FORWARD_ORDER.length;

/** Format a unix-seconds timestamp as a short bilingual relative time.
 *  Mirrors mesh-peer-list so the two surfaces read consistently. */
function relTime(unixSecs: number): string {
  if (!unixSecs) return "—";
  const secs = Math.max(0, Math.floor(Date.now() / 1000 - unixSecs));
  if (secs < 60) return `${secs} 秒前 / ${secs}s ago`;
  if (secs < 3600) return `${Math.floor(secs / 60)} 分前 / ${Math.floor(secs / 60)}m ago`;
  if (secs < 86400) return `${Math.floor(secs / 3600)} 小時前 / ${Math.floor(secs / 3600)}h ago`;
  return `${Math.floor(secs / 86400)} 天前 / ${Math.floor(secs / 86400)}d ago`;
}

/** PeerStatus union has no "Healthy": Online→green, Unhealthy→red, Unknown→muted.
 *  Identical mapping to mesh-peer-list. */
function dotColor(peerStatus: string): string {
  return peerStatus === "Online"
    ? "bg-phantom-success"
    : peerStatus === "Unhealthy"
      ? "bg-phantom-danger"
      : "bg-phantom-muted";
}

function statusLabel(peerStatus: string): string {
  return peerStatus === "Online"
    ? "在線 Online"
    : peerStatus === "Unhealthy"
      ? "降級 Unhealthy"
      : "未知 Unknown";
}

export default function OnboardingJoin({ onContinue, onBack }: OnboardingJoinProps) {
  const { peers, status, error: peersError, lastSyncMs, refresh } = useClusterPeers();
  const { impact } = useHaptics();

  // Which peer the user has tapped. Derived selection lives in local state
  // (not a frozen prop — there is no list prop to freeze).
  const [pickedPeerId, setPickedPeerId] = useState<string | null>(null);

  // Join-transition state (separate from the peer-list load, which the hook
  // owns). `joinError` is a thrown/rejected backend call; `softNote` is the
  // honest "backend deferred, advanced locally" case; `joined` is success.
  const [busy, setBusy] = useState(false);
  const [joinError, setJoinError] = useState<string | null>(null);
  const [softNote, setSoftNote] = useState<string | null>(null);
  const [joined, setJoined] = useState(false);

  // Async safety for the join transition: a seq counter + alive ref so only
  // the latest live call commits, and nothing sets state after unmount.
  const seqRef = useRef(0);
  const aliveRef = useRef(true);
  useEffect(() => {
    aliveRef.current = true;
    return () => {
      aliveRef.current = false;
    };
  }, []);

  const isLoading = status === "loading";

  const handleRefresh = useCallback(() => {
    void refresh();
  }, [refresh]);

  const handlePick = useCallback(
    (peerId: string) => {
      if (busy || joined) return;
      // Toggle: tapping the selected peer again clears the choice.
      setPickedPeerId((prev) => (prev === peerId ? null : peerId));
      setJoinError(null);
    },
    [busy, joined],
  );

  // Advance the FSM. `joining` true → record the picked peer; false → skip.
  const runJoin = useCallback(
    async (joining: boolean) => {
      if (busy) return;
      // Clear prior result at start so a stale message never lingers across
      // a transition.
      const mySeq = ++seqRef.current;
      setBusy(true);
      setJoinError(null);
      setSoftNote(null);

      // Record the chosen peer (or explicit single-machine) before advancing,
      // so the persisted onboarding context reflects the user's decision even
      // if the backend transition is still soft.
      const picked = joining ? peers.find((p) => p.peer_id === pickedPeerId) : undefined;
      patchContext({
        clusterIdHash: joining && picked ? picked.peer_id : null,
      });

      try {
        const result = await advanceOnboarding();
        if (!aliveRef.current || mySeq !== seqRef.current) return;
        setJoined(true);
        impact("medium");
        if (result.softFailed && result.errorMessage) {
          setSoftNote(
            joining
              ? "已記住你的選擇，但加入叢集的後端尚未接好，已先在本機推進。 / Choice saved, but the join backend is not yet wired — advanced locally."
              : "已先在本機推進（後端尚未接好）。 / Advanced locally (backend not yet wired).",
          );
        }
      } catch (e) {
        if (!aliveRef.current || mySeq !== seqRef.current) return;
        // A thrown/rejected backend call → real error state (distinct from the
        // honest empty/soft cases). The FSM stays put; the user can retry.
        const message = e instanceof Error ? e.message : String(e);
        setJoinError(message);
      } finally {
        if (aliveRef.current && mySeq === seqRef.current) {
          setBusy(false);
        }
      }
    },
    [busy, peers, pickedPeerId, impact],
  );

  // Primary CTA. After a successful (or soft) join, the same button hands off
  // to the next screen via onContinue (router wiring is out of scope here).
  const handlePrimary = useCallback(() => {
    if (busy) return;
    if (joined) {
      onContinue?.();
      return;
    }
    void runJoin(pickedPeerId !== null);
  }, [busy, joined, onContinue, pickedPeerId, runJoin]);

  const handleBack = useCallback(() => {
    if (busy) return;
    onBack?.();
  }, [busy, onBack]);

  // Honest empty: the load finished (not loading) with no error and no peers.
  const showEmpty = !isLoading && !peersError && peers.length === 0;
  // Honest loading: first load, nothing to show yet.
  const showLoading = isLoading && peers.length === 0 && !peersError;

  const primaryLabel = joined
    ? "繼續 / Continue"
    : pickedPeerId !== null
      ? "加入並繼續 / Join and continue"
      : "稍後再加入 / Skip for now";
  const primaryAria = joined
    ? "繼續 / Continue"
    : pickedPeerId !== null
      ? "加入選定節點並繼續 / Join the selected peer and continue"
      : "略過加入叢集，稍後再加入 / Skip joining a cluster for now";

  const stepNum = JOIN_STEP_INDEX < 0 ? 1 : JOIN_STEP_INDEX + 1;

  return (
    <div
      data-testid="onboarding-join"
      className="min-h-screen bg-phantom-bg text-phantom-text
        pt-[env(safe-area-inset-top)]
        pl-[env(safe-area-inset-left)] pr-[env(safe-area-inset-right)]"
    >
      <div className="flex min-h-screen flex-col">
        <main className="flex-1 overflow-y-auto px-5 pb-6 pt-6">
          {/* Step indicator copy. */}
          <p role="status" className="mb-4 text-center text-base text-phantom-muted">
            步驟 {stepNum} / {TOTAL_STEPS} · Step {stepNum} of {TOTAL_STEPS}
          </p>

          <header className="mb-5 flex items-center gap-3">
            <div className="flex min-h-[44px] min-w-[44px] items-center justify-center rounded-lg bg-phantom-primary text-phantom-bg">
              <Network aria-hidden="true" size={22} />
            </div>
            <div className="min-w-0">
              <h1 className="text-2xl font-semibold text-phantom-text">加入叢集</h1>
              <p className="mt-1 text-base text-phantom-muted">Join a cluster</p>
            </div>
          </header>

          <section className="mb-5 rounded-lg border border-phantom-border bg-phantom-card p-4">
            <p className="text-lg leading-7 text-phantom-text">
              選擇一個區網內發現的節點加入，或先以單機模式開始 — 之後仍可在「設定」中加入。
            </p>
            <p className="mt-3 text-base leading-6 text-phantom-muted">
              Pick a peer discovered on your network to join, or start single-machine for
              now — you can join later in Settings.
            </p>
          </section>

          {/* Discovered-peers section header + refresh. */}
          <div className="mb-2 flex items-center justify-between gap-3">
            <h2 className="text-base font-medium text-phantom-text">
              發現的節點 / Discovered peers
            </h2>
            <button
              type="button"
              onClick={handleRefresh}
              disabled={isLoading || busy || joined}
              aria-label="重新整理發現的節點 / Refresh discovered peers"
              className="flex min-h-[44px] items-center gap-1.5 rounded-lg border border-phantom-border
                bg-phantom-card px-3 text-base text-phantom-text transition
                hover:border-phantom-primary/40 disabled:opacity-60 motion-reduce:transition-none"
            >
              <RefreshCw
                size={16}
                aria-hidden="true"
                className={isLoading ? "animate-spin motion-reduce:animate-none" : ""}
              />
              <span>重新整理 / Refresh</span>
            </button>
          </div>

          {/* Peer-list load error (thrown/rejected — distinct from empty). */}
          {peersError ? (
            <div
              role="alert"
              className="mb-3 rounded-lg border border-phantom-danger/40 bg-phantom-danger/10 p-3 text-base text-phantom-danger"
            >
              無法搜尋節點：{peersError}
              <span className="mt-1 block text-sm opacity-80">
                Could not discover peers. Try refresh.
              </span>
            </div>
          ) : null}

          {/* Loading — first load, no data yet. */}
          {showLoading ? (
            <div
              role="status"
              className="flex min-h-[44px] items-center justify-center gap-2 py-8 text-base text-phantom-muted"
            >
              <Loader2 size={18} aria-hidden="true" className="animate-spin motion-reduce:animate-none" />
              搜尋區網節點中… / Discovering peers…
            </div>
          ) : null}

          {/* Honest empty — load finished, genuinely no peers. */}
          {showEmpty ? (
            <div className="rounded-lg border border-phantom-border bg-phantom-card p-6 text-center">
              <p className="text-base text-phantom-text">尚未發現節點 / No peers discovered</p>
              <p className="mt-1.5 text-sm text-phantom-muted">
                你可以重新整理再試，或先以單機模式繼續。
                <span className="mt-0.5 block">
                  Try refresh, or continue single-machine for now.
                </span>
              </p>
            </div>
          ) : null}

          {/* Peer rows — selectable. DOM order matches visual order. */}
          {peers.length > 0 ? (
            <ul className="space-y-2" aria-label="可加入的節點清單 / Joinable peers">
              {peers.map((p) => {
                const selected = p.peer_id === pickedPeerId;
                return (
                  <li key={p.peer_id}>
                    <button
                      type="button"
                      onClick={() => handlePick(p.peer_id)}
                      disabled={busy || joined}
                      aria-pressed={selected}
                      aria-label={`${selected ? "已選 / selected " : ""}${p.display_name || p.peer_id}，狀態 ${statusLabel(
                        p.status,
                      )} / status ${p.status}`}
                      className={`flex w-full min-h-[48px] items-center gap-3 rounded-lg border px-3 py-2.5 text-left transition
                        disabled:opacity-60 motion-reduce:transition-none ${
                          selected
                            ? "border-phantom-primary bg-phantom-primary/10"
                            : "border-phantom-border bg-phantom-card hover:border-phantom-primary/40"
                        }`}
                    >
                      <span
                        className={`inline-block h-3 w-3 flex-shrink-0 rounded-full ${dotColor(p.status)}`}
                        aria-hidden="true"
                      />
                      <span className="min-w-0 flex-1">
                        <span className="block truncate text-base text-phantom-text">
                          {p.display_name || p.peer_id}
                        </span>
                        <span className="mt-0.5 block text-sm text-phantom-muted">
                          {statusLabel(p.status)} · {relTime(p.last_seen_unix)}
                        </span>
                      </span>
                      {selected ? (
                        <Check
                          size={20}
                          aria-hidden="true"
                          className="flex-shrink-0 text-phantom-primary"
                        />
                      ) : null}
                    </button>
                  </li>
                );
              })}
            </ul>
          ) : null}

          {/* Last-sync footer line. */}
          {peers.length > 0 || lastSyncMs > 0 ? (
            <p className="pt-3 text-sm text-phantom-muted">
              {lastSyncMs > 0
                ? `最後同步 / Last sync：${relTime(Math.floor(lastSyncMs / 1000))}`
                : "尚未同步 / Not synced yet"}
            </p>
          ) : null}

          {/* Real join error (a rejected/thrown advance call). */}
          {joinError ? (
            <p
              role="alert"
              className="mt-4 rounded-lg border border-phantom-danger/40 bg-phantom-danger/10 p-3 text-base text-phantom-danger"
            >
              無法完成此步驟 / Could not complete this step：{joinError}
            </p>
          ) : null}

          {/* Honest soft note — backend deferred, advanced client-side. */}
          {softNote && !joinError ? (
            <p
              role="status"
              className="mt-4 rounded-lg border border-phantom-border bg-phantom-card p-3 text-base text-phantom-muted"
            >
              {softNote}
            </p>
          ) : null}

          {/* Success — only the confirmed (non-soft) path shows the green banner;
              when the backend was unwired the honest softNote above stands alone. */}
          {joined && !softNote ? (
            <p
              role="status"
              className="mt-4 flex items-center gap-2 rounded-lg border border-phantom-success/40 bg-phantom-success/10 p-3 text-base text-phantom-success"
            >
              <Check aria-hidden="true" size={18} />
              {pickedPeerId !== null
                ? "已加入叢集 / Joined the cluster"
                : "已略過，採單機模式 / Skipped — single-machine"}
            </p>
          ) : null}
        </main>

        {/* Sticky-bottom reachability footer. */}
        <footer className="sticky bottom-0 flex items-center gap-3 border-t border-phantom-border bg-phantom-bg/95 px-5 pt-4 pb-[max(0.75rem,env(safe-area-inset-bottom))] backdrop-blur">
          <button
            type="button"
            onClick={handleBack}
            disabled={busy}
            aria-label="返回 / Back"
            className="flex min-h-[48px] items-center justify-center gap-2 rounded-lg border border-phantom-border px-4 py-3 text-base font-medium text-phantom-text transition disabled:opacity-60 motion-reduce:transition-none"
          >
            <ArrowLeft aria-hidden="true" size={20} />
            返回 / Back
          </button>

          <button
            type="button"
            onClick={handlePrimary}
            disabled={busy}
            aria-label={primaryAria}
            className="flex min-h-[48px] flex-1 items-center justify-center gap-2 rounded-lg bg-phantom-primary px-4 py-3 text-base font-semibold text-phantom-bg transition disabled:opacity-60 motion-reduce:transition-none"
          >
            {busy ? (
              <Loader2 aria-hidden="true" size={20} className="animate-spin motion-reduce:animate-none" />
            ) : joined ? (
              <ArrowRight aria-hidden="true" size={20} />
            ) : pickedPeerId !== null ? (
              <Check aria-hidden="true" size={20} />
            ) : (
              <ArrowRight aria-hidden="true" size={20} />
            )}
            {busy ? "處理中 / Working" : primaryLabel}
          </button>
        </footer>
      </div>
    </div>
  );
}
