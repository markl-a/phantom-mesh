// Front-end helper for the SPEC-14 `providers_*` Tauri command surface.
//
// Wave H2.2 React conversation surface uses this module to:
//   1. resolve a provider slug from (class, latency) via `providers_select_provider`
//   2. fire `providers_complete_streaming` and listen to `providers_complete_event`
//   3. expose a `cancel()` handle so the UI can abort the in-flight listener
//      (note: backend `complete()` is spawn_blocking without a cancel token,
//      so this only detaches the frontend listener — the worker thread still
//      runs to completion. Documented for future per-token streaming upgrade.)
//
// The streaming event payload shape is set by
// `commands::providers_wire::providers_complete_streaming` and is one of:
//   { kind: "started", request_id: string }
//   { kind: "done",    request_id: string, response: ProviderResponse }
//   { kind: "error",   request_id: string, error: string }
//
// Backend NOTE: real per-token streaming is not yet exposed by the core
// `providers_wire`. The Tauri seam keeps the event-channel pattern stable so
// when core grows a token stream we widen the `kind` enum without churning
// either side.

import { safeInvoke as invoke, isTauri } from "./tauri-compat";
import type { ProviderRequest } from "./generated/providers/ProviderRequest";
import type { ProviderResponse } from "./generated/providers/ProviderResponse";
import type { ProviderClass } from "./generated/providers/ProviderClass";
import type { LatencyClass } from "./generated/providers/LatencyClass";
import type { Message as ProviderMessage } from "./generated/providers/Message";
import type { ResponseFormat } from "./generated/providers/ResponseFormat";
import type { ToolDef } from "./generated/providers/ToolDef";

const COMPLETE_EVENT = "providers_complete_event";

type StreamingEventPayload =
  | { kind: "started"; request_id: string }
  | { kind: "done"; request_id: string; response: ProviderResponse }
  | { kind: "error"; request_id: string; error: string };

export interface StreamingCallbacks {
  onStarted?: (requestId: string) => void;
  onToken?: (token: string) => void;          // reserved — fires on done with full text today
  onDone?: (response: ProviderResponse) => void;
  onError?: (error: string) => void;
}

export interface StreamHandle {
  requestId: string;
  cancel: () => void;
}

/**
 * Resolve a provider slug for the given class + latency by walking the
 * SPEC-14 routing chain (agents.toml → fallback chain → first healthy).
 *
 * @throws string error code from `ProviderError` (e.g. `provider.no_match_class`)
 */
export async function selectProvider(
  klass: ProviderClass,
  latency: LatencyClass,
): Promise<string> {
  if (!isTauri()) {
    // Browser preview cannot resolve providers without a Tauri runtime;
    // surface a typed error string the UI can pipe into `describeError`.
    throw new Error("providers_streaming_browser_mode");
  }
  return invoke<string>("providers_select_provider", { class: klass, latency });
}

/**
 * One-shot synchronous completion (no streaming). Returns the full response
 * after the upstream provider finishes. Use {@link streamComplete} for the
 * UI-friendly event-driven flow.
 */
export async function complete(req: ProviderRequest): Promise<ProviderResponse> {
  return invoke<ProviderResponse>("providers_complete", { req });
}

/**
 * Streaming-style completion via the Tauri event bus. Returns a handle with
 * the backend-allocated `request_id` and a `cancel()` that detaches the
 * frontend listener (see module note re: cancellation semantics).
 *
 * Browser-mode (non-Tauri) fallback: synthesises a single onError so the UI
 * gets a clear "needs desktop runtime" signal instead of silently hanging.
 */
export async function streamComplete(
  req: ProviderRequest,
  callbacks: StreamingCallbacks,
): Promise<StreamHandle> {
  // Use a stable per-call id so the listener can ignore stale events from
  // earlier in-flight requests (e.g. user clicks regenerate twice).
  const requestId = makeRequestId();

  if (!isTauri()) {
    // Browser preview cannot reach the providers wire — surface the limitation
    // immediately rather than silently hanging on a never-firing listener.
    queueMicrotask(() => callbacks.onError?.("providers_streaming_browser_mode"));
    return { requestId, cancel: () => {} };
  }

  // Lazy-load @tauri-apps/api/event so the browser bundle stays clean.
  const { listen } = await import("@tauri-apps/api/event");

  let unlisten: (() => void) | null = null;
  let detached = false;

  const detach = () => {
    if (detached) return;
    detached = true;
    if (typeof unlisten === "function") unlisten();
  };

  unlisten = await listen<StreamingEventPayload>(COMPLETE_EVENT, (event) => {
    const payload = event.payload;
    // Filter out events not addressed to this request id — Tauri's event bus
    // is global, so concurrent streams on the same window would otherwise
    // cross-contaminate.
    if (!payload || payload.request_id !== requestId) return;

    switch (payload.kind) {
      case "started":
        callbacks.onStarted?.(payload.request_id);
        break;
      case "done":
        // Today the backend ships the whole `response.text` in one shot; we
        // mirror it onto onToken so per-token consumers don't need a second
        // path once the core gains streaming.
        callbacks.onToken?.(payload.response.text);
        callbacks.onDone?.(payload.response);
        detach();
        break;
      case "error":
        callbacks.onError?.(payload.error);
        detach();
        break;
    }
  });

  try {
    await invoke<string>("providers_complete_streaming", {
      requestId,
      req,
    });
  } catch (e) {
    callbacks.onError?.(String(e));
    detach();
  }

  return { requestId, cancel: detach };
}

