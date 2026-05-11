import { create } from 'zustand';

export interface ActiveTool {
  id: string;  // tool + timestamp for uniqueness
  tool: string;
  args_preview: string;
  status: 'running' | 'done';
  output_preview?: string;
  startedAt: number;
}

interface StreamingStore {
  // State
  isStreaming: boolean;
  tokens: string;           // accumulated tokens for current response
  activeTools: ActiveTool[];
  costUsd: number | null;
  elapsedSecs: number | null;
  agentName: string | null;

  // Actions
  startStream: (agentName: string) => void;
  appendToken: (content: string) => void;
  endStream: (costUsd?: number, elapsedSecs?: number) => void;
  toolStart: (tool: string, args_preview: string) => void;
  toolDone: (tool: string, output_preview: string) => void;
  reset: () => void;
}

export const useStreamingStore = create<StreamingStore>()((set) => ({
  isStreaming: false,
  tokens: '',
  activeTools: [],
  costUsd: null,
  elapsedSecs: null,
  agentName: null,

  startStream: (agentName) => set({
    isStreaming: true,
    tokens: '',
    activeTools: [],
    costUsd: null,
    elapsedSecs: null,
    agentName,
  }),

  appendToken: (content) => set((s) => ({
    tokens: s.tokens + content,
  })),

  endStream: (costUsd, elapsedSecs) => set({
    isStreaming: false,
    costUsd: costUsd ?? null,
    elapsedSecs: elapsedSecs ?? null,
    activeTools: [],
  }),

  toolStart: (tool, args_preview) => set((s) => ({
    activeTools: [...s.activeTools, {
      id: `${tool}-${Date.now()}`,
      tool,
      args_preview,
      status: 'running',
      startedAt: Date.now(),
    }],
  })),

  toolDone: (tool, output_preview) => set((s) => ({
    activeTools: s.activeTools.map((t) =>
      t.tool === tool && t.status === 'running'
        ? { ...t, status: 'done' as const, output_preview }
        : t
    ),
  })),

  reset: () => set({
    isStreaming: false,
    tokens: '',
    activeTools: [],
    costUsd: null,
    elapsedSecs: null,
    agentName: null,
  }),
}));
