// Helper for SPEC-22 habit chip / text capture — thin wrapper over the four
// Tauri commands in `app/src-tauri/src/commands/capture_habit_wire.rs`:
//   - habit_create(def)        → void
//   - habit_checkin(checkin)   → HabitStreak
//   - habit_list()             → HabitSummary[]
//   - habit_streak(slug)       → HabitStreak
//
// The core capture_habit_wire is Stage-3 wired (persists to
// ~/.phantom-mesh/habits.sqlite — real chip_palette + habit_checkins tables),
// so these return real data in the Tauri app. describeHabitError still maps the
// typed wire errors (chip_not_found, etc.) for the UI. Mirrors captureFocus.ts.

import { safeInvoke as invoke } from "./tauri-compat";
import type { HabitCheckin } from "./generated/capture_habit/HabitCheckin";
import type { HabitCheckinSource } from "./generated/capture_habit/HabitCheckinSource";
import type { HabitStreak } from "./generated/capture_habit/HabitStreak";
import type { HabitSummary } from "./generated/capture_habit/HabitSummary";
import type { HabitDefinition } from "./generated/capture_habit/HabitDefinition";

/** A palette chip — display shell over a SPEC-22 HabitDefinition slug. */
export interface Chip {
  slug: string;
  label: string;
  emoji: string;
}

/**
 * SPEC-22 §31 v0.6.0 starter palette — 12 chips. User-editable later; this is
 * the default shown in the ChipPopover (SPEC-41 §10.3) and dashboard habit cards.
 */
export const STARTER_PALETTE: Chip[] = [
  { slug: "water", label: "水", emoji: "💧" },
  { slug: "coffee", label: "咖啡", emoji: "☕" },
  { slug: "exercise", label: "運動", emoji: "🏃" },
  { slug: "meditate", label: "冥想", emoji: "🧘" },
  { slug: "read", label: "讀書", emoji: "📖" },
  { slug: "walk", label: "走路", emoji: "🚶" },
  { slug: "quit_smoke", label: "戒菸", emoji: "🚭" },
  { slug: "quit_alcohol", label: "戒酒", emoji: "🍺" },
  { slug: "breath", label: "深呼吸", emoji: "🫁" },
  { slug: "stretch", label: "伸展", emoji: "🤸" },
  { slug: "journal", label: "寫日記", emoji: "✍" },
  { slug: "early_sleep", label: "早睡", emoji: "🌙" },
];

/** SPEC-22 §8.1 quantifiable chips → quick-pick options + unit. Non-listed
 *  slugs write directly with no qty. Interim: qty is encoded into `note`
 *  ("250ml") until the structured HabitMetadata wire field lands (🔒 core). */
export const QUANTIFIABLE: Record<string, { unit: string; quick: number[] }> = {
  water: { unit: "ml", quick: [250, 500] },
  coffee: { unit: "cup", quick: [1, 2] },
  exercise: { unit: "min", quick: [30, 45] },
  read: { unit: "min", quick: [15, 30] },
  walk: { unit: "min", quick: [15, 30] },
  meditate: { unit: "min", quick: [5, 10] },
  stretch: { unit: "min", quick: [5, 10] },
};

/** True if a chip should open the qty quick-pick before writing. */
export function isQuantifiable(slug: string): boolean {
  return slug in QUANTIFIABLE;
}

/** Build a HabitCheckin for "now" (timestampMs is i64 → bigint via ts-rs). */
export function buildCheckin(
  habitSlug: string,
  opts: { note?: string | null; source?: HabitCheckinSource } = {},
): HabitCheckin {
  return {
    habitSlug,
    timestampMs: BigInt(Date.now()),
    note: opts.note ?? null,
    source: opts.source ?? "manual",
  };
}

/** Record one check-in; returns the recomputed streak on success. */
export async function recordCheckin(checkin: HabitCheckin): Promise<HabitStreak> {
  // `timestampMs` is a BigInt (ts-rs maps Rust i64) → JSON-incompatible in Tauri invoke
  // ("Do not know how to serialize a BigInt"), which silently broke habit check-ins
  // (streak never persisted). Coerce to a plain number on the wire (epoch-ms « 2^53).
  const wire = { ...checkin, timestampMs: Number(checkin.timestampMs) };
  return invoke<HabitStreak>("habit_checkin", { checkin: wire });
}

/** Dashboard rollup — one HabitSummary per palette chip. */
export async function listHabits(): Promise<HabitSummary[]> {
  return invoke<HabitSummary[]>("habit_list");
}

/** Create a habit definition (palette chip). Errors if the slug already exists. */
export async function createHabit(def: HabitDefinition): Promise<void> {
  await invoke<null>("habit_create", { def });
}

/** Check in on a habit, creating it (daily, no tags) first if it doesn't exist
 *  yet. record_checkin errors ChipNotFound on an uncreated slug, so the app
 *  must register the palette chip before its first checkin (matching how
 *  `phantom habit create` then `checkin` works). Returns the recomputed streak. */
export async function ensureCheckin(
  slug: string,
  label: string,
  opts: { note?: string | null } = {},
): Promise<HabitStreak> {
  const existing = await listHabits().catch(() => [] as HabitSummary[]);
  if (!existing.some((h) => h.habitSlug === slug)) {
    await createHabit({
      slug,
      label: label || slug,
      targetFrequency: { kind: "daily" },
      tags: [],
      createdAt: new Date().toISOString(),
    });
  }
  return recordCheckin(buildCheckin(slug, { note: opts.note ?? null }));
}

/** Streak rollup for a single chip. */
export async function streak(habitSlug: string): Promise<HabitStreak> {
  return invoke<HabitStreak>("habit_streak", { habitSlug });
}

/** Map a habit.<code>:<detail> wire error to a UI-friendly Chinese string. */
export function describeHabitError(err: unknown): string {
  const s = String(err ?? "").trim();
  if (s.startsWith("habit.not_yet_wired")) return "後端尚未實作（SPEC-22 Stage 2 deferred）";
  if (s.startsWith("habit.chip_not_found")) return "找不到這個習慣 chip";
  if (s.startsWith("habit.chip_id_conflict")) return "chip 已存在";
  if (s.startsWith("habit.invalid_slug")) return "chip 代號格式不合法";
  if (s.startsWith("habit.palette_size_out_of_range")) return "palette 數量需在 6–12 之間";
  if (s.startsWith("habit.store")) return "寫入失敗";
  return s || "未知錯誤";
}
