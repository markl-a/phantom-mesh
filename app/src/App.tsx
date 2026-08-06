import { useState, useEffect } from "react";
import { Routes, Route, NavLink, Navigate, useLocation, useNavigate } from "react-router-dom";
import { listen } from "@tauri-apps/api/event";
import {
  MessageSquare, LayoutDashboard, Target,
  Globe, FileText, Settings, Menu, X, LogOut, Mic, ListChecks, Utensils, History, CalendarDays, Search, HeartHandshake, ShieldCheck, BookOpen,
} from "lucide-react";
import OnboardingHello from "./pages/onboarding-hello";
import StartupCheck from "./components/StartupCheck";
import { ONBOARDED_KEY, clearSession } from "./components/onboarding/types";

import Conversation from "./pages/Conversation";
import Dashboard from "./pages/Dashboard";
import Goals from "./pages/Goals";
import Browser from "./pages/Browser";
import PageViewer from "./pages/PageViewer";
import SettingsPage from "./pages/Settings";
import Terminal from "./pages/Terminal";
import SkillBank from "./pages/skill-bank";
import FocusPage from "./components/focus/FocusPage";
import FocusStartSheet from "./screens/macos/FocusStartSheet";
import HabitPage from "./screens/macos/HabitPage";
import FoodCapturePanel from "./components/food/FoodCapturePanel";
import EventTimeline from "./screens/macos/EventTimeline";
import CoachReviewReader from "./screens/macos/CoachReviewReader";
import RecallSearch from "./screens/macos/RecallSearch";
import DailyReflection from "./screens/macos/DailyReflection";

import { useIsMobile } from "./hooks/useIsMobile";
import MobileShell from "./components/mobile/MobileShell";
import MobileConversation from "./components/mobile/MobileConversation";
import MobileDispatch from "./components/mobile/MobileDispatch";
import MobileHistory from "./components/mobile/MobileHistory";
import MobileSettings from "./components/mobile/MobileSettings";
import MobileOnboardingV2 from "./components/mobile/MobileOnboardingV2";
import MobileFirstLaunch from "./components/mobile/MobileFirstLaunch";
import MobileJoinCluster from "./components/mobile/MobileJoinCluster";
import MobileMesh from "./components/mobile/MobileMesh";
import DemoScreen from "./components/mobile/DemoScreen";
import AppTemplate from "./components/mobile/AppTemplate";
import MobileApprovals from "./components/mobile/MobileApprovals";

type FirstLaunchStage =
  | { kind: "pick" }
  | { kind: "join"; discovered?: { host: string; port: number; url: string } }
  | { kind: "broker" };

