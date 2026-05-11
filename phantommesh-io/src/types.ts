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

  // Secrets (set via `wrangler secret put`)
  GOOGLE_CLIENT_SECRET: string;
  BROKER_JWT_SECRET: string;    // 32+ random bytes

  // 32 bytes (base64-encoded) used as master key for AES-256-GCM
  // encryption of user_settings.env_json. Never logged. Per-user data
  // keys are derived via HKDF(master, salt=user_id, info="env-vault").
  // Rotating this requires re-encrypting every row — defer until
  // there's tooling to do that atomically.
  ENV_VAULT_KEY: string;
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
};
