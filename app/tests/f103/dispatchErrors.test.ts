// F103 · Unit tests — friendly dispatch-error mapping.
//
// dispatch.rs throws stable `E_DISPATCH_*` codes; the mobile UI must turn
// them into actionable Traditional-Chinese copy instead of dumping a raw
// code at the user (E2E "every scenario is operable" requirement). These
// tests pin the leading-token match, the colon-detail handling, and the
// unknown-code passthrough so a future code rename surfaces here.

import { describe, expect, it } from 'vitest';
import { friendlyDispatchError } from '../../src/lib/dispatchErrors';

describe('friendlyDispatchError', () => {
  it('maps a bare auth-required code to a titled hint', () => {
    const f = friendlyDispatchError('E_DISPATCH_AUTH_REQUIRED');
    expect(f.title).toContain('登入');
    expect(f.hint).toBeTruthy();
    expect(f.raw).toBe('E_DISPATCH_AUTH_REQUIRED');
  });

  it('maps the network code', () => {
    const f = friendlyDispatchError('E_DISPATCH_NETWORK');
    expect(f.title).toContain('coordinator');
    expect(f.hint).toContain('Tailscale');
  });

  it('keeps the colon detail for URL_INVALID', () => {
    const f = friendlyDispatchError('E_DISPATCH_URL_INVALID: host not allowlisted');
    expect(f.title).toContain('網址');
    // The reason is appended to the hint in parentheses.
    expect(f.hint).toContain('host not allowlisted');
  });

  it('keeps the HTTP status detail', () => {
    const f = friendlyDispatchError('E_DISPATCH_HTTP_STATUS: HTTP 401');
    expect(f.title).toContain('coordinator');
    expect(f.hint).toContain('HTTP 401');
  });

  it('passes unknown strings through as the title verbatim', () => {
    const f = friendlyDispatchError('some unexpected error');
    expect(f.title).toBe('some unexpected error');
    expect(f.hint).toBeUndefined();
    expect(f.raw).toBe('some unexpected error');
  });

  it('tolerates non-string input', () => {
    const f = friendlyDispatchError(undefined);
    expect(f.title).toBe('派送失敗');
    expect(f.raw).toBe('');
  });

  it('maps caps-too-many and provider-unknown', () => {
    expect(friendlyDispatchError('E_DISPATCH_CAPS_TOO_MANY').hint).toContain('3');
    expect(friendlyDispatchError('E_DISPATCH_PROVIDER_UNKNOWN').title).toContain(
      'provider',
    );
  });

  it('tolerates leading/trailing whitespace on the code', () => {
    const f = friendlyDispatchError('  E_DISPATCH_NETWORK  ');
    expect(f.title).toContain('coordinator');
  });

  it('does not resolve Object.prototype members as codes', () => {
    // A null-prototype TABLE means a stray "constructor"/"toString" token
    // can't masquerade as a mapped entry (which would render a blank title).
    for (const evil of ['constructor', 'toString', 'hasOwnProperty', '__proto__']) {
      const f = friendlyDispatchError(evil);
      expect(f.title).toBe(evil); // passthrough, not a titleless object
      expect(f.hint).toBeUndefined();
    }
  });
});
