# phantommesh.io — Broker Frontend + Backend Design

> The web side of `phantom login`. This doc is the implementation
> contract: it tells you exactly what URLs phantommesh.io must serve
> and what shape the responses must take, because the phantom CLI
> already speaks this protocol (see `core/src/bin/phantom.rs`
> `login_broker`).
>
> Once phantommesh.io exists at the URLs below, **`phantom login` Just
> Works** — no further code change in the CLI is needed.

---

## 0. The Two Pieces

```
   phantom CLI (your laptop)        phantommesh.io                  OAuth providers
                                    ┌──────────────────┐             ┌──────────┐
                                    │  Frontend (web)  │             │  Google  │
   phantom login   ─────────────►   │   /              │  ─────►    │  Apple   │
                                    │   /login         │             │  email   │
                                    │   /auth/cli/start│             └──────────┘
                                    │                  │
                                    ├──────────────────┤
                                    │  Backend (API)   │
                                    │   /api/health    │
                                    │   /auth/*/start  │
                                    │   /auth/*/cb     │             ┌──────────┐
                                    │   /api/devices   │  ─────►    │ Database │
                                    │   /api/me        │             │ (D1/PG)  │
                                    └──────────────────┘             └──────────┘

   ◄───────  loopback :48181/oauth/callback  ─────────
```

phantommesh.io is **the trusted middleman**. It speaks OAuth to Google
/ Apple on the user's behalf, then hands the resulting identity to the
phantom CLI over a localhost loopback. The CLI never sees the user's
Google password, never sees Apple's id_token signing key, only sees a
JSON payload that says "here's who you are."

---

## 1. Wire Format (already implemented in CLI; freeze this)

### 1.1 Health probe

```
GET https://phantommesh.io/api/health

  200 OK   { "status": "ok", "version": "1.0.0", "providers": ["google","apple","email"] }
  503      anything else → CLI falls back to local provider menu
```

phantom CLI probes this with a 3-second timeout the moment a user runs
`phantom login` (no args). If it times out or 4xx/5xx, the CLI shows
the local fallback menu (email / google direct / apple stub).

### 1.2 Start the login flow

```
GET https://phantommesh.io/auth/cli/start
  ?device_id=8de1a55b-7412-...           (CLI's stable uuid)
  &port=48181                             (CLI loopback listener port)
  &redirect=http%3A%2F%2F127.0.0.1%3A48181%2Foauth%2Fcallback
```

Server behavior:

1. Validate `device_id` is a uuid, `port` is `[1024, 65535]`, `redirect`
   matches `^http://(127\.0\.0\.1|localhost):\d+/oauth/callback$`.
2. Set a server-side session cookie carrying `(device_id, redirect)`.
3. `302 → /login?cli=1` (or render a login page directly).

### 1.3 Provider OAuth dance (server-side)

After the user picks a provider on `/login`, the broker handles the
OAuth dance the way any web app would:

| Provider | Flow |
|---|---|
| Google | Standard PKCE OAuth via `accounts.google.com`. Configure your Google OAuth Client (Web App type) with `https://phantommesh.io/auth/google/callback` as the redirect URI. |
| Apple | "Sign in with Apple" web flow. Configure your Apple Service ID (`ai.phantommesh.auth` — already exists in `core/src/oauth.rs`) with the same redirect domain. Use the `.p8` private key + Team ID + Key ID for client secret JWT generation. |
| email | Server-side bcrypt validation against the broker's user DB. No external IdP. |

After the IdP redirects back, the broker:
1. Exchanges code → access_token + id_token (Google) or validates the
   Apple id_token signature.
2. Looks up / creates a row in `users` table keyed by `(provider, sub)`
   or `(email)`.
3. Records the device claim in `devices` table:
   `(device_id, owner_user_id, claimed_at, label=hostname)`.
4. Builds the **payload** (see 1.4).

### 1.4 Hand the identity to the CLI (the load-bearing step)

After the IdP dance completes, the broker redirects the **browser** to
the CLI's loopback URL with the identity payload as a base64-encoded
JSON in the query string:

```
302 Location: http://127.0.0.1:48181/oauth/callback?p=<base64url(json)>
```

The JSON payload:

```json
{
  "provider":     "google",
  "email":        "alice@example.com",
  "sub":          "1234567890",
  "name":         "Alice Cooper",
  "picture":      "https://lh3.googleusercontent.com/...",
  "id_token":     "<google id_token JWT>",
  "access_token": "<google access_token>",
  "broker_token": "<short-lived JWT issued by us, used for /api/* calls>",
  "broker_token_expires_at_ms": 1730000000000
}
```

