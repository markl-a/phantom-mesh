// Unit tests — describeError (lib/providers).
//
// describeError humanises provider/runtime error strings for the UI. All
// three MobileConversation send paths (local / provider / cluster) route
// their catch blocks through it, so a regression here would re-leak raw
// exceptions like "TypeError: Failed to fetch" to end users. Lock the
// mapping table, especially the native fetch/WKWebView humanisation.

import { describe, it, expect, vi } from 'vitest';

// describeError is a pure function; stub the tauri-compat import so loading
// providers.ts in jsdom never touches the (absent) Tauri runtime.
vi.mock('../../src/lib/tauri-compat', () => ({
  safeInvoke: vi.fn(),
  isTauri: () => false,
}));

import { describeError } from '../../src/lib/providers';

describe('describeError', () => {
  it('maps empty input to a generic message', () => {
    expect(describeError('')).toBe('未知錯誤');
  });

  it.each([
    ['rate_limit hit', 'Provider 已達速率上限，請稍後再試'],
    ['auth_error: bad key', 'Provider 認證失敗，請檢查 API key'],
    ['network_error', '網路連線失敗'],
    ['model_not_found', '找不到指定的 model'],
    ['context_too_long', '對話內容超過 model 上下文長度'],
    ['fallback_exhausted', '所有 fallback provider 都失敗了'],
    ['no_match_class', '沒有符合條件的 provider（檢查 agents.toml）'],
    ['cost_budget_exceeded', '已超出成本預算上限'],
  ])('maps known token %s', (input, expected) => {
    expect(describeError(input)).toBe(expected);
  });

  // The path every chat send relies on: a bare fetch/WKWebView failure must
  // not reach the UI verbatim.
  it.each([
    'Load failed',
    'TypeError: Failed to fetch',
    'NetworkError when attempting to fetch resource',
    'Error: ERR_CONNECTION_REFUSED',
    'net::ERR_FAILED',
  ])('humanises native fetch failure %s', (input) => {
    expect(describeError(input)).toBe('連線失敗（檢查網路，或目標服務是否啟動）');
  });

  it('explains browser-mode streaming limitation', () => {
    expect(describeError('providers_streaming_browser_mode')).toBe(
      'Provider streaming 只在桌面 / 行動 App 內運作（瀏覽器模式不支援）',
    );
  });

  it('returns an unrecognised error unchanged', () => {
    expect(describeError('some unmapped raw detail')).toBe('some unmapped raw detail');
  });
});
