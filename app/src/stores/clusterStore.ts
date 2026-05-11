import { create } from 'zustand';

export interface ClusterNode {
  nodeId: string;
  name: string;
  status: 'online' | 'offline' | 'suspected';
  role: 'coordinator' | 'worker';
  capabilities: string[];
  cpuLoad: number;
  memoryPct: number;
  activeTasks: number;
  uptimeSecs: number;
}

interface ClusterState {
  nodes: ClusterNode[];
  coordinatorId: string | null;
  add: (node: ClusterNode) => void;
  update: (nodeId: string, partial: Partial<ClusterNode>) => void;
  remove: (nodeId: string) => void;
  set: (nodes: ClusterNode[]) => void;
  setCoordinator: (nodeId: string) => void;
  handleEvent: (payload: { type: string; data: Record<string, unknown> }) => void;
}

export const useClusterStore = create<ClusterState>()((set) => ({
  nodes: [],
  coordinatorId: null,
  add: (node: ClusterNode) => set((s: ClusterState) => ({ nodes: [...s.nodes, node] })),
  update: (nodeId: string, partial: Partial<ClusterNode>) =>
    set((s: ClusterState) => ({
      nodes: s.nodes.map((n: ClusterNode) => (n.nodeId === nodeId ? { ...n, ...partial } : n)),
    })),
  remove: (nodeId: string) => set((s: ClusterState) => ({ nodes: s.nodes.filter((n: ClusterNode) => n.nodeId !== nodeId) })),
  set: (nodes: ClusterNode[]) => set({ nodes }),
  setCoordinator: (nodeId: string) => set({ coordinatorId: nodeId }),
  handleEvent: (payload: { type: string; data: Record<string, unknown> }) => {
    const { type, data } = payload;
    const nodeId = data.node_id as string;
    const name = data.name as string;
    if (type === 'NodeOnline') {
      set((s: ClusterState) => ({
        nodes: s.nodes.some((n: ClusterNode) => n.nodeId === nodeId)
          ? s.nodes.map((n: ClusterNode) => (n.nodeId === nodeId ? { ...n, status: 'online' as const } : n))
          : [...s.nodes, { nodeId, name, status: 'online' as const, role: 'worker' as const, capabilities: [], cpuLoad: 0, memoryPct: 0, activeTasks: 0, uptimeSecs: 0 }],
      }));
    } else if (type === 'NodeOffline') {
      set((s: ClusterState) => ({
        nodes: s.nodes.map((n: ClusterNode) => (n.nodeId === nodeId ? { ...n, status: 'offline' as const } : n)),
      }));
    } else if (type === 'ElectionComplete') {
      set({ coordinatorId: data.coordinator_id as string });
    }
  },
}));
