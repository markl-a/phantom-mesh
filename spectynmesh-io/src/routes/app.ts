// /app + /app/* — serves the dashboard SPA built by
// spectynmesh-io/dashboard/. Assets are uploaded to the existing
// R2 `BINARIES` bucket under the `app/` prefix by the deploy
// workflow before `wrangler deploy` runs (see F200 spec §Risk
// register row 1 for why we use a prefix rather than a separate
// bucket — keeps the wrangler.toml binding count flat).
//
// CSP per F200 spec §Scope:
//   default-src 'self';
//   script-src  'self';                     ← no unsafe-inline, no unsafe-eval
//   style-src   'self' 'unsafe-inline';     ← Tailwind atomic classes need this
//   img-src     'self' data: https://lh3.googleusercontent.com;
//   connect-src 'self';
//   frame-ancestors 'none';
//   base-uri 'self';
//   form-action 'self';
//
// The `lh3.googleusercontent.com` allowance is for the Google avatar
// rendered in the account chip (existing pattern from /account page).

import type { Context } from "hono";
import type { Env } from "../types";

const SPA_CSP = [
  "default-src 'self'",
  "script-src 'self'",
  "style-src 'self' 'unsafe-inline'",
  "img-src 'self' data: https://lh3.googleusercontent.com",
  "connect-src 'self'",
  "frame-ancestors 'none'",
  "base-uri 'self'",
  "form-action 'self'",
].join("; ");

// Extension → Content-Type map. Locked to the 4 file types the Vite
// build actually emits — anything else is a build-tool drift signal
// and 404s rather than getting a guessed MIME.
const MIME: Record<string, string> = {
  ".html": "text/html; charset=utf-8",
  ".js": "application/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".svg": "image/svg+xml",
  ".woff2": "font/woff2",
  ".ico": "image/x-icon",
  ".map": "application/json; charset=utf-8",
};

function extOf(path: string): string {
  const dot = path.lastIndexOf(".");
  if (dot < 0) return "";
  return path.slice(dot).toLowerCase();
}

function spaHeaders(extraHeaders: HeadersInit = {}): HeadersInit {
  return {
    "Content-Security-Policy": SPA_CSP,
    "X-Frame-Options": "DENY",
    "Referrer-Policy": "strict-origin-when-cross-origin",
    ...extraHeaders,
  };
}

// Hashed asset URLs get long-lived caching; index.html only gets a
// short window so a freshly deployed build is picked up within 60 s.
function cacheControlFor(path: string): string {
  if (path.endsWith("/index.html") || path === "app/index.html") {
    return "public, max-age=60, must-revalidate";
  }
  return "public, max-age=31536000, immutable";
}

/**
 * `GET /app` and `GET /app/*` — serves the dashboard SPA.
 *
 * Asset resolution order:
 *   1. If the URL maps to a real R2 object under `app/<rest>`, stream it.
 *   2. Otherwise (deep-link to a client-side route like `/app/cluster`),
 *      stream `app/index.html` so React Router can take over.
 *   3. If R2 has no `app/index.html` at all yet (e.g. F200 worker
 *      shipped but the dashboard hasn't been built into R2 in this
 *      env), return a small inline placeholder so the route is at
 *      least observable + CSP-correct.
 */
export async function appHandler(c: Context<{ Bindings: Env }>) {
  const url = new URL(c.req.url);
  // Strip the leading `/app/` (or `/app`) to get the R2 sub-key.
  const rest = url.pathname.replace(/^\/app\/?/, "");

  // Hashed-asset request (anything with a known extension under /app/).
  // Worker only serves files it recognizes by extension — anything else
  // (no extension, or extensions outside the build manifest) is treated
  // as a deep link and falls through to the SPA HTML.
  const ext = extOf(rest);
  const isAssetRequest = rest !== "" && ext && MIME[ext] && rest !== "index.html";
  if (isAssetRequest) {
    const key = `app/${rest}`;
    const obj = await c.env.BINARIES.get(key);
    if (obj) {
      return new Response(obj.body, {
        headers: spaHeaders({
          "Content-Type": MIME[ext],
          "Cache-Control": cacheControlFor(key),
          ETag: obj.httpEtag,
        }),
      });
    }
    // Hashed-asset request that 404'd in R2 — return a real 404 so
    // the browser surfaces the broken bundle deterministically
    // rather than getting served the index.html with the wrong
    // MIME (a JS request would `Unexpected token <` otherwise).
    return new Response("Not Found", {
      status: 404,
      headers: spaHeaders({ "Content-Type": "text/plain; charset=utf-8" }),
    });
  }

  // Root (/app, /app/, /app/index.html) or any deep link (/app/cluster,
  // /app/dispatch/abc123, …) — all serve the SPA shell.
  const indexObj = await c.env.BINARIES.get("app/index.html");
  if (indexObj) {
    return new Response(indexObj.body, {
      headers: spaHeaders({
        "Content-Type": "text/html; charset=utf-8",
        "Cache-Control": cacheControlFor("app/index.html"),
      }),
    });
  }

  // Inline placeholder — used in dev / staging before the SPA assets
  // have been uploaded. Keeps the route observable (200 + CSP) so the
  // deploy + auth gate can be smoke-tested independently of the SPA
  // build artifact.
  const placeholder = `<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <title>spectyn mesh — dashboard</title>
  </head>
  <body>
    <main style="font-family: system-ui, sans-serif; padding: 2rem;">
      <h1>spectyn mesh — dashboard</h1>
      <p>SPA bundle not yet uploaded to this environment.</p>
      <p>Operator: run the deploy workflow or
        <code>npm run build --workspace=dashboard</code> +
        upload <code>dist/app/*</code> to the R2 <code>app/</code> prefix.</p>
    </main>
  </body>
</html>`;
  return new Response(placeholder, {
    headers: spaHeaders({
      "Content-Type": "text/html; charset=utf-8",
      "Cache-Control": "no-store",
    }),
  });
}

// Exported so the unit tests can validate the CSP shape without
// going through a full miniflare round-trip.
export const __test = { SPA_CSP, MIME, cacheControlFor };
