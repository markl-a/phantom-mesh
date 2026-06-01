// Helper for the Recall (content search) surface — app counterpart of the TUI
// `/recall` + CLI `phantom recall` (BIG-GOAL P2 Life Track). Wraps the
// read-only `recall_search` Tauri command over the shared file event store.

import { safeInvoke as invoke } from "./tauri-compat";

export interface RecallHit {
  eventId: string;
  timestamp: string;
  kind: string;
  summary: string;
}

export type RecallKind = "food" | "focus" | "habit" | "text";

/** Search past Life Node events by content. Empty query → recent events. */
export async function recallSearch(opts: {
  query: string;
  kind?: RecallKind | null;
  since?: string | null;
  limit?: number;
}): Promise<RecallHit[]> {
  const res = await invoke<RecallHit[]>("recall_search", {
    query: opts.query,
    kind: opts.kind ?? null,
    since: opts.since ?? null,
    limit: opts.limit ?? 50,
  });
  return Array.isArray(res) ? res : [];
}

export const RECALL_KIND_META: Record<string, { label: string; emoji: string }> = {
  food: { label: "飲食", emoji: "🍽" },
  focus: { label: "專注", emoji: "🎯" },
  habit: { label: "習慣", emoji: "✅" },
  text: { label: "文字", emoji: "📝" },
};
