# Apple Sign In — operator setup runbook

The broker code for "Continue with Apple" is complete and tested, but
ships **dark**: the login button, the `/api/health` `"apple"` entry, and
the `/auth/apple/*` routes only activate once all four bindings below are
present. This runbook is the part only an account owner can do — the
Apple Developer portal steps plus the Cloudflare deploy.

Until these are set, `spectyn login apple` still works end-to-end against
any broker that *is* configured (it routes through `/auth/cli/start?provider=apple`).

## What the code already does

| Piece | Where |
|---|---|
| `GET /auth/apple/start` → Apple authorize (`response_mode=form_post`, scope `name email`) | `spectynmesh-io/src/routes/oauth.ts` |
| `POST /auth/apple/callback` → token exchange + loopback/cookie finish | same |
| ES256 client-secret JWT minted from the `.p8` | `spectynmesh-io/src/lib/oauth.ts::appleClientSecret` |
| SameSite=None nonce re-issue (so the cross-site `form_post` keeps the B2 binding) | `appleStart` |
| Button + health advertisement, gated on config | `routes/pages.ts`, `routes/health.ts` |
| `spectyn login apple` → broker with `provider=apple` hint | `core/src/bin/spectyn.rs` |

## Step 1 — Apple Developer portal

1. **App ID / Services ID.** In <https://developer.apple.com/account> →
   Certificates, IDs & Profiles → Identifiers:
   - Create (or reuse) an **App ID** with **Sign in with Apple** capability enabled.
   - Create a **Services ID** (this becomes `APPLE_CLIENT_ID`, e.g.
     `io.spectynmesh.signin`). Enable **Sign in with Apple** on it and click
     **Configure**:
     - **Primary App ID**: the App ID above.
     - **Domains**: `spectynmesh.com`
     - **Return URLs**: `https://spectynmesh.com/auth/apple/callback`
       (must match exactly — Apple is strict; no trailing slash).
2. **Key (.p8).** Keys → **+** → enable **Sign in with Apple** → register.
   Download the `AuthKey_<KEY_ID>.p8` **once** (Apple won't let you
   re-download). Note the 10-char **Key ID** (`APPLE_KEY_ID`).
3. **Team ID.** Top-right of the portal / Membership page — the 10-char
   **Team ID** (`APPLE_TEAM_ID`).

## Step 2 — Cloudflare worker config

In `spectynmesh-io/wrangler.toml` `[vars]` (already stubbed empty):

```toml
APPLE_CLIENT_ID = "io.spectynmesh.signin"   # your Services ID
APPLE_TEAM_ID   = "ABCDE12345"              # 10-char Team ID
APPLE_KEY_ID    = "KEY1234567"              # 10-char Key ID
```

Then the secret (the `.p8` PEM contents — keep the `-----BEGIN/END-----`
lines):

```sh
cd spectynmesh-io
wrangler secret put APPLE_PRIVATE_KEY        # paste the AuthKey_*.p8 body
#   prod:    wrangler secret put APPLE_PRIVATE_KEY
#   staging: wrangler secret put APPLE_PRIVATE_KEY --env staging
```

`appleClientSecret` accepts either real newlines or literal `\n`.

## Step 3 — deploy + verify

```sh
cd spectynmesh-io
npm run test:security          # 18 tests incl. the apple suite — should pass
wrangler deploy                # prod (CI-gated; or --env staging for a dry run)

# verify the flow is now lit:
curl -s https://spectynmesh.com/api/health | jq .providers
#   → [...,"apple"]   (absent until all four bindings are set)
```

Then end-to-end from a machine:

```sh
spectyn login apple            # opens browser → Apple → back to the CLI
```

## Notes / gotchas

- **Hide My Email**: users may arrive with a `…@privaterelay.appleid.com`
  relay address. The broker dedups display-only on email but keeps Apple's
  stable `sub`. Cross-provider / relay-change account merge is a separate
  task (dedup-by-`sub`) — see the project memory queue.
- **Name only on first consent**: Apple sends the display name once, in the
  `user` form field of the first callback, never in later `id_token`s. The
  callback captures it then; if a user revokes and re-consents, name returns.
- **Return URL drift**: if you change `APP_URL` or the worker route, update
  the Services ID Return URL to match or Apple returns `invalid_redirect_uri`.
- The CLI `spectyn login google` keeps its loopback path; only `apple` is
  broker-routed (Apple rejects `http://127.0.0.1` redirects).
