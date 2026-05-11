# phantommesh.io broker — Hono + Cloudflare Workers + D1

The web side of `phantom login`. Implements the wire contract from
[../docs/PHANTOMMESH-IO-DESIGN.md](../docs/PHANTOMMESH-IO-DESIGN.md).

```
src/
  index.ts                  Hono router (8 routes)
  types.ts                  Env binding types
  routes/
    health.ts               GET /api/health
    oauth.ts                /auth/cli/start + google + apple callbacks
    email.ts                POST /auth/email/{login,register}
    api.ts                  authenticated /api/me /api/devices
    pages.ts                server-rendered / /login /account
  lib/
    oauth.ts                PKCE + Google/Apple OAuth + broker JWT
    db.ts                   D1 wrappers (users / devices / tokens)
migrations/
  0001_init.sql             D1 schema
wrangler.toml               Cloudflare Worker config
```

---

## One-time setup

```bash
# 1. Tooling
npm install -g wrangler
cd phantommesh-io
npm install                 # installs hono + jose + types

# 2. Cloudflare account
wrangler login

# 3. D1 database
wrangler d1 create phantommesh-prod
# → paste the database_id into wrangler.toml under [[d1_databases]]
wrangler d1 execute phantommesh-prod --file=./migrations/0001_init.sql

# 4. KV namespace for OAuth sessions
wrangler kv:namespace create SESSIONS
# → paste the id into wrangler.toml under [[kv_namespaces]]

# 5. Secrets
wrangler secret put GOOGLE_CLIENT_SECRET    # from Google Cloud Console
wrangler secret put APPLE_KEY_ID            # from Apple Developer / Keys
wrangler secret put APPLE_P8_PRIVATE_KEY    # paste the .p8 contents (incl. -----BEGIN PRIVATE KEY-----)
wrangler secret put BROKER_JWT_SECRET       # any 32+ random bytes (base64 ok)

# 6. Deploy
wrangler deploy

# 7. Bind to phantommesh.io
#    - Cloudflare DNS: A/AAAA records for phantommesh.io → Cloudflare
#    - In wrangler.toml uncomment the [routes] block, then `wrangler deploy` again
```

---

## OAuth provider configuration

### Google Cloud Console

- Create an OAuth 2.0 Client ID, type **Web application**
- Authorized redirect URI: `https://phantommesh.io/auth/google/callback`
- The CLIENT_ID can be the existing `869770808980-…` (already in
  `wrangler.toml [vars]`); the CLIENT_SECRET is a new secret you set
  via `wrangler secret put`.

### Apple Developer Portal

- **Service ID**: `ai.phantommesh.auth` (already in
  `wrangler.toml [vars]` — created during the Tauri integration)
- **Domain + Return URL**: domain = `phantommesh.io`, return URL =
  `https://phantommesh.io/auth/apple/callback`
- **Key**: create a new "Sign in with Apple" key, download the `.p8`
- **Team ID**, **Key ID**, **.p8 contents** → into `wrangler secret put`

---

## Local dev

```bash
wrangler dev
# → http://localhost:8787

# CLI smoke test (point phantom at the dev broker)
PHANTOM_AUTH_URL=http://localhost:8787 phantom login
```

You'll need real Google/Apple OAuth secrets even for local dev, OR
test the email flow which doesn't touch any IdP:

```bash
PHANTOM_AUTH_URL=http://localhost:8787 phantom login   # menu → email
```

---

## Wire contract — must match the CLI's `login_broker`

The phantom CLI (`core/src/bin/phantom.rs`) speaks this protocol; any
deviation breaks every already-installed copy of phantom out there.

| Endpoint | Used by | Contract |
|---|---|---|
| `GET /api/health` | CLI 3-second probe | 200 → online; non-200/timeout → CLI falls back to local provider menu |
| `GET /auth/cli/start?device_id=&port=&redirect=` | CLI redirects browser here | Must validate UUID + port + http-loopback redirect; create KV session; redirect to `/login?state=…` |
| `GET /auth/google/start?state=` | Login page button | Standard PKCE → accounts.google.com |
| `GET /auth/google/callback` | Google IdP redirect | Exchange code → call `redirectToLoopback` |
| `POST /auth/apple/callback` | Apple IdP form_post | Same shape; client_secret JWT regenerated per call |
| Loopback `?p=base64(json)` | Hand identity to CLI | Payload must include `provider`, `email`, `sub?`, `name?`, `picture?`, `id_token`, `access_token`, `broker_token`, `broker_token_expires_at_ms` |
| `GET /api/me` | Authenticated CLI calls | Bearer broker_token → user profile |
| `GET /api/devices` | Authenticated CLI calls | Bearer broker_token → user's device list |

---

## Trust + privacy guarantees

The broker MUST NOT (rule, not a feature flag — see
[../docs/COMMERCIAL-DESIGN.md](../docs/COMMERCIAL-DESIGN.md) §2):

- Store provider passwords in plaintext (we use PBKDF2 SHA-256 100K)
- Receive any LLM provider API key (those stay in `~/.phantom-mesh/agents.toml`)
- Receive prompts / agent outputs / file paths / commit messages
- Retain Google/Apple `id_token`/`access_token` after the loopback
  redirect (they're forwarded to the CLI and discarded server-side
  in v1; future: optional per-account "remember tokens" with separate
  consent)
- Set the broker_token TTL longer than 7 days
- Telemetry-log anything that names a specific repo / file / prompt

---

## Cost projection (Cloudflare free tier)

| Tier | What's free |
|---|---|
| Workers requests | 100,000/day |
| KV reads | 100,000/day |
| KV writes | 1,000/day (login attempts; this is the binding constraint) |
| D1 reads | 5M/day |
| D1 storage | 5 GB total |

Login attempt = ~3 KV writes (start session, update during, delete on
callback). 1k login attempts/day fits comfortably; 10k/day pushes
into the $5/mo paid tier. Real-world steady-state usage will be
dominated by `/api/me` calls (1 KV read per call when we add session
caching), which has 100x more headroom.

---

## Roadmap

- ✅ MVP: `/api/health`, `/auth/cli/start`, Google + Apple OAuth, email
  tier, broker JWTs, D1 migrations, server-rendered login page.
- ⏳ Account dashboard at `/account` (consume `/api/me` + `/api/devices`)
- ⏳ Web mesh discovery — ping a device's Tailscale IP to confirm it's
  reachable; surface in the dashboard
- ⏳ Tauri/iOS/Android in-app browser flow (works today via the existing
  loopback redirect; documenting + screenshots only)
- ⏳ Billing — Pro / Team / Enterprise per
  [docs/COMMERCIAL-DESIGN.md](../docs/COMMERCIAL-DESIGN.md) §3
