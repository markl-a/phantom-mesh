// F103 · Mobile dispatch screen.
//
// Spec: docs/superpowers/specs/_current/E002-mobile-cluster-dispatch-ui.md
//   §"Dispatch screen". Feature spec:
//   docs/superpowers/features/F103-dispatch-screen-ui.md.
//
// Renders:
//   1. Prompt textarea (auto-grow, 1..=8000 chars enforced client-side
//      AND in Rust — see dispatch.rs::validate_prompt for the source of
//      truth).
//   2. Capability chip strip — toggleable subset of AVAILABLE_CAPS.
//      Tapping a chip adds/removes it; >MAX_CAPS_SELECTABLE blocks the
//      additional taps client-side, the Rust validator catches anything
//      that slips through.
//   3. Provider override dropdown — populated from the
//      `list_dispatch_providers` Tauri command.
//   4. Submit / Cancel button (disabled until prompt is non-whitespace).
//   5. Token-stream pre block — appends `dispatch::token::<id>` frames
//      as they arrive. Final phase renders a colored result block.
//
// Wire conventions (matches dispatch.rs):
//   - `dispatch_task` POSTs to {broker}/api/squad/dispatch, returns
//     { dispatch_id, started_at_unix }.
//   - `cancel_dispatch(dispatch_id)` flips the per-dispatch cancel
//     watch channel; the SSE reader emits a final
//     `{type:'status',phase:'cancelled'}` frame for the store.
//
// All `invoke()` calls go through `safeInvoke` (tauri-compat.ts) so dev
// builds in a plain browser don't crash — the fallback returns sensible
// defaults that produce a clear "broker not reachable" surface.

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Send, X, Loader2 } from 'lucide-react';
import { safeInvoke } from '../../lib/tauri-compat';
import { friendlyDispatchError } from '../../lib/dispatchErrors';
import {
  AVAILABLE_CAPS,
  MAX_CAPS_SELECTABLE,
  type DispatchFrame,
  type DispatchHandle,
  type DispatchProvider,
} from '../../lib/dispatchTypes';
import {
  useDispatchStore,
  isTerminalPhase,
  type DispatchPhase,
} from '../../stores/dispatchStore';

// ── Tauri-event listen seam (mirrors useClusterPeers.ts pattern) ─────────

export type DispatchListenFn = <T>(
  event: string,
  handler: (ev: { payload: T }) => void,
) => Promise<() => void>;

let _listenImpl: DispatchListenFn | null = null;

/** Test-seam: vitest swaps the underlying `listen` impl. */
export function __setDispatchListenImpl(impl: DispatchListenFn | null): void {
  _listenImpl = impl;
}

async function getListen(): Promise<DispatchListenFn> {
  if (_listenImpl) return _listenImpl;
  if (typeof window === 'undefined') return async () => () => {};
  const w = window as unknown as {
    __TAURI_INTERNALS__?: unknown;
    __TAURI__?: unknown;
  };
  if (!w.__TAURI_INTERNALS__ && !w.__TAURI__) {
    return async () => () => {};
  }
  const mod = (await import('@tauri-apps/api/event')) as unknown as {
    listen: DispatchListenFn;
  };
  return mod.listen;
}

// ── Helpers ──────────────────────────────────────────────────────────────

const MAX_PROMPT = 8_000;

function isPromptValid(p: string): boolean {
  const t = p.trim();
  return t.length > 0 && p.length <= MAX_PROMPT;
}

function phaseLabel(phase: DispatchPhase): string {
  switch (phase) {
    case 'submitting':
      return 'submitting…';
    case 'queued':
      return 'queued';
    case 'running':
      return 'running';
    case 'done':
      return 'done';
    case 'failed':
      return 'failed';
    case 'cancelled':
      return 'cancelled';
    default:
      return 'idle';
  }
}

function phaseColor(phase: DispatchPhase): string {
  switch (phase) {
    case 'done':
      return 'text-green-400';
    case 'failed':
      return 'text-spectyn-danger';
    case 'cancelled':
      return 'text-spectyn-muted';
    case 'running':
    case 'queued':
    case 'submitting':
      return 'text-spectyn-primary';
    default:
      return 'text-spectyn-muted';
  }
}

// ── Component ────────────────────────────────────────────────────────────

