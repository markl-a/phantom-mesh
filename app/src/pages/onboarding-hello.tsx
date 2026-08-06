// SPEC-31 onboarding-hello — route /onboarding; commands_used: onboarding_* (via lib/onboardingFsm.ts)
//
// GUI onboarding D1–D5 (English-only). Drives the shared SPEC-28 FSM through
// `lib/onboardingFsm.ts`, whose `onboarding_advance` now runs the REAL per-edge
// side-effects in core (login+identity mint, detached `spectyn serve` + mDNS
// advertise, provider detection + ranking) — the same backend the shipped CLI
// onboarding uses (a7c5701f).
//
// Decisions baked in:
//   D1  login-first  — the `created_identity` step does OAuth login (broker)
//                      THEN mints the ed25519 identity, in one step.
//   D2  English-only — no language picker (the old `picked_language` state is
//                      removed from FORWARD_ORDER).
//   D4  auto-mesh    — `joined_cluster` auto-starts a single-node serve +
//                      advertise; interactive peer-join + vault sync = Stage 2.
//   D5  provider     — `set_provider` detects subscription CLIs + Ollama and
//                      lets the user drag-reorder priority; Ollama is the
//                      always-available fallback.

import { useCallback, useEffect, useRef, useState } from "react";
import {
  ArrowLeft,
  ArrowRight,
  Check,
  GripVertical,
  KeyRound,
  Loader2,
  Network,
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
import {
  installBrokerLoginBridge,
  loadBrokerLoginStatus,
  onBrokerLoginResult,
  startBrokerLogin,
  type BrokerLoginFinishResponse,
} from "../lib/brokerLogin";
import { safeInvoke } from "../lib/tauri-compat";
import type { OnboardingState } from "../lib/generated/tauri/OnboardingState";
import { useHaptics } from "../lib/useHaptics";
import { ONBOARDED_KEY, IDENTITY_KEY } from "../components/onboarding/types";

// Step order comes straight from the FSM's exported FORWARD_ORDER (the single
// source of truth, mirroring the reachable forward edges in
// core/src/onboarding_wire.rs) — no parallel/redeclared list that could drift.
const STEP_ORDER = FORWARD_ORDER;

type StepCopy = {
  state: OnboardingState;
  title: string;
  body: string;
  icon: typeof KeyRound;
  // When false, this step's backend side-effect is NOT wired end-to-end yet;
  // we say so honestly rather than implying it fully ran. With D1–D5 wired,
  // every reachable step is `true`; deferred sub-features are called out inline.
  backendReady: boolean;
};

// English-only copy (D2). `picked_language` is intentionally absent from
// FORWARD_ORDER, so it is never rendered; we keep a stub entry only to satisfy
// the Record<OnboardingState, …> type for the retained-but-unreachable variant.
const STEPS: Record<OnboardingState, StepCopy> = {
  fresh_install: {
    state: "fresh_install",
    title: "Welcome to Spectyn Mesh",
    body: "Sign in with Google or Apple to link this device. Your identity key is generated locally and never leaves this device. Takes about 30 seconds.",
    icon: Sparkles,
    backendReady: true,
  },
  // Retained for type completeness only — never reached (D2: English-only).
  picked_language: {
    state: "picked_language",
    title: "Language",
    body: "Spectyn Mesh is English-only for now.",
    icon: Sparkles,
    backendReady: true,
  },
  created_identity: {
    state: "created_identity",
    title: "Your identity is ready",
    body: "We generated a local ed25519 key as your identity — stored in the system Keychain and never uploaded.",
    icon: KeyRound,
    // D1: OAuth login (broker loopback, on the first screen) + identity::init
    // are both wired.
    backendReady: true,
  },
  joined_cluster: {
    state: "joined_cluster",
    title: "Set up your mesh node",
    body: "Starting this machine as a mesh node so it is discoverable on your network. Joining an existing cluster (peer-pairing) comes later — you can do it from Settings.",
    icon: Network,
    // D4 Stage 1: serve + mDNS advertise are wired; peer-join is deferred.
    backendReady: true,
  },
  set_provider: {
    state: "set_provider",
    title: "Choose your model providers",
    body: "We detected the model providers you are already signed in to. Drag to set priority — the first reachable one is used. Local Ollama is added as an always-on fallback.",
    icon: Sparkles,
    // D5: detection + ranking are wired; broker-vault key sync is deferred.
    backendReady: true,
  },
  first_reply_received: {
    state: "first_reply_received",
    title: "You are all set",
    body: "Setup complete. Send your first message and the reply streams into the main chat.",
    icon: Send,
    backendReady: true,
  },
};

function stepIndex(state: OnboardingState): number {
  const idx = STEP_ORDER.indexOf(state);
  return idx < 0 ? 0 : idx;
}

// ─── Provider picker (D5) ───────────────────────────────────────────────────

/** A detected provider row for the priority picker. `slug` is the agents.toml
 *  block name the backend ranks on (claude_cli / codex_oauth / local-ollama).
 *  Note: the backend `perform_provider_detection` is the source of truth for
 *  what is actually written; this list is the user-facing priority UX hint and
 *  mirrors the same detection signals. */
type ProviderRow = { slug: string; label: string; fallback: boolean };

/** One free-tier cloud provider (mirrors the Rust `FreeProviderInfo`, snake_case). */
type FreeProviderInfo = {
  slug: string;
  display: string;
  provider_type: string;
  base_url: string;
  api_key_env: string;
  default_model: string;
  get_key_url: string;
  no_credit_card: boolean;
};

/** Shape returned by the `detect_free_provider` Tauri command. */
type FreeProviderSuggestion = {
  registry: FreeProviderInfo[];
  recommended: FreeProviderInfo;
  detected_from_env: string | null;
};

/** Shape returned by `validate_api_key`. */
type ValidationResult = { ok: boolean; models?: string[]; error?: string | null };

/** Shape returned by `read_claude_cli_token` / `read_codex_token`. */
interface FoundStatus {
  found?: boolean;
}

/** Shape returned by the `detect_local_servers` Tauri command (subset). */
interface LocalServerInfo {
  name?: string;
}

/** App.tsx (desktop + mobile) renders this as the first-launch flow and passes
 *  `onComplete` so it can flip its `onboarded` state and route into the app. */
interface OnboardingHelloProps {
  onComplete?: () => void;
}

export default function OnboardingHello({ onComplete }: OnboardingHelloProps = {}) {
  // Derive the initial state from the persisted FSM snapshot directly. The FSM
  // (localStorage) is the source of truth and is read synchronously.
  const [current, setCurrent] = useState<OnboardingState>(
    () => loadSnapshot().currentState,
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [softNote, setSoftNote] = useState<string | null>(null);
  const [finished, setFinished] = useState(false);

  // D1 login state for the created_identity step.
  const [loginBusy, setLoginBusy] = useState(false);
  const [loginIdentity, setLoginIdentity] = useState<BrokerLoginFinishResponse | null>(null);
  const [loginError, setLoginError] = useState<string | null>(null);

  // D5 provider picker state for the set_provider step.
  const [providers, setProviders] = useState<ProviderRow[]>([]);
  const [providersLoaded, setProvidersLoaded] = useState(false);
  const dragIndex = useRef<number | null>(null);

  // D5 free-plugin (default-on) state: surfaced when no subscription/local model
  // is found so a brand-new user can start on a free, no-credit-card tier.
  const [freeRecommended, setFreeRecommended] = useState<FreeProviderInfo | null>(null);
  const [freeDetectedSlug, setFreeDetectedSlug] = useState<string | null>(null);
  const [freeKey, setFreeKey] = useState("");
  const [freeBusy, setFreeBusy] = useState(false);
  const [freeError, setFreeError] = useState<string | null>(null);
  const [freeConfiguredSlug, setFreeConfiguredSlug] = useState<string | null>(null);

  const { impact } = useHaptics();

  // Async safety: a sequence counter + alive ref so only the latest live
  // transition commits state.
  const seqRef = useRef(0);
  const aliveRef = useRef(true);

  useEffect(() => {
    aliveRef.current = true;
    return () => {
      aliveRef.current = false;
    };
  }, []);

  // Install the broker login deep-link bridge once + adopt any existing session.
  useEffect(() => {
    void installBrokerLoginBridge();
    void loadBrokerLoginStatus().then((s) => {
      if (aliveRef.current && s) {
        setLoginIdentity(s);
        // Fold an already-active session into the FSM context too, so the
        // fresh_install → created_identity precondition (provider/sub) holds
        // without forcing a redundant sign-in click on returning users.
        patchContext({ identityProvider: s.provider, identitySub: s.email });
      }
    });
    const off = onBrokerLoginResult((result) => {
      if (!aliveRef.current) return;
      setLoginBusy(false);
      if (result.ok) {
        setLoginIdentity(result.identity);
        setLoginError(null);
        // Fold the OAuth result into the FSM context (D1 login-first) so the
        // backend `onboarding_advance` precondition (provider + sub required
        // before identity) is satisfied.
        patchContext({
          identityProvider: result.identity.provider,
          identitySub: result.identity.email,
        });
      } else {
        setLoginError(result.error);
      }
    });
    return off;
  }, []);

  const idx = stepIndex(current);
  const total = STEP_ORDER.length;
  const isLast = idx === total - 1;
  const step = STEPS[current];
  const StepIcon = step.icon;

  // Rollback is only sanctioned on the joined_cluster → created_identity edge.
  const canGoBack = current === "joined_cluster";

  // D5: load detected providers when entering the set_provider step.
  useEffect(() => {
    if (current !== "set_provider" || providersLoaded) return;
    let alive = true;
    void (async () => {
      const rows: ProviderRow[] = [];
      // Subscription CLIs — same signals the CLI wizard + backend rank on.
      // Priority: Claude > ChatGPT(Codex), then Ollama fallback last (D5b).
      try {
        const claude = await safeInvoke<FoundStatus>("read_claude_cli_token");
        if (claude?.found) {
          rows.push({ slug: "claude_cli", label: "Claude (Claude Code subscription)", fallback: false });
        }
      } catch {
        /* best-effort */
      }
      try {
        const codex = await safeInvoke<FoundStatus>("read_codex_token");
        if (codex?.found) {
          rows.push({ slug: "codex_oauth", label: "ChatGPT (Codex / Sign in with ChatGPT)", fallback: false });
        }
      } catch {
        /* best-effort */
      }
      try {
        const servers =
          (await safeInvoke<LocalServerInfo[]>("detect_local_servers")) ?? [];
        if (servers.some((s) => s.name === "ollama")) {
          rows.push({ slug: "local-ollama", label: "Local Ollama (always-on fallback)", fallback: true });
        }
      } catch {
        /* no local server probe — fine */
      }
      // Free-plugin (default-on): learn the recommended free provider + whether
      // a free key is already in the environment. A detected env key becomes a
      // row (zero-config); otherwise the recommended provider drives the
      // "start free" card rendered when nothing else is found.
      try {
        const free = await safeInvoke<FreeProviderSuggestion>("detect_free_provider");
        if (free) {
          if (alive) {
            setFreeRecommended(free.recommended);
            setFreeDetectedSlug(free.detected_from_env ?? null);
          }
          if (free.detected_from_env) {
            rows.push({
              slug: free.detected_from_env,
              label: `Free cloud — ${free.detected_from_env} (detected)`,
              fallback: true,
            });
          }
        }
      } catch {
        /* free-plugin probe is best-effort */
      }
      if (alive) {
        setProviders(rows);
        setProvidersLoaded(true);
      }
    })();
    return () => {
      alive = false;
    };
  }, [current, providersLoaded]);

  const runTransition = useCallback(
    async (
      kind: "advance" | "rollback",
      ctxPatch?: Parameters<typeof patchContext>[0],
    ) => {
      const mySeq = ++seqRef.current;
      setBusy(true);
      setError(null);
      setSoftNote(null);

      try {
        const result =
          kind === "advance"
            ? await advanceOnboarding(ctxPatch ?? {})
            : await rollbackOnboarding();

        if (!aliveRef.current || mySeq !== seqRef.current) return;

        setCurrent(result.state);
        if (result.softFailed && result.errorMessage) {
          setSoftNote("State advanced locally; backend not yet wired for this step.");
        }
        if (kind === "advance" && result.state === "first_reply_received") {
          impact("medium");
        }
      } catch (e) {
        if (!aliveRef.current || mySeq !== seqRef.current) return;
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

  // D1: start the broker OAuth login (opens system browser; the deep-link
  // bridge resolves via onBrokerLoginResult above).
  const handleLogin = useCallback(async () => {
    setLoginBusy(true);
    setLoginError(null);
    try {
      await startBrokerLogin();
    } catch (e) {
      setLoginBusy(false);
      setLoginError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  // D5 free-plugin: open the provider's "get a free key" page in the system
  // browser (never the embedded webview).
  const handleGetFreeKey = useCallback(() => {
    if (freeRecommended) {
      void safeInvoke("open_external_url", { url: freeRecommended.get_key_url });
    }
  }, [freeRecommended]);

  // D5 free-plugin: validate a pasted free key against the provider's /models
  // (reuses the validate_api_key Tauri command). On success the slug is marked
  // configured; the key itself is persisted later by finalize_onboarding_config.
  const handleValidateFreeKey = useCallback(async () => {
    if (!freeRecommended || !freeKey.trim()) return;
    setFreeBusy(true);
    setFreeError(null);
    try {
      const res = await safeInvoke<ValidationResult>("validate_api_key", {
        provider: freeRecommended.provider_type,
        key: freeKey.trim(),
      });
      if (res?.ok) {
        setFreeConfiguredSlug(freeRecommended.slug);
      } else {
        setFreeError(res?.error ?? "That key was rejected. Check it and try again.");
      }
    } catch (e) {
      setFreeError(e instanceof Error ? e.message : String(e));
    } finally {
      setFreeBusy(false);
    }
  }, [freeRecommended, freeKey]);

  // D5: write the agents.toml via the SAME core writer the CLI uses
  // (finalize_onboarding_config), then advance. The free provider (env-detected
  // or just-pasted) is included so a no-subscription user gets a working primary.
  const finalizeProvidersThenAdvance = useCallback(async () => {
    const subSlugs = providers
      .map((p) => p.slug)
      .filter((s) => s === "claude_cli" || s === "codex_oauth" || s === "gemini_oauth");
    const hasOllama = providers.some((p) => p.slug === "local-ollama");
    const freeSlug = freeConfiguredSlug ?? freeDetectedSlug ?? null;
    try {
      await safeInvoke("finalize_onboarding_config", {
        input: {
          ordered: subSlugs,
          free_slug: freeSlug,
          // Only send the pasted key when WE just configured it (env-detected
          // keys already live in the environment — nothing to persist).
          free_key: freeConfiguredSlug && freeKey.trim() ? freeKey.trim() : null,
          ollama_url: hasOllama ? "http://127.0.0.1:11434/v1" : null,
        },
      });
    } catch (e) {
      // Surface but don't hard-block — the FSM still records the provider slug.
      setError(e instanceof Error ? e.message : String(e));
    }
    const primary = subSlugs[0] ?? freeSlug ?? providers[0]?.slug ?? null;
    void runTransition("advance", primary ? { providerSlug: primary } : {});
  }, [providers, freeConfiguredSlug, freeDetectedSlug, freeKey, runTransition]);

  const handleContinue = useCallback(() => {
    if (busy) return;
    if (isLast) {
      patchContext({});
      // Persist the onboarded flag + a cosmetic identity profile (display name /
      // email / provider) so a reload skips onboarding and App.tsx's sidebar can
      // show who is signed in. The real ed25519 key + broker token live in the
      // keychain — only sanitised display values are mirrored to localStorage.
      try {
        localStorage.setItem(ONBOARDED_KEY, "true");
        if (loginIdentity) {
          localStorage.setItem(
            IDENTITY_KEY,
            JSON.stringify({
              provider: loginIdentity.provider,
              email: loginIdentity.email,
              display_name: loginIdentity.display_name ?? loginIdentity.email,
              avatar_url: null,
            }),
          );
        }
      } catch {
        /* private mode / quota — non-fatal, FSM snapshot still persisted */
      }
      setFinished(true);
      impact("medium");
      onComplete?.();
      return;
    }
    // D5: leaving set_provider — write agents.toml via the shared core writer
    // (incl. the free provider) THEN advance, so the GUI persists a real,
    // working config instead of relying on FSM side-effects alone.
    if (current === "set_provider") {
      void finalizeProvidersThenAdvance();
      return;
    }
    void runTransition("advance", {});
  }, [busy, current, finalizeProvidersThenAdvance, impact, isLast, loginIdentity, onComplete, runTransition]);

  const handleBack = useCallback(() => {
    if (busy || !canGoBack) return;
    void runTransition("rollback");
  }, [busy, canGoBack, runTransition]);

  // D5 drag-reorder handlers (HTML5 DnD; keyboard users can use the up/down
  // buttons rendered alongside each row).
  const reorder = useCallback((from: number, to: number) => {
    setProviders((prev) => {
      if (from === to || from < 0 || to < 0 || from >= prev.length || to >= prev.length) {
        return prev;
      }
      const next = prev.slice();
      const [moved] = next.splice(from, 1);
      next.splice(to, 0, moved!);
      return next;
    });
  }, []);

  // D1 (login-first): the continue button on the FIRST screen is gated on a
  // completed login, because the backend folds the login (provider/sub) into
  // the fresh_install → created_identity edge that Continue triggers.
  const continueDisabled =
    busy ||
    finished ||
    (current === "fresh_install" && !loginIdentity) ||
    // set_provider needs SOME provider: a detected row, an env-detected free
    // key, or a just-configured free key. Otherwise there's nothing to talk to.
    (current === "set_provider" &&
      providers.length === 0 &&
      !freeConfiguredSlug &&
      !freeDetectedSlug);

  return (
    <div
      data-testid="onboarding-hello"
      className="min-h-screen bg-spectyn-bg text-spectyn-text pt-[env(safe-area-inset-top)] pl-[env(safe-area-inset-left)] pr-[env(safe-area-inset-right)]"
    >
      <div className="flex min-h-screen flex-col">
        <main className="flex-1 overflow-y-auto px-5 pb-6 pt-6">
          {/* Step indicator — DOM order matches visual order. */}
          <nav
            aria-label="Setup progress"
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
                      ? "w-6 bg-spectyn-primary"
                      : reached
                        ? "w-2.5 bg-spectyn-primary/50"
                        : "w-2.5 bg-spectyn-border"
                  }`}
                />
              );
            })}
          </nav>
          <p role="status" className="mb-6 text-center text-base text-spectyn-muted">
            Step {idx + 1} of {total}
          </p>

          <header className="mb-6 flex items-center gap-3">
            <div className="flex min-h-[44px] min-w-[44px] items-center justify-center rounded-lg bg-spectyn-primary text-spectyn-bg">
              <StepIcon aria-hidden="true" size={22} />
            </div>
            <h1 className="text-2xl font-semibold text-spectyn-text">{step.title}</h1>
          </header>

          <section className="rounded-lg border border-spectyn-border bg-spectyn-card p-4">
            <p className="text-lg leading-7 text-spectyn-text">{step.body}</p>
          </section>

          {/* D1 (login-first): the sign-in lives on the FIRST screen. The
              backend folds the login (provider/sub) into the
              fresh_install → created_identity edge, so the user must sign in
              before Continue can mint the identity. */}
          {current === "fresh_install" && !finished ? (
            <section className="mt-4 rounded-lg border border-spectyn-border bg-spectyn-card p-4">
              {loginIdentity ? (
                <p
                  role="status"
                  className="flex items-center gap-2 text-base text-spectyn-success"
                >
                  <Check aria-hidden="true" size={18} />
                  Signed in as {loginIdentity.email} ({loginIdentity.provider}). Continue to
                  create your identity key.
                </p>
              ) : (
                <>
                  <button
                    type="button"
                    onClick={() => void handleLogin()}
                    disabled={loginBusy}
                    className="flex min-h-[48px] w-full items-center justify-center gap-2 rounded-lg bg-spectyn-primary px-4 py-3 text-base font-semibold text-spectyn-bg transition disabled:opacity-60 motion-reduce:transition-none"
                  >
                    {loginBusy ? (
                      <Loader2 aria-hidden="true" size={20} className="animate-spin motion-reduce:animate-none" />
                    ) : (
                      <KeyRound aria-hidden="true" size={20} />
                    )}
                    {loginBusy ? "Opening browser…" : "Sign in with Google or Apple"}
                  </button>
                  {loginError ? (
                    <p role="alert" className="mt-3 text-base text-spectyn-danger">
                      Sign-in failed: {loginError}
                    </p>
                  ) : null}
                </>
              )}
            </section>
          ) : null}

          {/* created_identity: login already happened on the first screen; this
              step confirms the local ed25519 key was minted. */}
          {current === "created_identity" && !finished ? (
            <section className="mt-4 rounded-lg border border-spectyn-border bg-spectyn-card p-4">
              <p
                role="status"
                className="flex items-center gap-2 text-base text-spectyn-success"
              >
                <Check aria-hidden="true" size={18} />
                Your local identity key is ready
                {loginIdentity ? ` for ${loginIdentity.email}` : ""}. Continue to set up
                your mesh.
              </p>
            </section>
          ) : null}

          {/* D5: provider priority picker on the set_provider step. */}
          {current === "set_provider" && !finished ? (
            <section className="mt-4 rounded-lg border border-spectyn-border bg-spectyn-card p-4">
              {!providersLoaded ? (
                <p role="status" className="flex items-center gap-2 text-base text-spectyn-muted">
                  <Loader2 aria-hidden="true" size={18} className="animate-spin motion-reduce:animate-none" />
                  Detecting your model providers…
                </p>
              ) : providers.length === 0 ? (
                freeRecommended ? (
                  <div className="flex flex-col gap-3">
                    <p className="text-base text-spectyn-text">
                      No subscription or local model found. Start free with{" "}
                      <span className="font-semibold">{freeRecommended.display}</span> — a
                      no-credit-card free tier. About a minute to set up.
                    </p>
                    {freeConfiguredSlug ? (
                      <p
                        role="status"
                        className="flex items-center gap-2 text-base text-spectyn-success"
                      >
                        <Check aria-hidden="true" size={18} />
                        {freeRecommended.display} is ready — continue to start chatting.
                      </p>
                    ) : (
                      <>
                        <button
                          type="button"
                          onClick={handleGetFreeKey}
                          className="flex min-h-[44px] items-center justify-center gap-2 rounded-lg border border-spectyn-border px-4 py-2 text-base font-medium text-spectyn-text transition motion-reduce:transition-none"
                        >
                          <KeyRound aria-hidden="true" size={18} />
                          Get a free key
                        </button>
                        <input
                          type="password"
                          value={freeKey}
                          onChange={(e) => setFreeKey(e.target.value)}
                          aria-label={`${freeRecommended.display} API key`}
                          placeholder={`Paste your ${freeRecommended.display} API key`}
                          className="min-h-[44px] rounded-lg border border-spectyn-border bg-spectyn-bg px-3 py-2 text-base text-spectyn-text"
                        />
                        <button
                          type="button"
                          onClick={() => void handleValidateFreeKey()}
                          disabled={freeBusy || !freeKey.trim()}
                          className="flex min-h-[44px] items-center justify-center gap-2 rounded-lg bg-spectyn-primary px-4 py-2 text-base font-semibold text-spectyn-bg transition disabled:opacity-60 motion-reduce:transition-none"
                        >
                          {freeBusy ? (
                            <Loader2 aria-hidden="true" size={18} className="animate-spin motion-reduce:animate-none" />
                          ) : (
                            <Check aria-hidden="true" size={18} />
                          )}
                          {freeBusy ? "Checking…" : "Use this key"}
                        </button>
                        {freeError ? (
                          <p role="alert" className="text-base text-spectyn-danger">
                            {freeError}
                          </p>
                        ) : null}
                      </>
                    )}
                  </div>
                ) : (
                  <p role="status" className="text-base text-spectyn-muted">
                    No signed-in providers or local Ollama detected yet. Sign in to a CLI
                    (claude / codex / gemini) or start Ollama, then come back.
                  </p>
                )
              ) : (
                <ul className="flex flex-col gap-2" aria-label="Provider priority order">
                  {providers.map((p, i) => (
                    <li
                      key={p.slug}
                      draggable
                      onDragStart={() => {
                        dragIndex.current = i;
                      }}
                      onDragOver={(e) => e.preventDefault()}
                      onDrop={(e) => {
                        e.preventDefault();
                        if (dragIndex.current !== null) {
                          reorder(dragIndex.current, i);
                          dragIndex.current = null;
                        }
                      }}
                      className="flex items-center gap-3 rounded-lg border border-spectyn-border bg-spectyn-bg p-3"
                    >
                      <GripVertical aria-hidden="true" size={18} className="shrink-0 text-spectyn-muted" />
                      <span className="min-w-[1.5rem] text-base font-semibold text-spectyn-primary">
                        {i + 1}
                      </span>
                      <span className="flex-1 text-base text-spectyn-text">{p.label}</span>
                      <span className="flex flex-col">
                        <button
                          type="button"
                          aria-label={`Move ${p.label} up`}
                          disabled={i === 0}
                          onClick={() => reorder(i, i - 1)}
                          className="px-2 text-spectyn-muted disabled:opacity-30"
                        >
                          ▲
                        </button>
                        <button
                          type="button"
                          aria-label={`Move ${p.label} down`}
                          disabled={i === providers.length - 1}
                          onClick={() => reorder(i, i + 1)}
                          className="px-2 text-spectyn-muted disabled:opacity-30"
                        >
                          ▼
                        </button>
                      </span>
                    </li>
                  ))}
                </ul>
              )}
            </section>
          ) : null}

          {/* Honest disclosure: this step's backend isn't fully wired yet. */}
          {!step.backendReady && !finished ? (
            <p
              role="status"
              className="mt-4 flex items-start gap-2 rounded-lg border border-spectyn-warning/40 bg-spectyn-warning/10 p-3 text-base text-spectyn-warning"
            >
              <Zap aria-hidden="true" size={18} className="mt-0.5 shrink-0" />
              <span>Backend for this step is still in progress; continuing completes it locally.</span>
            </p>
          ) : null}

          {/* Stage-2 honest TODO note on the mesh-node step (peer-join/vault). */}
          {current === "joined_cluster" && !finished ? (
            <p role="status" className="mt-4 rounded-lg border border-spectyn-border bg-spectyn-card p-3 text-base text-spectyn-muted">
              Single-node mode for now. Peer-pairing with an existing cluster and
              encrypted vault sync are coming soon (Stage 2).
            </p>
          ) : null}

          {/* Real backend error (a rejected/thrown call). */}
          {error ? (
            <p
              role="alert"
              className="mt-4 rounded-lg border border-spectyn-danger/40 bg-spectyn-danger/10 p-3 text-base text-spectyn-danger"
            >
              Could not complete this step: {error}
            </p>
          ) : null}

          {/* Soft fallback note (backend deferred, advanced client-side). */}
          {softNote && !error ? (
            <p
              role="status"
              className="mt-4 rounded-lg border border-spectyn-border bg-spectyn-card p-3 text-base text-spectyn-muted"
            >
              {softNote}
            </p>
          ) : null}

          {/* Completion state. */}
          {finished ? (
            <p
              role="status"
              className="mt-4 flex items-center gap-2 rounded-lg border border-spectyn-success/40 bg-spectyn-success/10 p-3 text-base text-spectyn-success"
            >
              <Check aria-hidden="true" size={18} />
              Setup complete — you are ready to go.
            </p>
          ) : null}
        </main>

        <footer className="sticky bottom-0 flex items-center gap-3 border-t border-spectyn-border bg-spectyn-bg/95 px-5 pt-4 pb-[max(0.75rem,env(safe-area-inset-bottom))] backdrop-blur">
          {canGoBack ? (
            <button
              type="button"
              onClick={handleBack}
              disabled={busy || finished}
              aria-label="Back"
              className="flex min-h-[48px] items-center justify-center gap-2 rounded-lg border border-spectyn-border px-4 py-3 text-base font-medium text-spectyn-text transition disabled:opacity-60 motion-reduce:transition-none"
            >
              <ArrowLeft aria-hidden="true" size={20} />
              Back
            </button>
          ) : null}

          <button
            type="button"
            onClick={handleContinue}
            disabled={continueDisabled}
            aria-label={isLast ? "Finish" : "Continue"}
            className="flex min-h-[48px] flex-1 items-center justify-center gap-2 rounded-lg bg-spectyn-primary px-4 py-3 text-base font-semibold text-spectyn-bg transition disabled:opacity-60 motion-reduce:transition-none"
          >
            {busy ? (
              <Loader2 aria-hidden="true" size={20} className="animate-spin motion-reduce:animate-none" />
            ) : isLast ? (
              <Check aria-hidden="true" size={20} />
            ) : (
              <ArrowRight aria-hidden="true" size={20} />
            )}
            {busy ? "Working" : finished ? "Done" : isLast ? "Finish" : "Continue"}
          </button>
        </footer>
      </div>
    </div>
  );
}
