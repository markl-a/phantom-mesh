// SPEC-31 onboarding-broker — route /onboarding/broker; commands_used: broker_login_start + broker_sync_from_vault (via lib/brokerLogin.ts startBrokerLogin/onBrokerLoginResult); onboarding_advance (via lib/onboardingFsm.ts advanceOnboarding)

import { useCallback, useEffect, useRef, useState } from "react";
import {
  ArrowLeft,
  Check,
  Cloud,
  CloudOff,
  Loader2,
  TriangleAlert,
} from "lucide-react";
import {
  startBrokerLogin,
  onBrokerLoginResult,
  type BrokerLoginFinishResponse,
  type BrokerSyncResponse,
} from "../lib/brokerLogin";
import { advanceOnboarding, FORWARD_ORDER } from "../lib/onboardingFsm";
import { useHaptics } from "../lib/useHaptics";

// Step position is derived from the FSM's single source of truth (FORWARD_ORDER).
// The broker (cloud relay) step maps to "set_provider" — logging into the cloud
// broker（中介伺服器）syncs the user's LLM keys + cluster peers, which is the
// provider-configuration moment in SPEC-28 §7.1. We never redeclare the order.
const BROKER_STEP = "set_provider" as const;
const STEP_INDEX = FORWARD_ORDER.indexOf(BROKER_STEP);
const TOTAL_STEPS = FORWARD_ORDER.length;

type Props = {
  onContinue?: () => void;
  onBack?: () => void;
};

// One of: idle (nothing started), connecting (browser handed off, awaiting the
// phantom://oauth/callback deep-link), done (logged in + advanced), error.
type Phase = "idle" | "connecting" | "done" | "error";

