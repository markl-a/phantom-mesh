// F205 — unit tests for the new dashboard control-plane endpoints.
//
// Same `node:test` pattern as test-security.ts so we don't pull in vitest.
// Each test stands up an in-memory D1 stub (just enough surface to back
// the queries db.ts issues) + a minimal DurableObjectNamespace stub that
// records the calls without instantiating the real DispatchStream class.
//
// Run: tsx --test scripts/test-f205.ts

import { test } from "node:test";
import assert from "node:assert/strict";
import app from "../src/index";
import type { Env } from "../src/types";
import { mintBrokerJwt } from "../src/lib/oauth";

/* ── In-memory D1 stub ─────────────────────────────────────────────────
   Bare-minimum interpreter for the SQL the F205 code path issues. We
   keep four tables in plain JS Maps and pattern-match on the statement
   text. This is intentionally not a generic SQL engine — adding a new
   query in db.ts means teaching this stub the shape. */

type Row = Record<string, unknown>;

interface MemDB {
  users: Row[];
  user_cluster_peers: Row[];
  dispatches: Row[];
  dispatch_recipes: Row[];
  user_preferences: Row[];
  broker_tokens: Row[];
}

function makeMemDb(): MemDB {
  return {
    users: [],
    user_cluster_peers: [],
    dispatches: [],
    dispatch_recipes: [],
    user_preferences: [],
    broker_tokens: [],
  };
}

function compileLike(pat: string): RegExp {
  const esc = pat.replace(/[.*+?^${}()|[\]\\]/g, "\\$&").replace(/%/g, ".*").replace(/_/g, ".");
  return new RegExp(`^${esc}$`);
}

function makeD1(mem: MemDB): D1Database {
  // Each .prepare() returns an object whose .bind() returns a thing with
  // .first / .all / .run, mirroring the real D1 surface.
  function prepare(sql: string): D1PreparedStatement {
    return {
      bind(...args: unknown[]) {
        return statementWithArgs(sql.trim(), args, mem);
      },
      first<T>() { return statementWithArgs(sql.trim(), [], mem).first<T>(); },
      all<T>()   { return statementWithArgs(sql.trim(), [], mem).all<T>(); },
      run()      { return statementWithArgs(sql.trim(), [], mem).run(); },
      raw()      { return Promise.resolve([] as unknown as never); },
    } as unknown as D1PreparedStatement;
  }
  return {
    prepare,
    batch: async <T>(stmts: D1PreparedStatement[]) => {
      const out: T[] = [];
      for (const s of stmts) {
        await (s as unknown as { run: () => Promise<T> }).run();
        out.push(undefined as unknown as T);
      }
      return out;
    },
    dump:  async () => new ArrayBuffer(0),
    exec:  async () => ({ count: 0, duration: 0 } as unknown as D1ExecResult),
  } as unknown as D1Database;
}

function statementWithArgs(sql: string, args: unknown[], mem: MemDB) {
  return {
    async first<T>(): Promise<T | null> {
      const rows = exec(sql, args, mem);
      return (rows[0] ?? null) as T | null;
    },
    async all<T>(): Promise<{ results: T[]; meta: Record<string, unknown> }> {
      const rows = exec(sql, args, mem);
      return { results: rows as T[], meta: {} };
    },
    async run() {
      const changes = execMutating(sql, args, mem);
      return { success: true, meta: { changes }, results: [] } as unknown as D1Result;
    },
  };
}

