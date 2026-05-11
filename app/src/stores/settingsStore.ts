import { create } from 'zustand';

interface SettingsState {
  theme: 'dark' | 'light';
  inferenceMode: 'auto' | 'local' | 'cloud';
  notificationsEnabled: boolean;
  toggleTheme: () => void;
  setInferenceMode: (mode: 'auto' | 'local' | 'cloud') => void;
  setPartial: (partial: Partial<SettingsState>) => void;
  handleEvent: (payload: { type: string; data: Record<string, unknown> }) => void;
}

export const useSettingsStore = create<SettingsState>()((set) => ({
  theme: 'dark',
  inferenceMode: 'auto',
  notificationsEnabled: true,
  toggleTheme: () => set((s: SettingsState) => ({ theme: s.theme === 'dark' ? 'light' : 'dark' })),
  setInferenceMode: (mode: 'auto' | 'local' | 'cloud') => set({ inferenceMode: mode }),
  setPartial: (partial: Partial<SettingsState>) => set(partial),
  handleEvent: (payload: { type: string; data: Record<string, unknown> }) => {
    if (payload.type === 'SettingsChanged') {
      set(payload.data as Partial<SettingsState>);
    }
  },
}));
