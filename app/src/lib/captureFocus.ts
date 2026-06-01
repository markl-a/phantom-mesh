// H2.3 wave — Dashboard wiring helper for SPEC-21 capture_focus.
//
// Thin wrapper around the four Tauri commands registered in
// `app/src-tauri/src/commands/capture_focus_wire.rs`:
//   - focus_start_session(req)              → returns session_id string
//   - focus_record_interruption(id, kind)   → void
//   - focus_complete_session(id)            → returns FocusSessionResult
//   - focus_analyze_session(result)         → returns AnalysisResult (deferred)
//
// All four are panic-catch-wrapped on the Rust side and surface a stable
// `focus.not_yet_wired:` prefix when the underlying Stage 4 helper is missing
// (start_session, record_interruption, analyze_session). complete_session is
// fully wired and may return a typed `focus.session_not_found:` error.
//
// Because no backend list endpoint exists yet (SPEC-16 query layer is the
// future home), recently completed sessions are mirrored to localStorage so
// the Dashboard event list has something to show across reloads.

import { safeInvoke as invoke } from "./tauri-compat";
import type { FocusSessionRequest } from "./generated/capture_focus/FocusSessionRequest";
import type { FocusSessionResult } from "./generated/capture_focus/FocusSessionResult";
import type { InterruptionKind } from "./generated/capture_focus/InterruptionKind";
import type { FocusMode } from "./generated/capture_focus/FocusMode";

const RECENT_STORAGE_KEY = "phantom_mesh_capture_focus_recent_v1";
const RECENT_CAP = 25;

export interface RecentFocusEvent {
  /** Server-issued session id (uuid v7 once Stage 4 lands). */
  sessionId: string;
  /** Mode the user picked when starting the session. */
  mode: FocusMode;
  /** Optional human label captured at start time. */
  label: string | null;
  /** UTC millis when the session was completed (= when this row landed). */
  completedAtMs: number;
  /** Result returned by `complete_session`. */
  result: FocusSessionResult;
}

/**
 * Default duration in milliseconds per preset mode. UI uses these unless the
 * user explicitly overrides via a custom number entry. Matches the canonical
 * mode-name → duration mapping in SPEC-21 §5.
 */
export const DEFAULT_DURATION_MS: Record<FocusMode, number> = {
  pomodoro25: 25 * 60 * 1000,
  deep_work50: 50 * 60 * 1000,
  sprint10: 10 * 60 * 1000,
  custom: 25 * 60 * 1000,
};

/**
 * Human-readable Traditional Chinese label per mode, used by the Dashboard
 * select / pill UI. Kept here so the component stays display-only.
 */
export const MODE_LABEL: Record<FocusMode, string> = {
  pomodoro25: "番茄鐘 25 分",
  deep_work50: "深度工作 50 分",
  sprint10: "短衝 10 分",
  custom: "自訂",
};

/** Build a request shape, defaulting `tag` to `["focus"]` per SPEC-21 §0. */
export function buildSessionRequest(
  mode: FocusMode,
  opts: { plannedDurationMs?: number; label?: string | null; tag?: string[] } = {},
): FocusSessionRequest {
  return {
    mode,
    plannedDurationMs: BigInt(opts.plannedDurationMs ?? DEFAULT_DURATION_MS[mode]),
    label: opts.label ?? null,
    tag: opts.tag && opts.tag.length > 0 ? opts.tag : ["focus"],
  };
}

/** Invoke `focus_start_session`; returns the session id on success. */
export async function startSession(req: FocusSessionRequest): Promise<string> {
  // `plannedDurationMs` is a BigInt (ts-rs maps Rust `u64`), but Tauri's `invoke`
  // serializes its args as JSON and JSON cannot encode BigInt → "Do not know how
  // to serialize a BigInt", which crashed session creation (SPEC-21) on the real
  // Tauri app. Coerce to a plain number on the wire (durations are well under
  // 2^53). Same BigInt-invoke class as the onboarding fix; the systemic remedy is
  // coerceBigInts() in tauri-compat.ts safeInvoke (still latent for food/habit/broker).
  const wire = { ...req, plannedDurationMs: Number(req.plannedDurationMs) };
  return invoke<string>("focus_start_session", { req: wire });
}

/** Invoke `focus_record_interruption`. */
export async function recordInterruption(
  sessionId: string,
  kind: InterruptionKind,
): Promise<void> {
  await invoke<null>("focus_record_interruption", { sessionId, kind });
}

/**
 * Invoke `focus_complete_session`. On success, mirror the row to localStorage
 * so the Dashboard event list survives reloads.
 */