/// Pure-read interpreter. Handles SELECT shapes used by db.ts.
function exec(sql: string, args: unknown[], mem: MemDB): Row[] {
  // SELECT * FROM users WHERE id = ?
  let m = sql.match(/^SELECT \* FROM users WHERE id = \?$/);
  if (m) return mem.users.filter(u => u.id === args[0]);
  m = sql.match(/^SELECT \* FROM users WHERE email = \?$/);
  if (m) return mem.users.filter(u => u.email === args[0]);

  // getUserClusterPeers
  if (sql.startsWith("SELECT name, url, label, capabilities, updated_at FROM user_cluster_peers")) {
    return mem.user_cluster_peers
      .filter(p => p.user_id === args[0])
      .sort((a, b) => String(a.name).localeCompare(String(b.name)));
  }

  // getDispatch (SELECT * FROM dispatches WHERE user_id = ? AND job_id = ?)
  if (sql.startsWith("SELECT * FROM dispatches WHERE user_id = ? AND job_id = ?")) {
    return mem.dispatches.filter(d => d.user_id === args[0] && d.job_id === args[1]);
  }

  // listDispatches paginated select — recognise the common skeleton
  if (sql.startsWith("SELECT * FROM dispatches WHERE") && sql.includes("ORDER BY started_at DESC")) {
    return filterDispatches(sql, args, mem)
      .sort((a, b) => Number(b.started_at) - Number(a.started_at))
      .slice(Number(args[args.length - 1] ?? 0), Number(args[args.length - 1] ?? 0) + Number(args[args.length - 2] ?? 50));
  }

  // FTS variant — skip FTS, just substring match prompt OR result.
  if (sql.includes("FROM dispatches d") && sql.includes("dispatches_fts MATCH ?")) {
    const q = String(args[args.length - 3] ?? "").toLowerCase();
    const lim = Number(args[args.length - 2] ?? 50);
    const offs = Number(args[args.length - 1] ?? 0);
    return filterDispatches(sql, args.slice(0, args.length - 3), mem)
      .filter(d => String(d.prompt).toLowerCase().includes(q) || String(d.result).toLowerCase().includes(q))
      .sort((a, b) => Number(b.started_at) - Number(a.started_at))
      .slice(offs, offs + lim);
  }

  // COUNT(*) FROM (...)
  if (sql.startsWith("SELECT COUNT(*) AS n FROM")) {
    // Approximate: count whatever filter the inner WHERE bind args used.
    const matching = filterDispatches("SELECT * FROM dispatches WHERE user_id = ?", args, mem);
    return [{ n: matching.length }];
  }

  // recipes
  if (sql.startsWith("SELECT * FROM dispatch_recipes WHERE user_id = ? ORDER BY")) {
    return mem.dispatch_recipes
      .filter(r => r.user_id === args[0])
      .sort((a, b) => Number(b.updated_at) - Number(a.updated_at));
  }
  if (sql.startsWith("SELECT * FROM dispatch_recipes WHERE user_id = ? AND id = ?")) {
    return mem.dispatch_recipes.filter(r => r.user_id === args[0] && r.id === args[1]);
  }

  // preferences
  if (sql.startsWith("SELECT heartbeat_secs, retention_days, updated_at FROM user_preferences")) {
    return mem.user_preferences.filter(p => p.user_id === args[0]);
  }

  // broker_tokens (revocation check)
  if (sql.startsWith("SELECT revoked_at, expires_at FROM broker_tokens")) {
    return mem.broker_tokens.filter(t => t.token_hash === args[0])
      .map(t => ({ revoked_at: t.revoked_at ?? null, expires_at: t.expires_at }));
  }

  // sessions for the aggregate read in /sessions/all-others context (none).
  // dispatches aggregate for caps
  if (sql.startsWith("SELECT required_caps, status, started_at FROM dispatches")) {
    return mem.dispatches
      .filter(d => d.user_id === args[0] && d.peer === args[1])
      .sort((a, b) => Number(b.started_at) - Number(a.started_at))
      .slice(0, 1000);
  }

  // Default — unrecognized read returns empty. Log to stderr so we notice
  // missing stub coverage when adding tests.
  if (sql.startsWith("SELECT")) {
    console.warn("[memdb] unhandled SELECT:", sql.slice(0, 80));
  }
  return [];
}

