import { Outlet, NavLink, useNavigate, useLocation } from "react-router-dom";
import ErrorBoundary from "../ErrorBoundary";
import { MessageSquare, Network, Settings as SettingsIcon, LogOut, Send, Clock, Mic, ClipboardList } from "lucide-react";
import { useEffect, useState } from "react";

// 3-tab bottom nav (merged "節點" + "集群" into one Mesh tab — 2026-05-23
// user direction). E002 added /dispatch + /history (F103 + F104) between
// cluster and settings: chat → cluster → dispatch → history → settings
// (operate → review → configure). Chat is landing.
const TABS = [
  { path: "/",         icon: MessageSquare, label: "對話" },
  { path: "/focus",    icon: Mic,           label: "專注" },
  { path: "/cluster",  icon: Network,       label: "集群" },
  { path: "/dispatch", icon: Send,          label: "派送" },
  { path: "/history",  icon: Clock,         label: "歷史" },
  { path: "/review",   icon: ClipboardList, label: "教練" },
  { path: "/settings", icon: SettingsIcon,  label: "設定" },
];

export default function MobileShell({ onLogout }: { onLogout?: () => void }) {
  const location = useLocation();
  const navigate = useNavigate();
  const [showHeader, setShowHeader] = useState(true);

  // Header text by route
  const titleByPath: Record<string, string> = {
    "/":         "對話",
    "/focus":    "專注",
    "/cluster":  "集群",
    "/dispatch": "派送",
    "/history":  "歷史",
    "/review":   "教練回顧",
    "/settings": "設定",
  };
  const title = titleByPath[location.pathname] ?? "Spectyn Mesh";

  // Auto-redirect retired routes to home on mobile (節點/集群 merged into /cluster)
  useEffect(() => {
    if (["/goals", "/browser", "/pages", "/dashboard"].includes(location.pathname)) {
      navigate("/cluster", { replace: true });
    }
  }, [location.pathname, navigate]);

  // Hide the tab header on routes that manage their own top chrome: chat (/)
  // is full-bleed for max area, and Focus (/focus) ships its own header
  // ("專注時段" + state). Stacking the tab header on Focus produced a visible
  // double header. The header-hidden branch routes the safe-area inset onto
  // <main> below, so these screens still clear the status bar.
  useEffect(() => {
    setShowHeader(location.pathname !== "/" && location.pathname !== "/focus");
  }, [location.pathname]);

  return (
    <div className="flex flex-col h-[100dvh] bg-spectyn-bg">
      {/* Top header (hidden on chat for max space). paddingTop carries the
          safe-area inset so the title clears the status bar / Dynamic Island
          now that the webview is edge-to-edge (viewport-fit=cover). */}
      {showHeader && (
        <header
          className="flex items-center justify-between px-4 py-3 border-b border-spectyn-border flex-shrink-0"
          style={{ paddingTop: "calc(env(safe-area-inset-top) + 0.75rem)" }}
        >
          <h1 className="text-base font-semibold text-spectyn-text">{title}</h1>
          {onLogout && (
            <button
              onClick={onLogout}
              className="text-spectyn-muted hover:text-spectyn-danger p-2 -m-2"
              aria-label="登出"
            >
              <LogOut size={18} />
            </button>
          )}
        </header>
      )}

      {/* Scrollable main. When the header is hidden (chat), the inset has to
          live here instead so chat content clears the status bar. */}
      <main
        className="flex-1 min-h-0 overflow-hidden"
        style={showHeader ? undefined : { paddingTop: "env(safe-area-inset-top)" }}
      >
        <ErrorBoundary resetKey={location.pathname}>
          <Outlet />
        </ErrorBoundary>
      </main>

      {/* Bottom tab nav with safe area */}
      <nav
        aria-label="主要導覽"
        className="flex-shrink-0 border-t border-spectyn-border bg-spectyn-bg flex"
        style={{ paddingBottom: "env(safe-area-inset-bottom)" }}
      >
        {TABS.map((tab) => (
          <NavLink
            key={tab.path}
            to={tab.path}
            end={tab.path === "/"}
            aria-label={tab.label}
            className={({ isActive }) =>
              `flex-1 flex flex-col items-center justify-center gap-1 py-2.5 transition-colors ${
                isActive
                  ? "text-spectyn-primary"
                  : "text-spectyn-muted hover:text-spectyn-text"
              }`
            }
          >
            <tab.icon size={22} aria-hidden="true" />
            <span className="text-[11px] font-medium">{tab.label}</span>
          </NavLink>
        ))}
      </nav>
    </div>
  );
}
