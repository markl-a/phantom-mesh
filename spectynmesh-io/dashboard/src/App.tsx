import { NavLink, Route, Routes, Navigate } from "react-router-dom";
import { useEffect, useState } from "react";
import { hasBrokerToken } from "./lib/auth";
import ClusterPage from "./pages/Cluster";
import DispatchPage from "./pages/Dispatch";
import HistoryPage from "./pages/History";
import SettingsPage from "./pages/Settings";

// F200 shell: sidebar + main pane only. F201-F204 fill in the actual
// page contents. The auth gate redirects to /login?next=/app the
// moment we detect no broker_token in localStorage — the OAuth dance
// itself is owned by the existing worker at /auth/google/start which
// will set the token and bounce us back here.
function AuthGate({ children }: { children: React.ReactNode }) {
  const [authed, setAuthed] = useState<boolean | null>(null);

  useEffect(() => {
    // First-paint check. The actual session lives in a cookie set by
    // the broker; the broker_token in localStorage is the dashboard's
    // mirror that lets us short-circuit the round-trip to /api/me on
    // every navigation. F205 will add /api/me/cluster-peers/caps etc.
    // — all of those will 401 if the cookie is stale and the SPA will
    // re-trigger this gate from the api.ts fetch wrapper.
    setAuthed(hasBrokerToken());
  }, []);

  if (authed === null) {
    return (
      <div className="flex h-full items-center justify-center text-spectyn-muted">
        loading…
      </div>
    );
  }
  if (!authed) {
    // Hard-redirect (not <Navigate>) so the browser does a fresh GET
    // to the broker's server-rendered /login page (it's outside the
    // SPA's React Router scope).
    const next = encodeURIComponent(window.location.pathname);
    window.location.replace(`/login?next=${next}`);
    return null;
  }
  return <>{children}</>;
}

function Sidebar() {
  const item = (to: string, label: string) => (
    <NavLink
      to={to}
      end={to === "/"}
      className={({ isActive }) =>
        `block rounded-md px-3 py-2 text-sm transition-colors ${
          isActive
            ? "bg-spectyn-primary/20 text-spectyn-primary"
            : "text-spectyn-text hover:bg-spectyn-card"
        }`
      }
    >
      {label}
    </NavLink>
  );

  return (
    <aside className="flex w-56 flex-col gap-1 border-r border-spectyn-border bg-spectyn-card p-3">
      <div className="mb-4 px-3 py-2 text-xs uppercase tracking-wider text-spectyn-muted">
        spectyn mesh
      </div>
      {item("/cluster", "Cluster")}
      {item("/dispatch", "Dispatch")}
      {item("/history", "History")}
      {item("/settings", "Settings")}
    </aside>
  );
}

export default function App() {
  return (
    <AuthGate>
      <div className="flex h-screen w-full">
        <Sidebar />
        <main className="flex-1 overflow-auto p-6">
          <Routes>
            <Route path="/" element={<Navigate to="/cluster" replace />} />
            <Route path="/cluster" element={<ClusterPage />} />
            <Route path="/dispatch" element={<DispatchPage />} />
            <Route path="/history" element={<HistoryPage />} />
            <Route path="/settings" element={<SettingsPage />} />
            <Route
              path="*"
              element={
                <div className="text-spectyn-muted">page not found</div>
              }
            />
          </Routes>
        </main>
      </div>
    </AuthGate>
  );
}
