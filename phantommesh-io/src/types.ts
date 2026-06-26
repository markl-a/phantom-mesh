// Cloudflare Worker bindings. Mirrors wrangler.toml exactly.

export type Env = {
  // D1 database
  DB: D1Database;
  // KV namespace for OAuth sessions (5-min TTL)
  SESSIONS: KVNamespace;
  // R2 bucket for distributed phantom binaries (served via /dist/*).
  // Objects: phantom-windows-x86_64.exe, phantom-darwin-arm64, etc.
  BINARIES: R2Bucket;

  // Vars
  APP_URL: string;
  GOOGLE_CLIENT_ID: string;
  BROKER_TOKEN_TTL_SECS: string;
  BROKER_VERSION: string;

  // Apple Sign In (optional — the flow is only wired when all four are
  // present, see lib/oauth.appleConfigured). CLIENT_ID is the Services
  // ID; TEAM_ID/KEY_ID are the 10-char Apple Developer identifiers.
  // These three are non-secret config ([vars]); the .p8 below is a secret.
  APPLE_CLIENT_ID: string;
  APPLE_TEAM_ID: string;
  APPLE_KEY_ID: string;

  // Cloudflare Web Analytics beacon token. Optional — when empty
  // string, no beacon is injected (dev/staging defaults). Set in
  // wrangler.toml [vars] for prod after creating the Web Analytics
  // site in the Cloudflare dashboard. Public by design (gets
  // embedded in HTML), so [vars] not [secrets].
  CF_ANALYTICS_TOKEN: string;

  // Secrets (set via `wrangler secret put`)
  GOOGLE_CLIENT_SECRET: string;
  // PKCS8 PEM contents of the Apple AuthKey_<KEY_ID>.p8. Optional; when
  // empty the Apple flow stays dark. Newlines may be real or "\n"-escaped
  // (lib/oauth.appleClientSecret unescapes).
  APPLE_PRIVATE_KEY: string;
  BROKER_JWT_SECRET: string;    // 32+ random bytes

  // 32 bytes (base64-encoded) used as master key for AES-256-GCM
  // encryption of user_settings.env_json. Never logged. Per-user data
  // keys are derived via HKDF(master, salt=user_id, info="env-vault").
  // Rotating this requires re-encrypting every row — defer until
  // there's tooling to do that atomically.
  ENV_VAULT_KEY: string;

  // F205: Durable Object namespace for streaming dispatch chunks.
  // One instance per dispatch job_id; fan-outs SSE events to every
  // subscriber tab. See src/durable/dispatch_stream.ts.
  DISPATCH_STREAM: DurableObjectNamespace;
};

export type UserRow = {
  id: number;
  email: string;
  provider: string;
  sub: string | null;
  display_name: string | null;
  avatar_url: string | null;
  password_hash: string | null;
  created_at: number;
  last_login_at: number;
};

export type DeviceRow = {
  device_id: string;
  user_id: number;
  label: string | null;
  public_addr: string | null;
  claimed_at: number;
  last_seen_at: number;
};

export type CliPayload = {
  provider: string;
  email: string;
  sub: string | null;
  name: string | null;
  picture: string | null;
  id_token: string;
  access_token: string;
  broker_token: string;
  broker_token_expires_at_ms: number;
};

export type OAuthSession = {
  mode: "cli" | "web";
  device_id: string;     // "" when mode === "web"
  redirect: string;      // "" when mode === "web"
  code_verifier: string;
  provider: string;
  created_at: number;
  // HMAC-SHA256(BROKER_JWT_SECRET, nonce_cookie_value) stored at
  // authStart/webStart time and re-verified at every subsequent hop
  // (googleStart, googleCallback, emailLogin, emailRegister). Binds
  // the OAuth dance to the originator's browser even if `state` leaks
  // through Referer / shoulder-surf / Google's logs.
  //
  // Audit B2 (HIGH) — 2026-05-15. Older KV records written before this
  // field existed are treated as "legacy / unbound" (see verifyNonceBinding
  // in lib/oauth.ts). New code paths always set this.
  nonce_hash?: string;
};
