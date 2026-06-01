// SPEC-17 §11.2 deep-link host→route mapping. core::dispatch_deep_link does the
// security filtering (allowlist / path-traversal / token sanitization) before
// this runs; deepLinkPath is the final defensive mapping — only known nav hosts
// resolve to a path, everything else returns null (no navigation).

import { describe, expect, it } from 'vitest';
import { deepLinkPath } from '../../src/lib/deepLink';

describe('deepLinkPath', () => {
  it('maps the known navigation hosts', () => {
    expect(deepLinkPath('chat')).toBe('/');
    expect(deepLinkPath('mesh')).toBe('/cluster');
    expect(deepLinkPath('settings')).toBe('/settings');
  });

  it('routes a settings sub-section deep-link', () => {
    expect(deepLinkPath('settings', 'cluster')).toBe('/settings/cluster');
    expect(deepLinkPath('settings', 'providers')).toBe('/settings/providers');
  });

  it('falls back to /settings for an empty or malformed sub-path', () => {
    expect(deepLinkPath('settings', '')).toBe('/settings');
    // defense-in-depth: a slash/traversal segment never builds a nested path
    expect(deepLinkPath('settings', 'a/b')).toBe('/settings');
    expect(deepLinkPath('settings', '..')).toBe('/settings');
  });

  it('returns null for hosts handled elsewhere or unknown (no navigation)', () => {
    // oauth + demo-mode have their own listeners; never navigate here.
    expect(deepLinkPath('oauth', 'callback')).toBeNull();
    expect(deepLinkPath('demo-mode')).toBeNull();
    expect(deepLinkPath('onboarding')).toBeNull();
    // unknown / future / garbage hosts must not deep-link anywhere
    expect(deepLinkPath('evil')).toBeNull();
    expect(deepLinkPath(undefined)).toBeNull();
    expect(deepLinkPath(null)).toBeNull();
    expect(deepLinkPath('')).toBeNull();
  });
});