export async function completeSession(
  sessionId: string,
  meta: { mode: FocusMode; label: string | null },
): Promise<FocusSessionResult> {
  const result = await invoke<FocusSessionResult>("focus_complete_session", {
    sessionId,
  });
  appendRecent({
    sessionId,
    mode: meta.mode,
    label: meta.label,
    completedAtMs: Date.now(),
    result,
  });
  return result;
}

/**
 * Invoke `focus_analyze_session`. Returns the unknown-shaped `AnalysisResult`;
 * Stage 4 deferred so callers should expect a `focus.not_yet_wired:` error
 * for the foreseeable future and gate UI accordingly.
 */
export async function analyzeSession(result: FocusSessionResult): Promise<unknown> {
  // `actualDurationMs` is a BigInt (ts-rs u64); JSON/Tauri invoke cannot encode
  // BigInt → "Do not know how to serialize a BigInt". Coerce to a plain number
  // on the wire (durations are well under 2^53), mirroring startSession.
  const wire = { ...result, actualDurationMs: Number(result.actualDurationMs) };
  return invoke<unknown>("focus_analyze_session", { result: wire });
}

/** The active focus session as seen on disk — shared across CLI / TUI / app
 *  (single active session). `null` when none is active or in web mode. */
export interface ActiveFocus {
  sessionId: string;
  startedAtMs: number;
  plannedDurationMs: number;
  task: string | null;
  interruptions: number;
}

/** Read the shared disk-backed focus session so the app can surface a session
 *  started in another surface (CLI `phantom focus` / TUI `/focus`). */
export async function focusStatus(): Promise<ActiveFocus | null> {
  const res = await invoke<ActiveFocus | null>("focus_status", {});
  if (!res || typeof (res as ActiveFocus).sessionId !== "string") return null;
  return res as ActiveFocus;
}

/**
 * Map a focus.<code>:<detail> wire error to a UI-friendly Chinese string.
 * Falls back to the raw string for unknown shapes.
 */
export function describeFocusError(err: unknown): string {
  const s = String(err ?? "").trim();
  if (s.startsWith("focus.not_yet_wired"))
    return "後端尚未實作（Stage 4 deferred）";
  if (s.startsWith("focus.session_not_found"))
    return "找不到對應的 session（可能已結束或失效）";
  if (s.startsWith("focus.session_already_active"))
    return "已有進行中的 session，請先結束";
  if (s.startsWith("focus.permission_denied"))
    return "權限不足（請至系統設定授權）";
  if (s.startsWith("focus.recorder_init"))
    return "錄音/擷取裝置初始化失敗";
  if (s.startsWith("focus.interrupted"))
    return "session 被外部中斷";
  if (s.startsWith("focus.takeaway_failed"))
    return "摘要產生失敗";
  return s || "未知錯誤";
}

// ─── localStorage cache for recent completed sessions ──────────────────────

/**
 * Custom replacer used to serialize `FocusSessionResult` to JSON — its
 * `actualDurationMs` field is a BigInt (ts-rs maps Rust `u64`), and plain
 * `JSON.stringify` throws on BigInt. We tag the value so the reviver can
 * round-trip it back.
 */
function bigIntReplacer(_key: string, value: unknown): unknown {
  if (typeof value === "bigint") return { __bigint: value.toString() };
  return value;
}

function bigIntReviver(_key: string, value: unknown): unknown {
  if (
    value &&
    typeof value === "object" &&
    "__bigint" in (value as Record<string, unknown>)
  ) {
    return BigInt((value as { __bigint: string }).__bigint);
  }
  return value;
}

export function listRecent(): RecentFocusEvent[] {
  if (typeof localStorage === "undefined") return [];
  try {
    const raw = localStorage.getItem(RECENT_STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw, bigIntReviver) as unknown;
    if (!Array.isArray(parsed)) return [];
    return parsed as RecentFocusEvent[];
  } catch {
    return [];
  }
}

export function clearRecent(): void {
  if (typeof localStorage === "undefined") return;
  localStorage.removeItem(RECENT_STORAGE_KEY);
}

function appendRecent(event: RecentFocusEvent): void {
  if (typeof localStorage === "undefined") return;
  const next = [event, ...listRecent()].slice(0, RECENT_CAP);
  try {
    localStorage.setItem(RECENT_STORAGE_KEY, JSON.stringify(next, bigIntReplacer));
  } catch {
    // Quota exceeded or storage disabled — fail silent; list is best-effort.
  }
}
