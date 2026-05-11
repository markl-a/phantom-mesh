import { create } from 'zustand';

interface CostState {
  totalCostUsd: number;
  dailyBudgetUsd: number;
  todaySpentUsd: number;
  addCost: (amount: number) => void;
  setBudget: (budget: number) => void;
  setPartial: (cost: Partial<CostState>) => void;
  handleEvent: (payload: { type: string; data: Record<string, unknown> }) => void;
}

export const useCostStore = create<CostState>()((set) => ({
  totalCostUsd: 0,
  dailyBudgetUsd: 10,
  todaySpentUsd: 0,
  addCost: (amount: number) => set((s: CostState) => ({ totalCostUsd: s.totalCostUsd + amount, todaySpentUsd: s.todaySpentUsd + amount })),
  setBudget: (budget: number) => set({ dailyBudgetUsd: budget }),
  setPartial: (partial: Partial<CostState>) => set(partial),
  handleEvent: (payload: { type: string; data: Record<string, unknown> }) => {
    if (payload.type === 'CostAlert') {
      set((s: CostState) => ({ todaySpentUsd: (payload.data.today_spent as number) || s.todaySpentUsd }));
    }
  },
}));
