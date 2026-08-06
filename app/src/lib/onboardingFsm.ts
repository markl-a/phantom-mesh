// onboardingFsm.ts — React-side wrapper around the SPEC-28 onboarding FSM
// Tauri commands (`onboarding_advance` + `onboarding_rollback` registered
// by app/src-tauri/src/commands/onboarding_wire.rs).
//
// Why this module exists
// ----------------------
// Several React components (`OnboardingHello` — the desktop+mobile D1–D5
// flow — plus the legacy `MobileOnboardingV2` / `MobileJoinCluster` paths)
// all advance the same FSM at slightly different moments. Each needs to:
//   1. load / persist the current `OnboardingStateSnapshot` + `OnboardingContext`
//   2. call `onboarding_advance` whenever the user finishes a step
//   3. call `onboarding_rollback` when the user backs out of a half-joined
//      cluster (sanctioned `JoinedCluster → CreatedIdentity` edge)
//   4. degrade gracefully while SPEC-28 Stage 3 (FSM body) is still
//      `unimplemented!()` — the wire layer surfaces this as the error
//      string `onboarding.not_yet_wired:...`. We treat that as a soft
//      success (mutate the snapshot client-side) so the UI keeps moving
//      and the user is never blocked on a deferred backend.
//
// All component-side state is stored under one localStorage key so the
// FSM survives a reload (matches §8 "Resume guarantee" intent).
//
// OSS-safe: no real hostnames / emails / device names hard-coded.
// Placeholders (`user42`, `127.0.0.1`, `example.com`) only.

import { safeInvoke } from "./tauri-compat";
import type { OnboardingState } from "./generated/tauri/OnboardingState";
import type { OnboardingStateSnapshot } from "./generated/onboarding/OnboardingStateSnapshot";
import type { OnboardingContext } from "./generated/onboarding/OnboardingContext";

const SNAPSHOT_KEY = "spectyn_mesh_onboarding_snapshot";
const CONTEXT_KEY = "spectyn_mesh_onboarding_context";

/** Wire layer panic prefix — see commands/onboarding_wire.rs `NOT_YET_WIRED`. */
const NOT_YET_WIRED_PREFIX = "onboarding.not_yet_wired";

/** Forward order for the GUI D1–D5 flow. Mirrors the reachable forward edges
 *  in `core/src/onboarding_wire.rs` `fsm_table_pseudo`.
 *
 *  D2 (English-only): `picked_language` is removed — `fresh_install` advances
 *  DIRECTLY into `created_identity` (D1 login-first: OAuth login + ed25519
 *  identity happen together in that one step). 5 reachable states total. */
export const FORWARD_ORDER: readonly OnboardingState[] = [
  "fresh_install",
  "created_identity", // D1: login + identity; D2: picked_language removed
  "joined_cluster",
  "set_provider",
  "first_reply_received",
];

/** Shape of the result returned by `advanceOnboarding` /
 *  `rollbackOnboarding`. `softFailed` is `true` when the backend is
 *  still `unimplemented!()` and we fell back to a client-only state
 *  bump. UIs should still treat that as success (don't block the user)
 *  but can surface the underlying error for diagnostics. */
export interface OnboardingTransitionResult {
  state: OnboardingState;
  softFailed: boolean;
  errorMessage: string | null;
}

function nowMs(): number {
  return Date.now();
}

function freshSnapshot(): OnboardingStateSnapshot {
  return {
    currentState: "fresh_install",
    enteredAtMs: BigInt(nowMs()),
    retryCount: 0,
    lastError: null,
  };
}

function freshContext(): OnboardingContext {
  return {
    clusterIdHash: null,
    identityFingerprint: null,
    providerSlug: null,
    demoRelayUsed: false,
    identityProvider: null,
    identitySub: null,
  };
}

/** Coerce any BigInt fields (e.g. snapshot.enteredAtMs, i64 via ts-rs) to
 *  Number before crossing the Tauri `invoke` boundary — `invoke` JSON-encodes
 *  its args and throws "Do not know how to serialize a BigInt", which blocked
 *  ALL onboarding (login) on Android. ms timestamps are well within
 *  Number.MAX_SAFE_INTEGER, so serde reads them back as i64. Shared by both
 *  advanceOnboarding and rollbackOnboarding so neither can regress. */
function toJsonSafe<T>(o: T): T {
  return JSON.parse(
    JSON.stringify(o, (_k, v) => (typeof v === "bigint" ? Number(v) : v)),
  ) as T;
}