function filterDispatches(sql: string, args: unknown[], mem: MemDB): Row[] {
  // Bind order matches db.ts: user_id, then any of peer, status, from, to, cap LIKE pattern.
  let idx = 0;
  const userId = args[idx++];
  let rows = mem.dispatches.filter(d => d.user_id === userId);
  if (sql.includes("peer = ?")) {
    const peer = args[idx++];
    rows = rows.filter(d => d.peer === peer);
  }
  if (sql.includes("status = ?")) {
    const status = args[idx++];
    rows = rows.filter(d => d.status === status);
  }
  if (sql.includes("started_at >= ?")) {
    const from = Number(args[idx++]);
    rows = rows.filter(d => Number(d.started_at) >= from);
  }
  if (sql.includes("started_at <= ?")) {
    const to = Number(args[idx++]);
    rows = rows.filter(d => Number(d.started_at) <= to);
  }
  if (sql.includes("required_caps LIKE ?")) {
    const pat = compileLike(String(args[idx++]));
    rows = rows.filter(d => pat.test(String(d.required_caps)));
  }
  return rows;
}

/// Mutating interpreter — INSERT/UPDATE/DELETE shapes used by db.ts.
function execMutating(sql: string, args: unknown[], mem: MemDB): number {
  // ── dispatches ──
  if (sql.startsWith("INSERT INTO dispatches")) {
    const [job_id, user_id, peer, provider, model, prompt, required_caps, started_at] = args;
    mem.dispatches.push({
      job_id, user_id, peer, provider, model, prompt, required_caps,
      status: "pending", result: "", error_message: null,
      started_at, completed_at: null,
    });
    return 1;
  }
  if (sql.startsWith("UPDATE dispatches")) {
    // Two variants — with result_append, without.
    if (sql.includes("result = substr(result || ?, 1, 524288)")) {
      const [status, error_message, completed, append, user_id, job_id] = args;
      const row = mem.dispatches.find(d => d.user_id === user_id && d.job_id === job_id);
      if (!row) return 0;
      row.status = status; row.error_message = error_message;
      if (completed !== null) row.completed_at = completed;
      row.result = String(row.result ?? "") + String(append ?? "");
      return 1;
    }
    const [status, error_message, completed, user_id, job_id] = args;
    const row = mem.dispatches.find(d => d.user_id === user_id && d.job_id === job_id);
    if (!row) return 0;
    row.status = status; row.error_message = error_message;
    if (completed !== null) row.completed_at = completed;
    return 1;
  }
  if (sql.startsWith("DELETE FROM dispatches WHERE started_at <")) {
    const before = mem.dispatches.length;
    const cutoff = Number(args[0]);
    mem.dispatches = mem.dispatches.filter(d => Number(d.started_at) >= cutoff);
    return before - mem.dispatches.length;
  }

  // ── recipes ──
  if (sql.startsWith("INSERT INTO dispatch_recipes")) {
    const [id, user_id, name, peer, provider, model, prompt, required_caps, created_at, updated_at] = args;
    const idx = mem.dispatch_recipes.findIndex(r => r.user_id === user_id && r.id === id);
    if (idx >= 0) {
      mem.dispatch_recipes[idx] = {
        ...mem.dispatch_recipes[idx], name, peer, provider, model, prompt, required_caps, updated_at,
      };
    } else {
      mem.dispatch_recipes.push({ id, user_id, name, peer, provider, model, prompt, required_caps, created_at, updated_at });
    }
    return 1;
  }
  if (sql.startsWith("DELETE FROM dispatch_recipes")) {
    const [user_id, id] = args;
    const before = mem.dispatch_recipes.length;
    mem.dispatch_recipes = mem.dispatch_recipes.filter(r => !(r.user_id === user_id && r.id === id));
    return before - mem.dispatch_recipes.length;
  }

  // ── preferences ──
  if (sql.startsWith("INSERT INTO user_preferences")) {
    const [user_id, heartbeat_secs, retention_days, updated_at] = args;
    const idx = mem.user_preferences.findIndex(p => p.user_id === user_id);
    if (idx >= 0) {
      mem.user_preferences[idx] = { user_id, heartbeat_secs, retention_days, updated_at };
    } else {
      mem.user_preferences.push({ user_id, heartbeat_secs, retention_days, updated_at });
    }
    return 1;
  }

  // ── cluster peers (caps update only — used by F205) ──
  if (sql.startsWith("UPDATE user_cluster_peers SET capabilities")) {
    const [capabilities, updated_at, user_id, name] = args;
    const row = mem.user_cluster_peers.find(p => p.user_id === user_id && p.name === name);
    if (!row) return 0;
    row.capabilities = capabilities;
    row.updated_at = updated_at;
    return 1;
  }

  // ── broker_tokens ──
  if (sql.startsWith("UPDATE broker_tokens SET revoked_at")) {
    const [revoked_at, user_id, keep_hash] = args;
    let n = 0;
    for (const t of mem.broker_tokens) {
      if (t.user_id === user_id && t.token_hash !== keep_hash && t.revoked_at == null) {
        t.revoked_at = revoked_at; n++;
      }
    }
    return n;
  }

  // Default — unrecognised statement, surface in test logs but don't
  // explode (some setup-only statements aren't worth stubbing).
  if (!sql.startsWith("CREATE") && !sql.startsWith("PRAGMA")) {
    console.warn("[memdb] unhandled mutating:", sql.slice(0, 80));
  }
  return 0;
}

