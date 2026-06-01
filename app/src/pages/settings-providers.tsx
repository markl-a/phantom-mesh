// SPEC-31 settings-providers — route /settings/providers; commands_used: provider_* (via lib/providers.ts)
//
// Mobile provider settings (供應商設定 / Provider settings).
//
// HONESTY NOTE: lib/providers.ts is the SPEC-14 *completion* surface — it
// exposes selectProvider / complete / streamComplete / validateConfig /
// describeError + generated types, but NO list/add/edit/test/remove CRUD
// helpers. The only real *read* path for "which providers are configured" is
// the daemon route `get_provider_health` (wired in lib/tauri-compat.ts →
// GET /api/providers/health). So:
//   • LIST  → real (get_provider_health). A thrown call = error state; a
//             null/empty result = honest empty state (never conflated).
//   • EDIT / TEST / REMOVE → NO backend helper exists in providers.ts and no
//             matching Tauri command is wired → rendered DISABLED with a
//             「尚未實作 / Not implemented」 note. We never fake a result.
//   • masked key last-4 → the health payload does NOT carry key bytes (by
//             design — keys live in the SPEC-13 vault, see ProviderConfig
//             doc-comment), so we show 「— 未提供 / not exposed」 honestly.
//   • + 新增供應商 → DISABLED (no add helper / command wired).

import { useEffect, useRef, useState } from "react";
import {
  Plus,
  RefreshCw,
  AlertTriangle,
  ServerOff,
  CircleCheck,
  CircleDot,
  Pencil,
  Activity,
  Trash2,
} from "lucide-react";
import { safeInvoke } from "../lib/tauri-compat";
import { describeError } from "../lib/providers";
import { useHaptics } from "../lib/useHaptics";

// ── Honest shape of the daemon health payload ───────────────────────────────
// We do not over-claim a strict schema; the route returns provider health and
// we read defensively. Each entry minimally identifies a provider by slug/name
// and an optional status string.
interface RawProviderHealth {
  slug?: string;
  name?: string;
  provider?: string;
  status?: string;
  healthy?: boolean;
  ok?: boolean;
  // masked key last-4 is intentionally absent from this route.
  key_last4?: string | null;
}

interface ProviderRow {
  id: string;
  name: string;
  status: "healthy" | "unhealthy" | "unknown";
  statusLabel: { zh: string; en: string };
  keyLast4: string | null;
}

type LoadState =
  | { phase: "loading" }
  | { phase: "ready"; rows: ProviderRow[] }
  | { phase: "empty" }
  | { phase: "error"; message: string };

function normalize(raw: unknown): ProviderRow[] {
  // Accept either `[...]` or `{ providers: [...] }` / `{ health: [...] }`.
  let list: unknown[] = [];
  if (Array.isArray(raw)) {
    list = raw;
  } else if (raw && typeof raw === "object") {
    const obj = raw as Record<string, unknown>;
    if (Array.isArray(obj.providers)) list = obj.providers;
    else if (Array.isArray(obj.health)) list = obj.health;
    else if (Array.isArray(obj.entries)) list = obj.entries;
  }

  // The `is RawProviderHealth` cast cannot vouch for a malformed runtime payload,
  // so coerce each candidate through a string guard before use — an object in
  // e.name would otherwise become a React child and crash the render.
  const str = (x: unknown): string | null =>
    typeof x === "string" && x.trim() ? x : null;

  return list
    .filter((e): e is RawProviderHealth => !!e && typeof e === "object")
    .map((e, i): ProviderRow => {
      const name = str(e.name) ?? str(e.slug) ?? str(e.provider) ?? `provider-${i + 1}`;
      const healthy =
        typeof e.healthy === "boolean"
          ? e.healthy
          : typeof e.ok === "boolean"
            ? e.ok
            : typeof e.status === "string"
              ? /^(ok|healthy|up|ready)$/i.test(e.status)
              : undefined;
      let status: ProviderRow["status"];
      let statusLabel: { zh: string; en: string };
      if (healthy === true) {
        status = "healthy";
        statusLabel = { zh: "正常", en: "Healthy" };
      } else if (healthy === false) {
        status = "unhealthy";
        statusLabel = { zh: "異常", en: "Unhealthy" };
      } else {
        status = "unknown";
        statusLabel = { zh: "未知", en: "Unknown" };
      }
      return {
        // index-suffixed so repeated names can't collide as React keys.
        id: `${str(e.slug) ?? name}#${i}`,
        name,
        status,
        statusLabel,
        keyLast4: typeof e.key_last4 === "string" ? e.key_last4 : null,
      };
    });
}

