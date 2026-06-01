import React from "react";
import ReactDOM from "react-dom/client";
import { BrowserRouter } from "react-router-dom";
import App from "./App";
import "./index.css";

// SPA entry point. Mounts inside the `/app` base so deep links like
// `/app/cluster` route correctly without server-side rewrites — the
// worker (phantommesh-io/src/index.ts) serves the same `index.html`
// for every `/app/*` URL and React Router handles the rest.
ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <BrowserRouter basename="/app">
      <App />
    </BrowserRouter>
  </React.StrictMode>,
);
