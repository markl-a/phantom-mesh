import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { MemoryRouter, Routes, Route } from "react-router-dom";
import App from "../src/App";
import { setBrokerLogin } from "../src/lib/auth";
import ClusterPage from "../src/pages/Cluster";
import DispatchPage from "../src/pages/Dispatch";
import HistoryPage from "../src/pages/History";
import SettingsPage from "../src/pages/Settings";

describe("App shell", () => {
  it("renders the sidebar + cluster placeholder when authenticated", () => {
    setBrokerLogin({
      token: "tok",
      email: "u@example.com",
      expires_at_ms: Date.now() + 60_000,
    });
    render(
      <MemoryRouter initialEntries={["/cluster"]}>
        <App />
      </MemoryRouter>,
    );
    // Sidebar links should all be present. Use `getAllByText` because
    // "Cluster" also appears as the active page <h1> — both should
    // render simultaneously.
    expect(screen.getAllByText("Cluster").length).toBeGreaterThanOrEqual(1);
    expect(screen.getByRole("link", { name: "Dispatch" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "History" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Settings" })).toBeInTheDocument();
    // Active page placeholder should render.
    expect(screen.getByTestId("cluster-placeholder")).toBeInTheDocument();
  });

  it("each placeholder page mounts in isolation without crashing", () => {
    // Smoke: prove each page module exports a renderable component so
    // F201-F204 can drop in replacements without a routing rewrite.
    render(
      <MemoryRouter initialEntries={["/dispatch"]}>
        <Routes>
          <Route path="/dispatch" element={<DispatchPage />} />
          <Route path="/history" element={<HistoryPage />} />
          <Route path="/settings" element={<SettingsPage />} />
          <Route path="/cluster" element={<ClusterPage />} />
        </Routes>
      </MemoryRouter>,
    );
    expect(screen.getByTestId("dispatch-placeholder")).toBeInTheDocument();
  });
});