/* ── In-memory KV stub (copied from test-security.ts shape) ───────────── */

function makeKv(): KVNamespace {
  const store = new Map<string, { value: string; expires: number }>();
  return {
    async get(key: string) { const e = store.get(key); if (!e) return null;
      if (e.expires && e.expires < Date.now()) { store.delete(key); return null; } return e.value; },
    async put(key: string, value: string, opts?: { expirationTtl?: number }) {
      const expires = opts?.expirationTtl ? Date.now() + opts.expirationTtl * 1000 : 0;
      store.set(key, { value, expires });
    },
    async delete(key: string) { store.delete(key); },
    async list() { return { keys: [], list_complete: true, cursor: "" }; },
    async getWithMetadata() { return { value: null, metadata: null }; },
  } as unknown as KVNamespace;
}

/* ── DurableObjectNamespace stub ──────────────────────────────────────
   Captures every fetch call so tests can assert the worker forwarded
   correctly. Returns a synthetic Response. */

interface DOCall { id: string; url: string; method: string; bodyText: string; ownerHeader: string }

function makeDoNs(captured: DOCall[]): DurableObjectNamespace {
  return {
    idFromName(name: string) { return { name, toString: () => name } as unknown as DurableObjectId; },
    idFromString(s: string) { return { name: s, toString: () => s } as unknown as DurableObjectId; },
    newUniqueId() { return { name: "uniq", toString: () => "uniq" } as unknown as DurableObjectId; },
    get(id: DurableObjectId): DurableObjectStub {
      const idStr = String((id as unknown as { name: string }).name ?? id);
      return {
        async fetch(input: RequestInfo, init?: RequestInit): Promise<Response> {
          const url = typeof input === "string" ? input : (input as Request).url;
          const method = (init?.method ?? "GET").toUpperCase();
          const owner = String((init?.headers as Record<string,string> | undefined)?.["X-Owner"] ?? "");
          let bodyText = "";
          if (init?.body) bodyText = typeof init.body === "string" ? init.body : String(init.body);
          captured.push({ id: idStr, url, method, bodyText, ownerHeader: owner });
          // Stub responses by route
          if (url.includes("/subscribe")) {
            return new Response("data: hello\n\n", {
              status: 200,
              headers: { "Content-Type": "text/event-stream" },
            });
          }
          if (url.includes("/publish")) {
            return new Response(JSON.stringify({ seq: captured.length }), {
              status: 200, headers: { "Content-Type": "application/json" },
            });
          }
          return new Response(JSON.stringify({ ok: true }), {
            status: 200, headers: { "Content-Type": "application/json" },
          });
        },
        id,
        name: idStr,
      } as unknown as DurableObjectStub;
    },
    jurisdiction() { return this; },
  } as unknown as DurableObjectNamespace;
}

/* ── Env factory ─────────────────────────────────────────────────────── */

const JWT_SECRET = "test-jwt-secret-must-be-at-least-32-bytes-long-for-hs256";

interface Ctx {
  env: Env;
  mem: MemDB;
  doCalls: DOCall[];
  user1Token: string;
  user2Token: string;
}

