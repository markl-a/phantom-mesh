// MobileOnboardingV2 — full-screen first-launch onboarding.
//
// Replaces the "open into chat then dig into Settings to log in" flow.
// First launch with no AuthState shows this single screen with one big
// "Sign in with Google" button. After OAuth completes via Safari → deep
// link → broker_login_finish → broker_sync_from_vault, the listener
// auto-dismisses the screen and the app drops into the normal chat UI.

import { useEffect, useState } from "react";
import {
  startBrokerLogin,
  loadBrokerLoginStatus,
  syncFromVault,
  onBrokerLoginResult,
  type BrokerLoginFinishResponse,
  type BrokerSyncResponse,
} from "../../lib/brokerLogin";
// Wire helper around the Wave H1.1 Tauri commands `onboarding_advance`
// + `onboarding_rollback` (registered in src-tauri/src/lib.rs:572-573).
import { advanceUntil } from "../../lib/onboardingFsm";

type Phase =
  | { kind: "checking" }
  | { kind: "needs-login" }
  | { kind: "redirecting" }
  | { kind: "syncing"; identity: BrokerLoginFinishResponse }
  | { kind: "complete"; identity: BrokerLoginFinishResponse; sync: BrokerSyncResponse }
  | { kind: "error"; message: string };

interface Props {
  onReady: () => void; // signals to App.tsx that onboarding is complete
}