const PRIMARY_NAV = [
  { path: "/",          icon: MessageSquare,   label: "對話" },
  { path: "/dashboard", icon: LayoutDashboard, label: "儀表板" },
  { path: "/focus",     icon: Mic,             label: "專注" },
  { path: "/habit",     icon: ListChecks,      label: "習慣" },
  { path: "/food",      icon: Utensils,        label: "飲食" },
  { path: "/timeline",  icon: History,         label: "時間軸" },
  { path: "/review",    icon: CalendarDays,    label: "回顧" },
  { path: "/reflection", icon: HeartHandshake, label: "對齊反思" },
  { path: "/recall",    icon: Search,          label: "回想" },
  { path: "/approvals", icon: ShieldCheck,     label: "審核" },
  { path: "/skills",    icon: BookOpen,        label: "技能庫" },
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
      const raw = localStorage.getItem("spectyn_mesh_identity");
      if (raw) setUserInfo(JSON.parse(raw));
    } catch { /* ignore */ }
  }, [onboarded]);

  // SPEC-41 F2/F3 — the Rust global-shortcut handler (Cmd+Shift+H / Cmd+Shift+F)
  // emits these events; route them to the chip quick-log / focus-start screens.
  // In web/browser mode `listen` rejects (no Tauri IPC) → harmless no-op.
  const navigate = useNavigate();
  useEffect(() => {
    const subs = [
      listen("shortcut://chip", () => navigate("/habit")),
      listen("shortcut://focus", () => navigate("/focus/start")),
      listen("shortcut://review", () => navigate("/review")),
      // SPEC-41 S1 menu-bar "開啟設定…" item.
      listen("tray://settings", () => navigate("/settings")),
    ];
    return () => {
      subs.forEach((p) => p.then((un) => un()).catch(() => {}));
    };
  }, [navigate]);

  const handleLogout = () => {
    clearSession();
    setOnboarded(false);
    setSelfCheckPassed(false);
    setUserInfo(null);
  };

  // First launch (every platform): the GUI D1–D5 login-first onboarding
  // (OnboardingHello) drives the shared SPEC-28 FSM through the real per-edge
  // side-effects (broker OAuth login + ed25519 identity mint, detached
  // `spectyn serve` + mDNS advertise, provider detection + ranking). English
  // only; no demo/join/key-paste/skip. Same component on desktop AND mobile —
  // it is responsive (safe-area insets + haptics) — so neither short-circuits
  // past it. Peer-join + vault sync stay Stage 2 (handled later in Settings).
  if (!onboarded) {
    return <OnboardingHello onComplete={() => setOnboarded(true)} />;
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

  // ─── Mobile branch ────────────────────────────────────────────────────────
  // INTERVIEW DEMO: render the polished, self-contained AppTemplate on mobile.
  // It's a real product-grade shell (top bar + 4 iOS tabs) that bypasses the
  // (currently broken) legacy mobile UI while reusing the working networking
  // (clusterPost → native swift_cluster_fetch + HMAC).
  // TO RESTORE the bare DemoScreen: `return <DemoScreen />;`
  // TO RESTORE the legacy mobile UI: `return <MobileApp onLogout={userInfo ? handleLogout : undefined} />;`
  if (isMobile) {
    return <AppTemplate />;
    // return <DemoScreen />;
    // return <MobileApp onLogout={userInfo ? handleLogout : undefined} />;
  }

  const renderNavLink = (item: typeof PRIMARY_NAV[0]) => (
    <NavLink
      key={item.path}
      to={item.path}
      end={item.path === "/"}
      className={({ isActive }) =>
        `flex items-center gap-2 px-3 py-2 rounded text-sm transition-colors ${
          isActive
            ? "bg-spectyn-primary/15 text-spectyn-primary"
            : "text-spectyn-text hover:bg-spectyn-card"
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
    <div className="flex h-screen bg-spectyn-bg">
      {/* Mobile header bar */}
      <div className="md:hidden fixed top-0 left-0 right-0 z-50 bg-spectyn-bg border-b border-spectyn-border px-3 py-2 flex items-center gap-2">
        <button onClick={() => setSidebarOpen(!sidebarOpen)} className="text-spectyn-text p-1">
          {sidebarOpen ? <X size={20} /> : <Menu size={20} />}
        </button>
        <h1 className="text-sm font-bold text-spectyn-primary">Spectyn Mesh</h1>
      </div>

      {/* Sidebar overlay on mobile */}
      {sidebarOpen && (
        <div className="md:hidden fixed inset-0 z-40 bg-black/50" onClick={() => setSidebarOpen(false)} />
      )}

      <aside className={`
        w-48 flex-shrink-0 border-r border-spectyn-border flex flex-col bg-spectyn-bg
        fixed md:relative inset-y-0 left-0 z-40
        transform transition-transform duration-200
        ${sidebarOpen ? 'translate-x-0' : '-translate-x-full md:translate-x-0'}
        md:transform-none
        pt-12 md:pt-0
      `}>
        <div className="px-3 py-3 hidden md:block">
          <h1 className="text-base font-bold text-spectyn-primary">Spectyn Mesh</h1>
        </div>
        <nav className="px-2 flex-1 overflow-y-auto space-y-1" onClick={handleNavClick}>
          {PRIMARY_NAV.map(renderNavLink)}

          <div className="pt-4 pb-2 px-3">
            <p className="text-[10px] font-semibold uppercase tracking-wider text-spectyn-muted">
              Labs
            </p>
          </div>
          {LABS_NAV.map(renderNavLink)}
        </nav>

        {/* User profile + logout */}
        <div className="px-2 py-3 border-t border-spectyn-border">
          {userInfo ? (
            <div className="flex items-center gap-2 px-2">
              {userInfo.avatar_url ? (
                <img src={userInfo.avatar_url} alt="" className="w-6 h-6 rounded-full flex-shrink-0" />
              ) : (
                <div className="w-6 h-6 rounded-full bg-spectyn-primary/20 flex items-center justify-center text-xs text-spectyn-primary flex-shrink-0">
                  {userInfo.display_name?.[0]?.toUpperCase() || '?'}
                </div>
              )}
              <div className="flex-1 min-w-0">
                <p className="text-xs text-spectyn-text truncate">{userInfo.display_name}</p>
                <p className="text-[10px] text-spectyn-muted truncate">{userInfo.email}</p>
              </div>
              <button
                onClick={handleLogout}
                title="登出"
                className="text-spectyn-muted hover:text-spectyn-danger p-1 flex-shrink-0 transition"
              >
                <LogOut size={14} />
              </button>
            </div>
          ) : (
            <button
              onClick={handleLogout}
              className="flex items-center gap-2 px-2 py-1 text-xs text-spectyn-muted hover:text-spectyn-text transition w-full"
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
          <Route path="/focus"      element={<FocusPage />} />
          <Route path="/focus/start" element={<FocusStartRoute />} />
          <Route path="/habit"      element={<HabitPage />} />
          <Route path="/food"       element={<FoodCapturePanel />} />
          <Route path="/timeline"   element={<EventTimeline />} />
          <Route path="/review"     element={<CoachReviewReader />} />
          <Route path="/reflection" element={<DailyReflection />} />
          <Route path="/recall"     element={<RecallSearch />} />
          <Route path="/approvals"  element={<ApprovalsPage />} />
          <Route path="/skills"     element={<SkillBank />} />
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

// ───────────────────────────────────────────────────────────────────────────
// MobileApp — first-launch picker + the 3-tab shell. Kept in this file (vs
// its own component) because App.tsx already owns the cross-platform
// branching and there's no other consumer.
//
// Onboarding state machine (lives in localStorage so a refresh / cold-start
// resumes wherever the user left off):
//   key `spectyn_mesh_v2_onboarded` ∈ {undefined,"true"}
//   key `spectyn_mesh_v2_onboarded_mode` ∈ {"demo","join","broker"}
// ───────────────────────────────────────────────────────────────────────────
function MobileApp({ onLogout }: { onLogout?: () => void }) {
  const [onboardedV2, setOnboardedV2] = useState(
    () => localStorage.getItem("spectyn_mesh_v2_onboarded") === "true"
  );
  const [stage, setStage] = useState<FirstLaunchStage>({ kind: "pick" });

  const markDone = (mode: "demo" | "join" | "broker") => {
    try {
      localStorage.setItem("spectyn_mesh_v2_onboarded", "true");
      localStorage.setItem("spectyn_mesh_v2_onboarded_mode", mode);
    } catch (_e) { /* localStorage might be restricted */ }
    setOnboardedV2(true);
  };

  if (!onboardedV2) {
    if (stage.kind === "pick") {
      return (
        <MobileFirstLaunch
          onPickedDemo={() => {
            // TODO Phase 2: bundle Cerebras key + write to clusterModeStore as
            // a special "demo://cerebras" pseudo-coordinator (clusterDispatch
            // detects that scheme and goes direct). For Phase 1 we just mark
            // onboarded and drop into chat — user gets a clear empty-state
            // until they configure a real provider.
            markDone("demo");
          }}
          onPickedJoin={(discovered) => setStage({ kind: "join", discovered })}
          onPickedSignIn={() => setStage({ kind: "broker" })}
        />
      );
    }
    if (stage.kind === "join") {
      return (
        <MobileJoinCluster
          discovered={stage.discovered}
          onDone={() => markDone("join")}
          onCancel={() => setStage({ kind: "pick" })}
        />
      );
    }
    if (stage.kind === "broker") {
      return (
        <MobileOnboardingV2
          onReady={() => markDone("broker")}
        />
      );
    }
  }

  return (
    <Routes>
      <Route element={<MobileShell onLogout={onLogout} />}>
        <Route path="/"           element={<MobileConversation />} />
        <Route path="/focus"      element={<FocusPage />} />
        <Route path="/cluster"    element={<MobileMesh />} />
        <Route path="/dispatch"   element={<MobileDispatch />} />
        <Route path="/history"    element={<MobileHistory />} />
        <Route
          path="/review"
          element={
            // CoachReviewReader is a desktop component with no scroll container
            // of its own (desktop gets it from the outer <main>). MobileShell's
            // <main> is overflow-hidden, so wrap it here to scroll + clear the
            // bottom tab bar via the safe-area inset.
            <div
              className="h-full overflow-y-auto p-4"
              style={{ paddingBottom: "calc(env(safe-area-inset-bottom) + 1.5rem)" }}
            >
              <CoachReviewReader />
            </div>
          }
        />
        <Route path="/settings"   element={<MobileSettings />} />
        <Route path="/settings/*" element={<MobileSettings />} />
        <Route path="*" element={<Navigate to="/" replace />} />
      </Route>
    </Routes>
  );
}

// apex-④ · Desktop approvals page — a thin wrapper around the same
// <MobileApprovals /> component used on mobile. No props ⇒ it reads
// baseUrl/secret from useClusterModeStore (coordinatorUrl + clusterSecret),
// which is what the desktop cluster settings write. Constrained width so the
// cards don't stretch edge-to-edge on a wide desktop <main>.
function ApprovalsPage() {
  return (
    <div className="max-w-2xl mx-auto h-full">
      <MobileApprovals />
    </div>
  );
}

// SPEC-41 §10.4 focus start sheet — interim route surface. The native
// Cmd+Shift+F trigger (SPEC-40 menubar.rs) is deferred; until then the sheet
// is reachable at /focus/start, rendered centered on a dimmed backdrop.
function FocusStartRoute() {
  const navigate = useNavigate();
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4">
      <FocusStartSheet
        onStart={() => navigate("/focus")}
        onCancel={() => navigate("/focus")}
      />
    </div>
  );
}

// (Habit logging now lives in the /habit HabitPage, which embeds ChipPopover.)
