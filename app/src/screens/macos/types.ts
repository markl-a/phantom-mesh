// SPEC-41 §9.3 / §7.3 — shared types for the macOS screen surfaces.
// Canonical wire home is app/src/types/macos_screens.ts (not yet created and out of
// this session's scope); the contract is defined locally here so the S1 screen stays
// self-contained. Consolidate into the canonical file once the backend wire lands.

export interface ScreenProps {
  screenId: string;
  onClose?: () => void;
  onResize?: (w: number, h: number) => void;
  initialSize?: [number, number];
}

export type MenuBarActionType =
  | "show_popover" | "open_settings" | "open_coach_reader"
  | "restart_daemon" | "open_about" | "quit" | "open_external_url";

export interface MenuBarItemSpec {
  item_id: string;
  label_zh: string;
  label_en: string;
  icon: string | null;
  shortcut: string | null;
  action: { type: MenuBarActionType } & Record<string, unknown>;
  visibility: "always" | "only_if_coach_pending"
            | "only_if_daemon_stopped" | "only_if_daemon_running";
  enabled: "always" | "only_if_daemon_reachable" | "only_if_onboarded";
  group: "header" | "capture" | "status" | "settings" | "quit";
}

export interface MenuBarState {
  peerAliveCount: number;       // alive peers for header summary
  daemonRunning: boolean;
  todayEventCount: number;      // "Today: N events"
  coachPending: boolean;        // show "Coach review ready" row
  onboarded: boolean;
}

export interface MenuBarDropdownProps extends ScreenProps {
  state: MenuBarState;
  items?: MenuBarItemSpec[];                 // optional override; else built-in default list
  onItemInvoke?: (item: MenuBarItemSpec) => void;
}

export type SettingsTab = "general" | "cluster" | "providers" | "privacy";

export interface SettingsWindowProps extends ScreenProps {
  targetTab?: SettingsTab;                    // initial tab (defaults to "general")
  onTabChange?: (tab: SettingsTab) => void;
  onOpenAddPeer?: () => void;                 // Cluster tab → S12 mesh_peer_add_wizard
  onOpenCoachReviews?: () => void;            // Privacy tab → S6 coach_review_list
  onRestartDaemon?: () => void;
  onOpenDaemonLog?: () => void;
  onWipeAllData?: () => void;                 // Privacy tab → destructive confirm (caller owns)
}

export type ClusterChoice = "join_existing" | "create_new" | "single_machine";

export interface OnboardingWizardProps extends ScreenProps {
  initialStep?: 1 | 2 | 3 | 4;
  onComplete?: (summary: { cluster: ClusterChoice; providerConfigured: boolean }) => void;
  onAddProvider?: () => void;                 // step 3 → provider setup (caller / S4 providers)
  onUseDemoRelay?: () => void;                // step 3 → 30s demo-relay shortcut
  onSendFirstMessage?: (text: string) => void; // step 4 → first chat send
}
