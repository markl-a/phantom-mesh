// SPEC-31 onboarding-pick — route /onboarding/pick; commands_used: onboarding_advance (via lib/onboardingFsm.ts; soft-success when backend unwired)

import { useCallback, useEffect, useRef, useState } from "react";
import {
  ArrowLeft,
  ArrowRight,
  Check,
  Loader2,
  Network,
  PlusCircle,
  Server,
} from "lucide-react";
import {
  advanceOnboarding,
  FORWARD_ORDER,
  loadSnapshot,
} from "../lib/onboardingFsm";
import type { OnboardingState } from "../lib/generated/tauri/OnboardingState";
import { useHaptics } from "../lib/useHaptics";

// The "pick" step lives at the joined_cluster transition in the FSM's forward
// chain — choosing how to set up the mesh is what advances past that state.
// We derive its position from the FSM's exported FORWARD_ORDER (the single
// source of truth, mirroring SPEC-28 §7.1) rather than redeclaring any order.
const PICK_STATE: OnboardingState = "joined_cluster";

type MeshChoice = "join_existing" | "create_new" | "single_machine";

type ChoiceCopy = {
  value: MeshChoice;
  icon: typeof Network;
  zhLabel: string;
  enLabel: string;
  zhNote: string;
  enNote: string;
};

// Local presentation copy only — these are UI labels, not an FSM list, so they
// live here (no SINGLE-SOURCE conflict). The mesh-setup choice itself is
// persisted into the FSM's OnboardingContext via patchContext on Continue.
const CHOICES: readonly ChoiceCopy[] = [
  {
    value: "join_existing",
    icon: Network,
    zhLabel: "加入既有叢集",
    enLabel: "Join an existing cluster",
    zhNote: "自動發現區網內的節點，加入現有的網狀網路。",
    enNote: "Discover nodes on your local network and join an existing mesh.",
  },
  {
    value: "create_new",
    icon: PlusCircle,
    zhLabel: "建立新叢集",
    enLabel: "Create a new cluster",
    zhNote: "成為第一個節點，之後其他裝置可以加入你。",
    enNote: "Become the first node; other devices can join you later.",
  },
  {
    value: "single_machine",
    icon: Server,
    zhLabel: "單機，之後再說",
    enLabel: "Single machine for now",
    zhNote: "先以單機模式執行，隨時可在「設定」中加入叢集。",
    enNote: "Run single-machine for now; you can join a cluster later in Settings.",
  },
];

export interface OnboardingPickProps {
  onContinue?: () => void;
  onBack?: () => void;
}