async function setup(): Promise<Ctx> {
  const mem = makeMemDb();
  // Seed two users so cross-tenant tests have a foil.
  const now = Date.now();
  mem.users.push({
    id: 1, email: "alice@test", provider: "google", sub: "alice-sub",
    display_name: "Alice", avatar_url: null, password_hash: null,
    created_at: now, last_login_at: now,
  });
  mem.users.push({
    id: 2, email: "bob@test", provider: "google", sub: "bob-sub",
    display_name: "Bob", avatar_url: null, password_hash: null,
    created_at: now, last_login_at: now,
  });
  // Each user gets one peer for the F201 cap-aggregate test.
  mem.user_cluster_peers.push({
    user_id: 1, name: "node-a", url: "http://100.0.0.1:7878", label: null,
    capabilities: JSON.stringify(["gpu", "rust"]), updated_at: now,
  });
  mem.user_cluster_peers.push({
    user_id: 2, name: "macmini", url: "http://100.0.0.2:7878", label: null,
    capabilities: JSON.stringify(["camera"]), updated_at: now,
  });

  // Mint broker tokens for each + record them so tokenIsRevoked returns false.
  const t1 = await mintBrokerJwt({ secret: JWT_SECRET, userId: 1, deviceId: "dev-1", ttlSecs: 3600 });
  const t2 = await mintBrokerJwt({ secret: JWT_SECRET, userId: 2, deviceId: "dev-2", ttlSecs: 3600 });
  mem.broker_tokens.push({ token_hash: await sha256Hex(t1.token), user_id: 1, device_id: "dev-1", issued_at: now, expires_at: now + 3_600_000, revoked_at: null });
  mem.broker_tokens.push({ token_hash: await sha256Hex(t2.token), user_id: 2, device_id: "dev-2", issued_at: now, expires_at: now + 3_600_000, revoked_at: null });

  const doCalls: DOCall[] = [];
  const env: Env = {
    DB: makeD1(mem),
    SESSIONS: makeKv(),
    BINARIES: {} as R2Bucket,
    APP_URL: "https://phantommesh.io",
    GOOGLE_CLIENT_ID: "test",
    BROKER_TOKEN_TTL_SECS: "3600",
    BROKER_VERSION: "test",
    CF_ANALYTICS_TOKEN: "",
    GOOGLE_CLIENT_SECRET: "test",
    BROKER_JWT_SECRET: JWT_SECRET,
    ENV_VAULT_KEY: Buffer.alloc(32, 1).toString("base64"),
    DISPATCH_STREAM: makeDoNs(doCalls),
  };
  return { env, mem, doCalls, user1Token: t1.token, user2Token: t2.token };
}

async function sha256Hex(s: string): Promise<string> {
  const data = new TextEncoder().encode(s);
  const hash = await crypto.subtle.digest("SHA-256", data);
  return [...new Uint8Array(hash)].map(b => b.toString(16).padStart(2, "0")).join("");
}

function authHdr(token: string): Record<string, string> {
  return { Authorization: `Bearer ${token}`, "Content-Type": "application/json" };
}

/* ─────────────────────────────────────────────────────────────────────── */
/* Tests                                                                    */
/* ─────────────────────────────────────────────────────────────────────── */

test("[F205] POST /api/me/dispatch/start creates a job and returns job_id", async () => {
  const ctx = await setup();
  const res = await app.request("https://phantommesh.io/api/me/dispatch/start", {
    method: "POST",
    headers: authHdr(ctx.user1Token),
    body: JSON.stringify({ peer: "node-a", provider: "groq", model: "llama-3.3", prompt: "hi", required_caps: ["gpu"] }),
  }, ctx.env);
  assert.equal(res.status, 200);
  const j = await res.json() as { job_id: string; started_at: number };
  assert.ok(j.job_id && j.job_id.length > 8, "job_id should be a non-empty uuid");
  assert.equal(ctx.mem.dispatches.length, 1);
  assert.equal(ctx.mem.dispatches[0].user_id, 1);
  assert.equal(ctx.mem.dispatches[0].peer, "node-a");
});

test("[F205] POST /api/me/dispatch/start without auth returns 401", async () => {
  const ctx = await setup();
  const res = await app.request("https://phantommesh.io/api/me/dispatch/start", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ peer: "node-a" }),
  }, ctx.env);
  assert.equal(res.status, 401);
});