export default function MobileDispatch() {
  const current = useDispatchStore((s) => s.current);
  const byId = useDispatchStore((s) => s.byId);
  const startDispatch = useDispatchStore((s) => s.startDispatch);
  const applyFrame = useDispatchStore((s) => s.applyFrame);
  const markCancelled = useDispatchStore((s) => s.markCancelled);

  const [prompt, setPrompt] = useState('');
  const [caps, setCaps] = useState<string[]>([]);
  const [provider, setProvider] = useState<string>(''); // '' = broker picks
  const [providers, setProviders] = useState<DispatchProvider[]>([]);
  const [submitError, setSubmitError] = useState<string | null>(null);

  // Tear down the per-dispatch event listener on unmount.
  const unlistenRef = useRef<(() => void) | null>(null);
  useEffect(() => {
    return () => {
      if (unlistenRef.current) unlistenRef.current();
    };
  }, []);

  // Pull provider allow-list once. Fire-and-forget; an empty list just
  // means "no override dropdown options" — broker default still applies.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const list = await safeInvoke<DispatchProvider[]>('list_dispatch_providers');
        if (!cancelled && Array.isArray(list)) setProviders(list);
      } catch {
        /* non-fatal — dropdown stays empty */
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const currentState = current ? byId[current] : null;
  const currentPhase = currentState?.phase ?? 'idle';
  const inFlight =
    currentState !== null &&
    currentState !== undefined &&
    !isTerminalPhase(currentPhase);

  // ── caps toggle ──
  const toggleCap = useCallback((cap: string) => {
    setCaps((prev) => {
      if (prev.includes(cap)) return prev.filter((c) => c !== cap);
      if (prev.length >= MAX_CAPS_SELECTABLE) return prev; // capped
      return [...prev, cap];
    });
  }, []);

  // ── submit ──
  const submit = useCallback(async () => {
    if (!isPromptValid(prompt)) return;
    setSubmitError(null);

    // Detach any previous listener (one-active-dispatch UI per F103 §scope).
    if (unlistenRef.current) {
      unlistenRef.current();
      unlistenRef.current = null;
    }

    try {
      const handle = await safeInvoke<DispatchHandle>('dispatch_task', {
        request: {
          prompt,
          required_caps: caps,
          provider_override: provider || null,
        },
      });
      const id = handle.dispatch_id;
      startDispatch({
        id,
        prompt,
        caps: [...caps],
        provider: provider || undefined,
        startedAt: handle.started_at_unix * 1000,
      });

      // Attach the per-dispatch event listener.
      const listen = await getListen();
      const evName = `dispatch::token::${id}`;
      unlistenRef.current = await listen<DispatchFrame>(evName, (ev) => {
        applyFrame(id, ev.payload);
      });
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setSubmitError(msg);
    }
  }, [prompt, caps, provider, startDispatch, applyFrame]);

  // ── cancel ──
  const cancel = useCallback(async () => {
    if (!current) return;
    if (currentState && isTerminalPhase(currentState.phase)) return;
    try {
      await safeInvoke('cancel_dispatch', { dispatchId: current });
    } catch {
      /* non-fatal — store will still mark cancelled below */
    }
    markCancelled(current);
  }, [current, currentState, markCancelled]);

  const tokenJoined = useMemo(
    () => (currentState ? currentState.tokens.join('') : ''),
    [currentState],
  );

  // ── render ──
  const submitDisabled = !isPromptValid(prompt) || inFlight;
  return (
    <div
      className="flex flex-col h-full overflow-y-auto"
      data-testid="mobile-dispatch-root"
    >
      <div className="px-4 py-3 border-b border-spectyn-border flex items-center justify-between">
        <div className="text-sm text-spectyn-text">Dispatch</div>
        {currentState && (
          <div
            className={`text-xs font-medium ${phaseColor(currentPhase)}`}
            data-testid="dispatch-phase"
          >
            {phaseLabel(currentPhase)}
          </div>
        )}
      </div>

      <div className="p-3 space-y-3">
        {/* Prompt input */}
        <div>
          <label
            htmlFor="dispatch-prompt"
            className="block text-xs text-spectyn-muted mb-1"
          >
            What should I do?
          </label>
          <textarea
            id="dispatch-prompt"
            data-testid="dispatch-prompt"
            value={prompt}
            onChange={(e) => setPrompt(e.target.value.slice(0, MAX_PROMPT))}
            disabled={inFlight}
            rows={3}
            className="w-full bg-spectyn-card border border-spectyn-border rounded-lg p-2 text-sm text-spectyn-text resize-y min-h-[64px]"
            placeholder="Summarize this URL, run an experiment, fetch a stat…"
          />
          <div className="text-[10px] text-spectyn-muted mt-0.5 text-right">
            {prompt.length} / {MAX_PROMPT}
          </div>
        </div>

        {/* Caps chips */}
        <div>
          <div className="text-xs text-spectyn-muted mb-1">
            Require capabilities (optional, max {MAX_CAPS_SELECTABLE})
          </div>
          <div className="flex flex-wrap gap-1.5" data-testid="caps-chips">
            {AVAILABLE_CAPS.map((cap) => {
              const active = caps.includes(cap);
              const disabledByCap =
                !active && caps.length >= MAX_CAPS_SELECTABLE;
              return (
                <button
                  key={cap}
                  type="button"
                  onClick={() => toggleCap(cap)}
                  disabled={inFlight || disabledByCap}
                  aria-pressed={active}
                  data-testid={`cap-chip-${cap}`}
                  className={`text-xs px-2 py-1 rounded-full border transition-colors ${
                    active
                      ? 'bg-spectyn-primary/15 border-spectyn-primary text-spectyn-primary'
                      : 'bg-spectyn-card border-spectyn-border text-spectyn-muted hover:text-spectyn-text'
                  } ${disabledByCap ? 'opacity-50' : ''}`}
                >
                  {cap}
                </button>
              );
            })}
          </div>
        </div>

        {/* Provider override */}
        {providers.length > 0 && (
          <div>
            <label
              htmlFor="dispatch-provider"
              className="block text-xs text-spectyn-muted mb-1"
            >
              Provider override (optional)
            </label>
            <select
              id="dispatch-provider"
              data-testid="dispatch-provider"
              value={provider}
              onChange={(e) => setProvider(e.target.value)}
              disabled={inFlight}
              className="w-full bg-spectyn-card border border-spectyn-border rounded-lg p-2 text-sm text-spectyn-text"
            >
              <option value="">(broker picks)</option>
              {providers.map((p) => (
                <option key={p.name} value={p.name} disabled={!p.configured}>
                  {p.name}
                  {p.configured ? '' : ' (no key)'}
                </option>
              ))}
            </select>
          </div>
        )}

        {/* Submit / Cancel */}
        <div className="flex items-center gap-2">
          {inFlight ? (
            <button
              type="button"
              onClick={() => {
                void cancel();
              }}
              data-testid="dispatch-cancel"
              className="flex-1 flex items-center justify-center gap-2 bg-spectyn-card border border-spectyn-border text-spectyn-text rounded-lg py-2 text-sm hover:bg-spectyn-bg"
            >
              <X size={16} /> Cancel
            </button>
          ) : (
            <button
              type="button"
              onClick={() => {
                void submit();
              }}
              disabled={submitDisabled}
              data-testid="dispatch-submit"
              className="flex-1 flex items-center justify-center gap-2 bg-spectyn-primary text-white rounded-lg py-2 text-sm font-medium disabled:opacity-40 disabled:cursor-not-allowed"
            >
              {currentPhase === 'submitting' ? (
                <Loader2 size={16} className="animate-spin" />
              ) : (
                <Send size={16} />
              )}
              Dispatch
            </button>
          )}
        </div>

        {submitError && (() => {
          const friendly = friendlyDispatchError(submitError);
          return (
            <div
              className="px-2 space-y-0.5"
              data-testid="dispatch-error"
              role="alert"
            >
              <div className="text-xs text-spectyn-danger">{friendly.title}</div>
              {friendly.hint && (
                <div className="text-[11px] text-spectyn-muted">
                  {friendly.hint}
                </div>
              )}
              <div className="text-[10px] text-spectyn-muted/60 font-mono">
                {friendly.raw}
              </div>
            </div>
          );
        })()}

        {/* Token-stream output */}
        {currentState && (
          <div className="space-y-2" data-testid="dispatch-stream-region">
            <pre
              data-testid="dispatch-tokens"
              className="bg-spectyn-card border border-spectyn-border rounded-lg p-2 text-xs text-spectyn-text whitespace-pre-wrap break-words max-h-[40vh] overflow-y-auto"
            >
              {tokenJoined || '​'}
            </pre>

            {currentState.phase === 'done' && currentState.result && (
              <div
                className="bg-spectyn-card border border-green-500/40 rounded-lg p-2 text-sm text-spectyn-text"
                data-testid="dispatch-result"
              >
                <div className="text-[10px] text-green-400 uppercase tracking-wider mb-1">
                  result
                </div>
                {currentState.result}
              </div>
            )}

            {currentState.phase === 'failed' && (
              <div
                className="bg-spectyn-card border border-spectyn-danger/40 rounded-lg p-2 text-sm text-spectyn-text"
                data-testid="dispatch-failure"
                role="alert"
              >
                <div className="text-[10px] text-spectyn-danger uppercase tracking-wider mb-1">
                  {currentState.errorCode || 'error'}
                </div>
                {currentState.errorMessage || 'Dispatch failed'}
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
