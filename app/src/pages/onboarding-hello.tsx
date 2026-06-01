// SPEC-31 onboarding-hello — route /onboarding; commands_used: onboarding_* (via lib/onboardingFsm.ts)

import { useCallback, useEffect, useRef, useState } from "react";
import {
  ArrowLeft,
  ArrowRight,
  Check,
  KeyRound,
  Loader2,
  Network,
  Plus,
  Send,
  Sparkles,
  Zap,
} from "lucide-react";
import {
  advanceOnboarding,
  FORWARD_ORDER,
  loadSnapshot,
  patchContext,
  rollbackOnboarding,
} from "../lib/onboardingFsm";
import type { OnboardingState } from "../lib/generated/tauri/OnboardingState";
import { useHaptics } from "../lib/useHaptics";

// Step order comes straight from the FSM's exported FORWARD_ORDER (the single
// source of truth, mirroring SPEC-28 §7.1) — no parallel/redeclared list that
// could silently drift. Used only to map a state → indicator position.
const STEP_ORDER = FORWARD_ORDER;

type StepCopy = {
  state: OnboardingState;
  zhTitle: string;
  enTitle: string;
  zhBody: string;
  enBody: string;
  icon: typeof KeyRound;
  // When set, the backend for this step is not yet wired end-to-end; we say so
  // honestly rather than implying it fully ran.
  backendReady: boolean;
};

const STEPS: Record<OnboardingState, StepCopy> = {
  fresh_install: {
    state: "fresh_install",
    zhTitle: "歡迎使用 Phantom Mesh",
    enTitle: "Welcome to Phantom Mesh",
    zhBody:
      "30 秒完成設定。你的身分金鑰只存在這台裝置，永不上雲。準備好就開始。",
    enBody:
      "Set up in 30 seconds. Your identity key stays on this device and never leaves it. Tap continue to begin.",
    icon: Sparkles,
    backendReady: true,
  },
  picked_language: {
    state: "picked_language",
    zhTitle: "語言已選定",
    enTitle: "Language selected",
    zhBody:
      "介面語言會跟隨你的系統設定（繁體中文 / English）。你隨時可在「設定」中更改。",
    enBody:
      "The interface follows your system language (zh-TW / English). You can change it anytime in Settings.",
    icon: Sparkles,
    backendReady: true,
  },
  created_identity: {
    state: "created_identity",
    zhTitle: "建立身分金鑰",
    enTitle: "Create your identity key",
    zhBody:
      "產生一組裝置本機的 ed25519 金鑰當作你的身分，儲存在系統 Keychain，絕不上傳。",
    enBody:
      "Generate a local ed25519 key as your identity, stored in the system Keychain — never uploaded.",
    icon: KeyRound,
    // identity_init wiring is deferred; advancing past this is a client-side
    // bump today (see onboardingFsm soft-fail handling).
    backendReady: false,
  },
  joined_cluster: {
    state: "joined_cluster",
    zhTitle: "加入叢集",
    enTitle: "Join a cluster",
    zhBody:
      "可加入區網內既有叢集，或先以單機模式執行。之後仍可在「設定」中加入。",
    enBody:
      "Join an existing cluster on your network, or start single-machine for now. You can join later in Settings.",
    icon: Network,
    backendReady: false,
  },
  set_provider: {
    state: "set_provider",
    zhTitle: "設定模型供應商",
    enTitle: "Set up a model provider",
    zhBody:
      "設定至少一個模型供應商，或使用 demo-relay（30 秒免設定）先試用。",
    enBody:
      "Configure at least one model provider, or use the demo-relay (zero setup) to try it now.",
    icon: Plus,
    backendReady: false,
  },
  first_reply_received: {
    state: "first_reply_received",
    zhTitle: "一切就緒",
    enTitle: "You are all set",
    zhBody:
      "設定完成。送出第一則訊息，回覆會串流顯示於主對話視窗。",
    enBody:
      "Setup complete. Send your first message and the reply streams into the main chat.",
    icon: Send,
    backendReady: true,
  },
};

function stepIndex(state: OnboardingState): number {
  const idx = STEP_ORDER.indexOf(state);
  return idx < 0 ? 0 : idx;
}