export default function SettingsProviders() {
  const { impact } = useHaptics();
  const [state, setState] = useState<LoadState>({ phase: "loading" });

  // ── Async safety: sequence guard (latest-wins) + alive ref ────────────────
  const seqRef = useRef(0);
  const aliveRef = useRef(true);

  useEffect(() => {
    aliveRef.current = true;
    return () => {
      aliveRef.current = false;
    };
  }, []);

  async function load() {
    const seq = ++seqRef.current;
    // Clear prior data at load start so nothing stale lingers on screen.
    setState({ phase: "loading" });
    try {
      const raw = await safeInvoke<unknown>("get_provider_health");
      if (!aliveRef.current || seq !== seqRef.current) return; // stale / unmounted
      const rows = normalize(raw);
      // Honest distinction: a non-throwing null/empty result is EMPTY, not error.
      setState(rows.length === 0 ? { phase: "empty" } : { phase: "ready", rows });
    } catch (e) {
      if (!aliveRef.current || seq !== seqRef.current) return;
      // A thrown/rejected backend call is an ERROR state (never faked as empty).
      setState({ phase: "error", message: describeError(String(e)) });
    }
  }

  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const onRefresh = () => {
    void load();
  };

  // ── Per-row actions: NO backend wired → disabled + 尚未實作 ────────────────
  const notImplementedTitle = "尚未實作 / Not implemented";

  return (
    <div
      data-testid="settings-providers"
      className="flex min-h-screen flex-col bg-phantom-bg text-phantom-text
                 pt-[env(safe-area-inset-top)]
                 pl-[env(safe-area-inset-left)]
                 pr-[env(safe-area-inset-right)]"
    >
      {/* Header */}
      <header className="flex items-center justify-between gap-3 px-4 py-3">
        <div className="min-w-0">
          <h1 className="text-lg font-semibold leading-tight">供應商 / Providers</h1>
          <p className="text-base text-phantom-muted">
            管理 LLM 供應商 / Manage LLM providers
          </p>
        </div>
        <button
          type="button"
          onClick={onRefresh}
          disabled={state.phase === "loading"}
          aria-label="重新整理 / Refresh"
          className="flex min-h-[44px] min-w-[44px] items-center justify-center
                     rounded-xl border border-phantom-border bg-phantom-card
                     text-phantom-text transition-colors
                     motion-reduce:transition-none
                     disabled:opacity-50 active:bg-phantom-bg"
        >
          <RefreshCw
            aria-hidden="true"
            className={
              "h-5 w-5 " +
              (state.phase === "loading"
                ? "animate-spin motion-reduce:animate-none"
                : "")
            }
          />
        </button>
      </header>

      {/* Scrollable content (DOM order = visual order) */}
      <main className="flex-1 overflow-y-auto px-4 pb-4">
        {state.phase === "loading" && (
          <div
            role="status"
            aria-label="載入中 / Loading"
            className="flex flex-col items-center justify-center gap-3 py-16
                       text-phantom-muted"
          >
            <RefreshCw
              aria-hidden="true"
              className="h-6 w-6 animate-spin motion-reduce:animate-none"
            />
            <p className="text-base">載入供應商中… / Loading providers…</p>
          </div>
        )}

        {state.phase === "error" && (
          <div
            role="alert"
            className="mt-4 flex flex-col items-start gap-3 rounded-xl border
                       border-phantom-danger/40 bg-phantom-card p-4"
          >
            <div className="flex items-center gap-2 text-phantom-danger">
              <AlertTriangle aria-hidden="true" className="h-5 w-5 shrink-0" />
              <span className="text-base font-medium">
                載入失敗 / Failed to load
              </span>
            </div>
            <p className="break-words text-base text-phantom-text">
              {state.message}
            </p>
            <button
              type="button"
              onClick={onRefresh}
              aria-label="重試 / Retry"
              className="min-h-[44px] rounded-xl border border-phantom-border
                         bg-phantom-bg px-4 text-base text-phantom-text
                         transition-colors motion-reduce:transition-none
                         active:bg-phantom-card"
            >
              重試 / Retry
            </button>
          </div>
        )}

        {state.phase === "empty" && (
          <div
            role="status"
            className="flex flex-col items-center justify-center gap-3 py-16
                       text-center text-phantom-muted"
          >
            <ServerOff aria-hidden="true" className="h-8 w-8" />
            <p className="text-base">
              尚未設定任何供應商 / No providers configured yet
            </p>
            <p className="max-w-xs text-base text-phantom-muted">
              新增供應商功能尚未實作（後端未接線）。 / Adding providers is not
              implemented yet (backend not wired).
            </p>
          </div>
        )}

        {state.phase === "ready" && (
          <ul className="mt-2 flex flex-col gap-3" aria-label="供應商清單 / Provider list">
            {state.rows.map((row) => (
              <li
                key={row.id}
                className="rounded-xl border border-phantom-border bg-phantom-card p-4"
              >
                {/* Name + status */}
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0">
                    <p className="truncate text-lg font-medium text-phantom-text">
                      {row.name}
                    </p>
                    <div className="mt-1 flex items-center gap-1.5">
                      {row.status === "healthy" ? (
                        <CircleCheck
                          aria-hidden="true"
                          className="h-4 w-4 text-phantom-success"
                        />
                      ) : row.status === "unhealthy" ? (
                        <AlertTriangle
                          aria-hidden="true"
                          className="h-4 w-4 text-phantom-danger"
                        />
                      ) : (
                        <CircleDot
                          aria-hidden="true"
                          className="h-4 w-4 text-phantom-warning"
                        />
                      )}
                      <span
                        className={
                          "text-base " +
                          (row.status === "healthy"
                            ? "text-phantom-success"
                            : row.status === "unhealthy"
                              ? "text-phantom-danger"
                              : "text-phantom-warning")
                        }
                      >
                        {row.statusLabel.zh} / {row.statusLabel.en}
                      </span>
                    </div>
                  </div>
                </div>

                {/* Masked key last-4 (honestly not exposed by the health route) */}
                <p className="mt-2 text-base text-phantom-muted">
                  API key:{" "}
                  {row.keyLast4 ? (
                    <span className="font-mono text-phantom-text">
                      ···· {row.keyLast4}
                    </span>
                  ) : (
                    <span>— 未提供 / not exposed</span>
                  )}
                </p>

                {/* Row actions — all disabled (no backend helper / command wired) */}
                <div className="mt-3 flex flex-wrap items-center gap-2">
                  <button
                    type="button"
                    disabled
                    title={notImplementedTitle}
                    aria-label={`編輯 ${row.name}（尚未實作）/ Edit ${row.name} (not implemented)`}
                    className="flex min-h-[44px] items-center gap-1.5 rounded-xl
                               border border-phantom-border bg-phantom-bg px-3
                               text-base text-phantom-muted opacity-50"
                  >
                    <Pencil aria-hidden="true" className="h-4 w-4" />
                    編輯 / Edit
                  </button>
                  <button
                    type="button"
                    disabled
                    title={notImplementedTitle}
                    aria-label={`測試 ${row.name}（尚未實作）/ Test ${row.name} (not implemented)`}
                    className="flex min-h-[44px] items-center gap-1.5 rounded-xl
                               border border-phantom-border bg-phantom-bg px-3
                               text-base text-phantom-muted opacity-50"
                  >
                    <Activity aria-hidden="true" className="h-4 w-4" />
                    測試 / Test
                  </button>
                  <button
                    type="button"
                    disabled
                    title={notImplementedTitle}
                    aria-label={`移除 ${row.name}（尚未實作）/ Remove ${row.name} (not implemented)`}
                    className="flex min-h-[44px] items-center gap-1.5 rounded-xl
                               border border-phantom-border bg-phantom-bg px-3
                               text-base text-phantom-muted opacity-50"
                  >
                    <Trash2 aria-hidden="true" className="h-4 w-4" />
                    移除 / Remove
                  </button>
                </div>
                <p className="mt-2 text-base text-phantom-muted">
                  編輯／測試／移除尚未實作（後端未接線）。 / Edit / Test / Remove
                  not implemented yet (backend not wired).
                </p>
              </li>
            ))}
          </ul>
        )}
      </main>

      {/* Sticky-bottom CTA — reachability + safe-area bottom inset lives HERE
          (not on the root, to avoid doubling the inset). */}
      <footer
        className="sticky bottom-0 border-t border-phantom-border bg-phantom-bg
                   px-4 pt-3 pb-[max(0.75rem,env(safe-area-inset-bottom))]"
      >
        <button
          type="button"
          disabled
          title={notImplementedTitle}
          onClick={() => {
            // Honest no-op success haptic placeholder: this button is disabled
            // because no add-provider backend is wired. impact() stays here to
            // document where the success haptic belongs once wired.
            impact("medium");
          }}
          aria-label="新增供應商（尚未實作）/ Add provider (not implemented)"
          className="flex min-h-[48px] w-full items-center justify-center gap-2
                     rounded-xl bg-phantom-primary text-base font-semibold
                     text-phantom-bg transition-opacity
                     motion-reduce:transition-none opacity-50"
        >
          <Plus aria-hidden="true" className="h-5 w-5" />
          新增供應商 / Add provider
        </button>
        <p className="mt-2 text-center text-base text-phantom-muted">
          尚未實作（後端未接線） / Not implemented (backend not wired)
        </p>
      </footer>
    </div>
  );
}
