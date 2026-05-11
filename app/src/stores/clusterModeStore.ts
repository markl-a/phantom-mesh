import { create } from "zustand";
import { persist } from "zustand/middleware";

interface ClusterModeState {
  enabled: boolean;
  coordinatorUrl: string; // e.g. http://100.87.93.58:7878
  clusterSecret: string;  // shared HMAC key
  setEnabled: (v: boolean) => void;
  setCoordinatorUrl: (v: string) => void;
  setClusterSecret: (v: string) => void;
  isConfigured: () => boolean;
}

/**
 * Cluster mode: when ON, chat dispatches via the coordinator's
 * /rpc/task/assign instead of running locally. The coordinator routes
 * the task to the best-available worker (could be this node, or any peer).
 *
 * Requires:
 *  - coordinatorUrl: where to POST /rpc/task/assign
 *  - clusterSecret: shared HMAC-SHA256 key for X-Cluster-Auth header
 */
export const useClusterModeStore = create<ClusterModeState>()(
  persist(
    (set, get) => ({
      enabled: false,
      coordinatorUrl: "http://100.87.93.58:7878", // Mac Tailscale default
      clusterSecret: "",
      setEnabled: (v) => set({ enabled: v }),
      setCoordinatorUrl: (v) => set({ coordinatorUrl: v.trim().replace(/\/$/, "") }),
      setClusterSecret: (v) => set({ clusterSecret: v.trim() }),
      isConfigured: () => {
        const s = get();
        return s.clusterSecret.length > 0 && s.coordinatorUrl.length > 0;
      },
    }),
    { name: "phantom-mesh-cluster-mode" }
  )
);
