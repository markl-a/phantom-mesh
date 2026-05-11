import { create } from 'zustand';

type PageArea = 'tasks' | 'devices' | 'settings';

interface PageState {
  currentArea: PageArea;
  commandPaletteOpen: boolean;
  heroInputFrozen: boolean;
  setArea: (area: PageArea) => void;
  openCommandPalette: () => void;
  closeCommandPalette: () => void;
  handleEvent: (payload: { type: string; data: Record<string, unknown> }) => void;
}

export const usePageStore = create<PageState>()((set) => ({
  currentArea: 'tasks' as PageArea,
  commandPaletteOpen: false,
  heroInputFrozen: false,
  setArea: (area: PageArea) => set({ currentArea: area }),
  openCommandPalette: () => set({ commandPaletteOpen: true, heroInputFrozen: true }),
  closeCommandPalette: () => set({ commandPaletteOpen: false, heroInputFrozen: false }),
  handleEvent: (_payload: { type: string; data: Record<string, unknown> }) => {
    // Page store doesn't handle domain events
  },
}));
