// Server-rendered HTML pages. No SPA, no JS framework — Hono ships
// with a tiny JSX-like helper but we use template literals for
// simplicity.

import type { Context } from "hono";
import type { Env } from "../types";
import { getCookie } from "hono/cookie";
import { authn } from "./api";
import { getUserById, getUserDevices, getUserSettings, ALLOWED_ENV_KEYS, getUserClusterPeers, getUserIdentities } from "../lib/db";
import { SESSION_COOKIE } from "./oauth";
import { csrfToken } from "../lib/oauth";

// Removed 2026-05-05: the API side (routes/api.ts) was already
// multi-tenant scoped by user_id from the JWT — every authenticated
// user can read/write their own vault. The UI gate that used to be
// here was a leftover from the v1 single-email release; removing it
// lets any logged-in user manage their own LLM keys + cluster peers
// instead of seeing a half-empty /account page.

const STYLE = `
  *{box-sizing:border-box}
  body{font-family:-apple-system,system-ui,Segoe UI,sans-serif;background:#0c0c10;color:#e8e2d4;
       margin:0;padding:0;min-height:100vh}
  .wrap{max-width:480px;margin:80px auto;padding:24px}
  .logo{font-size:48px;color:#d6b270;text-align:center;margin-bottom:12px}
  h1{font-size:24px;font-weight:500;margin:0 0 8px;text-align:center}
  .sub{color:#8a8578;text-align:center;font-size:14px;margin-bottom:32px}
  .card{background:#16161c;border:1px solid #2a2a35;border-radius:12px;padding:24px;
        display:flex;flex-direction:column;gap:12px}
  button,a.btn{display:flex;align-items:center;gap:12px;width:100%;padding:14px 16px;
        background:#1d1d24;color:#e8e2d4;border:1px solid #2a2a35;border-radius:8px;
        font-size:15px;cursor:pointer;text-decoration:none;justify-content:center;
        font-family:inherit}
  button:hover,a.btn:hover{border-color:#d6b270;background:#1d1d24}
  button.email{background:#d6b270;color:#0c0c10;border:0;font-weight:600;margin-top:8px}
  input{width:100%;padding:11px 12px;background:#0c0c10;color:#e8e2d4;border:1px solid #2a2a35;
        border-radius:6px;font-size:14px;font-family:inherit}
  input:focus{border-color:#d6b270;outline:none}
  details summary{cursor:pointer;color:#8a8578;font-size:13px;margin-top:8px;list-style:none;
                  text-align:center}
  details summary::after{content:" ▸";color:#4d4a42}
  details[open] summary::after{content:" ▾"}
  .pad{display:flex;flex-direction:column;gap:10px;margin-top:16px}
  .footer{text-align:center;color:#4d4a42;font-size:12px;margin-top:32px}
  .footer a{color:#6a6a78}
`;

