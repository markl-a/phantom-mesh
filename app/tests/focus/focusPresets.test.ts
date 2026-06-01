import { describe, it, expect } from "vitest";
import { FOCUS_PRESETS, presetToPlan } from "@/lib/focusPresets";

describe("focusPresets", () => {
  it("exposes 10/15/25/50 + custom in order", () => {
    expect(FOCUS_PRESETS.map((p) => p.key)).toEqual(["p10", "p15", "p25", "p50", "custom"]);
  });
  it("maps the 15-min preset to custom mode + 15 min (no sprint15 enum)", () => {
    expect(presetToPlan("p15", 25)).toEqual({ mode: "custom", plannedMs: 15 * 60 * 1000 });
  });
  it("maps 10/25/50 to their real FocusMode", () => {
    expect(presetToPlan("p10", 25)).toEqual({ mode: "sprint10", plannedMs: 10 * 60 * 1000 });
    expect(presetToPlan("p25", 25)).toEqual({ mode: "pomodoro25", plannedMs: 25 * 60 * 1000 });
    expect(presetToPlan("p50", 25)).toEqual({ mode: "deep_work50", plannedMs: 50 * 60 * 1000 });
  });
  it("custom uses the user minutes (min 1)", () => {
    expect(presetToPlan("custom", 40)).toEqual({ mode: "custom", plannedMs: 40 * 60 * 1000 });
    expect(presetToPlan("custom", 0)).toEqual({ mode: "custom", plannedMs: 1 * 60 * 1000 });
  });
});
