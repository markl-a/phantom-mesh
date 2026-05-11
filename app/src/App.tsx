import { useState, useEffect } from "react";
import { Routes, Route, NavLink, Navigate, useLocation } from "react-router-dom";
import {
  MessageSquare, LayoutDashboard, Target,
  Globe, FileText, Settings, Menu, X, LogOut,
} from "lucide-react";
import OnboardingQuickStart, { clearSession } from "./components/onboarding/OnboardingQuickStart";
import StartupCheck from "./components/StartupCheck";
import { ONBOARDED_KEY } from "./components/onboarding/types";

import Conversation from "./pages/Conversation";
import Dashboard from "./pages/Dashboard";
import Goals from "./pages/Goals";
import Browser from "./pages/Browser";
import PageViewer from "./pages/PageViewer";
import SettingsPage from "./pages/Settings";
import Terminal from "./pages/Terminal";

import { useIsMobile } from "./hooks/useIsMobile";
import MobileShell from "./components/mobile/MobileShell";
import MobileConversation from "./components/mobile/MobileConversation";
import MobileDashboard from "./components/mobile/MobileDashboard";
import MobileSettings from "./components/mobile/MobileSettings";
import MobileOnboardingV2 from "./components/mobile/MobileOnboardingV2";

const PRIMARY_NAV = [
  { path: "/",          icon: MessageSquare,   label: "對話" },
  { path: "/dashboard", icon: LayoutDashboard, label: "儀表板" },
  { path: "/settings",  icon: Settings,        label: "設定" },
];

const LABS_NAV = [
  { path: "/goals",     icon: Target,          label: "目標" },
  { path: "/browser",   icon: Globe,           label: "瀏覽器" },
  { path: "/pages",     icon: FileText,        label: "頁面" },
];

