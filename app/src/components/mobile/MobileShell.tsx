import { Outlet, NavLink, useNavigate, useLocation } from "react-router-dom";
import { MessageSquare, Network, Settings as SettingsIcon, LogOut } from "lucide-react";
import { useEffect, useState } from "react";

const TABS = [
  { path: "/",          icon: MessageSquare, label: "對話" },
  { path: "/dashboard", icon: Network,       label: "節點" },
  { path: "/settings",  icon: SettingsIcon,  label: "設定" },
];

export default function MobileShell({ onLogout }: { onLogout?: () => void }) {
  const location = useLocation();
  const navigate = useNavigate();
  const [showHeader, setShowHeader] = useState(true);

  // Header text by route
  const titleByPath: Record<string, string> = {
    "/":          "對話",
    "/dashboard": "節點",
    "/settings":  "設定",
  };
  const title = titleByPath[location.pathname] ?? "Phantom Mesh";

  // Auto-redirect Labs routes to home on mobile
  useEffect(() => {
    if (["/goals", "/browser", "/pages"].includes(location.pathname)) {
      navigate("/", { replace: true });
    }
  }, [location.pathname, navigate]);

  // Hide header on conversation route to maximize chat area
  useEffect(() => {
    setShowHeader(location.pathname !== "/");
  }, [location.pathname]);

  return (
    <div className="flex flex-col h-[100dvh] bg-phantom-bg">
      {/* Top header (hidden on chat for max space) */}
      {showHeader && (
        <header className="flex items-center justify-between px-4 py-3 border-b border-phantom-border flex-shrink-0">
          <h1 className="text-base font-semibold text-phantom-text">{title}</h1>
          {onLogout && (
            <button
              onClick={onLogout}
              className="text-phantom-muted hover:text-phantom-danger p-2 -m-2"
              aria-label="登出"
            >
              <LogOut size={18} />
            </button>
          )}
        </header>
      )}

      {/* Scrollable main */}
      <main className="flex-1 min-h-0 overflow-hidden">
        <Outlet />
      </main>

      {/* Bottom tab nav with safe area */}
      <nav
        className="flex-shrink-0 border-t border-phantom-border bg-phantom-bg flex"
        style={{ paddingBottom: "env(safe-area-inset-bottom)" }}
      >
        {TABS.map((tab) => (
          <NavLink
            key={tab.path}
            to={tab.path}
            end={tab.path === "/"}
            className={({ isActive }) =>
              `flex-1 flex flex-col items-center justify-center gap-1 py-2.5 transition-colors ${
                isActive
                  ? "text-phantom-primary"
                  : "text-phantom-muted hover:text-phantom-text"
              }`
            }
          >
            <tab.icon size={22} />
            <span className="text-[11px] font-medium">{tab.label}</span>
          </NavLink>
        ))}
      </nav>
    </div>
  );
}
