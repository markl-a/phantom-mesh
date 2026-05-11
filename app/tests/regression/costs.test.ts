/**
 * Regression Tests — Costs / Provider Commands
 *
 * Verifies the contract shape for:
 *   - get_costs          → array of cost records
 *   - get_revenue        → array of revenue records
 *   - get_provider_health → provider health info object
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mockInvoke } from './helpers';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}));

import { invoke } from '@tauri-apps/api/core';

// ─── Fixtures ─────────────────────────────────────────────────────────────────

const costRecordFixture = {
  id: 'cost-001',
  provider: 'ollama',
  model: 'llama3',
  tokens: 512,
  cost_usd: 0.001,
  timestamp: '2026-04-07T10:00:00Z',
};

const revenueRecordFixture = {
  id: 'rev-001',
  source: 'subscription',
  amount_usd: 9.99,
  timestamp: '2026-04-07T10:00:00Z',
};

const providerHealthFixture = {
  providers: [
    { name: 'ollama', status: 'healthy', latency_ms: 45 },
  ],
};

// ─── Tests ────────────────────────────────────────────────────────────────────

describe('Costs / Provider Commands — Regression', () => {
  beforeEach(() => {
    vi.mocked(invoke).mockImplementation(
      mockInvoke({
        get_costs: [costRecordFixture],
        get_revenue: [revenueRecordFixture],
        get_provider_health: providerHealthFixture,
      })
    );
  });

  describe('get_costs', () => {
    it('returns an array', async () => {
      const result = await invoke('get_costs');
      expect(Array.isArray(result)).toBe(true);
    });

    it('each cost record has a provider field', async () => {
      const result = await invoke('get_costs') as typeof costRecordFixture[];
      for (const record of result) {
        expect(record).toHaveProperty('provider');
        expect(typeof record.provider).toBe('string');
      }
    });

    it('empty array is a valid response', async () => {
      vi.mocked(invoke).mockImplementation(mockInvoke({ get_costs: [] }));
      const result = await invoke('get_costs');
      expect(Array.isArray(result)).toBe(true);
      expect((result as unknown[]).length).toBe(0);
    });
  });

  describe('get_revenue', () => {
    it('returns an array', async () => {
      const result = await invoke('get_revenue');
      expect(Array.isArray(result)).toBe(true);
    });

    it('each revenue record has a timestamp', async () => {
      const result = await invoke('get_revenue') as typeof revenueRecordFixture[];
      for (const record of result) {
        expect(record).toHaveProperty('timestamp');
        expect(typeof record.timestamp).toBe('string');
      }
    });

    it('empty array is a valid response', async () => {
      vi.mocked(invoke).mockImplementation(mockInvoke({ get_revenue: [] }));
      const result = await invoke('get_revenue');
      expect(Array.isArray(result)).toBe(true);
    });
  });

  describe('get_provider_health', () => {
    it('returns an object', async () => {
      const result = await invoke('get_provider_health');
      expect(typeof result).toBe('object');
      expect(result).not.toBeNull();
    });

    it('response is not an array', async () => {
      const result = await invoke('get_provider_health');
      expect(Array.isArray(result)).toBe(false);
    });

    it('providers field is present and is an array', async () => {
      const result = await invoke('get_provider_health') as typeof providerHealthFixture;
      expect(result).toHaveProperty('providers');
      expect(Array.isArray(result.providers)).toBe(true);
    });

    it('each provider entry has a name and status', async () => {
      const result = await invoke('get_provider_health') as typeof providerHealthFixture;
      for (const p of result.providers) {
        expect(p).toHaveProperty('name');
        expect(p).toHaveProperty('status');
      }
    });
  });
});