The CLI's `login_broker` already extracts `provider / email / sub /
name / picture / id_token / access_token` from this exact shape (see
core/src/bin/phantom.rs). New fields are additive — the CLI ignores
them.

> **Why query-string-base64 and not POST**: CORS is annoying on the
> CLI loopback side, redirect+query is one less moving part. The
> token is exposed only on the local user's URL bar for milliseconds.
> If you want stricter, use a one-time exchange code:
>   `Location: http://127.0.0.1:48181/oauth/callback?code=<short-lived>`
>   then have the CLI POST that code to
>   `https://phantommesh.io/api/exchange` to get the real payload.
>   Costs one extra round-trip; gains zero token in URL.

### 1.5 Authenticated API surface (for `phantom devices` etc., later)

After the CLI has `broker_token`, it can call:

```
GET  /api/me                            → user profile
GET  /api/devices                       → [ {device_id, label, last_seen, ...}, ... ]
POST /api/devices/{device_id}/claim     → claim a discovery'd device
DELETE /api/devices/{device_id}         → revoke
```

All authenticated with `Authorization: Bearer <broker_token>`. None
of these are required for v1 login — they're for the cluster
discovery layer added later.

---

## 2. Recommended tech stack

For the broker, the cheapest-to-run + lowest-operational-burden
combo we've seen for projects this size:

