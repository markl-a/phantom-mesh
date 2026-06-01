// Lifecycle · Unit tests — cost store (dashboard "spend tracking" journey).
//
// As the user dispatches work, each completed call adds to today's spend and
// the lifetime total; the dashboard CostPanel reads todaySpentUsd vs
// dailyBudgetUsd to show the budget gauge. Backend CostAlert events also push
// an authoritative today-spent figure in. These are pure zustand reducers in
// src/stores/costStore.ts (no Tauri), tested directly. Mirrors the reset
// convention used in tests/f101/clusterPeersStore.test.ts.

import { beforeEach, describe, expect, it } from "vitest";
import { useCostStore } from "../../src/stores/costStore";

beforeEach(() => {
  // Restore the store to its initial shape so spend doesn't leak across cases.
  useCostStore.setState({
    totalCostUsd: 0,
    dailyBudgetUsd: 10,
    todaySpentUsd: 0,
  });
});

describe("costStore — defaults", () => {
  it("starts at zero spend with a $10 default daily budget", () => {
    const s = useCostStore.getState();
    expect(s.totalCostUsd).toBe(0);
    expect(s.todaySpentUsd).toBe(0);
    expect(s.dailyBudgetUsd).toBe(10);
  });
});

describe("costStore — addCost accumulates", () => {
  it("adds each charge to BOTH the lifetime total and today's spend", () => {
    useCostStore.getState().addCost(0.25);
    useCostStore.getState().addCost(0.75);
    const s = useCostStore.getState();
    expect(s.totalCostUsd).toBeCloseTo(1.0, 10);
    expect(s.todaySpentUsd).toBeCloseTo(1.0, 10);
  });

  it("handles a zero charge as a no-op on the figures", () => {
    useCostStore.getState().addCost(0);
    const s = useCostStore.getState();
    expect(s.totalCostUsd).toBe(0);
    expect(s.todaySpentUsd).toBe(0);
  });
});

describe("costStore — budget", () => {
  it("setBudget replaces the daily budget without touching spend", () => {
    useCostStore.getState().addCost(2);
    useCostStore.getState().setBudget(50);
    const s = useCostStore.getState();
    expect(s.dailyBudgetUsd).toBe(50);
    expect(s.todaySpentUsd).toBe(2); // unchanged
  });

  it("lets the dashboard detect an over-budget day", () => {
    useCostStore.getState().setBudget(1);
    useCostStore.getState().addCost(1.5);
    const s = useCostStore.getState();
    expect(s.todaySpentUsd).toBeGreaterThan(s.dailyBudgetUsd);
  });
});

describe("costStore — handleEvent (backend CostAlert)", () => {
  it("overwrites today's spend from a CostAlert event", () => {
    useCostStore.getState().addCost(3);
    useCostStore.getState().handleEvent({
      type: "CostAlert",
      data: { today_spent: 7.5 },
    });
    expect(useCostStore.getState().todaySpentUsd).toBe(7.5);
  });

  it("keeps the prior spend when the event omits today_spent", () => {
    useCostStore.getState().addCost(3);
    useCostStore.getState().handleEvent({ type: "CostAlert", data: {} });
    expect(useCostStore.getState().todaySpentUsd).toBe(3);
  });

  it("ignores unrelated event types", () => {
    useCostStore.getState().addCost(3);
    useCostStore.getState().handleEvent({
      type: "SomethingElse",
      data: { today_spent: 999 },
    });
    expect(useCostStore.getState().todaySpentUsd).toBe(3);
  });
});