test("[F205] POST /api/me/dispatch/start with missing peer returns 400", async () => {
  const ctx = await setup();
  const res = await app.request("https://phantommesh.io/api/me/dispatch/start", {
    method: "POST", headers: authHdr(ctx.user1Token),
    body: JSON.stringify({ provider: "groq", model: "llama-3.3", prompt: "hi" }),
  }, ctx.env);
  assert.equal(res.status, 400);
});

test("[F205] POST /api/me/dispatch/stream/:job_id forwards to DispatchStream DO", async () => {
  const ctx = await setup();
  // Seed a job first
  const start = await app.request("https://phantommesh.io/api/me/dispatch/start", {
    method: "POST", headers: authHdr(ctx.user1Token),
    body: JSON.stringify({ peer: "node-a", prompt: "x" }),
  }, ctx.env);
  const { job_id } = await start.json() as { job_id: string };

  const pub = await app.request(`https://phantommesh.io/api/me/dispatch/stream/${job_id}`, {
    method: "POST", headers: authHdr(ctx.user1Token),
    body: JSON.stringify({ kind: "chunk", data: "hello " }),
  }, ctx.env);
  assert.equal(pub.status, 200);
  // DO must have received exactly one publish call with X-Owner=1
  const publishCalls = ctx.doCalls.filter(c => c.url.includes("/publish"));
  assert.equal(publishCalls.length, 1);
  assert.equal(publishCalls[0].ownerHeader, "1");
  // D1 row should have been appended with the chunk text
  const row = ctx.mem.dispatches.find(d => d.job_id === job_id);
  assert.equal(row?.status, "running");
  assert.equal(row?.result, "hello ");
});

test("[F205] GET /api/me/dispatch/stream/:job_id returns text/event-stream", async () => {
  const ctx = await setup();
  const start = await app.request("https://phantommesh.io/api/me/dispatch/start", {
    method: "POST", headers: authHdr(ctx.user1Token),
    body: JSON.stringify({ peer: "node-a", prompt: "x" }),
  }, ctx.env);
  const { job_id } = await start.json() as { job_id: string };

  const sub = await app.request(`https://phantommesh.io/api/me/dispatch/stream/${job_id}`, {
    method: "GET", headers: { Authorization: `Bearer ${ctx.user1Token}` },
  }, ctx.env);
  assert.equal(sub.status, 200);
  assert.equal(sub.headers.get("Content-Type"), "text/event-stream");
});

test("[F205] POST /api/me/dispatch/:job_id/cancel marks row cancelled + calls DO", async () => {
  const ctx = await setup();
  const start = await app.request("https://phantommesh.io/api/me/dispatch/start", {
    method: "POST", headers: authHdr(ctx.user1Token),
    body: JSON.stringify({ peer: "node-a", prompt: "x" }),
  }, ctx.env);
  const { job_id } = await start.json() as { job_id: string };

  const can = await app.request(`https://phantommesh.io/api/me/dispatch/${job_id}/cancel`, {
    method: "POST", headers: authHdr(ctx.user1Token),
  }, ctx.env);
  assert.equal(can.status, 200);
  const row = ctx.mem.dispatches.find(d => d.job_id === job_id);
  assert.equal(row?.status, "cancelled");
  const cancelCalls = ctx.doCalls.filter(c => c.url.includes("/cancel"));
  assert.equal(cancelCalls.length, 1);
});

test("[F205] GET /api/me/dispatches lists user's history only", async () => {
  const ctx = await setup();
  // Seed one dispatch for each user
  ctx.mem.dispatches.push({
    job_id: "j1", user_id: 1, peer: "node-a", provider: "groq", model: "m", prompt: "alice prompt",
    required_caps: "[]", status: "done", result: "", error_message: null, started_at: Date.now(), completed_at: Date.now(),
  });
  ctx.mem.dispatches.push({
    job_id: "j2", user_id: 2, peer: "macmini", provider: "groq", model: "m", prompt: "bob prompt",
    required_caps: "[]", status: "done", result: "", error_message: null, started_at: Date.now(), completed_at: Date.now(),
  });
  const res = await app.request("https://phantommesh.io/api/me/dispatches", {
    method: "GET", headers: { Authorization: `Bearer ${ctx.user1Token}` },
  }, ctx.env);
  assert.equal(res.status, 200);
  const j = await res.json() as { rows: Array<{ job_id: string; prompt: string }> };
  assert.equal(j.rows.length, 1);
  assert.equal(j.rows[0].job_id, "j1");
  assert.equal(j.rows[0].prompt, "alice prompt");
});

