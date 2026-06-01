// J1 onboarding-wizard step machine (SPEC-34 Screen 1 / Flow 1 / SPEC-28).
// Pure step order + the localStorage "onboarded" flag (back-compat with the
// retired v2 mode-picker key). The SPEC-28 FSM snapshot is advanced separately
// via lib/onboardingFsm.ts — this module only owns wizard navigation.
export type WizardStep = "welcome" | "permissions" | "palette" | "mesh" | "done";

export const WIZARD_STEPS: WizardStep[] = ["welcome", "permissions", "palette", "mesh", "done"];

/** Back-compat: the v2 mode-picker used this key; reuse so existing installs skip onboarding. */
export const ONBOARDED_KEY = "phantom_mesh_v2_onboarded";

export function nextStep(s: WizardStep): WizardStep {
  const i = WIZARD_STEPS.indexOf(s);
  return WIZARD_STEPS[Math.min(i + 1, WIZARD_STEPS.length - 1)]!;
}

export function prevStep(s: WizardStep): WizardStep {
  const i = WIZARD_STEPS.indexOf(s);
  return WIZARD_STEPS[Math.max(i - 1, 0)]!;
}

export function isOnboarded(): boolean {
  if (typeof localStorage === "undefined") return false;
  return localStorage.getItem(ONBOARDED_KEY) === "true";
}

export function markOnboarded(): void {
  if (typeof localStorage === "undefined") return;
  try {
    localStorage.setItem(ONBOARDED_KEY, "true");
  } catch {
    /* private mode / quota — ignore */
  }
}
