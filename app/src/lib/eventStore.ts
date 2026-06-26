// Helper for SPEC-16 event storage (read side) — wraps the Tauri commands in
// `app/src-tauri/src/commands/event_storage_wire.rs`:
//   - events_query(query)        → EventRecord[]   (plaintext metadata; bodies encrypted)
//   - events_search(query, limit) → string[]        (FTS5 → matching event ids)
//
// query_events is real (reads meta.json sidecars, no decryption), so the
// timeline shows real captured events (food/focus/habit/text) written via
// POST /api/events or the capture pipelines.

import { safeInvoke as invoke } from "./tauri-compat";
import type { EventRecord } from "./generated/event_storage/EventRecord";
import type { EventStoreQuery } from "./generated/event_storage/EventStoreQuery";
import type { EventKind } from "./generated/rpc/EventKind";

/** Build an EventStoreQuery; all filters optional. */
export function buildQuery(
  opts: { kind?: EventKind | null; dateIso?: string | null; tag?: string | null; limit?: number } = {},
): EventStoreQuery {
  return {
    dateIso: opts.dateIso ?? null,
    kind: opts.kind ?? null,
    tag: opts.tag ?? null,
    limit: opts.limit ?? 50,
    offset: null,
  };
}

/** Query the event store → metadata records (newest-first is the caller's job). */
export async function queryEvents(query: EventStoreQuery): Promise<EventRecord[]> {
  const res = await invoke<EventRecord[]>("events_query", { query });
  return Array.isArray(res) ? res : [];
}

/** FTS5 search → matching event ids. */
export async function searchEvents(query: string, limit = 20): Promise<string[]> {
  const res = await invoke<string[]>("events_search", { query, limit });
  return Array.isArray(res) ? res : [];
}

/** Quick text-note capture → new event id. App twin of `/note` + `phantom
 *  note`; writes a kind="note" Life Node event to the shared store. */
export async function captureNote(text: string, tags?: string[]): Promise<string | null> {
  const res = await invoke<string>("note_capture", { text, tags: tags ?? null });
  return typeof res === "string" && res ? res : null;
}

/** Delete a single event by id (BIG-GOAL reversibility). App twin of `phantom
 *  data delete <event-id>`. Resolves to the deleted id, or throws the typed
 *  error string (event_delete.failed: …) for the UI to surface. */
export async function deleteEvent(eventId: string): Promise<string> {
  return invoke<string>("event_delete", { eventId });
}

/** One event's full detail — metadata + decrypted LLM analysis. App twin of
 *  `phantom event show <id>`. Analysis fields are null when the event has no
 *  analysis.json (or it's locked without the identity key). */
export interface EventDetail {
  eventId: string;
  timestamp: string;
  kind: string;
  tags: string[];
  summary: string | null;
  suggestion: string | null;
  goalImpact: string | null;
  confidence: number | null;
  modelId: string | null;
}

/** Load one event's full detail by id. Returns null in web mode (unwired). */
export async function showEvent(eventId: string): Promise<EventDetail | null> {
  const res = await invoke<EventDetail>("event_show", { eventId });
  if (!res || typeof (res as EventDetail).eventId !== "string") return null;
  return res as EventDetail;
}

/** Display metadata per event kind. */
export const KIND_META: Record<EventKind, { label: string; emoji: string }> = {
  food: { label: "飲食", emoji: "🍽" },
  focus: { label: "專注", emoji: "🎯" },
  habit: { label: "習慣", emoji: "✅" },
  dispatch: { label: "派工", emoji: "📤" },
  text: { label: "文字", emoji: "📝" },
};

export function describeEventError(err: unknown): string {
  const s = String(err ?? "").trim();
  if (s.startsWith("events.not_yet_wired")) return "事件儲存後端暫時無法使用";
  if (s.includes("decryption")) return "部分事件需要解鎖金鑰才能讀取內容";
  return s || "未知錯誤";
}