/** Serialize a snapshot for localStorage — BigInt isn't JSON-serializable,
 *  so we coerce `enteredAtMs` to a number. Loss of precision past 2^53 is
 *  irrelevant for a wall-clock ms value. */
function serializeSnapshot(snap: OnboardingStateSnapshot): string {
  return JSON.stringify({
    currentState: snap.currentState,
    enteredAtMs: Number(snap.enteredAtMs),
    retryCount: snap.retryCount,
    lastError: snap.lastError,
  });
}

function parseSnapshot(raw: string): OnboardingStateSnapshot | null {
  try {
    const obj = JSON.parse(raw) as {
      currentState: OnboardingState;
      enteredAtMs: number;
      retryCount: number;
      lastError: string | null;
    };
    if (!FORWARD_ORDER.includes(obj.currentState)) return null;
    return {
      currentState: obj.currentState,
      enteredAtMs: BigInt(obj.enteredAtMs),
      retryCount: obj.retryCount,
      lastError: obj.lastError,
    };
  } catch {
    return null;
  }
}

/** Load the persisted snapshot, or a fresh one if nothing is stored. */
export function loadSnapshot(): OnboardingStateSnapshot {
  if (typeof localStorage === "undefined") return freshSnapshot();
  const raw = localStorage.getItem(SNAPSHOT_KEY);
  if (!raw) return freshSnapshot();
  return parseSnapshot(raw) ?? freshSnapshot();
}

/** Load the persisted context, or a fresh one. */
export function loadContext(): OnboardingContext {
  if (typeof localStorage === "undefined") return freshContext();
  const raw = localStorage.getItem(CONTEXT_KEY);
  if (!raw) return freshContext();
  try {
    const parsed = JSON.parse(raw) as OnboardingContext;
    return {
      clusterIdHash: parsed.clusterIdHash ?? null,
      identityFingerprint: parsed.identityFingerprint ?? null,
      providerSlug: parsed.providerSlug ?? null,
      demoRelayUsed: Boolean(parsed.demoRelayUsed),
      identityProvider: parsed.identityProvider ?? null,
      identitySub: parsed.identitySub ?? null,
    };
  } catch {
    return freshContext();
  }
}

/** Persist a snapshot — best effort, swallow localStorage errors
 *  (Safari private mode etc.). */
function saveSnapshot(snap: OnboardingStateSnapshot): void {
  if (typeof localStorage === "undefined") return;
  try {
    localStorage.setItem(SNAPSHOT_KEY, serializeSnapshot(snap));
  } catch {
    /* private mode / quota — ignore */
  }
}

function saveContext(ctx: OnboardingContext): void {
  if (typeof localStorage === "undefined") return;
  try {
    localStorage.setItem(CONTEXT_KEY, JSON.stringify(ctx));
  } catch {
    /* ignore */
  }
}

/** Apply a patch to the persisted context and return the merged value. */
export function patchContext(patch: Partial<OnboardingContext>): OnboardingContext {
  const merged: OnboardingContext = { ...loadContext(), ...patch };
  saveContext(merged);
  return merged;
}

/** Reset all onboarding state — used by "wipe account" / debug menus. */
export function resetOnboarding(): void {
  if (typeof localStorage === "undefined") return;
  try {
    localStorage.removeItem(SNAPSHOT_KEY);
    localStorage.removeItem(CONTEXT_KEY);
  } catch {
    /* ignore */
  }
}

/** Client-side fallback: bump the snapshot to the next forward state.
 *  Used only when the backend wire returns `onboarding.not_yet_wired`
 *  (SPEC-28 Stage 3 deferred). Once Stage 3 lands this branch becomes
 *  dead code — the backend will own the transition. */
function clientForward(snap: OnboardingStateSnapshot): OnboardingState {
  const idx = FORWARD_ORDER.indexOf(snap.currentState);
  if (idx < 0 || idx >= FORWARD_ORDER.length - 1) return snap.currentState;
  return FORWARD_ORDER[idx + 1]!;
}

/** Client-side fallback for the one sanctioned rollback edge
 *  (`JoinedCluster → CreatedIdentity`). Every other state returns
 *  unchanged, matching the SPEC-28 §7.1 NoOp contract. */
function clientRollback(snap: OnboardingStateSnapshot): OnboardingState {
  return snap.currentState === "joined_cluster" ? "created_identity" : snap.currentState;
}

function isNotYetWiredError(err: unknown): boolean {
  const msg = err instanceof Error ? err.message : String(err);
  return msg.startsWith(NOT_YET_WIRED_PREFIX);
}