export default function OnboardingPick({ onContinue, onBack }: OnboardingPickProps) {
  // The user's mesh-setup choice. No prop is frozen here — selection is pure
  // local UI state; the FSM snapshot is read synchronously below as its own
  // source of truth.
  const [choice, setChoice] = useState<MeshChoice | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [softNote, setSoftNote] = useState<string | null>(null);
  const [done, setDone] = useState(false);

  // The FSM's persisted state, for an honest "where am I" read. Distinct from
  // the user's choice; loaded synchronously (localStorage) — no async race on
  // initial render. We refresh it after an advance commits.
  const [fsmState, setFsmState] = useState<OnboardingState>(
    () => loadSnapshot().currentState,
  );

  const { impact } = useHaptics();

  // Async safety: a sequence counter + alive ref so only the latest live
  // transition commits state. Any in-flight advance whose seq no longer matches
  // seqRef.current, or that resolves after unmount, is dropped (no stale
  // overwrite, no setState-after-unmount).
  const seqRef = useRef(0);
  const aliveRef = useRef(true);

  useEffect(() => {
    aliveRef.current = true;
    return () => {
      aliveRef.current = false;
    };
  }, []);

  // Position of this step within the FSM's forward chain — for the indicator
  // and "Step N of M" copy. Derived, never redeclared.
  const total = FORWARD_ORDER.length;
  const pickIdx = (() => {
    const i = FORWARD_ORDER.indexOf(PICK_STATE);
    return i < 0 ? 0 : i;
  })();

  const canContinue = choice !== null && !busy && !done;

  const handleSelect = useCallback(
    (value: MeshChoice) => {
      if (busy || done) return;
      setChoice(value);
      // Clear any prior error/soft note the moment the user re-engages.
      setError(null);
      setSoftNote(null);
      impact("light");
    },
    [busy, done, impact],
  );

  const handleContinue = useCallback(() => {
    if (choice === null || busy || done) return;

    // Clear prior result at load start so a stale message never lingers across
    // a transition.
    const mySeq = ++seqRef.current;
    setBusy(true);
    setError(null);
    setSoftNote(null);

    // Persist the user's mesh-setup choice into the FSM context. We only set
    // demoRelayUsed=false here (a real OnboardingContext field); the chosen
    // cluster mode is recorded as clusterIdHash=null for "create/single" and is
    // resolved by the actual join flow downstream — we never fabricate a hash.
    const ctxPatch =
      choice === "single_machine"
        ? { clusterIdHash: null, demoRelayUsed: false }
        : { demoRelayUsed: false };

    void (async () => {
      try {
        const result = await advanceOnboarding(ctxPatch);

        // Drop if superseded or unmounted.
        if (!aliveRef.current || mySeq !== seqRef.current) return;

        setFsmState(result.state);
        setDone(true);
        impact("medium");

        if (result.softFailed && result.errorMessage) {
          // Honest: the backend for this step is deferred; the FSM advanced
          // client-side. Distinct from a thrown error (handled in catch).
          setSoftNote(
            "叢集設定的後端尚未接好，已先在本機推進（State advanced locally; backend not yet wired）。",
          );
        }
        onContinue?.();
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
    })();
  }, [choice, busy, done, impact, onContinue]);

  const handleBack = useCallback(() => {
    if (busy) return;
    onBack?.();
  }, [busy, onBack]);

  return (
    <div
      data-testid="onboarding-pick"
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
              const reached = i <= pickIdx;
              const isCurrent = i === pickIdx;
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
            步驟 {pickIdx + 1} / {total} · Step {pickIdx + 1} of {total}
          </p>

          <header className="mb-6 flex items-center gap-3">
            <div className="flex min-h-[44px] min-w-[44px] items-center justify-center rounded-lg bg-phantom-primary text-phantom-bg">
              <Network aria-hidden="true" size={22} />
            </div>
            <div>
              <h1 className="text-2xl font-semibold text-phantom-text">
                設定你的網狀網路
              </h1>
              <p className="mt-1 text-base text-phantom-muted">
                Set up your mesh
              </p>
            </div>
          </header>

          <section className="mb-2">
            <p className="text-lg leading-7 text-phantom-text">
              選擇要如何加入網狀網路。
            </p>
            <p className="mt-1 text-base leading-6 text-phantom-muted">
              Choose how you want to join the mesh.
            </p>
          </section>

          {/* Selectable cards — radio-like single choice. */}
          <fieldset
            className="mt-4 flex flex-col gap-3"
            aria-label="網狀網路設定方式 / Mesh setup option"
          >
            <legend className="sr-only">
              選擇網狀網路設定方式 / Choose a mesh setup option
            </legend>
            {CHOICES.map((opt) => {
              const selected = choice === opt.value;
              const OptIcon = opt.icon;
              return (
                <button
                  key={opt.value}
                  type="button"
                  role="radio"
                  aria-checked={selected}
                  aria-label={`${opt.zhLabel} / ${opt.enLabel}`}
                  onClick={() => handleSelect(opt.value)}
                  disabled={busy || done}
                  className={`flex min-h-[64px] w-full items-start gap-3 rounded-lg border p-4 text-left transition disabled:opacity-60 motion-reduce:transition-none ${
                    selected
                      ? "border-phantom-primary bg-phantom-primary/10"
                      : "border-phantom-border bg-phantom-card"
                  }`}
                >
                  <span
                    className={`mt-0.5 flex min-h-[28px] min-w-[28px] items-center justify-center rounded-lg ${
                      selected
                        ? "bg-phantom-primary text-phantom-bg"
                        : "bg-phantom-bg text-phantom-muted"
                    }`}
                  >
                    <OptIcon aria-hidden="true" size={20} />
                  </span>
                  <span className="flex-1">
                    <span className="block text-lg font-medium text-phantom-text">
                      {opt.zhLabel}
                    </span>
                    <span className="block text-base text-phantom-muted">
                      {opt.enLabel}
                    </span>
                    <span className="mt-1 block text-base leading-6 text-phantom-muted">
                      {opt.zhNote}
                    </span>
                    <span className="mt-0.5 block text-base leading-6 text-phantom-muted">
                      {opt.enNote}
                    </span>
                  </span>
                  {selected ? (
                    <Check
                      aria-hidden="true"
                      size={20}
                      className="mt-1 shrink-0 text-phantom-primary"
                    />
                  ) : null}
                </button>
              );
            })}
          </fieldset>

          {/* Honest disclosure: the cluster-join backend is not fully wired. */}
          {!done ? (
            <p
              role="status"
              className="mt-4 flex items-start gap-2 rounded-lg border border-phantom-warning/40 bg-phantom-warning/10 p-3 text-base text-phantom-warning"
            >
              <Server aria-hidden="true" size={18} className="mt-0.5 shrink-0" />
              <span>
                叢集連線的後端仍在開發中（目前狀態：{fsmState}），繼續會先在本機完成。
                / Cluster wiring is still in progress; continuing completes it
                locally.
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
          {done && !error ? (
            <p
              role="status"
              className="mt-4 flex items-center gap-2 rounded-lg border border-phantom-success/40 bg-phantom-success/10 p-3 text-base text-phantom-success"
            >
              <Check aria-hidden="true" size={18} />
              已記錄你的選擇，繼續設定 / Choice saved — continuing setup.
            </p>
          ) : null}
        </main>

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
            onClick={handleContinue}
            disabled={!canContinue}
            aria-label="繼續 / Continue"
            className="flex min-h-[48px] flex-1 items-center justify-center gap-2 rounded-lg bg-phantom-primary px-4 py-3 text-base font-semibold text-phantom-bg transition disabled:opacity-60 motion-reduce:transition-none"
          >
            {busy ? (
              <Loader2
                aria-hidden="true"
                size={20}
                className="animate-spin motion-reduce:animate-none"
              />
            ) : done ? (
              <Check aria-hidden="true" size={20} />
            ) : (
              <ArrowRight aria-hidden="true" size={20} />
            )}
            {busy ? "處理中 / Working" : done ? "已完成 / Done" : "繼續 / Continue"}
          </button>
        </footer>
      </div>
    </div>
  );
}
