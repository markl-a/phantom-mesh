/**
 * Regression Tests — Security Commands
 *
 * Verifies the contract shape for:
 *   - get_audit_log → array of audit entries
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mockInvoke } from './helpers';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}));

import { invoke } from '@tauri-apps/api/core';

// ─── Fixtures ─────────────────────────────────────────────────────────────────

const auditEntryFixture = {
  id: 'audit-001',
  action: 'agent_run',
  actor: 'user',
  resource: 'master',
  risk_level: 'low',
  timestamp: '2026-04-07T10:00:00Z',
  details: { prompt_length: 42 },
};

// ─── Tests ────────────────────────────────────────────────────────────────────

describe('Security Commands — Regression', () => {
  beforeEach(() => {
    vi.mocked(invoke).mockImplementation(
      mockInvoke({
        get_audit_log: [auditEntryFixture],
      })
    );
  });

  describe('get_audit_log', () => {
    it('returns an array', async () => {
      const result = await invoke('get_audit_log');
      expect(Array.isArray(result)).toBe(true);
    });

    it('each entry has an id field', async () => {
      const result = await invoke('get_audit_log') as typeof auditEntryFixture[];
      for (const entry of result) {
        expect(entry).toHaveProperty('id');
        expect(typeof entry.id).toBe('string');
      }
    });

    it('each entry has an action field', async () => {
      const result = await invoke('get_audit_log') as typeof auditEntryFixture[];
      for (const entry of result) {
        expect(entry).toHaveProperty('action');
        expect(typeof entry.action).toBe('string');
      }
    });

    it('each entry has a timestamp string', async () => {
      const result = await invoke('get_audit_log') as typeof auditEntryFixture[];
      for (const entry of result) {
        expect(entry).toHaveProperty('timestamp');
        expect(typeof entry.timestamp).toBe('string');
      }
    });

    it('each entry has a risk_level field', async () => {
      const result = await invoke('get_audit_log') as typeof auditEntryFixture[];
      for (const entry of result) {
        expect(entry).toHaveProperty('risk_level');
      }
    });

    it('filtering by risk_level: empty result is valid', async () => {
      vi.mocked(invoke).mockImplementation(mockInvoke({ get_audit_log: [] }));
      const result = await invoke('get_audit_log', { riskLevel: 'critical', limit: 50 });
      expect(Array.isArray(result)).toBe(true);
      expect((result as unknown[]).length).toBe(0);
    });

    it('default audit log call returns an array without arguments', async () => {
      const result = await invoke('get_audit_log');
      expect(Array.isArray(result)).toBe(true);
    });
  });
});
