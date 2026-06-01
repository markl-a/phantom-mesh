import { describe, it, expect, beforeEach } from "vitest";
import {
  WIZARD_STEPS, nextStep, prevStep, isOnboarded, markOnboarded, ONBOARDED_KEY,
} from "@/lib/onboardingWizard";

beforeEach(() => localStorage.clear());

describe("onboardingWizard", () => {
  it("has 5 steps in order", () => {
    expect(WIZARD_STEPS).toEqual(["welcome", "permissions", "palette", "mesh", "done"]);
  });
  it("advances and stops at done", () => {
    expect(nextStep("welcome")).toBe("permissions");
    expect(nextStep("done")).toBe("done");
  });
  it("retreats and stops at welcome", () => {
    expect(prevStep("permissions")).toBe("welcome");
    expect(prevStep("welcome")).toBe("welcome");
  });
  it("persists onboarded flag", () => {
    expect(isOnboarded()).toBe(false);
    markOnboarded();
    expect(localStorage.getItem(ONBOARDED_KEY)).toBe("true");
    expect(isOnboarded()).toBe(true);
  });
});