export default function OnboardingBroker({ onContinue, onBack }: Props) {
  // Derive handlers straight from props — never freeze a prop in a useState
  // initializer. Defaults are applied at call sites via `?.()`.
  const [phase, setPhase] = useState<Phase>("idle");
  const [error, setError] = useState<string | null>(null);
  const [identity, setIdentity] = useState<BrokerLoginFinishResponse | null>(
    null,
  );
  const [sync, setSync] = useState<BrokerSyncResponse | null>(null);
  const [softNote, setSoftNote] = useState<string | null>(null);

  const { impact } = useHaptics();

  // Async safety: seq counter + alive ref so only the latest live request
  // commits. The deep-link callback can fire long after the button press
  // (Safari round-trip), and the component may have re-rendered or unmounted.
  const seqRef = useRef(0);
  const aliveRef = useRef(true);
  // True only while a Connect attempt is genuinely awaiting its (async, deep-link)
  // broker result. handleConnect arms it; consuming a result, pressing Skip, or a
  // new attempt clears/re-arms it. This is what makes the global result callback
  // reject a stale round-trip that lands after the user superseded it — a fire-time
  // `seqRef.current` read could not (it always equalled the latest seq).
  const awaitingResultRef = useRef(false);

  useEffect(() => {
    aliveRef.current = true;
    return () => {
      aliveRef.current = false;
    };
  }, []);

  const busy = phase === "connecting";

  // Advance the FSM once a broker identity is confirmed. The set_provider
  // backend transition may itself be deferred (soft-fail); we surface that
  // honestly but still treat it as forward progress.
  // `mySeq` is the seq of the attempt that triggered this advance; every commit
  // after the await is dropped if a Skip/new-Connect superseded it (mySeq mismatch)
  // or the component unmounted (aliveRef).
  const advanceAfterLogin = useCallback(async (mySeq: number) => {
    try {
      const result = await advanceOnboarding({
        demoRelayUsed: false,
        providerSlug: "broker",
      });
      if (!aliveRef.current || mySeq !== seqRef.current) return;
      if (result.softFailed && result.errorMessage) {
        setSoftNote(
          "已連上雲端中介，但推進步驟的後端尚未接好，已先在本機完成（Cloud broker linked; step advanced locally — backend not yet wired）。",
        );
      }
    } catch (e) {
      // FSM advance threw — keep the login result visible but tell the user
      // the step could not be recorded. Distinct from a login error.
      if (!aliveRef.current || mySeq !== seqRef.current) return;
      const message = e instanceof Error ? e.message : String(e);
      setSoftNote(
        `已連上雲端中介，但無法記錄此步驟（Linked, but could not record step）：${message}`,
      );
    }
  }, []);

  // Subscribe to the broker login result for the lifetime of the screen. The
  // bridge (installBrokerLoginBridge) is wired once at app startup; here we
  // only listen. Stale results (superseded seq / unmounted) are dropped.
  useEffect(() => {
    const unsubscribe = onBrokerLoginResult((result) => {
      // Drop the result unless we're alive AND still genuinely awaiting one
      // (Skip / unmount / a finished attempt clear the gate). Consume it
      // synchronously so a duplicate/late deep-link round-trip can't re-commit.
      // NOTE: the brokerLogin listener payload carries NO attempt nonce/device_id
      // (BrokerLoginListener = {ok,identity,sync}|{ok,error}), so we cannot strictly
      // correlate a result to a specific Connect attempt — we accept it for whatever
      // attempt is currently awaiting. This is safe: broker login is idempotent (same
      // device identity), and the alive + skip-clears-gate guards cover the only
      // harmful case (committing after the user moved on). A precise fix needs an
      // attempt-id echoed in the result — requested upstream via outbox.
      if (!aliveRef.current || !awaitingResultRef.current) return;
      awaitingResultRef.current = false;
      // Capture this attempt's seq so every commit after the advanceAfterLogin
      // await is dropped if a Skip/new-Connect superseded it mid-await.
      const mySeq = seqRef.current;
      void (async () => {
        if (result.ok) {
          if (!aliveRef.current || mySeq !== seqRef.current) return;
          setError(null);
          setIdentity(result.identity);
          setSync(result.sync ?? null);
          await advanceAfterLogin(mySeq);
          if (!aliveRef.current || mySeq !== seqRef.current) return;
          setPhase("done");
          impact("medium");
        } else {
          if (!aliveRef.current || mySeq !== seqRef.current) return;
          // A rejected/failed login → real error state (NOT a fake empty/skip).
          setIdentity(null);
          setSync(null);
          setPhase("error");
          setError(result.error);
        }
      })();
    });
    return unsubscribe;
  }, [advanceAfterLogin, impact]);

  const handleConnect = useCallback(() => {
    if (busy) return;
    // Bump seq so any in-flight prior attempt's callback is ignored, arm the
    // awaiting-result gate, and clear prior data at load start.
    seqRef.current += 1;
    const mySeq = seqRef.current;
    awaitingResultRef.current = true;
    setError(null);
    setIdentity(null);
    setSync(null);
    setSoftNote(null);
    setPhase("connecting");
    void (async () => {
      try {
        await startBrokerLogin();
        // Success here only means the browser handoff was triggered. The real
        // result arrives asynchronously via onBrokerLoginResult above, so we
        // stay in "connecting" and wait.
      } catch (e) {
        // Drop the rejection if this attempt was superseded (Skip / new Connect)
        // or the gate was cleared — otherwise it would clobber the skipped flow.
        if (!aliveRef.current || mySeq !== seqRef.current || !awaitingResultRef.current) return;
        // Disarm the gate: the browser handoff failed, so NO deep-link result is
        // coming for this attempt. Leaving it armed would let a late/duplicate
        // global broker result later commit "done" over this error state.
        awaitingResultRef.current = false;
        const message = e instanceof Error ? e.message : String(e);
        setPhase("error");
        setError(message);
      }
    })();
  }, [busy]);

  // Skip = continue single-machine. Honest: no broker linked, demoRelayUsed
  // stays false and no providerSlug is set. We still advance the FSM.
  const handleSkip = useCallback(() => {
    // Skip works even mid-connect (button is NOT disabled while busy) so the user
    // can escape a hung handoff. Clear the awaiting gate FIRST and unconditionally
    // so a late broker result for the superseded Connect is dropped by the callback.
    awaitingResultRef.current = false;
    seqRef.current += 1;
    setError(null);
    setSoftNote(null);
    setPhase("idle");
    void (async () => {
      const mySeq = seqRef.current;
      try {
        const result = await advanceOnboarding({});
        if (!aliveRef.current || mySeq !== seqRef.current) return;
        if (result.softFailed && result.errorMessage) {
          setSoftNote(
            "已略過雲端中介（單機模式），步驟後端尚未接好，已先在本機完成（Skipped; advanced locally — backend not yet wired）。",
          );
        }
        impact("light");
        onContinue?.();
      } catch (e) {
        if (!aliveRef.current || mySeq !== seqRef.current) return;
        const message = e instanceof Error ? e.message : String(e);
        setPhase("error");
        setError(message);
      }
    })();
  }, [busy, impact, onContinue]);

  const handleContinue = useCallback(() => {
    if (busy) return;
    impact("light");
    onContinue?.();
  }, [busy, impact, onContinue]);

  const handleBack = useCallback(() => {
    if (busy) return;
    onBack?.();
  }, [busy, onBack]);

  return (
    <div
      data-testid="onboarding-broker"
      className="min-h-screen bg-phantom-bg text-phantom-text pt-[env(safe-area-inset-top)] pl-[env(safe-area-inset-left)] pr-[env(safe-area-inset-right)]"
    >
      <div className="flex min-h-screen flex-col">
        <main className="flex-1 overflow-y-auto px-5 pb-6 pt-6">
          {/* Step indicator — DOM order matches visual order. */}
          <nav
            aria-label="設定進度 / Setup progress"
            className="mb-6 flex items-center justify-center gap-2"
          >
            {FORWARD_ORDER.map((s, i) => {
              const isCurrent = i === STEP_INDEX;
              const reached = i <= STEP_INDEX;
              return (
                <span
                  key={s}
                  aria-hidden="true"
                  className={`h-2.5 min-h-[10px] rounded-full transition-colors motion-reduce:transition-none ${
                    isCurrent
                      ? "w-6 bg-phantom-primary"
                      : reached
                        ? "w-2.5 bg-phantom-primary/50"
                        : "w-2.5 bg-phantom-border"
                  }`}
                />
              );
            })}
          </nav>
          <p
            role="status"
            className="mb-6 text-center text-base text-phantom-muted"
          >
            步驟 {STEP_INDEX + 1} / {TOTAL_STEPS} · Step {STEP_INDEX + 1} of{" "}
            {TOTAL_STEPS}
          </p>

          <header className="mb-6 flex items-center gap-3">
            <div className="flex min-h-[44px] min-w-[44px] items-center justify-center rounded-lg bg-phantom-primary text-phantom-bg">
              <Cloud aria-hidden="true" size={22} />
            </div>
            <div>
              <h1 className="text-2xl font-semibold text-phantom-text">
                連結雲端中介（選用）
              </h1>
              <p className="mt-1 text-base text-phantom-muted">
                Link a cloud broker (optional)
              </p>
            </div>
          </header>

          <section className="rounded-lg border border-phantom-border bg-phantom-card p-4">
            <p className="text-lg leading-7 text-phantom-text">
              登入雲端中介（relay，中繼伺服器），即可在裝置間同步模型金鑰與叢集節點清單。不想用雲端？直接略過，以單機模式繼續。
            </p>
            <p className="mt-3 text-base leading-6 text-phantom-muted">
              Sign in to the cloud broker (relay) to sync your model keys and
              cluster peers across devices. Prefer to stay local? Skip and
              continue single-machine.
            </p>
          </section>

          {/* Connecting: handed off to the browser, awaiting deep-link callback. */}
          {phase === "connecting" ? (
            <p
              role="status"
              className="mt-4 flex items-start gap-2 rounded-lg border border-phantom-border bg-phantom-card p-3 text-base text-phantom-muted"
            >
              <Loader2
                aria-hidden="true"
                size={18}
                className="mt-0.5 shrink-0 animate-spin motion-reduce:animate-none"
              />
              <span>
                已在瀏覽器開啟登入頁，請完成登入後返回 App。 / Opened the sign-in
                page in your browser — finish there and return to the app.
              </span>
            </p>
          ) : null}

          {/* Success: logged in + (best-effort) synced. */}
          {phase === "done" && identity ? (
            <section
              role="status"
              className="mt-4 rounded-lg border border-phantom-success/40 bg-phantom-success/10 p-3 text-base text-phantom-success"
            >
              <p className="flex items-center gap-2 font-medium">
                <Check aria-hidden="true" size={18} />
                已連結雲端中介 / Cloud broker linked
              </p>
              <p className="mt-1 text-phantom-text">
                {identity.display_name ?? identity.email}（{identity.provider}）
              </p>
              {sync ? (
                <p className="mt-1 text-phantom-muted">
                  已同步 {sync.keys_written.length} 組金鑰、{sync.peers_count}{" "}
                  個叢集節點 / Synced {sync.keys_written.length} keys,{" "}
                  {sync.peers_count} peers
                </p>
              ) : (
                <p className="mt-1 text-phantom-muted">
                  已登入，但保險庫同步尚未完成（可稍後重試）。 / Signed in; vault
                  sync did not complete (you can retry later).
                </p>
              )}
            </section>
          ) : null}

          {/* Soft note: FSM step advanced locally (backend deferred). */}
          {softNote && phase !== "error" ? (
            <p
              role="status"
              className="mt-4 rounded-lg border border-phantom-border bg-phantom-card p-3 text-base text-phantom-muted"
            >
              {softNote}
            </p>
          ) : null}

          {/* Real error: a thrown/rejected login or FSM call. */}
          {phase === "error" && error ? (
            <p
              role="alert"
              className="mt-4 flex items-start gap-2 rounded-lg border border-phantom-danger/40 bg-phantom-danger/10 p-3 text-base text-phantom-danger"
            >
              <TriangleAlert
                aria-hidden="true"
                size={18}
                className="mt-0.5 shrink-0"
              />
              <span>無法連結雲端中介 / Could not link broker：{error}</span>
            </p>
          ) : null}
        </main>

        <footer className="sticky bottom-0 flex flex-col gap-3 border-t border-phantom-border bg-phantom-bg/95 px-5 pt-4 pb-[max(0.75rem,env(safe-area-inset-bottom))] backdrop-blur">
          {phase === "done" ? (
            <button
              type="button"
              onClick={handleContinue}
              disabled={busy}
              aria-label="繼續 / Continue"
              className="flex min-h-[48px] w-full items-center justify-center gap-2 rounded-lg bg-phantom-primary px-4 py-3 text-base font-semibold text-phantom-bg transition disabled:opacity-60 motion-reduce:transition-none"
            >
              <Check aria-hidden="true" size={20} />
              繼續 / Continue
            </button>
          ) : (
            <button
              type="button"
              onClick={handleConnect}
              disabled={busy}
              aria-label="登入雲端中介 / Sign in to cloud broker"
              className="flex min-h-[48px] w-full items-center justify-center gap-2 rounded-lg bg-phantom-primary px-4 py-3 text-base font-semibold text-phantom-bg transition disabled:opacity-60 motion-reduce:transition-none"
            >
              {busy ? (
                <Loader2
                  aria-hidden="true"
                  size={20}
                  className="animate-spin motion-reduce:animate-none"
                />
              ) : (
                <Cloud aria-hidden="true" size={20} />
              )}
              {busy ? "連結中 / Connecting" : "登入雲端中介 / Sign in to broker"}
            </button>
          )}

          <div className="flex items-center gap-3">
            {onBack ? (
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
            ) : null}

            {phase !== "done" ? (
              <button
                type="button"
                onClick={handleSkip}
                aria-label="略過，單機模式 / Skip, single-machine"
                className="flex min-h-[48px] flex-1 items-center justify-center gap-2 rounded-lg border border-phantom-border px-4 py-3 text-base font-medium text-phantom-muted transition disabled:opacity-60 motion-reduce:transition-none"
              >
                <CloudOff aria-hidden="true" size={20} />
                略過（單機）/ Skip
              </button>
            ) : null}
          </div>
        </footer>
      </div>
    </div>
  );
}