export default function MobileOnboardingV2({ onReady }: Props) {
  const [phase, setPhase] = useState<Phase>({ kind: "checking" });
  const [advanceWarning, setAdvanceWarning] = useState<string | null>(null);

  // Drive the SPEC-28 FSM forward up to `first_reply_received` (terminal
  // state) and then signal the parent it can swap to the main chat UI.
  // We swallow real backend errors here too (best-effort): once the user
  // has reached the "ready" point in V2 there is no UI affordance to
  // recover an FSM advance failure — surfacing it would only confuse.
  // The helper itself already silences the `not_yet_wired` stage-3 path.
  const completeOnboarding = async (
    ctxPatch: Parameters<typeof advanceUntil>[1] = {},
  ) => {
    try {
      await advanceUntil("first_reply_received", ctxPatch);
    } catch (e) {
      setAdvanceWarning(e instanceof Error ? e.message : String(e));
      // eslint-disable-next-line no-console
      console.warn("[MobileOnboardingV2] advanceUntil failed (non-fatal)", e);
    }
    onReady();
  };

  // Step 1: on mount, check if user already has a working AuthState.
  useEffect(() => {
    let cancelled = false;
    loadBrokerLoginStatus()
      .then((status) => {
        if (cancelled) return;
        if (status && status.broker_token_expires_at_ms > Date.now()) {
          // Already logged in + token still valid → fast-forward.
          // Mark provider as broker-vault sourced so downstream knows.
          void completeOnboarding({ providerSlug: "broker_vault" });
        } else {
          setPhase({ kind: "needs-login" });
        }
      })
      .catch(() => {
        if (cancelled) return;
        setPhase({ kind: "needs-login" });
      });
    return () => { cancelled = true; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [onReady]);

  // Step 2: subscribe to deep-link OAuth callback so the screen advances
  // the moment Safari hands the broker payload back.
  useEffect(() => {
    const off = onBrokerLoginResult(async (result) => {
      if (!result.ok) {
        setPhase({ kind: "error", message: result.error });
        return;
      }
      // result.identity is set; sync may or may not be present (best-
      // effort in installBrokerLoginBridge). If missing, run it here.
      if (result.sync) {
        setPhase({ kind: "complete", identity: result.identity, sync: result.sync });
        // Auto-advance to chat after a brief "✓ done" pause.
        setTimeout(() => {
          void completeOnboarding({ providerSlug: "broker_vault" });
        }, 1500);
      } else {
        setPhase({ kind: "syncing", identity: result.identity });
        try {
          const sync = await syncFromVault();
          setPhase({ kind: "complete", identity: result.identity, sync });
          setTimeout(() => {
            void completeOnboarding({ providerSlug: "broker_vault" });
          }, 1500);
        } catch (e) {
          setPhase({ kind: "error", message: `vault sync failed: ${String(e)}` });
        }
      }
    });
    return off;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [onReady]);

  const handleSignIn = async () => {
    setPhase({ kind: "redirecting" });
    try {
      await startBrokerLogin("https://phantommesh.io");
      // Listener picks it up from here.
    } catch (e) {
      setPhase({ kind: "error", message: String(e) });
    }
  };

  return (
    <div className="min-h-screen bg-spectyn-bg text-spectyn-text flex flex-col items-center justify-center px-6 py-8">
      <div className="text-7xl text-spectyn-primary mb-4">◆</div>
      <h1 className="text-2xl font-medium mb-2">Spectyn Mesh</h1>
      <p className="text-spectyn-muted text-sm text-center mb-8 leading-relaxed max-w-sm">
        登入後自動同步你 phantommesh.io 帳號裡的
        <br />LLM API keys + cluster peers + 設定到本機。
      </p>

      <div className="w-full max-w-sm space-y-3">
        {phase.kind === "checking" && (
          <div className="text-center text-spectyn-muted text-sm py-8">讀取登入狀態 …</div>
        )}

        {phase.kind === "needs-login" && (
          <>
            <button
              onClick={handleSignIn}
              className="w-full bg-spectyn-primary text-spectyn-bg font-semibold px-6 py-4 rounded-xl active:opacity-80 text-base"
            >
              以 phantommesh.io 登入（Google / Apple）
            </button>
            <button
              onClick={() => { void completeOnboarding(); }}
              className="w-full text-spectyn-muted text-xs underline mt-2"
            >
              先不登入 — 稍後再從設定填 API key
            </button>
          </>
        )}

        {phase.kind === "redirecting" && (
          <div className="space-y-3 text-center">
            <div className="text-sm text-spectyn-text">已開啟 Safari …</div>
            <div className="text-xs text-spectyn-muted leading-relaxed">
              完成 Google 登入後 Safari 會自動跳「在 Spectyn Mesh 中開啟？」對話框，按 Open 即可。
            </div>
            <button
              onClick={() => setPhase({ kind: "needs-login" })}
              className="text-xs text-spectyn-muted underline"
            >
              取消登入
            </button>
          </div>
        )}

        {phase.kind === "syncing" && (
          <div className="space-y-2 text-center">
            <div className="text-sm text-spectyn-text">登入成功：{phase.identity.email}</div>
            <div className="text-xs text-spectyn-muted">正在同步 LLM keys + cluster peers …</div>
          </div>
        )}

        {phase.kind === "complete" && (
          <div className="space-y-2 text-center">
            <div className="text-3xl text-emerald-400">✓</div>
            <div className="text-sm text-spectyn-text">已同步 {phase.sync.keys_written.length} 個 LLM key</div>
            <div className="text-xs text-spectyn-muted">
              cluster peers: {phase.sync.peers_count}
            </div>
            <div className="text-xs text-spectyn-muted mt-2">即將進入對話 …</div>
          </div>
        )}

        {phase.kind === "error" && (
          <div className="space-y-3 text-center">
            <div className="text-sm text-red-400 break-words">登入失敗：{phase.message}</div>
            <button
              onClick={() => setPhase({ kind: "needs-login" })}
              className="w-full bg-spectyn-card border border-spectyn-border text-spectyn-text px-4 py-2.5 rounded-lg active:opacity-80 text-sm"
            >
              重試
            </button>
            <button
              onClick={() => { void completeOnboarding(); }}
              className="text-xs text-spectyn-muted underline"
            >
              跳過登入，手動填 key
            </button>
          </div>
        )}
      </div>

      {advanceWarning && (
        <div className="mt-4 w-full max-w-sm p-2 bg-amber-500/10 border border-amber-500/30 rounded text-amber-300 text-[11px] break-words">
          狀態同步警告：{advanceWarning}
        </div>
      )}

      <div className="text-[10px] text-spectyn-muted mt-12 text-center max-w-sm">
        ◆ spectyn mesh — 你的多裝置 AI agent
      </div>
    </div>
  );
}