test("[F205] GET /api/me/dispatches/:job_id returns 404 across users (tenant isolation)", async () => {
  const ctx = await setup();
  ctx.mem.dispatches.push({
    job_id: "alice-job", user_id: 1, peer: "node-a", provider: "", model: "", prompt: "secret",
    required_caps: "[]", status: "done", result: "secret-data", error_message: null,
    started_at: Date.now(), completed_at: Date.now(),
  });
  // Bob asks for Alice's job by id — must 404.
  const res = await app.request("https://phantommesh.io/api/me/dispatches/alice-job", {
    method: "GET", headers: { Authorization: `Bearer ${ctx.user2Token}` },
  }, ctx.env);
  assert.equal(res.status, 404, "cross-tenant fetch must 404");
});

test("[F205] GET /api/me/recipes lists and POST /recipes creates", async () => {
  const ctx = await setup();
  const empty = await app.request("https://phantommesh.io/api/me/recipes", {
    method: "GET", headers: { Authorization: `Bearer ${ctx.user1Token}` },
  }, ctx.env);
  const { recipes } = await empty.json() as { recipes: unknown[] };
  assert.equal(recipes.length, 0);

  const create = await app.request("https://phantommesh.io/api/me/recipes", {
    method: "POST", headers: authHdr(ctx.user1Token),
    body: JSON.stringify({ name: "weekly digest", peer: "node-a", provider: "groq", prompt: "summarise commits" }),
  }, ctx.env);
  assert.equal(create.status, 200);
  const { recipe } = await create.json() as { recipe: { id: string; name: string } };
  assert.equal(recipe.name, "weekly digest");

  // POST without name → 400
  const bad = await app.request("https://phantommesh.io/api/me/recipes", {
    method: "POST", headers: authHdr(ctx.user1Token),
    body: JSON.stringify({ prompt: "x" }),
  }, ctx.env);
  assert.equal(bad.status, 400);
});

test("[F205] DELETE /api/me/recipes/:id 404s when recipe belongs to another user", async () => {
  const ctx = await setup();
  // Alice creates
  const create = await app.request("https://phantommesh.io/api/me/recipes", {
    method: "POST", headers: authHdr(ctx.user1Token),
    body: JSON.stringify({ name: "alice-only" }),
  }, ctx.env);
  const { recipe } = await create.json() as { recipe: { id: string } };

  // Bob tries to delete
  const del = await app.request(`https://phantommesh.io/api/me/recipes/${recipe.id}`, {
    method: "DELETE", headers: { Authorization: `Bearer ${ctx.user2Token}` },
  }, ctx.env);
  assert.equal(del.status, 404);
});

test("[F205] GET/PUT /api/me/preferences returns defaults and clamps input", async () => {
  const ctx = await setup();
  const get = await app.request("https://phantommesh.io/api/me/preferences", {
    method: "GET", headers: { Authorization: `Bearer ${ctx.user1Token}` },
  }, ctx.env);
  assert.equal(get.status, 200);
  const j = await get.json() as { preferences: { heartbeat_secs: number; retention_days: number } };
  assert.equal(j.preferences.heartbeat_secs, 30);
  assert.equal(j.preferences.retention_days, 90);

  // PUT with out-of-range values: should be clamped (1 → 10, 9999 → 365)
  const put = await app.request("https://phantommesh.io/api/me/preferences", {
    method: "PUT", headers: authHdr(ctx.user1Token),
    body: JSON.stringify({ heartbeat_secs: 1, retention_days: 9999 }),
  }, ctx.env);
  const after = await put.json() as { preferences: { heartbeat_secs: number; retention_days: number } };
  assert.equal(after.preferences.heartbeat_secs, 10);
  assert.equal(after.preferences.retention_days, 365);
});