export default function OnboardingHello() {
  // Derive the initial state from the persisted FSM snapshot directly. We do
  // NOT freeze a prop in a useState initializer — there are no props here; the
  // FSM (localStorage) is the source of truth and is read synchronously.
  const [current, setCurrent] = useState<OnboardingState>(
    () => loadSnapshot().currentState,
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [softNote, setSoftNote] = useState<string | null>(null);
  const [finished, setFinished] = useState(false);

  const { impact } = useHaptics();

  // Async safety: a sequence counter + alive ref so only the latest live
  // transition commits state. Any in-flight advance/rollback whose seq no
  // longer matches `seqRef.current`, or that resolves after unmount, is dropped.
  const seqRef = useRef(0);
  const aliveRef = useRef(true);

  useEffect(() => {
    aliveRef.current = true;
    return () => {
      aliveRef.current = false;
    };
  }, []);

  const idx = stepIndex(current);
  const total = STEP_ORDER.length;
  const isLast = idx === total - 1;
  const step = STEPS[current];
  const StepIcon = step.icon;

  // Rollback is only sanctioned on the joined_cluster → created_identity edge
  // (SPEC-28 §7.1). On every other step "Back" is intentionally unavailable.
  const canGoBack = current === "joined_cluster";

  const runTransition = useCallback(
    async (
      kind: "advance" | "rollback",
      ctxPatch?: Parameters<typeof patchContext>[0],
    ) => {
      // Clear prior result at load start so a stale message can never linger
      // on screen across a transition.
      const mySeq = ++seqRef.current;
      setBusy(true);
      setError(null);
      setSoftNote(null);

      try {
        const result =
          kind === "advance"
            ? await advanceOnboarding(ctxPatch ?? {})
            : await rollbackOnboarding();

        // Drop if superseded or unmounted.
        if (!aliveRef.current || mySeq !== seqRef.current) return;

        setCurrent(result.state);
        if (result.softFailed && result.errorMessage) {
          // Honest: the backend for this step is deferred; we moved the FSM
          // client-side. Distinct from a thrown error (handled in catch).
          setSoftNote(
            "此步驟的後端尚未接好，已先在本機推進（State advanced locally; backend not yet wired）。",
          );
        }
        if (kind === "advance" && result.state === "first_reply_received") {
          impact("medium");
        }
      } catch (e) {
        if (!aliveRef.current || mySeq !== seqRef.current) return;
        // A thrown/rejected backend call → real error state (NOT an empty/soft
        // state). The FSM stays put; the user can retry.
        const message = e instanceof Error ? e.message : String(e);
        setError(message);
      } finally {
        if (aliveRef.current && mySeq === seqRef.current) {
          setBusy(false);
        }
      }
    },
    [impact],
  );

  const handleContinue = useCallback(() => {
    if (busy) return;
    if (isLast) {
      // Final step: mark onboarding done locally and hand off to the app.
      // (Router wiring is out of scope; we surface a done state.)
      patchContext({});
      setFinished(true);
      impact("medium");
      return;
    }
    // Attach the relevant context patch as the user finishes each step.
    const ctxPatch =
      current === "set_provider" ? { demoRelayUsed: false } : {};
    void runTransition("advance", ctxPatch);
  }, [busy, current, impact, isLast, runTransition]);

  const handleBack = useCallback(() => {
    if (busy || !canGoBack) return;
    void runTransition("rollback");
  }, [busy, canGoBack, runTransition]);

  return (
    <div
      data-testid="onboarding-hello"
      className="min-h-screen bg-phantom-bg text-phantom-text pt-[env(safe-area-inset-top)] pl-[env(safe-area-inset-left)] pr-[env(safe-area-inset-right)]"
    >
      <div className="flex min-h-screen flex-col">
        <main className="flex-1 overflow-y-auto px-5 pb-6 pt-6">
          {/* Step indicator — DOM order matches visual order. */}
          <nav
            aria-label="設定進度 / Setup progress"
            className="mb-6 flex items-center justify-center gap-2"
          >
            {STEP_ORDER.map((s, i) => {
              const reached = i <= idx;
              const isCurrent = i === idx;
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
            步驟 {idx + 1} / {total} · Step {idx + 1} of {total}
          </p>

          <header className="mb-6 flex items-center gap-3">
            <div className="flex min-h-[44px] min-w-[44px] items-center justify-center rounded-lg bg-phantom-primary text-phantom-bg">
              <StepIcon aria-hidden="true" size={22} />
            </div>
            <div>
              <h1 className="text-2xl font-semibold text-phantom-text">
                {step.zhTitle}
              </h1>
              <p className="mt-1 text-base text-phantom-muted">
                {step.enTitle}
              </p>
            </div>
          </header>

          <section className="rounded-lg border border-phantom-border bg-phantom-card p-4">
            <p className="text-lg leading-7 text-phantom-text">{step.zhBody}</p>
            <p className="mt-3 text-base leading-6 text-phantom-muted">
              {step.enBody}
            </p>
          </section>

          {/* Honest disclosure: this step's backend isn't fully wired yet. */}
          {!step.backendReady && !finished ? (
            <p
              role="status"
              className="mt-4 flex items-start gap-2 rounded-lg border border-phantom-warning/40 bg-phantom-warning/10 p-3 text-base text-phantom-warning"
            >
              <Zap aria-hidden="true" size={18} className="mt-0.5 shrink-0" />
              <span>
                此步驟的後端仍在開發中，繼續會先在本機完成。 / Backend for this
                step is still in progress; continuing completes it locally.
              </span>
            </p>
          ) : null}

          {/* Real backend error (a rejected/thrown call). */}
          {error ? (
            <p
              role="alert"
              className="mt-4 rounded-lg border border-phantom-danger/40 bg-phantom-danger/10 p-3 text-base text-phantom-danger"
            >
              無法完成此步驟 / Could not complete this step：{error}
            </p>
          ) : null}

          {/* Soft fallback note (backend deferred, advanced client-side). */}
          {softNote && !error ? (
            <p
              role="status"
              className="mt-4 rounded-lg border border-phantom-border bg-phantom-card p-3 text-base text-phantom-muted"
            >
              {softNote}
            </p>
          ) : null}

          {/* Completion state. */}
          {finished ? (
            <p
              role="status"
              className="mt-4 flex items-center gap-2 rounded-lg border border-phantom-success/40 bg-phantom-success/10 p-3 text-base text-phantom-success"
            >
              <Check aria-hidden="true" size={18} />
              設定完成，開始使用 / Setup complete — you are ready to go.
            </p>
          ) : null}
        </main>

        <footer className="sticky bottom-0 flex items-center gap-3 border-t border-phantom-border bg-phantom-bg/95 px-5 pt-4 pb-[max(0.75rem,env(safe-area-inset-bottom))] backdrop-blur">
          {canGoBack ? (
            <button
              type="button"
              onClick={handleBack}
              disabled={busy || finished}
              aria-label="返回 / Back"
              className="flex min-h-[48px] items-center justify-center gap-2 rounded-lg border border-phantom-border px-4 py-3 text-base font-medium text-phantom-text transition disabled:opacity-60 motion-reduce:transition-none"
            >
              <ArrowLeft aria-hidden="true" size={20} />
              返回 / Back
            </button>
          ) : null}

          <button
            type="button"
            onClick={handleContinue}
            disabled={busy || finished}
            aria-label={
              isLast ? "完成 / Finish" : "繼續 / Continue"
            }
            className="flex min-h-[48px] flex-1 items-center justify-center gap-2 rounded-lg bg-phantom-primary px-4 py-3 text-base font-semibold text-phantom-bg transition disabled:opacity-60 motion-reduce:transition-none"
          >
            {busy ? (
              <Loader2
                aria-hidden="true"
                size={20}
                className="animate-spin motion-reduce:animate-none"
              />
            ) : isLast ? (
              <Check aria-hidden="true" size={20} />
            ) : (
              <ArrowRight aria-hidden="true" size={20} />
            )}
            {busy
              ? "處理中 / Working"
              : finished
                ? "已完成 / Done"
                : isLast
                  ? "完成 / Finish"
                  : "繼續 / Continue"}
          </button>
        </footer>
      </div>
    </div>
  );
}
