// MIUI (小米系統) compatibility guide bridge — SPEC-33 §6(E) / SPEC-34 §9 + G6.
//
// MIUI / Redmi devices kill background apps far more aggressively than stock
// Android, so phantom's foreground MeshNodeService gets reaped overnight unless
// the user adds it to the auto-start whitelist AND the battery-optimization
// deny-list. We CANNOT set these programmatically (no public MIUI API) — the
// guide only deep-links the user to the right system screens and, as the
// substance, spells out the manual steps (which work regardless of whether the
// native intent launch is wired yet).
//
// Command contract (SPEC-34 §9, lines 458-462) — implemented natively in
// app/src-tauri/android (Kotlin intents + Jetpack DataStore):
//   miui_guide_check_should_show {} -> { should_show, last_dismissed_ms }
//   miui_guide_dismiss { dont_show_again } -> { ok }
//   miui_guide_open_autostart {} -> { ok }            (launches MIUI 安全中心 自啟動)
//   miui_guide_open_battery_optimization {} -> { ok } (ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS)
//
// All calls go through safeInvoke, so before the native side lands (or in a
// plain-browser dev build) they resolve to graceful defaults instead of
// throwing — the dialog still renders its manual-steps guidance.

import { safeInvoke } from "./tauri-compat";

export interface MiuiShouldShow {
  should_show: boolean;
  /** True when this device runs MIUI (Xiaomi / Redmi / POCO). */
  is_miui: boolean;
  last_dismissed_ms: number | null;
}

/** Native decides whether the guide applies: is_miui (ro.miui.ui.version.code
 *  present) AND not "don't show again". Defaults to not-showing / not-MIUI when
 *  the command is absent — the manual Settings entry is the fallback. */
export async function checkShouldShowMiuiGuide(): Promise<MiuiShouldShow> {
  try {
    const r = await safeInvoke<MiuiShouldShow>("miui_guide_check_should_show");
    if (r && typeof r.should_show === "boolean") {
      return { is_miui: !!r.is_miui, last_dismissed_ms: r.last_dismissed_ms ?? null, should_show: r.should_show };
    }
  } catch {
    /* native command absent — fall through to the safe default */
  }
  return { should_show: false, is_miui: false, last_dismissed_ms: null };
}

/** Persist the user's dismissal. `dontShowAgain` flips the permanent flag
 *  (only a manual Settings reset re-enables the auto-pop after that). */
export async function dismissMiuiGuide(dontShowAgain: boolean): Promise<void> {
  try {
    await safeInvoke("miui_guide_dismiss", { dontShowAgain });
  } catch {
    /* best-effort — losing the flag only means the guide may re-pop later */
  }
}

/** Best-effort deep-link to MIUI 安全中心 → 應用管理 → 自啟動.
 *  Returns false when the native intent isn't wired (caller then leans on the
 *  always-visible manual steps). */
export async function openMiuiAutostart(): Promise<boolean> {
  try {
    const r = await safeInvoke<{ ok?: boolean }>("miui_guide_open_autostart");
    return !!(r && r.ok);
  } catch {
    return false;
  }
}

/** Best-effort deep-link to the battery-optimization "don't optimize" prompt. */
export async function openMiuiBatteryOptimization(): Promise<boolean> {
  try {
    const r = await safeInvoke<{ ok?: boolean }>(
      "miui_guide_open_battery_optimization",
    );
    return !!(r && r.ok);
  } catch {
    return false;
  }
}
