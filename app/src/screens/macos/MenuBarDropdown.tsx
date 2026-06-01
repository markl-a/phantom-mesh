// SPEC-41 §10.2 — S1 menu bar dropdown (the NSStatusItem hub).
//
// Central navigation surface: cluster-health header, capture shortcuts (S2 habit /
// S3 focus), today's event count + conditional coach-review row (S5), and the
// settings / restart / about / quit group. Self-contained: it never invokes a Tauri
// command (the menubar_* backend in §9.1 does not exist yet) — every action is raised
// to the caller via onItemInvoke callback props (same idiom as ChipPopover). Live
// peer health comes from the existing useClusterPeers hook. Wireframe: SPEC-41 §10.2.

import { useEffect } from "react";
import {
  BarChart3,
  Info,
  Lightbulb,
  PencilLine,
  Power,
  RotateCw,
  Settings,
  Timer,
} from "lucide-react";
import { useClusterPeers } from "../../hooks/useClusterPeers";
import type { MenuBarDropdownProps, MenuBarItemSpec } from "./types";

const DEFAULT_ITEMS: MenuBarItemSpec[] = [
  {
    item_id: "header_summary",
    label_zh: "Phantom Mesh",
    label_en: "Phantom Mesh",
    icon: null,
    shortcut: null,
    action: { type: "open_about" },
    visibility: "always",
    enabled: "always",
    group: "header",
  },
  {
    item_id: "quick_habit_log",
    label_zh: "📝 Quick habit log…",
    label_en: "Quick habit log…",
    icon: "pencil",
    shortcut: "⌘⇧H",
    action: { type: "show_popover", kind: "habit" },
    visibility: "always",
    enabled: "only_if_onboarded",
    group: "capture",
  },
  {
    item_id: "start_focus_session",
    label_zh: "⏱ Start focus session…",
    label_en: "Start focus session…",
    icon: "timer",
    shortcut: "⌘⇧F",
    action: { type: "show_popover", kind: "focus" },
    visibility: "always",
    enabled: "only_if_onboarded",
    group: "capture",
  },
  {
    item_id: "today_events",
    label_zh: "今日事件",
    label_en: "Today events",
    icon: "chart",
    shortcut: null,
    action: { type: "open_settings" },
    visibility: "always",
    enabled: "always",
    group: "status",
  },
  {
    item_id: "coach_review_ready",
    label_zh: "💡 教練回顧已就緒（點擊閱讀）",
    label_en: "Coach review ready",
    icon: "lightbulb",
    shortcut: null,
    action: { type: "open_coach_reader" },
    visibility: "only_if_coach_pending",
    enabled: "only_if_onboarded",
    group: "status",
  },
  {
    item_id: "open_settings",
    label_zh: "⚙ 開啟設定…",
    label_en: "Open settings…",
    icon: "settings",
    shortcut: null,
    action: { type: "open_settings" },
    visibility: "always",
    enabled: "always",
    group: "settings",
  },
  {
    item_id: "restart_daemon",
    label_zh: "↻ 重啟背景程式",
    label_en: "Restart daemon",
    icon: "restart",
    shortcut: null,
    action: { type: "restart_daemon" },
    visibility: "always",
    enabled: "always",
    group: "settings",
  },
  {
    item_id: "open_about",
    label_zh: "ℹ 關於 / 版本",
    label_en: "About / version",
    icon: "info",
    shortcut: null,
    action: { type: "open_about" },
    visibility: "always",
    enabled: "always",
    group: "settings",
  },
  {
    item_id: "quit",
    label_zh: "⏻ 結束 Phantom Mesh",
    label_en: "Quit Phantom Mesh",
    icon: "power",
    shortcut: null,
    action: { type: "quit" },
    visibility: "always",
    enabled: "always",
    group: "quit",
  },
];

const iconClass = "h-4 w-4 shrink-0 text-phantom-primary";

function ItemIcon({ icon }: { icon: string | null }) {
  switch (icon) {
    case "pencil":
      return <PencilLine className={iconClass} aria-hidden="true" />;
    case "timer":
      return <Timer className={iconClass} aria-hidden="true" />;
    case "chart":
      return <BarChart3 className={iconClass} aria-hidden="true" />;
    case "lightbulb":
      return <Lightbulb className={iconClass} aria-hidden="true" />;
    case "settings":
      return <Settings className={iconClass} aria-hidden="true" />;
    case "restart":
      return <RotateCw className={iconClass} aria-hidden="true" />;
    case "info":
      return <Info className={iconClass} aria-hidden="true" />;
    case "power":
      return <Power className={iconClass} aria-hidden="true" />;
    default:
      return <span className="h-4 w-4 shrink-0" aria-hidden="true" />;
  }
}

