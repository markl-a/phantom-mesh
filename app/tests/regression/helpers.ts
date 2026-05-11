/**
 * Regression Test Helpers
 *
 * Provides utilities for mocking the Tauri invoke() API in regression tests.
 * Tests verify the CONTRACT (return shape) of Tauri commands, not implementation details.
 */

import { vi } from 'vitest';

/**
 * Creates a vi.fn() that resolves to the given response.
 * Usage: pass as the `invoke` implementation in vi.mock().
 *
 * @param responseMap  A map of command name -> return value.
 *                     Commands not in the map will reject with "unknown command".
 */
export function mockInvoke(responseMap: Record<string, unknown>) {
  return vi.fn(async (command: string, _args?: Record<string, unknown>) => {
    if (command in responseMap) {
      return responseMap[command];
    }
    throw new Error(`Unknown command: ${command}`);
  });
}

/**
 * Creates a mock invoke that always resolves to a fixed value regardless of command name.
 */
export function mockInvokeAlways(response: unknown) {
  return vi.fn(async (_command: string, _args?: Record<string, unknown>) => response);
}

/**
 * Creates a mock invoke that always rejects with a given error message.
 */
export function mockInvokeError(message: string) {
  return vi.fn(async (_command: string, _args?: Record<string, unknown>) => {
    throw new Error(message);
  });
}

// ─── Shape assertion helpers ──────────────────────────────────────────────────

/**
 * Asserts that `obj` has all the given keys.
 */
export function expectKeys(obj: unknown, keys: string[]): void {
  if (typeof obj !== 'object' || obj === null) {
    throw new Error(`Expected object, got ${typeof obj}`);
  }
  for (const key of keys) {
    if (!(key in obj)) {
      throw new Error(`Expected key "${key}" to be present in ${JSON.stringify(obj)}`);
    }
  }
}