export default function App() {
  const [onboarded, setOnboarded] = useState(() =>
    localStorage.getItem(ONBOARDED_KEY) === "true"
  );
  const [selfCheckPassed, setSelfCheckPassed] = useState(false);
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const location = useLocation();
  const isMobile = useIsMobile();

  const [userInfo, setUserInfo] = useState<{ display_name: string; email: string; avatar_url?: string | null } | null>(null);
  useEffect(() => {
    try {
      const raw = localStorage.getItem("phantom_mesh_identity");
      if (raw) setUserInfo(JSON.parse(raw));
    } catch { /* ignore */ }
  }, [onboarded]);

  const handleLogout = () => {
    clearSession();
    setOnboarded(false);
    setSelfCheckPassed(false);
    setUserInfo(null);
  };

  // Mobile (iOS/Android): skip the desktop onboarding (hardware scan +
  // provider picker) and StartupCheck — neither applies to a thin client.
  // Configuration on mobile happens inside Settings → 從 Mac 匯入設定 (token
  // import) or Settings → Cluster 派送. Land directly on MobileShell.
  if (!isMobile && !onboarded) {
    return <OnboardingQuickStart onComplete={() => setOnboarded(true)} />;
  }

  if (!isMobile && !selfCheckPassed) {
    return (
      <StartupCheck
        onPass={() => setSelfCheckPassed(true)}
        onResetOnboarding={() => {
          setOnboarded(false);
          setSelfCheckPassed(false);
        }}
      />
    );
  }

  // ─── /term route — full-viewport terminal shell, no chrome (v1.5 §1) ─
  // Same component on every platform (web, Tauri desktop, iOS, Android).
  // Lives ABOVE the mobile/desktop branching so neither MobileShell's
  // tabs nor the desktop sidebar wrap it.
  if (location.pathname === "/term") {
    return (
      <Routes>
        <Route path="/term" element={<Terminal />} />
      </Routes>
    );
  }

  // ─── Mobile branch: minimal 3-tab shell ──────────────────────────────────
  if (isMobile) {
    // First-launch onboarding gate. MobileOnboardingV2 checks AuthState
    // on mount: if a valid broker_token is already saved, it immediately
    // calls onReady() and we drop straight into chat. Otherwise the user
    // sees a single big "登入 phantommesh.io" button that drives the full
    // OAuth → vault sync chain before letting them into the app.
    const ONBOARDED_LOCAL = "phantom_mesh_v2_onboarded";
    const onboardedV2 = localStorage.getItem(ONBOARDED_LOCAL) === "true";
    if (!onboardedV2) {
      return (
        <MobileOnboardingV2
          onReady={() => {
            try { localStorage.setItem(ONBOARDED_LOCAL, "true"); } catch (_e) {/* ignore */}
            // Force re-render of App.
            window.location.reload();
          }}
        />
      );
    }

    return (
      <Routes>
        <Route element={<MobileShell onLogout={userInfo ? handleLogout : undefined} />}>
          <Route path="/"           element={<MobileConversation />} />
          <Route path="/dashboard"  element={<MobileDashboard />} />
          <Route path="/settings"   element={<MobileSettings />} />
          <Route path="/settings/*" element={<MobileSettings />} />
          {/* Anything else → home */}
          <Route path="*" element={<Navigate to="/" replace />} />
        </Route>
      </Routes>
    );
  }

  const renderNavLink = (item: typeof PRIMARY_NAV[0]) => (
    <NavLink
      key={item.path}
      to={item.path}
      end={item.path === "/"}
      className={({ isActive }) =>
        `flex items-center gap-2 px-3 py-2 rounded text-sm transition-colors ${
          isActive
            ? "bg-phantom-primary/15 text-phantom-primary"
            : "text-phantom-text hover:bg-phantom-card"
        }`
      }
    >
      <item.icon size={16} />
      {item.label}
    </NavLink>
  );

  // Close sidebar on navigation (mobile)
  const handleNavClick = () => setSidebarOpen(false);

  return (
    <div className="flex h-screen bg-phantom-bg">
      {/* Mobile header bar */}
      <div className="md:hidden fixed top-0 left-0 right-0 z-50 bg-phantom-bg border-b border-phantom-border px-3 py-2 flex items-center gap-2">
        <button onClick={() => setSidebarOpen(!sidebarOpen)} className="text-phantom-text p-1">
          {sidebarOpen ? <X size={20} /> : <Menu size={20} />}
        </button>
        <h1 className="text-sm font-bold text-phantom-primary">Phantom Mesh</h1>
      </div>

      {/* Sidebar overlay on mobile */}
      {sidebarOpen && (
        <div className="md:hidden fixed inset-0 z-40 bg-black/50" onClick={() => setSidebarOpen(false)} />
      )}

      <aside className={`
        w-48 flex-shrink-0 border-r border-phantom-border flex flex-col bg-phantom-bg
        fixed md:relative inset-y-0 left-0 z-40
        transform transition-transform duration-200
        ${sidebarOpen ? 'translate-x-0' : '-translate-x-full md:translate-x-0'}
        md:transform-none
        pt-12 md:pt-0
      `}>
        <div className="px-3 py-3 hidden md:block">
          <h1 className="text-base font-bold text-phantom-primary">Phantom Mesh</h1>
        </div>
        <nav className="px-2 flex-1 overflow-y-auto space-y-1" onClick={handleNavClick}>
          {PRIMARY_NAV.map(renderNavLink)}

          <div className="pt-4 pb-2 px-3">
            <p className="text-[10px] font-semibold uppercase tracking-wider text-phantom-muted">
              Labs
            </p>
          </div>
          {LABS_NAV.map(renderNavLink)}
        </nav>

        {/* User profile + logout */}
        <div className="px-2 py-3 border-t border-phantom-border">
          {userInfo ? (
            <div className="flex items-center gap-2 px-2">
              {userInfo.avatar_url ? (
                <img src={userInfo.avatar_url} alt="" className="w-6 h-6 rounded-full flex-shrink-0" />
              ) : (
                <div className="w-6 h-6 rounded-full bg-phantom-primary/20 flex items-center justify-center text-xs text-phantom-primary flex-shrink-0">
                  {userInfo.display_name?.[0]?.toUpperCase() || '?'}
                </div>
              )}
              <div className="flex-1 min-w-0">
                <p className="text-xs text-phantom-text truncate">{userInfo.display_name}</p>
                <p className="text-[10px] text-phantom-muted truncate">{userInfo.email}</p>
              </div>
              <button
                onClick={handleLogout}
                title="登出"
                className="text-phantom-muted hover:text-phantom-danger p-1 flex-shrink-0 transition"
              >
                <LogOut size={14} />
              </button>
            </div>
          ) : (
            <button
              onClick={handleLogout}
              className="flex items-center gap-2 px-2 py-1 text-xs text-phantom-muted hover:text-phantom-text transition w-full"
            >
              <LogOut size={14} />
              登出 / 重新設定
            </button>
          )}
        </div>
      </aside>

      <main className="flex-1 overflow-y-auto p-4 md:p-6 pt-14 md:pt-6">
        <Routes>
          <Route path="/"           element={<Conversation />} />
          <Route path="/dashboard"  element={<Dashboard />} />
          <Route path="/goals"      element={<Goals />} />
          <Route path="/browser"    element={<Browser />} />
          <Route path="/pages"      element={<PageViewer />} />
          <Route path="/settings/*" element={<SettingsPage />} />
          {/* Legacy redirects */}
          <Route path="/chat"       element={<Navigate to="/" replace />} />
          <Route path="/cluster"    element={<Navigate to="/settings/agents" replace />} />
          <Route path="/agents"     element={<Navigate to="/settings/agents" replace />} />
          <Route path="/tasks"      element={<Navigate to="/dashboard" replace />} />
          <Route path="/economy"    element={<Navigate to="/dashboard" replace />} />
          <Route path="/providers"  element={<Navigate to="/settings/providers" replace />} />
          <Route path="/channels"   element={<Navigate to="/settings/channels" replace />} />
          <Route path="/tools"      element={<Navigate to="/settings/tools" replace />} />
          <Route path="/memory"     element={<Navigate to="/settings/memory" replace />} />
          <Route path="/network"    element={<Navigate to="/settings/agents" replace />} />
          <Route path="/security"   element={<Navigate to="/settings/security" replace />} />
          <Route path="/evolution"  element={<Navigate to="/settings/evolution" replace />} />
          <Route path="/logs"       element={<Navigate to="/settings/logs" replace />} />
          <Route path="/hands"      element={<Navigate to="/settings/hands" replace />} />
        </Routes>
      </main>
    </div>
  );
}
