// Duration presets for the focus idle screen. Operator decision 2026-05-30:
// ship 10/15/25/50 + custom. FocusMode is a generated (ts-rs) enum with no
// `sprint15`, so the 15-min preset rides on mode:"custom" + a fixed 15-min
// duration — frontend-only, no core change. A future `sprint15` enum value
// (cross-scope) can replace this mapping later.
import type { FocusMode } from "./generated/capture_focus/FocusMode";

export interface FocusPreset {
  key: "p10" | "p15" | "p25" | "p50" | "custom";
  label: string;
  /** minutes for fixed presets; null = use the user's custom-minute input */
  minutes: number | null;
  mode: FocusMode;
}

export const FOCUS_PRESETS: FocusPreset[] = [
  { key: "p10", label: "短衝 10 分", minutes: 10, mode: "sprint10" },
  { key: "p15", label: "短衝 15 分", minutes: 15, mode: "custom" },
  { key: "p25", label: "番茄鐘 25 分", minutes: 25, mode: "pomodoro25" },
  { key: "p50", label: "深度工作 50 分", minutes: 50, mode: "deep_work50" },
  { key: "custom", label: "自訂", minutes: null, mode: "custom" },
];

export function presetToPlan(
  key: FocusPreset["key"],
  customMin: number,
): { mode: FocusMode; plannedMs: number } {
  const p = FOCUS_PRESETS.find((x) => x.key === key) ?? FOCUS_PRESETS[2]!;
  const mins = p.minutes ?? Math.max(1, customMin);
  return { mode: p.mode, plannedMs: mins * 60 * 1000 };
}
