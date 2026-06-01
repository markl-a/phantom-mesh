import { describe, it, expect } from "vitest";
import { decideSystemBack } from "./systemBack";

// SPEC-34 §10D — pure-function tests for the Android system-back decision.
// No render / jsdom needed: decideSystemBack maps a history index to an action.
describe("decideSystemBack (SPEC-34 §10D)", () => {
  it("idx 0 → passthrough (first SPA entry → let Android exit)", () => {
    expect(decideSystemBack(0)).toBe("passthrough");
  });

  it("idx 1 → navigate-back (SPA history to pop)", () => {
    expect(decideSystemBack(1)).toBe("navigate-back");
  });

  it("idx 2 → navigate-back", () => {
    expect(decideSystemBack(2)).toBe("navigate-back");
  });

  it("large idx → navigate-back", () => {
    expect(decideSystemBack(999)).toBe("navigate-back");
  });

  it("undefined idx → passthrough (treated as first entry)", () => {
    expect(decideSystemBack(undefined)).toBe("passthrough");
  });

  it("null idx → passthrough (treated as first entry)", () => {
    expect(decideSystemBack(null)).toBe("passthrough");
  });
});