/**
 * Validate a `ProviderConfig` against the wire-level rules before persisting
 * it (slug non-empty, api_key_ref non-empty, default_model non-empty, etc.).
 */
export async function validateConfig(config: unknown): Promise<void> {
  return invoke<void>("providers_validate_config", { config });
}

// ── Helpers ────────────────────────────────────────────────────────────────

/**
 * Convenience builder so callers don't have to remember the camelCase /
 * `null` shape required by ts-rs. Pairs nicely with chat UIs that already
 * track messages as `{ role, content }[]`.
 */
export function buildRequest(opts: {
  model: string;
  messages: ProviderMessage[];
  systemPrompt?: string | null;
  maxTokens?: number | null;
  temperature?: number | null;
  responseFormat?: ResponseFormat;
  tools?: ToolDef[];
}): ProviderRequest {
  return {
    model: opts.model,
    systemPrompt: opts.systemPrompt ?? null,
    messages: opts.messages,
    maxTokens: opts.maxTokens ?? null,
    temperature: opts.temperature ?? null,
    responseFormat: opts.responseFormat ?? "plain_text",
    tools: opts.tools ?? [],
  };
}

/**
 * Map a `ProviderError` code (returned by the backend as a stringified enum)
 * to a user-friendly Traditional Chinese line. Falls through to the raw
 * string so devs can still see the underlying `provider.xxx` slug.
 */
export function describeError(err: string): string {
  if (!err) return "未知錯誤";
  if (err.includes("rate_limit")) return "Provider 已達速率上限，請稍後再試";
  if (err.includes("auth_error")) return "Provider 認證失敗，請檢查 API key";
  if (err.includes("network_error")) return "網路連線失敗";
  // Native fetch/WKWebView failures surface as "Load failed" / "Failed to
  // fetch" / a bare TypeError — humanise them rather than leaking the raw
  // exception string to the UI.
  if (/load failed|failed to fetch|networkerror|err_connection|err_failed/i.test(err)) {
    return "連線失敗（檢查網路，或目標服務是否啟動）";
  }
  if (err.includes("model_not_found")) return "找不到指定的 model";
  if (err.includes("context_too_long")) return "對話內容超過 model 上下文長度";
  if (err.includes("fallback_exhausted")) return "所有 fallback provider 都失敗了";
  if (err.includes("no_match_class")) return "沒有符合條件的 provider（檢查 agents.toml）";
  if (err.includes("cost_budget_exceeded")) return "已超出成本預算上限";
  if (err.includes("providers_streaming_browser_mode")) {
    return "Provider streaming 只在桌面 / 行動 App 內運作（瀏覽器模式不支援）";
  }
  // No usable LLM key. The backend returns a long CLI/TUI-oriented message
  // ("All providers failed ... no key ... env var X unset ... run `spectyn
  // config pull` ... Open /priority in TUI") — none of which applies on iOS.
  // Replace it with a short, actionable prompt to set a key in Settings.
  if (/no provider had a usable key|_api_key unset|no key —/i.test(err)) {
    return "還沒設定 LLM API key — 到「設定 → 手動填 LLM API key」貼上 OPENAI 或 GROQ key，或登入 phantommesh.io，即可開始對話。";
  }
  // Cluster dispatch HMAC auth failure — the X-Cluster-Auth header didn't match
  // the coordinator's cluster_secret. Point at the fix instead of leaking
  // "assign 401: unauthorized — bad or missing X-Cluster-Auth".
  if (/x-cluster-auth|assign 401|assign 403|cluster.*unauthorized/i.test(err)) {
    return "Cluster 認證失敗：你填的 secret 與 coordinator 不符。到「設定 → Cluster 派送」確認 secret 跟 coordinator 的 cluster_secret 一致。";
  }
  return err;
}

function makeRequestId(): string {
  // Lightweight uuid v4 alternative — `crypto.randomUUID` lives in modern
  // browsers + Tauri 2 webview. Falls back to a timestamp+random tuple if
  // a host strips it (e.g. iOS WKWebView prior to 16.4).
  try {
    if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
      return crypto.randomUUID();
    }
  } catch { /* swallow — fall through to fallback */ }
  return `req-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
}

// Re-export the generated types so consumers only import from one path.
export type {
  ProviderRequest,
  ProviderResponse,
  ProviderClass,
  LatencyClass,
  ProviderMessage,
  ResponseFormat,
};
