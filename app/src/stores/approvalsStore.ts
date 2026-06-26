// apex-④ · Phone approvals store.
//
// Mirrors `dispatchStore.ts`: a tiny Zustand store holding the set of
// pending high-risk approvals the governed-run loop is waiting on. The
// phone (this app) shows each card and the operator taps Approve / Deny /
// Stop; that decision is POSTed to the backend's /rpc/inbox (topic =
// approval_id) where the governor's escalator picks it up — see
// core/src/governed_run/escalation.rs::correlated().
//
// The pending list itself comes from the backend's /rpc/approvals/list
// ({ pending: ApprovalCard[] }); `<MobileApprovals />` polls it every 5s
// and calls setItems(). We key by approval_id (object map, not Map) so a
// zustand shallow-equal selector works for the "list of ids" case, exactly
// like dispatchStore's `byId`.

import { create } from 'zustand';

/** One pending high-risk action awaiting a phone decision. Shape matches the
 *  backend's /rpc/approvals/list `pending[]` entries. */
export interface ApprovalCard {
  approval_id: string;
  task_id: string;
  tool: string;
  /** Risk level string, e.g. "execute_high" / "high" / "medium". */
  risk: string;
  reason: string;
  /** Unix-millis the action was raised; used to render an age. */
  created_ms: number;
}

export interface ApprovalsStoreState {
  /** Object map keyed by approval_id (object, not Map, so shallow selectors
   *  work — same convention as dispatchStore.byId). */
  items: Record<string, ApprovalCard>;
  /** Replace the whole pending set from a fresh /rpc/approvals/list poll. */
  setItems: (cards: ApprovalCard[]) => void;
  /** Optimistically drop one card after a decision is accepted by the backend. */
  removeItem: (approval_id: string) => void;
}

export const useApprovalsStore = create<ApprovalsStoreState>()((set) => ({
  items: {},

  setItems: (cards) =>
    set(() => {
      const next: Record<string, ApprovalCard> = {};
      for (const c of cards) {
        if (c && typeof c.approval_id === 'string' && c.approval_id) {
          next[c.approval_id] = c;
        }
      }
      return { items: next };
    }),

  removeItem: (approval_id) =>
    set((s) => {
      if (!(approval_id in s.items)) return s;
      const next = { ...s.items };
      delete next[approval_id];
      return { items: next };
    }),
}));