test("[F205] PUT /api/me/peer-capabilities 404s for unknown peer", async () => {
  const ctx = await setup();
  const res = await app.request("https://phantommesh.io/api/me/peer-capabilities", {
    method: "PUT", headers: authHdr(ctx.user1Token),
    body: JSON.stringify({ peer: "nonexistent", capabilities: ["gpu"] }),
  }, ctx.env);
  assert.equal(res.status, 404);
});

test("[F205] PUT /api/me/peer-capabilities updates caps for known peer", async () => {
  const ctx = await setup();
  const res = await app.request("https://phantommesh.io/api/me/peer-capabilities", {
    method: "PUT", headers: authHdr(ctx.user1Token),
    body: JSON.stringify({ peer: "node-a", capabilities: ["gpu", "camera"] }),
  }, ctx.env);
  assert.equal(res.status, 200);
  const row = ctx.mem.user_cluster_peers.find(p => p.user_id === 1 && p.name === "node-a");
  assert.equal(row?.capabilities, JSON.stringify(["gpu", "camera"]));
});

test("[F205] GET /api/me/cluster-peers/:peer/caps aggregates running counts", async () => {
  const ctx = await setup();
  const now = Date.now();
  // 2 running gpu dispatches + 1 done rust dispatch.
  ctx.mem.dispatches.push({ job_id: "a", user_id: 1, peer: "node-a", provider: "", model: "", prompt: "",
    required_caps: JSON.stringify(["gpu"]), status: "running", result: "", error_message: null, started_at: now, completed_at: null });
  ctx.mem.dispatches.push({ job_id: "b", user_id: 1, peer: "node-a", provider: "", model: "", prompt: "",
    required_caps: JSON.stringify(["gpu"]), status: "running", result: "", error_message: null, started_at: now, completed_at: null });
  ctx.mem.dispatches.push({ job_id: "c", user_id: 1, peer: "node-a", provider: "", model: "", prompt: "",
    required_caps: JSON.stringify(["rust"]), status: "done", result: "", error_message: null, started_at: now, completed_at: now });

  const res = await app.request("https://phantommesh.io/api/me/cluster-peers/node-a/caps", {
    method: "GET", headers: { Authorization: `Bearer ${ctx.user1Token}` },
  }, ctx.env);
  assert.equal(res.status, 200);
  const j = await res.json() as { caps: Array<{ cap: string; running_count: number; last_run_at: number | null }> };
  // Sorted: gpu, rust
  assert.equal(j.caps.length, 2);
  const gpu = j.caps.find(c => c.cap === "gpu");
  const rust = j.caps.find(c => c.cap === "rust");
  assert.equal(gpu?.running_count, 2);
  assert.equal(rust?.running_count, 0);
});

test("[F205] DELETE /api/me/sessions/all-others revokes other tokens but keeps current", async () => {
  const ctx = await setup();
  // Add a second token for Alice
  const now = Date.now();
  const altHash = await sha256Hex("alice-other-token");
  ctx.mem.broker_tokens.push({ token_hash: altHash, user_id: 1, device_id: "dev-1b", issued_at: now, expires_at: now + 3_600_000, revoked_at: null });

  const res = await app.request("https://phantommesh.io/api/me/sessions/all-others", {
    method: "DELETE", headers: { Authorization: `Bearer ${ctx.user1Token}` },
  }, ctx.env);
  assert.equal(res.status, 200);
  const j = await res.json() as { revoked: number };
  assert.equal(j.revoked, 1);
  // Current token still un-revoked
  const currentHash = await sha256Hex(ctx.user1Token);
  const current = ctx.mem.broker_tokens.find(t => t.token_hash === currentHash);
  assert.equal(current?.revoked_at, null);
  // Other token now revoked
  const other = ctx.mem.broker_tokens.find(t => t.token_hash === altHash);
  assert.notEqual(other?.revoked_at, null);
});

test("[F205] GET /api/me/dispatches without auth returns 401", async () => {
  const ctx = await setup();
  const res = await app.request("https://phantommesh.io/api/me/dispatches", {
    method: "GET",
  }, ctx.env);
  assert.equal(res.status, 401);
});
