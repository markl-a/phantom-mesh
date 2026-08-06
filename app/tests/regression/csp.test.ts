/**
 * Regression Tests — Tauri CSP (V8 C-1 / F-CRIT-1)
 *
 * Locks in the narrow connect-src allowlist set in
 * app/src-tauri/tauri.conf.json so a careless future edit can't silently
 * re-introduce the "connect-src http: https: ws: wss:" footgun that
 * allows an XSS gadget to exfiltrate broker_token to arbitrary hosts.
 *
 * If you need to add a new destination (e.g. a new third-party API the
 * renderer talks to directly), add it to BOTH:
 *   1. tauri.conf.json's connect-src
 *   2. ALLOWED_CONNECT_SRC_TOKENS below
 * and document the destination in the inline comment in tauri.conf.json.
 */

import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

const TAURI_CONF_PATH = join(__dirname, '..', '..', 'src-tauri', 'tauri.conf.json');

interface TauriConf {
  app?: {
    security?: {
      csp?: string;
    };
  };
}

function loadCsp(): string {
  const raw = readFileSync(TAURI_CONF_PATH, 'utf-8');
  const conf = JSON.parse(raw) as TauriConf;
  const csp = conf?.app?.security?.csp;
  if (!csp) throw new Error('tauri.conf.json has no app.security.csp');
  return csp;
}

function parseDirective(csp: string, name: string): string[] {
  const directives = csp.split(';').map((d) => d.trim()).filter(Boolean);
  for (const d of directives) {
    const parts = d.split(/\s+/);
    if (parts[0] === name) return parts.slice(1);
  }
  return [];
}

describe('Tauri CSP (V8 C-1 / F-CRIT-1)', () => {
  const csp = loadCsp();

  describe('connect-src', () => {
    const sources = parseDirective(csp, 'connect-src');

    it('is present (must not fall back to default-src)', () => {
      expect(sources.length).toBeGreaterThan(0);
    });

    it("does NOT allow bare 'http:' (was the C-1 exfil hole)", () => {
      expect(sources).not.toContain('http:');
    });

    it("does NOT allow bare 'https:' (was the C-1 exfil hole)", () => {
      expect(sources).not.toContain('https:');
    });

    it("does NOT allow bare 'ws:' (was the C-1 exfil hole)", () => {
      expect(sources).not.toContain('ws:');
    });

    it("does NOT allow bare 'wss:' (was the C-1 exfil hole)", () => {
      expect(sources).not.toContain('wss:');
    });

    it("keeps 'self' (required for Tauri IPC + same-origin /api)", () => {
      expect(sources).toContain("'self'");
    });

    it('allows http://localhost:* (local spectyn daemon)', () => {
      expect(sources).toContain('http://localhost:*');
    });

    it('allows http://127.0.0.1:* (local spectyn daemon)', () => {
      expect(sources).toContain('http://127.0.0.1:*');
    });

    it('allows the production broker host (https://phantommesh.io)', () => {
      // Either the apex or a wildcard that covers it is acceptable.
      const broker = sources.some(
        (s) => s === 'https://phantommesh.io' || s === 'https://*.phantommesh.io',
      );
      expect(broker).toBe(true);
    });

    // Any addition beyond this set requires updating the test + the
    // comment in tauri.conf.json. This is the lock.
    const ALLOWED_CONNECT_SRC_TOKENS = new Set([
      "'self'",
      'ipc:',
      'http://ipc.localhost',
      'http://localhost:*',
      'http://127.0.0.1:*',
      'http://*:7878',
      'https://api.telegram.org',
      'https://phantommesh.io',
      'https://*.phantommesh.io',
      'https://*.ts.net',
    ]);

    it('contains only known/allowlisted sources', () => {
      const unexpected = sources.filter((s) => !ALLOWED_CONNECT_SRC_TOKENS.has(s));
      expect(unexpected).toEqual([]);
    });
  });

  describe('other directives (must NOT be widened by this fix)', () => {
    it("script-src is still 'self' only — no 'unsafe-inline' or 'unsafe-eval'", () => {
      const sources = parseDirective(csp, 'script-src');
      expect(sources).toContain("'self'");
      expect(sources).not.toContain("'unsafe-inline'");
      expect(sources).not.toContain("'unsafe-eval'");
    });

    it('style-src keeps its existing shape (self + unsafe-inline for Tailwind)', () => {
      const sources = parseDirective(csp, 'style-src');
      expect(sources).toContain("'self'");
      // 'unsafe-inline' was already present pre-fix; the audit notes it as
      // a separate hardening item, not blocked by this PR.
      expect(sources).toContain("'unsafe-inline'");
    });

    it('img-src keeps data: + https: (markdown rendering needs both)', () => {
      const sources = parseDirective(csp, 'img-src');
      expect(sources).toContain("'self'");
      expect(sources).toContain('data:');
      expect(sources).toContain('https:');
    });
  });
});
