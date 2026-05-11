// Mobile thin-shell mode.
//
// On Android/iOS Tauri builds we don't render the full React desktop UI.
// Instead we redirect the WebView to a remote `phantom serve` instance so
// mobile devices share the exact same web frontend (`core/web/`) that
// browsers see. The host is configurable via localStorage (PHANTOM_HOST,
// PHANTOM_PORT, PHANTOM_SCHEME) — defaults to http://localhost:7878/.
//
// Reversible: deleting this file + the call in main.tsx restores the
// original behaviour. Desktop builds are completely untouched.

const HOST_KEY = "PHANTOM_HOST";
const PORT_KEY = "PHANTOM_PORT";
const SCHEME_KEY = "PHANTOM_SCHEME";
const DEFAULT_HOST = "localhost";
const DEFAULT_PORT = "7878";
const DEFAULT_SCHEME = "http";

/** Best-effort detection of a Tauri mobile WebView. */
function isMobileTauri(): boolean {
  // Tauri exposes the special `tauri:` / `tauri-localhost:` protocol AND
  // injects `window.__TAURI_INTERNALS__` (v2) or `window.__TAURI__` (v1/v2).
  const w = window as any;
  const hasTauri = !!(w.__TAURI_INTERNALS__ || w.__TAURI__);
  if (!hasTauri) return false;

  // UA sniff is good enough — Tauri mobile uses the system WebView so the
  // UA contains "Android" or iOS markers ("iPhone"/"iPad").
  const ua = navigator.userAgent || "";
  return /Android|iPhone|iPad|iPod/i.test(ua);
}

/**
 * Build the connect-settings UI directly through the DOM instead of via
 * `document.write` + inline `<script>` + `onclick=`. Tauri's default CSP
 * blocks inline-script and inline-event-handler execution, so the previous
 * approach left the Connect button as dead pixels (root cause: CSP rule
 * "Executing inline event handler violates script-src 'self' 'sha256-...'").
 *
 * The handler now lives in this compiled module's own scope (whose hash
 * IS allowlisted), so addEventListener wires up cleanly.
 */
function showSettingsForm(currentHost: string, currentPort: string): void {
  // Inject one-time stylesheet (no inline style attribute on elements).
  if (!document.getElementById("phantom-thinshell-style")) {
    const style = document.createElement("style");
    style.id = "phantom-thinshell-style";
    style.textContent = `
      body{font-family:-apple-system,system-ui,sans-serif;background:#0b0d12;color:#e6e6e6;
           margin:0;padding:24px;display:flex;flex-direction:column;gap:16px;min-height:100vh}
      .pts h1{font-size:20px;margin:0 0 8px}
      .pts label{display:block;font-size:13px;color:#9aa0a6;margin-bottom:4px}
      .pts input{width:100%;padding:12px;font-size:16px;background:#1a1d24;color:#fff;
                 border:1px solid #2a2f3a;border-radius:8px;box-sizing:border-box}
      .pts button{padding:14px;font-size:16px;background:#3b82f6;color:#fff;border:0;
                  border-radius:8px;font-weight:600;cursor:pointer;width:100%}
      .pts button:active{background:#2563eb}
      .pts small{color:#6b7280;font-size:12px;line-height:1.5}
      .pts-status{font-size:12px;color:#fbbf24;min-height:18px;margin-top:4px}
      .pts-foot{margin-top:24px}
    `;
    document.head.appendChild(style);
  }

  // Tear down any prior root render — keep the body but replace its
  // children with our settings UI. We don't use document.open() / write()
  // because Tauri's CSP nonces are reset by full-document replacement.
  document.body.innerHTML = "";
  document.title = "Phantom Mesh — connect";

  const root = document.createElement("div");
  root.className = "pts";

  const h1 = document.createElement("h1");
  h1.textContent = "Connect to phantom serve";
  root.appendChild(h1);

  const lead = document.createElement("small");
  lead.textContent =
    "Run `phantom serve` on your Mac/PC, then enter its Tailscale hostname or LAN IP below.";
  root.appendChild(lead);

  const hostWrap = document.createElement("div");
  const hostLabel = document.createElement("label");
  hostLabel.textContent = "Host";
  const hostInput = document.createElement("input");
  hostInput.id = "pts-host";
  hostInput.value = currentHost;
  hostInput.autocapitalize = "off";
  hostInput.autocomplete = "off";
  hostWrap.appendChild(hostLabel);
  hostWrap.appendChild(hostInput);
  root.appendChild(hostWrap);

  const portWrap = document.createElement("div");
  const portLabel = document.createElement("label");
  portLabel.textContent = "Port";
  const portInput = document.createElement("input");
  portInput.id = "pts-port";
  portInput.value = currentPort;
  portInput.inputMode = "numeric";
  portWrap.appendChild(portLabel);
  portWrap.appendChild(portInput);
  root.appendChild(portWrap);

  const btn = document.createElement("button");
  btn.id = "pts-connect";
  btn.type = "button";
  btn.textContent = "Connect";
  root.appendChild(btn);

  const status = document.createElement("div");
  status.className = "pts-status";
  root.appendChild(status);

  const foot = document.createElement("small");
  foot.className = "pts-foot";
  foot.textContent =
    "tip: button doesn't react? long-press the app icon → App info → Force stop → reopen.";
  root.appendChild(foot);

  document.body.appendChild(root);

  const onConnect = () => {
    const h = (hostInput.value || "").trim();
    const p = (portInput.value || "").trim() || DEFAULT_PORT;
    if (!h) {
      status.textContent = "host is required";
      hostInput.focus();
      return;
    }
    try {
      localStorage.setItem(HOST_KEY, h);
      localStorage.setItem(PORT_KEY, p);
    } catch (_) {
      // private mode / partitioned storage — non-fatal, just navigate
    }
    let scheme = DEFAULT_SCHEME;
    try {
      scheme = localStorage.getItem(SCHEME_KEY) || DEFAULT_SCHEME;
    } catch (_) {}
    const url = `${scheme}://${h}:${p}/`;
    status.textContent = `opening ${url} …`;
    try {
      window.location.href = url;
    } catch (e1) {
      try {
        window.location.replace(url);
      } catch (e2) {
        status.textContent = `navigate failed: ${(e2 as Error)?.message ?? e2}`;
      }
    }
  };

  btn.addEventListener("click", onConnect);
  // Allow Enter on either input to submit.
  for (const inp of [hostInput, portInput]) {
    inp.addEventListener("keydown", (e) => {
      if ((e as KeyboardEvent).key === "Enter") onConnect();
    });
  }
}

/**
 * If running inside a mobile Tauri shell, swap the document for a
 * redirect to the remote phantom serve and return true (caller should
 * skip React mount). Otherwise return false.
 */
export function maybeRedirectToRemoteFrontend(): boolean {
  if (!isMobileTauri()) return false;

  const host = localStorage.getItem(HOST_KEY) || "";
  const port = localStorage.getItem(PORT_KEY) || DEFAULT_PORT;
  const scheme = localStorage.getItem(SCHEME_KEY) || DEFAULT_SCHEME;

  // First launch: no host saved → show settings form via DOM API
  // (NOT document.write — that path runs without our compiled-script
  // CSP nonce so inline event handlers get blocked silently).
  if (!host) {
    showSettingsForm(DEFAULT_HOST, port);
    return true;
  }

  const url = `${scheme}://${host}:${port}/`;
  // Use replace so the redirect doesn't pollute history.
  window.location.replace(url);
  return true;
}
