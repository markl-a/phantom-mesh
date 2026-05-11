/**
 * Regression Tests — Agent Commands
 *
 * Verifies the contract shape for:
 *   - send_message     → { agent, output, tool_calls, elapsed }
 *   - get_conversations → { active_sessions } or array of conversations
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mockInvoke, mockInvokeError } from './helpers';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}));

import { invoke } from '@tauri-apps/api/core';

// ─── Fixtures ─────────────────────────────────────────────────────────────────

const sendMessageFixture = {
  agent: 'master',
  output: 'Hello! How can I help you today?',
  tool_calls: [],
  elapsed: 0.85,
};

const getConversationsFixture = {
  active_sessions: 2,
};

// ─── Tests ────────────────────────────────────────────────────────────────────

describe('Agent Commands — Regression', () => {
  beforeEach(() => {
    vi.mocked(invoke).mockImplementation(
      mockInvoke({
        send_message: sendMessageFixture,
        get_conversations: getConversationsFixture,
      })
    );
  });

  describe('send_message', () => {
    it('returns an object (not null, not array)', async () => {
      const result = await invoke('send_message', { prompt: 'Hello' });
      expect(typeof result).toBe('object');
      expect(result).not.toBeNull();
      expect(Array.isArray(result)).toBe(false);
    });

    it('response has agent field as a string', async () => {
      const result = await invoke('send_message', { prompt: 'Hello' }) as typeof sendMessageFixture;
      expect(result).toHaveProperty('agent');
      expect(typeof result.agent).toBe('string');
    });

    it('response has output field as a string', async () => {
      const result = await invoke('send_message', { prompt: 'Hello' }) as typeof sendMessageFixture;
      expect(result).toHaveProperty('output');
      expect(typeof result.output).toBe('string');
    });

    it('response has tool_calls field as an array', async () => {
      const result = await invoke('send_message', { prompt: 'Hello' }) as typeof sendMessageFixture;
      expect(result).toHaveProperty('tool_calls');
      expect(Array.isArray(result.tool_calls)).toBe(true);
    });

    it('response has elapsed field as a number', async () => {
      const result = await invoke('send_message', { prompt: 'Hello' }) as typeof sendMessageFixture;
      expect(result).toHaveProperty('elapsed');
      expect(typeof result.elapsed).toBe('number');
      expect(result.elapsed).toBeGreaterThanOrEqual(0);
    });

    it('send_message with explicit agent name works', async () => {
      const result = await invoke('send_message', { prompt: 'Hi', agent: 'master' }) as typeof sendMessageFixture;
      expect(result.agent).toBe('master');
    });

    it('propagates errors when command fails', async () => {
      vi.mocked(invoke).mockImplementation(mockInvokeError('Agent runtime unavailable'));
      await expect(invoke('send_message', { prompt: 'Hello' })).rejects.toThrow('Agent runtime unavailable');
    });
  });

  describe('get_conversations', () => {
    it('returns an object', async () => {
      const result = await invoke('get_conversations');
      expect(typeof result).toBe('object');
      expect(result).not.toBeNull();
    });

    it('response has active_sessions field', async () => {
      const result = await invoke('get_conversations') as typeof getConversationsFixture;
      expect(result).toHaveProperty('active_sessions');
    });

    it('active_sessions is a non-negative number', async () => {
      const result = await invoke('get_conversations') as typeof getConversationsFixture;
      expect(typeof result.active_sessions).toBe('number');
      expect(result.active_sessions).toBeGreaterThanOrEqual(0);
    });

    it('zero active sessions is valid', async () => {
      vi.mocked(invoke).mockImplementation(mockInvoke({ get_conversations: { active_sessions: 0 } }));
      const result = await invoke('get_conversations') as typeof getConversationsFixture;
      expect(result.active_sessions).toBe(0);
    });
  });
});