function isVisible(item: MenuBarItemSpec, state: MenuBarDropdownProps["state"]) {
  switch (item.visibility) {
    case "only_if_coach_pending":
      return state.coachPending;
    case "only_if_daemon_stopped":
      return !state.daemonRunning;
    case "only_if_daemon_running":
      return state.daemonRunning;
    default:
      return true;
  }
}

function isEnabled(item: MenuBarItemSpec, state: MenuBarDropdownProps["state"]) {
  switch (item.enabled) {
    case "only_if_daemon_reachable":
      return state.daemonRunning;
    case "only_if_onboarded":
      return state.onboarded;
    default:
      return true;
  }
}

function isInformational(item: MenuBarItemSpec) {
  return item.group === "header" || item.item_id === "today_events";
}

export default function MenuBarDropdown({
  state,
  items = DEFAULT_ITEMS,
  onItemInvoke,
  onClose,
}: MenuBarDropdownProps) {
  const { peers } = useClusterPeers();
  // Live peers from the hook are the source of truth when present; only
  // "Online" counts as alive ("Unknown" is not yet confirmed reachable).
  // When the hook has no peers yet, fall back to the caller-provided count.
  const peerAliveCount =
    peers.length > 0
      ? peers.filter((peer) => peer.status === "Online").length
      : state.peerAliveCount;
  const visibleItems = items.filter((item) => isVisible(item, state));

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose?.();
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [onClose]);

  const headerDotClass = state.daemonRunning
    ? peerAliveCount > 0
      ? "bg-phantom-success"
      : "bg-phantom-muted"
    : "bg-phantom-warning";

  const renderLabel = (item: MenuBarItemSpec) => {
    if (item.group === "header") {
      return `Phantom Mesh — ${
        state.daemonRunning ? `${peerAliveCount} peer alive` : "daemon stopped"
      }`;
    }
    if (item.item_id === "today_events") {
      return `📊 今日 ${state.todayEventCount} 筆事件`;
    }
    return item.label_zh;
  };

  return (
    <div
      data-testid="menu-bar-dropdown"
      role="menu"
      aria-label="Phantom Mesh"
      className="w-80 overflow-hidden rounded-lg border border-phantom-border bg-phantom-card text-phantom-text shadow-xl"
    >
      {visibleItems.map((item, index) => {
        const enabled = isEnabled(item, state);
        const informational = isInformational(item);
        const separated = index > 0 && visibleItems[index - 1]?.group !== item.group;
        const rowClass = [
          "flex w-full items-center gap-3 px-4 py-2.5 text-left text-sm",
          separated ? "border-t border-phantom-border" : "",
          informational ? "cursor-default" : "hover:bg-phantom-primary/10",
          !enabled && !informational ? "cursor-not-allowed opacity-50" : "",
        ].join(" ");

        const content = (
          <>
            {item.group === "header" ? (
              <span
                className={`h-2.5 w-2.5 shrink-0 rounded-full ${headerDotClass}`}
                aria-hidden="true"
              />
            ) : (
              <ItemIcon icon={item.icon} />
            )}

            <span className="min-w-0 flex-1 truncate">
              <span className="block truncate font-medium">{renderLabel(item)}</span>
              {item.item_id === "quit" && (
                <span className="block truncate text-xs text-phantom-muted">
                  不影響背景程式
                </span>
              )}
            </span>

            {item.shortcut && (
              <span className="shrink-0 text-xs text-phantom-muted">{item.shortcut}</span>
            )}
          </>
        );

        if (informational) {
          return (
            <div
              key={item.item_id}
              role="menuitem"
              aria-disabled="true"
              className={rowClass}
            >
              {content}
            </div>
          );
        }

        return (
          <button
            key={item.item_id}
            type="button"
            role="menuitem"
            disabled={!enabled}
            aria-disabled={!enabled}
            className={rowClass}
            onClick={() => {
              if (!enabled) return;
              onItemInvoke?.(item);
              onClose?.();
            }}
          >
            {content}
          </button>
        );
      })}
    </div>
  );
}
