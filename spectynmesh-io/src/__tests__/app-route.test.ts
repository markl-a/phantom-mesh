// Worker-side unit tests for the /app/* handler.
// Runs under `tsx --test` (matches the existing scripts/test-security.ts
// pattern). Pulls in the handler directly with a hand-rolled R2 stub so
// we don't need miniflare for what is fundamentally a routing + header
// assertion. F205 will graduate this to a full miniflare integration
// test once the DO bindings + new endpoints land.

import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { appHandler, __test } from "../routes/app";
import type { Context } from "hono";
import type { Env } from "../types";

type R2Body = string;

function makeR2(objects: Record<string, R2Body>) {
  return {
    async get(key: string) {
      const body = objects[key];
      if (body === undefined) return null;
      return {
        body: new Response(body).body,
        httpEtag: `"etag-${key}"`,
      };
    },
    async head(key: string) {
      const body = objects[key];
      if (body === undefined) return null;
      return { httpEtag: `"etag-${key}"` };
    },
  } as unknown as R2Bucket;
}

function makeCtx(url: string, env: Partial<Env>): Context<{ Bindings: Env }> {
  // Hand-rolled minimal Context — appHandler only touches req.url and
  // env.BINARIES, so we don't need a real Hono instance.
  return {
    req: { url },
    env: env as Env,
  } as unknown as Context<{ Bindings: Env }>;
}

describe("appHandler CSP", () => {
  it("includes the F200 policy verbatim with no unsafe-eval", () => {
    assert.ok(__test.SPA_CSP.includes("script-src 'self'"));
    assert.ok(!__test.SPA_CSP.includes("unsafe-eval"));
    assert.ok(!__test.SPA_CSP.includes("googleapis.com"));
    assert.ok(!__test.SPA_CSP.includes("cdnjs"));
    assert.ok(__test.SPA_CSP.includes("frame-ancestors 'none'"));
  });

  it("style-src allows unsafe-inline for Tailwind atomic classes", () => {
    // Documented exception per F200 spec §Scope — Tailwind needs
    // inline style attributes. Test pins the exception so a future
    // tighten doesn't silently regress the rendering.
    assert.ok(__test.SPA_CSP.includes("style-src 'self' 'unsafe-inline'"));
  });
});

describe("appHandler cache control", () => {
  it("uses immutable long max-age for hashed assets", () => {
    assert.strictEqual(
      __test.cacheControlFor("app/assets/main-abc123.js"),
      "public, max-age=31536000, immutable",
    );
  });

  it("uses short revalidating max-age for index.html", () => {
    assert.strictEqual(
      __test.cacheControlFor("app/index.html"),
      "public, max-age=60, must-revalidate",
    );
  });
});

describe("appHandler routing", () => {
  it("streams index.html for /app", async () => {
    const env = { BINARIES: makeR2({ "app/index.html": "<html>spa</html>" }) };
    const res = await appHandler(makeCtx("https://x/app", env));
    assert.strictEqual(res.status, 200);
    assert.strictEqual(
      res.headers.get("Content-Type"),
      "text/html; charset=utf-8",
    );
    assert.ok(
      res.headers.get("Content-Security-Policy")?.includes("default-src 'self'"),
    );
    assert.strictEqual(await res.text(), "<html>spa</html>");
  });

  it("serves index.html for deep links like /app/cluster", async () => {
    const env = {
      BINARIES: makeR2({ "app/index.html": "<html>deep</html>" }),
    };
    const res = await appHandler(makeCtx("https://x/app/cluster", env));
    assert.strictEqual(res.status, 200);
    assert.strictEqual(await res.text(), "<html>deep</html>");
  });

  it("streams hashed JS asset with the JS MIME type", async () => {
    const env = {
      BINARIES: makeR2({
        "app/assets/main-abc.js": "console.log('ok')",
      }),
    };
    const res = await appHandler(
      makeCtx("https://x/app/assets/main-abc.js", env),
    );
    assert.strictEqual(res.status, 200);
    assert.strictEqual(
      res.headers.get("Content-Type"),
      "application/javascript; charset=utf-8",
    );
    assert.strictEqual(
      res.headers.get("Cache-Control"),
      "public, max-age=31536000, immutable",
    );
  });

  it("returns 404 for an unknown hashed asset (does NOT fall through to index.html)", async () => {
    // Regression guard: a JS request that hits index.html would crash
    // the browser with "Unexpected token <". Must 404 deterministically.
    const env = {
      BINARIES: makeR2({ "app/index.html": "<html/>" }),
    };
    const res = await appHandler(
      makeCtx("https://x/app/assets/missing.js", env),
    );
    assert.strictEqual(res.status, 404);
  });

  it("falls back to inline placeholder when R2 has no index.html yet", async () => {
    const env = { BINARIES: makeR2({}) };
    const res = await appHandler(makeCtx("https://x/app", env));
    assert.strictEqual(res.status, 200);
    const body = await res.text();
    assert.ok(body.includes("SPA bundle not yet uploaded"));
    assert.ok(
      res.headers.get("Content-Security-Policy")?.includes("script-src 'self'"),
    );
  });
});
