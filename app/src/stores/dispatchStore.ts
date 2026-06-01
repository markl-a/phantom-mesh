// F103 · Dispatch store (E002 mobile dispatch screen).
//
// Holds the in-flight + most-recent dispatch state for `<MobileDispatch />`.
// Per F103 spec §Scope:
//
//   DispatchState = {
//     id, prompt, caps, provider?, phase, tokens: string[],
//     startedAt, completedAt?, error?, result?
//   }
//
//   store = { byId: Map<id, DispatchState>, current: string | null }
//
// Why a separate store from `streamingStore.ts`: the desktop streaming
// store tracks a single in-flight chat response and resets on each new
// turn. Dispatch is a queue-able primitive — multiple dispatches can be
// in flight at once, and the History screen (F104) needs to enumerate
// them. Keeping the shapes independent avoids entangling those two
// surfaces.

import { create } from 'zustand';
import type { DispatchFrame } from '../lib/dispatchTypes';

export type DispatchPhase =
  | 'idle'
  | 'submitting'
  | 'queued'
  | 'running'
  | 'done'
  | 'failed'
  | 'cancelled';

export interface DispatchState {
  id: string;
  prompt: string;
  caps: string[];
  provider?: string;
  phase: DispatchPhase;
  /** Accumulated token chunks in arrival order. UI joins with ''. */
  tokens: string[];
  /** Final result body (for `done` phase). */
  result?: string;
  /** Error code + message for `failed` phase. */
  errorCode?: string;
  errorMessage?: string;
  startedAt: number;
  completedAt?: number;
}

export interface DispatchStoreState {
  /** Object map keyed by dispatch_id. Object (not Map) so zustand's
   *  shallow-equal selector works for the common "list of ids" case. */
  byId: Record<string, DispatchState>;
  /** id of the dispatch the UI is currently rendering — null when idle. */
  current: string | null;

  // ── reducers ──
  startDispatch: (init: {
    id: string;
    prompt: string;
    caps: string[];
    provider?: string;
    startedAt: number;
  }) => void;
  /** Apply one parsed SSE frame to a known dispatch id (no-op if unknown). */
  applyFrame: (id: string, frame: DispatchFrame) => void;
  /** Force phase = 'cancelled'. The Rust side already sent a
   *  status:"cancelled" frame for the source-of-truth path; this is
   *  defense-in-depth for the user-tap-Cancel UX path. */
  markCancelled: (id: string) => void;
  setCurrent: (id: string | null) => void;
  reset: () => void;
}

/** Treat `done` / `failed` / `cancelled` as terminal — Cancel becomes a
 *  no-op once we're here (F103 risk register: "User taps Cancel after
 *  `done` arrives — UI inconsistency"). */
export function isTerminalPhase(phase: DispatchPhase): boolean {
  return phase === 'done' || phase === 'failed' || phase === 'cancelled';
}

export const useDispatchStore = create<DispatchStoreState>()((set) => ({
  byId: {},
  current: null,

  startDispatch: ({ id, prompt, caps, provider, startedAt }) =>
    set((s) => ({
      byId: {
        ...s.byId,
        [id]: {
          id,
          prompt,
          caps,
          provider,
          phase: 'submitting',
          tokens: [],
          startedAt,
        },
      },
      current: id,
    })),

  applyFrame: (id, frame) =>
    set((s) => {
      const cur = s.byId[id];
      if (!cur) return s;
      // Once we're terminal, ignore stray late frames — the broker
      // sometimes emits one more status after `done` due to flushing.
      if (isTerminalPhase(cur.phase) && frame.type !== 'error') {
        return s;
      }
      const next: DispatchState = { ...cur };
      switch (frame.type) {
        case 'token':
          next.tokens = [...cur.tokens, frame.text];
          // First token → we're definitely running.
          if (cur.phase === 'submitting' || cur.phase === 'queued') {
            next.phase = 'running';
          }
          break;
        case 'status':
          if (frame.phase === 'cancelled') {
            next.phase = 'cancelled';
            next.completedAt = Date.now();
          } else if (frame.phase === 'queued') {
            next.phase = 'queued';
          } else if (frame.phase === 'running') {
            next.phase = 'running';
          }
          break;
        case 'done':
          next.phase = 'done';
          next.result = frame.result;
          next.completedAt = Date.now();
          break;
        case 'error':
          next.phase = 'failed';
          next.errorCode = frame.code;
          next.errorMessage = frame.message;
          next.completedAt = Date.now();
          break;
        // 'other' / unknown — preserve state, don't crash on broker bumps.
        default:
          break;
      }
      return { byId: { ...s.byId, [id]: next } };
    }),

  markCancelled: (id) =>
    set((s) => {
      const cur = s.byId[id];
      if (!cur || isTerminalPhase(cur.phase)) return s;
      return {
        byId: {
          ...s.byId,
          [id]: { ...cur, phase: 'cancelled', completedAt: Date.now() },
        },
      };
    }),

  setCurrent: (id) => set({ current: id }),

  reset: () => set({ byId: {}, current: null }),
}));