/// Cloudflare Web Analytics beacon. Returns an empty string when the
/// token is unset (dev/staging) so we don't ship a broken script tag
/// pointing at a "" token. Defer-loaded so the page renders before
/// the beacon JS is evaluated. Privacy-friendly: no cookies, no
/// fingerprinting; just page-view + load-time aggregates that show
/// up on the Cloudflare dashboard.
function analyticsBeacon(env: Env): string {
  if (!env.CF_ANALYTICS_TOKEN) return "";
  // Token escaping: the value goes inside a JSON literal embedded
  // in HTML attribute, so we need to be safe against both " and <.
  // Cloudflare-issued tokens are 32-char hex, but better to be
  // defensive in case the format changes.
  const token = env.CF_ANALYTICS_TOKEN.replace(/[<>"'&]/g, "");
  return `<script defer src="https://static.cloudflareinsights.com/beacon.min.js" data-cf-beacon='{"token":"${token}"}'></script>`;
}

export function landingPage(c: Context<{ Bindings: Env }>) {
  return c.html(`<!doctype html>
<html><head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width,initial-scale=1">
  <title>phantom mesh</title>
  <style>${STYLE}</style>
</head><body>
  <div class="wrap">
    <div class="logo">◆</div>
    <h1>phantom mesh</h1>
    <p class="sub">a self-hostable AI agent that runs everywhere
       <br><span style="color:#6abe6a;font-size:12px">● Worker live · ${new Date().toISOString().slice(0,16)}Z</span></p>
    <div class="card">
      <a class="btn" href="/auth/web/start">Log in</a>
      <a class="btn" href="https://github.com/markl-a/phantom-mesh">GitHub</a>
    </div>

    <div class="card" style="margin-top:16px">
      <div style="font-weight:500">Install phantom CLI</div>
      <p style="color:#8a8578;font-size:12px;margin:0 0 4px;text-transform:uppercase;letter-spacing:0.5px">macOS / Linux</p>
      <div style="display:flex;align-items:flex-start;gap:8px">
        <pre id="cmd-sh" style="background:#0c0c10;border:1px solid #2a2a35;border-radius:6px;padding:10px 12px;color:#d6b270;font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:13px;margin:0;overflow-x:auto;white-space:pre;word-break:keep-all;flex:1">curl -fsSL https://phantommesh.io/install.sh | sh</pre>
        <button class="copy-btn" data-target="cmd-sh" style="padding:8px 10px;background:#1d1d24;color:#8a8578;border:1px solid #2a2a35;border-radius:6px;font-size:11px;cursor:pointer;font-family:inherit;flex:0 0 auto">copy</button>
      </div>
      <p style="color:#8a8578;font-size:12px;margin:14px 0 4px;text-transform:uppercase;letter-spacing:0.5px">Windows (PowerShell)</p>
      <div style="display:flex;align-items:flex-start;gap:8px">
        <pre id="cmd-ps1" style="background:#0c0c10;border:1px solid #2a2a35;border-radius:6px;padding:10px 12px;color:#d6b270;font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:13px;margin:0;overflow-x:auto;white-space:pre;word-break:keep-all;flex:1">iwr -useb https://phantommesh.io/install.ps1 | iex</pre>
        <button class="copy-btn" data-target="cmd-ps1" style="padding:8px 10px;background:#1d1d24;color:#8a8578;border:1px solid #2a2a35;border-radius:6px;font-size:11px;cursor:pointer;font-family:inherit;flex:0 0 auto">copy</button>
      </div>
      <details>
        <summary>direct binary downloads</summary>
        <div class="pad">
          <a class="btn" href="/dist/phantom-darwin-arm64">macOS (Apple Silicon)</a>
          <a class="btn" href="/dist/phantom-windows-x86_64.exe">Windows x86_64</a>
        </div>
      </details>
    </div>

    <div class="footer">
      Broker version ${c.env.BROKER_VERSION} · spec: <a href="https://github.com/markl-a/phantom-mesh/blob/main/docs/PHANTOMMESH-IO-DESIGN.md">design doc</a>
    </div>
  </div>
  <script>
  document.querySelectorAll('button.copy-btn').forEach(b=>{
    b.addEventListener('click',async()=>{
      const t=document.getElementById(b.dataset.target);
      try{await navigator.clipboard.writeText(t.textContent);
        const o=b.textContent;b.textContent='copied';b.style.color='#6abe6a';b.style.borderColor='#6abe6a';
        setTimeout(()=>{b.textContent=o;b.style.color='';b.style.borderColor=''},1500);}catch(e){}
    });
  });
  </script>
  ${analyticsBeacon(c.env)}
</body></html>`);
}

export function loginPage(c: Context<{ Bindings: Env }>) {
  const state = c.req.query("state") ?? "";
  const isCli = !!state;
  return c.html(`<!doctype html>
<html><head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width,initial-scale=1">
  <title>Sign in · phantom mesh</title>
  <style>${STYLE}</style>
</head><body>
  <div class="wrap">
    <div class="logo">◆</div>
    <h1>Sign in</h1>
    <p class="sub">${isCli
      ? "phantom is asking permission for this device"
      : "Welcome — pick how you'd like to sign in"}</p>
    <div class="card">
      <a class="btn" href="/auth/google/start?state=${encodeURIComponent(state)}">
        <svg width="18" height="18" viewBox="0 0 18 18" fill="none">
          <path d="M17.64 9.2c0-.64-.06-1.25-.16-1.84H9v3.48h4.84a4.14 4.14 0 0 1-1.79 2.71v2.26h2.9c1.7-1.56 2.69-3.87 2.69-6.61z" fill="#4285F4"/>
          <path d="M9 18c2.43 0 4.47-.81 5.96-2.18l-2.9-2.26c-.8.54-1.83.86-3.06.86-2.35 0-4.34-1.59-5.05-3.72H.96v2.34A8.99 8.99 0 0 0 9 18z" fill="#34A853"/>
          <path d="M3.95 10.71a5.4 5.4 0 0 1 0-3.42V4.95H.96a8.99 8.99 0 0 0 0 8.1l2.99-2.34z" fill="#FBBC05"/>
          <path d="M9 3.58c1.32 0 2.5.45 3.43 1.35l2.57-2.58A8.97 8.97 0 0 0 9 0a8.99 8.99 0 0 0-8.04 4.95l2.99 2.34C4.66 5.17 6.65 3.58 9 3.58z" fill="#EA4335"/>
        </svg>
        Continue with Google
      </a>
      <details>
        <summary>Sign in with email</summary>
        <form method="POST" action="/auth/email/login" class="pad">
          <input type="hidden" name="state" value="${escapeHtml(state)}">
          <input name="email" type="email" placeholder="email" required autocomplete="email">
          <input name="password" type="password" placeholder="password" required autocomplete="current-password">
          <button class="email" type="submit">Sign in</button>
        </form>
      </details>

      <details>
        <summary>Create an account with email</summary>
        <form method="POST" action="/auth/email/register" class="pad">
          <input type="hidden" name="state" value="${escapeHtml(state)}">
          <input name="email" type="email" placeholder="email" required autocomplete="email">
          <input name="password" type="password" placeholder="password (≥8 chars)" required autocomplete="new-password">
          <button class="email" type="submit">Create account</button>
        </form>
      </details>
    </div>
    <div class="footer">
      Privacy: we store email + device ids only. We never see your prompts, files, or LLM API keys.
    </div>
  </div>
  ${analyticsBeacon(c.env)}
</body></html>`);
}

export async function accountPage(c: Context<{ Bindings: Env }>) {
  const id = await authn(c);
  if (!id) return c.redirect("/auth/web/start");

  const user = await getUserById(c.env, id.userId);
  if (!user) return c.redirect("/auth/web/start");
  const devices = await getUserDevices(c.env, id.userId);
  const identities = await getUserIdentities(c.env, id.userId);

  const deviceRows = devices.length === 0
    ? `<p class="empty">No devices linked yet. Run <code>phantom login</code> on a machine to link it.</p>`
    : devices.map(d => `
        <div class="device">
          <div>
            <div class="device-label">${escapeHtml(d.label ?? d.device_id.slice(0, 8))}</div>
            <div class="device-meta">${escapeHtml(d.device_id)} · last seen ${timeAgo(d.last_seen_at)}</div>
          </div>
        </div>`).join("");

  // LLM key vault form — rendered for every authenticated user.
  // The API (routes/api.ts) is already multi-tenant scoped by user_id
  // from the verified JWT, so showing the form to everyone matches what
  // the API allows. Each user sees + edits their own vault only.

  // Cookie value = the broker JWT we want the CLI to use. It's HttpOnly
  // so the browser JS can't read document.cookie — we embed it into the
  // page HTML at render time. The page is HTTPS-only so it's not in
  // cleartext on the wire, and it's only inside this user's own session.
  const cookieSession = getCookie(c, SESSION_COOKIE) ?? "";
  const sessionToken  = cookieSession;
  const csrf          = await csrfToken(c.env.BROKER_JWT_SECRET, cookieSession);

  const stored = await getUserSettings(c.env, id.userId);
  const fields = [...ALLOWED_ENV_KEYS].map(key => {
    const v = stored.env[key] ?? "";
    const placeholder = v ? `${"•".repeat(Math.max(0, v.length - 4))}${escapeHtml(v.slice(-4))}` : "(unset)";
    return `
      <label class="kv">
        <span class="kv-name">${escapeHtml(key)}</span>
        <input type="password" name="${escapeHtml(key)}" value="" placeholder="${placeholder}" autocomplete="off" />
      </label>`;
  }).join("");
  const lastUpdated = stored.updated_at > 0 ? `last updated ${timeAgo(stored.updated_at)}` : "never saved";

  // First-run hint: if this user has never saved any keys, show
  // direct links to the providers' free-tier signup pages so they
  // know where to get the values to paste in. Disappears once at
  // least one key has been saved (no point taking up space then).
  const hasAnyKey = Object.keys(stored.env).length > 0;
  const onboardingHint = hasAnyKey ? "" : `
    <div style="background:#16161c;border:1px dashed #4d4a42;border-radius:6px;padding:12px;
                margin:0 0 12px;font-size:13px;color:#8a8578">
      <strong style="color:#d6b270">Where to get free keys:</strong>
      <ul style="margin:8px 0 0;padding-left:20px;line-height:1.7">
        <li><a href="https://console.groq.com/keys" target="_blank" rel="noreferrer" style="color:#d6b270">Groq</a> — free tier, very fast Llama models (recommended starting point)</li>
        <li><a href="https://openrouter.ai/keys" target="_blank" rel="noreferrer" style="color:#d6b270">OpenRouter</a> — single key, many models, generous free tier on selected ones</li>
        <li><a href="https://aistudio.google.com/app/apikey" target="_blank" rel="noreferrer" style="color:#d6b270">Google AI Studio</a> — Gemini, free tier</li>
        <li><a href="https://platform.openai.com/api-keys" target="_blank" rel="noreferrer" style="color:#d6b270">OpenAI</a> · <a href="https://console.anthropic.com/settings/keys" target="_blank" rel="noreferrer" style="color:#d6b270">Anthropic</a> — paid, optional</li>
      </ul>
      <div style="margin-top:8px;color:#6a6a78">
        Paste any one of them in the matching field below; phantom auto-failovers
        between configured providers, so you only need one to start.
      </div>
    </div>`;

  const settingsBlock = `
    <div class="section-title">LLM provider keys
      <span style="text-transform:none;letter-spacing:0;color:#6a6a78;font-weight:normal"> · ${lastUpdated}</span>
    </div>
    <p style="color:#8a8578;font-size:13px;margin:0 0 12px">
      Saved here, pulled on each machine via <code>phantom config pull</code>.
      Empty fields keep the existing stored value (don't blank a key by
      leaving the box empty — type a single space to clear it explicitly).
      Stored encrypted-at-rest in D1; only your authenticated session can read.
    </p>
    ${onboardingHint}
    <form id="settings-form" class="kv-form">
      ${fields}
      <div class="kv-actions">
        <button type="submit" class="primary">Save</button>
        <span id="settings-status" class="kv-status"></span>
      </div>
    </form>`;

  return c.html(`<!doctype html>
<html><head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width,initial-scale=1">
  <title>Account · phantom mesh</title>
  <style>${STYLE}
    .row{display:flex;align-items:center;gap:12px;margin-bottom:24px}
    .avatar{width:48px;height:48px;border-radius:50%;background:#1d1d24;
            display:flex;align-items:center;justify-content:center;font-size:20px;color:#d6b270}
    .avatar img{width:100%;height:100%;border-radius:50%}
    .who{flex:1}
    .who-name{font-size:18px}
    .who-email{color:#8a8578;font-size:13px}
    .section-title{font-size:13px;color:#8a8578;text-transform:uppercase;letter-spacing:.08em;margin:24px 0 8px}
    .device{display:flex;align-items:center;justify-content:space-between;padding:12px 14px;
            background:#1d1d24;border:1px solid #2a2a35;border-radius:8px;margin-bottom:8px}
    .device-label{font-size:14px}
    .device-meta{color:#6a6a78;font-size:12px;font-family:ui-monospace,monospace}
    .empty{color:#8a8578;font-size:14px;text-align:center;padding:16px}
    .ghost{background:transparent;border:1px solid #2a2a35;color:#8a8578;
           padding:6px 12px;font-size:12px;border-radius:6px;width:auto;cursor:pointer}
    .ghost:hover{border-color:#d6b270;color:#d6b270}
    .inline{margin:0}
    .danger{background:transparent;color:#a06060;border:1px solid #2a2a35}
    .danger:hover{border-color:#a06060;background:#2a1818}
    code{background:#0c0c10;padding:2px 6px;border-radius:4px;font-size:12px}
    .kv-form{display:flex;flex-direction:column;gap:8px}
    .kv{display:flex;align-items:center;gap:10px;background:#1d1d24;border:1px solid #2a2a35;
        border-radius:6px;padding:8px 10px}
    .kv-name{flex:0 0 160px;font-family:ui-monospace,monospace;font-size:12px;color:#d6b270}
    .kv input{background:#0c0c10;border:1px solid #2a2a35;flex:1}
    .kv input::placeholder{color:#4d4a42;font-family:ui-monospace,monospace;font-size:12px}
    .kv-actions{display:flex;align-items:center;gap:12px;margin-top:8px}
    .kv-actions button.primary{background:#d6b270;color:#0c0c10;border:0;font-weight:600;
                                 width:auto;padding:8px 18px}
    .kv-status{color:#6a6a78;font-size:12px}
    .kv-status.ok{color:#6abe6a}
    .kv-status.err{color:#a06060}
  </style>
</head><body>
  <div class="wrap">
    <div class="logo">◆</div>
    <h1>Account</h1>
    <div class="card">
      <div class="row">
        <div class="avatar">${user.avatar_url
          ? `<img src="${escapeHtml(user.avatar_url)}" alt="">`
          : (user.email[0] ?? "?").toUpperCase()}</div>
        <div class="who">
          <div class="who-name">${escapeHtml(user.display_name ?? user.email.split("@")[0])}</div>
          <div class="who-email">${escapeHtml(user.email)} · via ${escapeHtml(user.provider)}</div>
        </div>
      </div>

      <div class="section-title">Sign-in methods (${identities.length})</div>
      ${identities.length === 0
        ? `<p class="empty">No sign-in methods recorded yet. Re-login to populate.</p>`
        : identities.map(idn => `
        <div class="device">
          <div>
            <div class="device-label">${escapeHtml(providerLabel(idn.provider))}</div>
            <div class="device-meta">first linked ${timeAgo(idn.first_linked_ms)} · last used ${timeAgo(idn.last_used_ms)}</div>
          </div>
        </div>`).join("")}

      <div class="section-title">Linked devices (${devices.length})</div>
      ${deviceRows}

      ${settingsBlock}

      <div class="section-title">Sync to CLI</div>
      <p style="color:#8a8578;font-size:13px;margin:0 0 12px">
        On any machine that has phantom installed, paste this once. Token
        is your current login session — keep it private (anyone with this
        string can read your stored keys until you log out).
      </p>
      <div style="background:#0c0c10;border:1px solid #2a2a35;border-radius:6px;padding:10px 12px;
                  font-family:ui-monospace,monospace;font-size:12px;color:#d6b270;
                  word-break:break-all;position:relative">
        <div id="cli-cmd-mask" style="color:#4d4a42">phantom config pull --token=•••••••••••••••• --url=https://phantommesh.io</div>
        <div id="cli-cmd-real" style="display:none;color:#d6b270"></div>
        <div style="margin-top:10px;display:flex;gap:8px">
          <button type="button" id="cli-reveal" class="ghost" style="width:auto">Reveal</button>
          <button type="button" id="cli-copy"   class="ghost" style="width:auto" disabled>Copy</button>
          <span id="cli-copy-status" style="color:#6abe6a;font-size:12px;align-self:center"></span>
        </div>
      </div>

      ${await renderClusterPeersBlock(c, id.userId)}

      <div class="section-title">Plan</div>
      <p style="color:#8a8578;font-size:14px;margin:0 0 12px">
        <strong style="color:#e8e2d4">Free</strong> — local-only mesh, unlimited devices.
        <span style="color:#6a6a78"> · Pro / Team coming soon.</span>
      </p>

      <form method="POST" action="/auth/logout" class="inline" style="margin-top:16px">
        <input type="hidden" name="csrf" value="${escapeHtml(csrf)}" />
        <button type="submit" class="ghost danger">Sign out</button>
      </form>
    </div>
    <div class="footer">
      Privacy: prompts and files never leave your machines. LLM API keys
      saved in the form above are stored encrypted-in-transit + isolated
      per user; only your authenticated session can read them.
    </div>
  </div>
  <script>
    // Sync-to-CLI: server-side embedded the current broker JWT into the
    // hidden #cli-cmd-real div. Reveal swaps the masked version for the
    // real one; Copy puts it on the clipboard.
    (function () {
      const realToken = ${JSON.stringify(sessionToken)};
      const realCmd = 'phantom config pull --token=' + realToken + ' --url=https://phantommesh.io';
      const mask = document.getElementById('cli-cmd-mask');
      const real = document.getElementById('cli-cmd-real');
      const reveal = document.getElementById('cli-reveal');
      const copy = document.getElementById('cli-copy');
      const status = document.getElementById('cli-copy-status');
      if (!realToken) {
        if (reveal) reveal.disabled = true;
        if (mask) mask.textContent = '(no session token — re-login)';
        return;
      }
      real.textContent = realCmd;
      reveal.addEventListener('click', () => {
        const showing = real.style.display !== 'none';
        if (showing) {
          real.style.display = 'none'; mask.style.display = '';
          reveal.textContent = 'Reveal';
          copy.disabled = true;
        } else {
          mask.style.display = 'none'; real.style.display = '';
          reveal.textContent = 'Hide';
          copy.disabled = false;
        }
      });
      copy.addEventListener('click', async () => {
        try {
          await navigator.clipboard.writeText(realCmd);
          status.textContent = '✓ copied';
          setTimeout(() => status.textContent = '', 2000);
        } catch (e) {
          status.style.color = '#a06060';
          status.textContent = '✗ ' + e.message;
        }
      });
    })();

    // Submit handler: gather only the non-empty inputs (so unchecked rows
    // don't blank stored values), POST to /api/me/settings, surface the
    // masked round-trip result so the user sees what's actually saved.
    document.getElementById('settings-form').addEventListener('submit', async (e) => {
      e.preventDefault();
      const status = document.getElementById('settings-status');
      status.className = 'kv-status';
      status.textContent = 'saving...';
      const env = {};
      for (const input of e.target.querySelectorAll('input[type="password"]')) {
        if (input.value === '') continue;
        env[input.name] = input.value;
      }
      // Guard: empty submit is almost always a "did it save? let me press
      // Save again" reflex right after a successful save+reload (when all
      // boxes are placeholder-only). Server side now does merge semantics
      // so this would be a no-op anyway, but explaining it to the user
      // beats a silent ✓ that didn't store anything new.
      if (Object.keys(env).length === 0) {
        status.className = 'kv-status err';
        status.textContent = 'no changes — type a key into a box first (existing keys are kept untouched)';
        return;
      }
      try {
        const r = await fetch('/api/me/settings', {
          method: 'PUT',
          headers: { 'Content-Type': 'application/json' },
          credentials: 'same-origin',
          body: JSON.stringify({ env }),
        });
        if (!r.ok) throw new Error('HTTP ' + r.status);
        const data = await r.json();
        status.className = 'kv-status ok';
        const n = Object.keys(data.env || {}).length;
        status.textContent = '✓ saved · ' + n + ' keys stored';
        for (const input of e.target.querySelectorAll('input[type="password"]')) input.value = '';
        setTimeout(() => location.reload(), 1500);
      } catch (err) {
        status.className = 'kv-status err';
        status.textContent = '✗ ' + (err.message || 'failed');
      }
    });

    // ── Cluster peers editor ────────────────────────────────────────
    (function () {
      const initEl = document.getElementById('peers-init');
      if (!initEl) return;
      let peers = [];
      try { peers = JSON.parse(initEl.textContent || '[]'); } catch (e) {}
      const rowsHost = document.getElementById('peers-rows');
      const status = document.getElementById('peers-status');

      function row(p) {
        const div = document.createElement('div');
        div.className = 'kv';
        const badge = p.badgeText
          ? '<span style="background:' + p.badgeColor + ';color:#0c0c10;padding:2px 6px;border-radius:3px;font-size:10px;font-weight:600;margin-right:6px">' + p.badgeText + '</span>'
          : '';
        const lastSeen = p.lastSeen
          ? '<span style="color:#6a6a78;font-size:11px;margin-right:6px;font-family:ui-monospace,monospace">' + p.lastSeen + '</span>'
          : '';
        // Two-line layout to fit the new capabilities column without
        // making rows too wide on narrower screens.
        const wrap = document.createElement('div');
        wrap.style.cssText = 'display:flex;flex-direction:column;gap:4px;width:100%';
        wrap.innerHTML =
          '<div style="display:flex;align-items:center;gap:8px">' +
            '<input class="peer-name"  placeholder="name (e.g. node-a)"   value="' + escapeAttr(p.name)  + '" style="flex:0 0 130px">' +
            '<input class="peer-url"   placeholder="http://100.x.y.z:7878" value="' + escapeAttr(p.url)   + '" style="flex:1">' +
            badge + lastSeen +
            '<button type="button" class="ghost peer-rm" style="width:auto;flex:0 0 auto">×</button>' +
          '</div>' +
          '<div style="display:flex;align-items:center;gap:8px">' +
            '<input class="peer-label"  placeholder="label"                            value="' + escapeAttr(p.label || '')        + '" style="flex:0 0 130px">' +
            '<input class="peer-caps"   placeholder="capabilities (comma-separated): rust, build, gpu, ..." value="' + escapeAttr(p.capabilities || '') + '" style="flex:1">' +
          '</div>';
        div.appendChild(wrap);
        wrap.querySelector('.peer-rm').addEventListener('click', () => div.remove());
        return div;
      }
      function escapeAttr(s) {
        return String(s).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;');
      }
      peers.forEach(p => rowsHost.appendChild(row(p)));

      document.getElementById('peers-add').addEventListener('click', () => {
        rowsHost.appendChild(row({ name: '', url: '', label: '' }));
      });

      document.getElementById('peers-save').addEventListener('click', async () => {
        status.className = 'kv-status';
        status.textContent = 'saving...';
        const collected = [];
        for (const r of rowsHost.querySelectorAll('.kv')) {
          const name  = r.querySelector('.peer-name').value;
          const url   = r.querySelector('.peer-url').value;
          const label = r.querySelector('.peer-label').value;
          const caps  = r.querySelector('.peer-caps').value;
          collected.push({ name, url, label, capabilities: caps });
        }
        try {
          const r = await fetch('/api/me/cluster-peers', {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            credentials: 'same-origin',
            body: JSON.stringify({ peers: collected }),
          });
          if (!r.ok) throw new Error('HTTP ' + r.status);
          const data = await r.json();
          status.className = 'kv-status ok';
          status.textContent = '✓ saved · ' + (data.peers || []).length + ' peers';
          // Soft refresh so the count badge in the section title updates
          setTimeout(() => location.reload(), 1500);
        } catch (err) {
          status.className = 'kv-status err';
          status.textContent = '✗ ' + (err.message || 'failed');
        }
      });
    })();
  </script>
  ${analyticsBeacon(c.env)}
</body></html>`);
}

/// Cluster peers editor — small table of {name, url, label} rows the
/// user can add/remove, with a Save button that PUTs the whole list.
/// Empty rows are filtered server-side, so the user can leave a blank
/// trailing row in the form without it hitting the DB. JSON-bridges to
/// the inline <script> via a data attribute so the JS doesn't have to
/// fetch on page load.
async function renderClusterPeersBlock(c: Context<{ Bindings: Env }>, userId: number): Promise<string> {
  const peers = await getUserClusterPeers(c.env, userId);
  const rows = peers.length === 0 ? [{ name: "", url: "", label: null, updated_at: 0 }] : peers;
  // Pre-rendered "alive / stale / offline" badges per peer based on
  // updated_at age. Avoids needing a JS poll on page load.
  const now = Date.now();
  const peersJson = JSON.stringify(rows.map(p => {
    const ageMs = p.updated_at > 0 ? now - p.updated_at : -1;
    let badgeText = "";
    let badgeColor = "#6a6a78";
    if (ageMs < 0) { badgeText = "never"; badgeColor = "#6a6a78"; }
    else if (ageMs <       5 * 60_000) { badgeText = "alive";       badgeColor = "#6abe6a"; }
    else if (ageMs <      60 * 60_000) { badgeText = "stale";       badgeColor = "#d6b270"; }
    else                                { badgeText = "offline";     badgeColor = "#a06060"; }
    const caps = (p as { capabilities?: string[] }).capabilities ?? [];
    return {
      name: p.name, url: p.url, label: p.label ?? "",
      capabilities: caps.join(", "),
      lastSeen: p.updated_at > 0 ? humanAgo(ageMs) : "never",
      badgeText, badgeColor,
    };
  }));
  return `
      <div class="section-title">Cluster peers
        <span style="text-transform:none;letter-spacing:0;color:#6a6a78;font-weight:normal">
          · ${peers.length} configured
        </span>
      </div>
      <p style="color:#8a8578;font-size:13px;margin:0 0 12px">
        Mesh nodes that <code>phantom cluster join/status</code> dispatches RPC to.
        Pulled by every machine via <code>phantom config pull</code>; saving here
        immediately propagates on next pull. The <strong>last-seen</strong> badge
        is updated whenever a node runs <code>phantom login</code> or
        <code>phantom config pull</code>. URLs are Tailscale (100.x.y.z) or LAN;
        the actual cross-node auth secret is CLUSTER_SECRET above.
      </p>
      <div id="peers-rows"></div>
      <div class="kv-actions">
        <button type="button" id="peers-add" class="ghost" style="width:auto">+ Add row</button>
        <button type="button" id="peers-save" class="primary">Save</button>
        <span id="peers-status" class="kv-status"></span>
      </div>
      <script id="peers-init" type="application/json">${peersJson}</script>`;
}

/// "37s ago" / "5m ago" / "2h ago" / "3d ago" — same shape as cli_config.rs::human_age.
function humanAgo(ms: number): string {
  const s = Math.floor(ms / 1000);
  if (s < 60)        return `${s}s ago`;
  if (s < 3600)      return `${Math.floor(s / 60)}m ago`;
  if (s < 86_400)    return `${Math.floor(s / 3600)}h ago`;
  return `${Math.floor(s / 86_400)}d ago`;
}

function timeAgo(ms: number): string {
  const s = Math.floor((Date.now() - ms) / 1000);
  if (s < 60)    return `${s}s ago`;
  if (s < 3600)  return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  return `${Math.floor(s / 86400)}d ago`;
}

function escapeHtml(s: string): string {
  return s.replace(/[&<>"']/g, (ch) => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", "\"": "&quot;", "'": "&#39;"
  }[ch] ?? ch));
}

function providerLabel(provider: string): string {
  switch (provider) {
    case "google": return "Google";
    case "email":  return "Email + password";
    case "apple":  return "Apple";
    default:       return provider;
  }
}
