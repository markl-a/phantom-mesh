import { create } from 'zustand';
import { safeInvoke } from '../lib/tauri-compat';
import type { ProjectInfo } from '../lib/phase2-types';

// Re-export so existing consumers (e.g. ProjectSelector.tsx) keep working
// without changing their imports.
export type { ProjectInfo };

interface ProjectStore {
  currentProject: ProjectInfo | null;
  recentProjects: ProjectInfo[];
  isLoading: boolean;

  loadCurrentProject: () => Promise<void>;
  setProject: (path: string) => Promise<void>;
  loadRecentProjects: () => Promise<void>;
}

export const useProjectStore = create<ProjectStore>()((set) => ({
  currentProject: null,
  recentProjects: [],
  isLoading: false,

  loadCurrentProject: async () => {
    set({ isLoading: true });
    try {
      const info = await safeInvoke<ProjectInfo>('get_project_info');
      set({ currentProject: info ?? null, isLoading: false });
    } catch {
      set({ isLoading: false });
    }
  },

  setProject: async (path: string) => {
    set({ isLoading: true });
    try {
      const info = await safeInvoke<ProjectInfo>('set_project_cwd', { path });
      if (info) {
        set({ currentProject: info, isLoading: false });
        await safeInvoke('add_recent_project', { cwd: path });
      } else {
        set({ isLoading: false });
      }
    } catch {
      set({ isLoading: false });
    }
  },

  loadRecentProjects: async () => {
    try {
      const projects = await safeInvoke<ProjectInfo[]>('list_recent_projects') ?? [];
      set({ recentProjects: projects });
    } catch {
      // ignore
    }
  },
}));
