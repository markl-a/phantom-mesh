import { create } from "zustand";
import { persist } from "zustand/middleware";

interface ClusterModeState {
  enabled: boolean;
  coordinatorUrl: string; // e.g. http://192.0.2.10:7878
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
      // Defaults are intentionally empty: the user picks a mode in
      // MobileFirstLaunch, then MobileJoinCluster (or MobileDemoMode)
      // writes real values here. Pre-seeded values caused PII-class
      // leaks (operator's personal Tailnet hostname + secret shipped
      // in every IPA) — see spec/2026-05-23-mobile-redesign-v2.md §10.
      enabled: false,
      coordinatorUrl: "",
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