/** Advance the FSM one step. Persists the resulting snapshot + context
 *  on success. Caller passes any `Partial<OnboardingContext>` patch to
 *  merge before the call (e.g. set `providerSlug` when the user just
 *  finished provider config). */
export async function advanceOnboarding(
  ctxPatch: Partial<OnboardingContext> = {},
): Promise<OnboardingTransitionResult> {
  const snapshot = loadSnapshot();
  const ctx = patchContext(ctxPatch);
  try {
    const next = await safeInvoke<OnboardingState>("onboarding_advance", {
      snapshot: toJsonSafe(snapshot),
      ctx: toJsonSafe(ctx),
    });
    const nextSnap: OnboardingStateSnapshot = {
      currentState: next,
      enteredAtMs: BigInt(nowMs()),
      retryCount: 0,
      lastError: null,
    };
    saveSnapshot(nextSnap);
    return { state: next, softFailed: false, errorMessage: null };
  } catch (e) {
    const message = e instanceof Error ? e.message : String(e);
    if (isNotYetWiredError(e)) {
      // Backend deferred — do the bump client-side so the UI keeps moving.
      const next = clientForward(snapshot);
      saveSnapshot({
        currentState: next,
        enteredAtMs: BigInt(nowMs()),
        retryCount: 0,
        lastError: null,
      });
      // eslint-disable-next-line no-console
      console.warn(
        "[onboardingFsm] backend not yet wired, applied client fallback",
        snapshot.currentState,
        "→",
        next,
      );
      return { state: next, softFailed: true, errorMessage: message };
    }
    // Real error — bump retry count, leave state alone.
    saveSnapshot({
      ...snapshot,
      retryCount: snapshot.retryCount + 1,
      lastError: message,
    });
    throw new Error(message);
  }
}

/** Rollback the FSM (only `JoinedCluster → CreatedIdentity` actually
 *  moves; every other state is NoOp per SPEC-28 §7.1). */
export async function rollbackOnboarding(): Promise<OnboardingTransitionResult> {
  const snapshot = loadSnapshot();
  try {
    const next = await safeInvoke<OnboardingState>("onboarding_rollback", {
      snapshot: toJsonSafe(snapshot),
    });
    const nextSnap: OnboardingStateSnapshot = {
      currentState: next,
      enteredAtMs: BigInt(nowMs()),
      retryCount: 0,
      lastError: null,
    };
    saveSnapshot(nextSnap);
    return { state: next, softFailed: false, errorMessage: null };
  } catch (e) {
    const message = e instanceof Error ? e.message : String(e);
    if (isNotYetWiredError(e)) {
      const next = clientRollback(snapshot);
      saveSnapshot({
        currentState: next,
        enteredAtMs: BigInt(nowMs()),
        retryCount: 0,
        lastError: null,
      });
      // eslint-disable-next-line no-console
      console.warn(
        "[onboardingFsm] backend not yet wired, applied client rollback",
        snapshot.currentState,
        "→",
        next,
      );
      return { state: next, softFailed: true, errorMessage: message };
    }
    saveSnapshot({
      ...snapshot,
      retryCount: snapshot.retryCount + 1,
      lastError: message,
    });
    throw new Error(message);
  }
}

/** Force the FSM forward until it reaches `target` (inclusive). Each
 *  step persists. Used by entry-point components (e.g. demo mode picks
 *  the whole chain at once). Returns the final result. */
export async function advanceUntil(
  target: OnboardingState,
  ctxPatch: Partial<OnboardingContext> = {},
): Promise<OnboardingTransitionResult> {
  const targetIdx = FORWARD_ORDER.indexOf(target);
  if (targetIdx < 0) {
    throw new Error(`unknown onboarding state: ${target}`);
  }
  let lastResult: OnboardingTransitionResult = {
    state: loadSnapshot().currentState,
    softFailed: false,
    errorMessage: null,
  };
  // Apply ctx patch once up front so all subsequent advances see it.
  if (Object.keys(ctxPatch).length > 0) {
    patchContext(ctxPatch);
  }
  let guard = 0;
  while (FORWARD_ORDER.indexOf(loadSnapshot().currentState) < targetIdx) {
    lastResult = await advanceOnboarding();
    guard += 1;
    if (guard > FORWARD_ORDER.length) {
      // Safety belt — should never trigger; FSM has only 6 states.
      break;
    }
  }
  return lastResult;
}
