// Helper for the Daily Review reader (SPEC-41 macOS screen #3, P2 Life Track).
// Wraps two Tauri commands in `app/src-tauri/src/commands/daily_review_wire.rs`:
//   - daily_review_load(date?)            → DailyReviewView   (offline; no LLM/network)
//   - daily_review_generate(date?, save?) → DailyReviewView   (adds the Gemini
//        "Tomorrow's one action" pass + optional save — == `spectyn coach review`)
//
// Both reuse the same backend as `spectyn coach review` (life_node::daily_review),
// so the screen shows the real captured Life Node events (food / focus / habit /
// text) for a date, grouped by goal-tag, plus the coaching action on generate.

import { safeInvoke as invoke } from "./tauri-compat";
import type { DailyReviewView } from "./generated/daily_review/DailyReviewView";

/** ISO (YYYY-MM-DD) for local-today — the default the backend also uses. */
export function todayIso(): string {
  const d = new Date();
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

/** Load the daily review for `date` (defaults to today). Never throws on a
 *  missing store — the backend returns a locked/empty view instead. */
export async function loadDailyReview(date?: string): Promise<DailyReviewView | null> {
  const res = await invoke<DailyReviewView>("daily_review_load", { date: date ?? null });
  // tauri-compat httpFallback may return {} for an unknown command in web mode.
  if (!res || typeof (res as DailyReviewView).markdown !== "string") return null;
  return res as DailyReviewView;
}

/** Generate (and by default persist) a full coach review for `date` — runs the
 *  Gemini "Tomorrow's one action" pass on top of the aggregate, mirroring
 *  `spectyn coach review --save`. Degrades gracefully (the action becomes a
 *  `(skipped: …)` note) when there's no GEMINI_API_KEY. Returns null in web mode
 *  where the command is unwired. */
export async function generateReview(date: string, save = true): Promise<DailyReviewView | null> {
  const res = await invoke<DailyReviewView>("daily_review_generate", { date, save });
  if (!res || typeof (res as DailyReviewView).markdown !== "string") return null;
  return res as DailyReviewView;
}

/** Pull the "Tomorrow's one action" coaching line out of a generated review.
 *  Returns null when the review is aggregate-only (load path never adds it).
 *  `skipped` is true when the LLM pass was skipped (no key / error) so callers
 *  can show a muted hint instead of presenting the footer text as advice. */
export function extractTomorrowAction(markdown: string): { text: string; skipped: boolean } | null {
  const m = markdown.match(/##\s+Tomorrow's one action\s*\n+([\s\S]*?)(?:\n#{1,2}\s|$)/);
  const body = m?.[1]?.trim();
  if (!body) return null;
  return { text: body, skipped: body.startsWith("(skipped") };
}

/** One parsed line of the aggregate Markdown for rendering. */
export type ReviewRow =
  | { kind: "title"; text: string }
  | { kind: "count"; text: string }
  | { kind: "group"; tag: string; n: number }
  | { kind: "bullet"; eventKind: string; time: string; summary: string }
  | { kind: "note"; text: string };

/** Parse the `aggregate()` Markdown into typed rows. Mirrors the structure the
 *  backend emits: `# Daily review — DATE`, `**Events captured:** N`,
 *  `## tag (n)`, `- **kind** (timestamp): summary`. Unknown lines → note. */
export function parseReview(markdown: string): ReviewRow[] {
  const rows: ReviewRow[] = [];
  for (const raw of markdown.split("\n")) {
    const line = raw.trimEnd();
    if (!line.trim()) continue;
    let m: RegExpMatchArray | null;
    if ((m = line.match(/^#\s+(.*)$/))) {
      rows.push({ kind: "title", text: m[1].trim() });
    } else if ((m = line.match(/^\*\*Events captured:\*\*\s*(.*)$/))) {
      rows.push({ kind: "count", text: m[1].trim() });
    } else if ((m = line.match(/^##\s+(.*?)\s*\((\d+)\)\s*$/))) {
      rows.push({ kind: "group", tag: m[1].trim(), n: Number(m[2]) });
    } else if ((m = line.match(/^[-*]\s+\*\*(.+?)\*\*\s*\((.+?)\):\s*(.*)$/))) {
      rows.push({ kind: "bullet", eventKind: m[1].trim(), time: hhmm(m[2].trim()), summary: m[3].trim() });
    } else {
      rows.push({ kind: "note", text: line.trim() });
    }
  }
  return rows;
}

/** Reduce an ISO timestamp (or any string) to HH:MM. Events are stored with
 *  `Local::now().to_rfc3339()` (offset preserved), so the literal HH:MM in the
 *  string is already the user's local time — extract it directly rather than
 *  let `new Date()` re-shift a UTC `Z` timestamp into a different local hour. */
function hhmm(ts: string): string {
  const lit = ts.match(/T(\d{2}:\d{2})/) || ts.match(/\b(\d{2}:\d{2})\b/);
  if (lit) return lit[1];
  const d = new Date(ts);
  if (!Number.isNaN(d.getTime())) {
    const p = (n: number) => String(n).padStart(2, "0");
    return `${p(d.getHours())}:${p(d.getMinutes())}`;
  }
  return ts;
}

export const KIND_EMOJI: Record<string, string> = {
  food: "🍽",
  focus: "🎯",
  habit: "✅",
  text: "📝",
  image: "🖼",
  audio: "🎤",
};
