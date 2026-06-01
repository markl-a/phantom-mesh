// Unit tests — coerceBigInts (the systemic Tauri-invoke BigInt guard).
//
// ts-rs maps Rust u64 → TS bigint; Tauri's invoke can't serialize BigInt
// ("Do not know how to serialize a BigInt"). safeInvoke coerces bigint→number
// at the IPC boundary so no command can hit that crash. These tests pin the
// coercion: every bigint becomes a number, other types are untouched, nesting
// is handled, class instances are left alone, and the result is JSON-safe.

import { describe, expect, it } from 'vitest';
import { coerceBigInts } from '../../src/lib/tauri-compat';

describe('coerceBigInts', () => {
  it('coerces a top-level bigint to a number', () => {
    expect(coerceBigInts(123n)).toBe(123);
    expect(typeof coerceBigInts(123n)).toBe('number');
  });

  it('coerces bigints inside nested objects', () => {
    const out = coerceBigInts({ req: { plannedDurationMs: 1500000n, label: 'x' } }) as {
      req: { plannedDurationMs: unknown; label: unknown };
    };
    expect(out.req.plannedDurationMs).toBe(1500000);
    expect(typeof out.req.plannedDurationMs).toBe('number');
    expect(out.req.label).toBe('x');
  });

  it('coerces bigints inside arrays (incl. nested)', () => {
    const out = coerceBigInts({ items: [1n, { ts: 2n }, [3n]] }) as {
      items: [number, { ts: number }, number[]];
    };
    expect(out.items[0]).toBe(1);
    expect(out.items[1].ts).toBe(2);
    expect(out.items[2][0]).toBe(3);
  });

  it('leaves non-bigint primitives untouched', () => {
    const input = { a: 'str', b: 42, c: true, d: null, e: undefined };
    expect(coerceBigInts(input)).toEqual(input);
  });

  it('passes class instances through without recursing', () => {
    const d = new Date(0);
    const out = coerceBigInts({ when: d }) as { when: unknown };
    expect(out.when).toBe(d); // same reference, not rebuilt
  });

  it('produces JSON-serializable output for an all-bigint payload', () => {
    const out = coerceBigInts({ a: 1n, nested: { b: 2n, arr: [3n] } });
    expect(() => JSON.stringify(out)).not.toThrow();
  });

  it('leaves a bigint above MAX_SAFE_INTEGER as-is (no silent truncation)', () => {
    const huge = BigInt(Number.MAX_SAFE_INTEGER) + 10n;
    const out = coerceBigInts({ id: huge }) as { id: unknown };
    expect(out.id).toBe(huge); // not coerced → would fail loudly downstream
    expect(typeof out.id).toBe('bigint');
  });

  it('does not let an own "__proto__" key pollute the result prototype', () => {
    const malicious = JSON.parse('{"__proto__": {"polluted": true}}');
    const out = coerceBigInts(malicious) as Record<string, unknown>;
    // Object.prototype must not gain "polluted"; out keeps a normal prototype.
    expect(({} as Record<string, unknown>).polluted).toBeUndefined();
    expect(Object.getPrototypeOf(out)).toBe(Object.prototype);
  });
});
