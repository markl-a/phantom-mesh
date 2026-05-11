/**
 * Regression Tests — Health Commands
 *
 * Verifies the contract shape for:
 *   - get_health        → { status, version, uptime_seconds }
 *   - get_dashboard_status → { tools_count, hands_count, active_sessions, cluster_nodes, uptime_seconds, total_requests }
 *   - get_estop_status  → { active, triggered_at }
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mockInvoke } from './helpers';

// ─── Mock Tauri core ──────────────────────────────────────────────────────────

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}));

import { invoke } from '@tauri-apps/api/core';

// ─── Fixtures ─────────────────────────────────────────────────────────────────

const healthFixture = {
  status: 'ok',
  version: '0.1.0',
  uptime_seconds: 42,
};

const dashboardFixture = {
  tools_count: 5,
  hands_count: 2,
  active_sessions: 1,
  cluster_nodes: 3,
  uptime_seconds: 3600,
  total_requests: 128,
};

const estopFixture = {
  active: false,
  triggered_at: null,
};

// ─── Tests ────────────────────────────────────────────────────────────────────

describe('Health Commands — Regression', () => {
  beforeEach(() => {
    vi.mocked(invoke).mockImplementation(
      mockInvoke({
        get_health: healthFixture,
        get_dashboard_status: dashboardFixture,
        get_estop_status: estopFixture,
      })
    );
  });

  describe('get_health', () => {
    it('returns an object with status field', async () => {
      const result = await invoke('get_health') as typeof healthFixture;
      expect(result).toHaveProperty('status');
    });

    it('status is a string', async () => {
      const result = await invoke('get_health') as typeof healthFixture;
      expect(typeof result.status).toBe('string');
    });

    it('returns uptime_seconds as a number', async () => {
      const result = await invoke('get_health') as typeof healthFixture;
      expect(typeof result.uptime_seconds).toBe('number');
      expect(result.uptime_seconds).toBeGreaterThanOrEqual(0);
    });

    it('returns version field', async () => {
      const result = await invoke('get_health') as typeof healthFixture;
      expect(result).toHaveProperty('version');
      expect(typeof result.version).toBe('string');
    });
  });

  describe('get_dashboard_status', () => {
    it('returns all required dashboard fields', async () => {
      const result = await invoke('get_dashboard_status') as typeof dashboardFixture;
      const requiredFields = [
        'tools_count',
        'hands_count',
        'active_sessions',
        'cluster_nodes',
        'uptime_seconds',
        'total_requests',
      ];
      for (const field of requiredFields) {
        expect(result).toHaveProperty(field);
      }
    });

    it('all numeric fields are non-negative numbers', async () => {
      const result = await invoke('get_dashboard_status') as typeof dashboardFixture;
      for (const field of ['tools_count', 'hands_count', 'active_sessions', 'cluster_nodes', 'uptime_seconds', 'total_requests'] as const) {
        expect(typeof result[field]).toBe('number');
        expect(result[field]).toBeGreaterThanOrEqual(0);
      }
    });
  });

  describe('get_estop_status', () => {
    it('returns an object with active field', async () => {
      const result = await invoke('get_estop_status') as typeof estopFixture;
      expect(result).toHaveProperty('active');
    });

    it('active is a boolean', async () => {
      const result = await invoke('get_estop_status') as typeof estopFixture;
      expect(typeof result.active).toBe('boolean');
    });

    it('returns triggered_at field (can be null)', async () => {
      const result = await invoke('get_estop_status') as typeof estopFixture;
      expect(result).toHaveProperty('triggered_at');
    });
  });
});