| Layer | Recommendation | Why |
|---|---|---|
| Hosting | **Cloudflare Workers + Pages** | $0 free tier covers 100K req/day; no cold start; Workers KV / D1 for state |
| Frontend | **Next.js 15 / SvelteKit** on Cloudflare Pages | edge-deployed, OAuth callback runs at edge |
| Backend | **Hono** (Cloudflare-Worker-friendly) or Next.js API routes | Minimal Express-like surface; fits the broker's 6 routes nicely |
| Database | **Cloudflare D1** (SQLite at the edge) | Free 5 GB, perfect for users + devices + sessions |
| Sessions | **Cloudflare KV** (TTL'd) | 60 s session for the OAuth dance, 7-day for the broker_token |
| OAuth lib | **`@auth/core`** (Auth.js v5) | Speaks Google + Apple + email; framework-agnostic |
| Apple `.p8` key signing | **`jose`** (npm) | EC private key → JWT in 3 lines |

Estimated infra cost at 1k MAU: **\$0/mo** (everything fits free tier).
At 10k MAU: ~$5/mo. At 100k: ~$25/mo. Mostly D1 row-storage growth.

If you'd rather not Cloudflare:

- **Fly.io machine + Postgres** — more familiar; one $1.94/mo VM
- **Vercel + Neon** — same ergonomics; pricing similar
- **Self-host on the Mac coordinator** — phantom serve already
  has axum + reqwest + sqlite; you could literally run the broker
  from the same binary if you wanted to keep things minimal. Less
  good for a public service (NAT / restart / TLS) but great for
  isolated team deployments

---

## 3. Database schema (D1 / Postgres)

```sql
CREATE TABLE users (
    id           INTEGER PRIMARY KEY,
    email        TEXT NOT NULL UNIQUE,
    provider     TEXT NOT NULL,        -- 'google' | 'apple' | 'email'
    sub          TEXT,                  -- IdP subject
    display_name TEXT,
    avatar_url   TEXT,
    password_hash TEXT,                 -- bcrypt; only set when provider='email'
    created_at   INTEGER NOT NULL,
    last_login_at INTEGER NOT NULL
);

CREATE TABLE devices (
    device_id    TEXT PRIMARY KEY,      -- the phantom CLI's uuid
    user_id      INTEGER NOT NULL REFERENCES users(id),
    label        TEXT,                  -- hostname, mac model, etc.
    public_addr  TEXT,                  -- last-seen tailscale IP
    claimed_at   INTEGER NOT NULL,
    last_seen_at INTEGER NOT NULL
);

CREATE TABLE oauth_sessions (
    state        TEXT PRIMARY KEY,
    device_id    TEXT NOT NULL,
    redirect     TEXT NOT NULL,
    code_verifier TEXT NOT NULL,
    created_at   INTEGER NOT NULL
);                                       -- TTL'd by Cloudflare KV after 5 min

CREATE TABLE broker_tokens (
    token_hash   TEXT PRIMARY KEY,       -- SHA-256 of the actual JWT
    user_id      INTEGER NOT NULL REFERENCES users(id),
    device_id    TEXT,                   -- which device the token was issued for
    issued_at    INTEGER NOT NULL,
    expires_at   INTEGER NOT NULL,
    revoked_at   INTEGER
);

CREATE INDEX idx_devices_user ON devices(user_id);
CREATE INDEX idx_tokens_user ON broker_tokens(user_id);
```

Around ~200 KB per active user with all the joins; 5 GB D1 free tier
holds roughly 25k users.

---

## 4. The five files you need to write

If you go with the Cloudflare Workers + Hono stack, the entire broker
fits in five files. Here is the skeleton (ready to copy):

### 4.1 `src/index.ts`

```typescript
import { Hono } from "hono";
import { cors } from "hono/cors";
import { health } from "./routes/health";
import { authStart, googleCallback, appleCallback } from "./routes/oauth";
import { me, devices } from "./routes/api";
import { login } from "./routes/login";

const app = new Hono<{ Bindings: Env }>();
app.use("*", cors({ origin: ["https://phantommesh.io"] }));

app.get("/api/health", health);
app.get("/auth/cli/start", authStart);
app.get("/auth/google/callback", googleCallback);
app.get("/auth/apple/callback", appleCallback);
app.get("/api/me", me);
app.get("/api/devices", devices);
app.get("/login", login);          // server-renders the buttons page

export default app;
```

### 4.2 `src/routes/oauth.ts` (the load-bearing one)

```typescript
import type { Context } from "hono";
import { generatePkce, exchangeGoogleCode, validateApple } from "../lib/oauth";
import { upsertUser, claimDevice, mintBrokerToken } from "../lib/db";

export async function authStart(c: Context) {
  const device_id = c.req.query("device_id");
  const port = parseInt(c.req.query("port") ?? "0");
  const redirect = c.req.query("redirect") ?? "";
  if (!isUuid(device_id) || port < 1024 || port > 65535) {
    return c.text("invalid params", 400);
  }
  if (!/^http:\/\/(127\.0\.0\.1|localhost):\d+\/oauth\/callback$/.test(redirect)) {
    return c.text("invalid redirect", 400);
  }
  const state = crypto.randomUUID();
  const { verifier, challenge } = generatePkce();
  await c.env.SESSIONS.put(state, JSON.stringify({ device_id, redirect, verifier }), { expirationTtl: 300 });
  return c.redirect(`/login?state=${state}&challenge=${challenge}`);
}

export async function googleCallback(c: Context) {
  const code = c.req.query("code");
  const state = c.req.query("state");
  const session = JSON.parse(await c.env.SESSIONS.get(state) ?? "{}");
  if (!session.device_id) return c.text("session expired", 400);

  const { id_token, access_token, claims } = await exchangeGoogleCode(c.env, code, session.verifier);
  const user = await upsertUser(c.env, {
    email: claims.email, sub: claims.sub, provider: "google",
    display_name: claims.name, avatar_url: claims.picture,
  });
  await claimDevice(c.env, session.device_id, user.id);
  const broker_token = await mintBrokerToken(c.env, user.id, session.device_id);

  const payload = btoa(JSON.stringify({
    provider: "google", email: claims.email, sub: claims.sub,
    name: claims.name, picture: claims.picture,
    id_token, access_token,
    broker_token,
    broker_token_expires_at_ms: Date.now() + 7*86400_000,
  }));
  return c.redirect(`${session.redirect}?p=${encodeURIComponent(payload)}`);
}

export async function appleCallback(c: Context) {
  // Apple returns id_token as form_post — same shape as Google after that.
  // Use jose to verify Apple's RS256 signature against
  // https://appleid.apple.com/auth/keys
  // ... (mirror googleCallback)
}
```

### 4.3 `src/lib/oauth.ts` — Google + Apple OAuth helpers

Standard PKCE; ~80 lines. Use `@auth/core/providers/google` and
`@auth/core/providers/apple` if you want them off-the-shelf, or roll
your own — Google's flow is just `POST oauth2.googleapis.com/token`
with the right form fields.

### 4.4 `src/lib/db.ts` — D1 wrappers

`upsertUser`, `getUser`, `claimDevice`, `getDevices`, `mintBrokerToken`,
`verifyBrokerToken`. ~120 lines of straightforward SQL.

### 4.5 `src/routes/login.tsx` — the actual sign-in page

A 50-line server-rendered page with three buttons:
"Continue with Google", "Continue with Apple", "Sign in with email".
Each button is a form POST that picks up `state` from the query string
and redirects to the appropriate `/auth/{provider}/start` route.

---

## 5. Implementation roadmap

| Day | Scope |
|---|---|
| **1** | Cloudflare account + D1 + KV + Workers project bootstrapped. `/api/health` returns 200. phantom CLI's broker probe goes from "offline" to "online" — but everything else still 404s. |
| **2** | `/auth/cli/start` + `/login` page + the Google OAuth callback wired end-to-end. First successful `phantom login` round-trip (Google only). |
| **3** | Apple Sign In wired (this is the longest single integration — needs the .p8 key, Team ID, Service ID setup in Apple Developer console). |
| **4** | Email/password (bcrypt) + the `/api/me`/`/api/devices` endpoints. |
| **5** | DNS + TLS cert for phantommesh.io (Cloudflare proxy is one click). Polish login page. Add account dashboard. |
| **6** | Rate limiting + abuse protection + monitoring. Deploy to production. |

That's the broker MVP. ~6 days of one focused engineer; sooner if you
let phantom autoevolve do parts of it (run `phantom evolve` against
the broker repo with the OAuth library docs in context).

---

## 6. What the CLI side guarantees today

- The CLI **already** probes `https://phantommesh.io/api/health` with
  a 3-second timeout the instant `phantom login` is run with no args.
  When it returns 200, the CLI hands off completely.
- The CLI listens on `127.0.0.1:48181/oauth/callback` for either GET
  (with `?p=base64(json)`) or POST (with `body: json`). Either shape
  works — pick whichever is easier on the broker side.
- The CLI saves whatever JSON the broker sends back to
  `~/.phantom-mesh/auth.json` mode 0600 with these keys preserved:
  `provider`, `email`, `sub`, `display_name`, `avatar_url`, `id_token`,
  `access_token`. Anything else in the payload is dropped.
- `device_id` is a stable UUID v4 the CLI generates on first login and
  reuses across logout/login. The broker should treat the
  `(user_id, device_id)` pair as the canonical "this Mac of Alice."
- The user can override the broker URL with
  `PHANTOM_AUTH_URL=https://my-self-hosted-broker.example.com`,
  letting Enterprise self-host (per
  [docs/COMMERCIAL-DESIGN.md](COMMERCIAL-DESIGN.md) §2 hard rule #4).
- The CLI never trusts the broker for anything beyond identity —
  agent execution, tools, secrets all stay local. The broker only
  gets to say "this email + device pair is who they claim to be."

---

## 7. Trust + privacy guarantees the broker MUST honor

Sign in trust line: when a user delegates auth to phantommesh.io,
they're trusting us with three things and ONLY three things:

1. **Their email** (so we can identify their account)
2. **Their device IDs** (so they can see which devices are theirs)
3. **A short-lived broker_token** (so we can authenticate `/api/me`
   calls from the CLI)

The broker MUST NOT:

- Store provider passwords in plaintext (use bcrypt for email tier)
- Receive any LLM provider API key — those stay in the user's
  `~/.phantom-mesh/agents.toml` and never come anywhere near phantommesh.io
- Receive the user's prompts, agent outputs, file paths, or commit
  messages
- Retain `id_token` / `access_token` from Google / Apple longer than
  needed to verify the OAuth dance — discard immediately after the
  CLI receives them
- Set the broker_token TTL longer than 7 days
- Telemetry-log anything that names a specific repo / file / user
  agent prompt

The first bullet that gets violated is the bullet that gets us
forked. (See `docs/COMMERCIAL-DESIGN.md` §2.)

---

## 8. Anti-checklist (don't do these)

| ❌ Don't | Because |
|---|---|
| Require login to use `phantom` | OSS-binary contract — works without account forever. |
| Add metering on `/api/me` calls in the free tier | Discoverability of OSS users matters more than rate-limit revenue. |
| Build any "Pro feature" that the broker enforces in-binary | Move it to a separate package; phantom-core stays open. |
| Bundle our cloud broker URL into the binary in a way that can't be turned off | `PHANTOM_AUTH_URL=''` must always work. |
| Use 1st-party analytics (GA / Mixpanel) on phantommesh.io | Plausible / Umami self-hosted only. |

---

## 9. Open questions (decide at start of day 1)

| Q | Lean |
|---|---|
| Hold-out for "Sign in with phantom-mesh GitHub OAuth"? | Add as a 4th provider in week 2. Big audience overlap with our user base. |
| Can users self-merge two emails (e.g. login with Google then later add Apple to same account)? | Yes, day 4. Schema already supports it (provider is per-user, not per-row). |
| Free tier device cap? | None for v1. Cap at 10 devices when we add Pro. |
| Open-source the broker? | Yes — BSL 1.1 → Apache 2.0 after 4 years. Same Tailscale pattern as docs/COMMERCIAL-DESIGN.md §7. |

---

## 10. After phantommesh.io ships

Once `https://phantommesh.io/api/health` returns 200 and the OAuth
dance succeeds, the existing `phantom` binary just works — no rebuild,
no re-flash needed. Users on already-installed phantom v0.1.0 will
get broker login on their next `phantom login` run because the broker
URL was hardcoded as the default at build time.

This is the load-bearing reason the CLI side was wired up first.
