import { safeInvoke } from "./tauri-compat";

// SPEC-31 §HIG — haptic feedback on primary actions. tauri-plugin-haptics is not
// installed yet (SPEC-30 deferred); this fires a best-effort command that fail-soft
// no-ops via safeInvoke today and lights up once the plugin lands. Never throws.
export type HapticStyle = "light" | "medium" | "heavy";

export function useHaptics() {
  const impact = (style: HapticStyle = "medium") => {
    void safeInvoke("plugin:haptics|impact_feedback", { style }).catch(() => {});
  };

  return { impact };
}
