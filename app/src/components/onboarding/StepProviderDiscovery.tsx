import { useState, useEffect, useCallback } from 'react';
import { safeInvoke as invoke } from '../../lib/tauri-compat';
import type {
  OnboardingData,
  DiscoveredProvider,
  CopilotTokenStatus,
  GcloudAdcStatus,
  ClaudeCliStatus,
  UserIdentity,
  WizardStep,
} from './types';

interface Props {
  data: OnboardingData;
  updateData: (partial: Partial<OnboardingData>) => void;
  goNext: () => void;
  goBack: () => void;
  goTo: (step: WizardStep) => void;
}

type ScanStatus = 'idle' | 'scanning' | 'done' | 'error';

// Service login config
interface ServiceConfig {
  id: string;
  label: string;
  desc: string;
  action: 'oauth' | 'detect_or_open';
  // For detect_or_open: first try local token, if not found open URL
  detectCmd?: string;
  fallbackUrl?: string;
  fallbackHint?: string;
}

const SERVICES: ServiceConfig[] = [
  {
    id: 'google',
    label: 'Google Gemini',
    desc: 'Google OAuth 登入',
    action: 'oauth',
  },
  {
    id: 'copilot',
    label: 'GitHub Copilot',
    desc: '偵測本地 token / 開啟訂閱頁',
    action: 'detect_or_open',
    detectCmd: 'read_copilot_token',
    fallbackUrl: 'https://github.com/features/copilot',
    fallbackHint: '已開啟 GitHub Copilot 頁面。訂閱並安裝 VS Code 擴充套件後，點「重新掃描」。',
  },
  {
    id: 'claude',
    label: 'Claude / Anthropic',
    desc: '偵測 CLI token / 開啟 Console',
    action: 'detect_or_open',
    detectCmd: 'read_claude_cli_token',
    fallbackUrl: 'https://console.anthropic.com/settings/keys',
    fallbackHint: '已開啟 Anthropic Console。取得 API Key 後在下一步「手動新增」輸入。',
  },
];

export default function StepProviderDiscovery({
  data,
  updateData,
  goNext,
  goBack,
  goTo,
}: Props) {
  const [scanStatus, setScanStatus] = useState<ScanStatus>('idle');
  const [error, setError] = useState<string | null>(null);
  const [activeHint, setActiveHint] = useState<string | null>(null);
  const [checking, setChecking] = useState<string | null>(null);

  const runScan = useCallback(async () => {
    setScanStatus('scanning');
    setError(null);
    try {
      const results = await invoke<Array<{
        name: string;
        providerType: string;
        source: string;
        tier: string;
        displayLabel: string;
        models: string[];
      }>>('scan_credentials');

      const providers: DiscoveredProvider[] = results.map((r: any) => ({
        name: r.name,
        providerType: r.providerType,
        source: r.source as DiscoveredProvider['source'],
        enabled: true,
        tier: r.tier as DiscoveredProvider['tier'],
        models: r.models,
        displayLabel: r.displayLabel,
      }));
      updateData({ discoveredProviders: providers });

      const ollama = providers.find((p) => p.name === 'ollama');
      if (ollama) {
        updateData({ ollamaEnabled: true });
      }

      setScanStatus('done');
    } catch (e) {
      setError(String(e));
      setScanStatus('error');
    }
  }, [updateData]);

  // Auto-scan on mount
  useEffect(() => {
    if (data.discoveredProviders.length > 0) {
      setScanStatus('done');
      return;
    }
    void runScan();
  }, []);

  const toggleProvider = (name: string) => {
    const updated = data.discoveredProviders.map((p) =>
      p.name === name ? { ...p, enabled: !p.enabled } : p
    );
    updateData({ discoveredProviders: updated });
  };

  // ── Google: real OAuth login via browser ──
  const handleGoogleOAuth = async () => {
    setChecking('google');
    setActiveHint(null);
    try {
      const identity = await invoke<UserIdentity>('oauth_sign_in', { provider: 'google' });
      // OAuth succeeded — add as discovered provider
      const exists = data.discoveredProviders.some(
        (p) => p.name === 'gemini' && p.source === 'token_file'
      );
      if (!exists) {
        updateData({
          discoveredProviders: [
            ...data.discoveredProviders,
            {
              name: 'gemini',
              providerType: 'gemini',
              source: 'token_file',
              enabled: true,
              tier: 'free',
              models: [],
              displayLabel: `Google Gemini (${identity.email})`,
            },
          ],
        });
      }
      setActiveHint('google_ok');
    } catch (e) {
      const msg = String(e);
      if (msg.includes('timeout') || msg.includes('cancelled')) {
        setActiveHint('google_cancelled');
      } else {
        setActiveHint('google_error');
      }
    }
    setChecking(null);
  };

  // ── Copilot: detect local token, or open subscription page ──
  const handleCopilotLogin = async () => {
    setChecking('copilot');
    setActiveHint(null);
    try {
      const status = await invoke<CopilotTokenStatus>('read_copilot_token');
      if (status.found) {
        const exists = data.discoveredProviders.some((p) => p.name === 'copilot');
        if (!exists) {
          updateData({
            discoveredProviders: [
              ...data.discoveredProviders,
              {
                name: 'copilot',
                providerType: 'copilot',
                source: 'token_file',
                enabled: true,
                tier: 'subscription',
                models: [],
                displayLabel: `GitHub Copilot (${status.user ?? ''})`,
              },
            ],
          });
        }
      } else {
        // No local token → open GitHub Copilot page
        await invoke('open_external_url', { url: 'https://github.com/features/copilot' });
        setActiveHint('copilot_opened');
      }
    } catch {
      try {
        await invoke('open_external_url', { url: 'https://github.com/features/copilot' });
        setActiveHint('copilot_opened');
      } catch {
        setActiveHint('copilot_error');
      }
    }
    setChecking(null);
  };

  // ── Claude: detect CLI token, or open Anthropic Console ──
  const handleClaudeLogin = async () => {
    setChecking('claude');
    setActiveHint(null);
    try {
      const status = await invoke<ClaudeCliStatus>('read_claude_cli_token');
      if (status.found) {
        const exists = data.discoveredProviders.some((p) => p.name === 'claude_cli');
        if (!exists) {
          updateData({
            discoveredProviders: [
              ...data.discoveredProviders,
              {
                name: 'claude_cli',
                providerType: 'claude_cli',
                source: 'token_file',
                enabled: true,
                tier: 'subscription',
                models: [],
                displayLabel: 'Claude CLI',
              },
            ],
          });
        }
      } else {
        // No local token → open Anthropic Console
        await invoke('open_external_url', { url: 'https://console.anthropic.com/settings/keys' });
        setActiveHint('claude_opened');
      }
    } catch {
      try {
        await invoke('open_external_url', { url: 'https://console.anthropic.com/settings/keys' });
        setActiveHint('claude_opened');
      } catch {
        setActiveHint('claude_error');
      }
    }
    setChecking(null);
  };

  // ── GCloud ADC: detect or open install page ──
  const handleGcloudLogin = async () => {
    setChecking('gcloud');
    setActiveHint(null);
    try {
      const status = await invoke<GcloudAdcStatus>('read_gcloud_adc');
      if (status.found) {
        const exists = data.discoveredProviders.some(
          (p) => p.name === 'gemini' && p.source === 'token_file'
        );
        if (!exists) {
          updateData({
            discoveredProviders: [
              ...data.discoveredProviders,
              {
                name: 'gemini',
                providerType: 'gemini',
                source: 'token_file',
                enabled: true,
                tier: 'free',
                models: [],
                displayLabel: `Google Gemini (gcloud${status.project ? ` — ${status.project}` : ''})`,
              },
            ],
          });
        }
      } else {
        await invoke('open_external_url', { url: 'https://aistudio.google.com/apikey' });
        setActiveHint('gcloud_opened');
      }
    } catch {
      try {
        await invoke('open_external_url', { url: 'https://aistudio.google.com/apikey' });
        setActiveHint('gcloud_opened');
      } catch {
        setActiveHint('gcloud_error');
      }
    }
    setChecking(null);
  };

  const enabledCount = data.discoveredProviders.filter((p) => p.enabled).length;
  const hasProvider = enabledCount > 0;

  // Hide buttons for already-detected services
  const hasCopilot = data.discoveredProviders.some((p) => p.name === 'copilot');
  const hasGemini = data.discoveredProviders.some((p) => p.name === 'gemini');
  const hasClaude = data.discoveredProviders.some((p) => p.name === 'claude_cli');

  const HINT_MESSAGES: Record<string, { type: 'success' | 'info' | 'warn'; text: string }> = {
    google_ok: { type: 'success', text: 'Google 登入成功！已加入 Provider 列表。' },
    google_cancelled: { type: 'warn', text: '登入已取消或逾時。請重試。' },
    google_error: { type: 'warn', text: 'Google 登入失敗。你也可以在下一步手動輸入 API Key。' },
    copilot_opened: { type: 'info', text: '已開啟 GitHub Copilot 頁面。完成訂閱並安裝擴充套件後，點擊「重新掃描」。' },
    copilot_error: { type: 'warn', text: '無法開啟瀏覽器。請手動前往 github.com/features/copilot。' },
    claude_opened: { type: 'info', text: '已開啟 Anthropic Console。取得 API Key 後在下一步「手動新增」輸入。' },
    claude_error: { type: 'warn', text: '無法開啟瀏覽器。請手動前往 console.anthropic.com。' },
    gcloud_opened: { type: 'info', text: '已開啟 Google AI Studio。取得 API Key 後在下一步「手動新增」輸入。' },
    gcloud_error: { type: 'warn', text: '無法開啟瀏覽器。請手動前往 aistudio.google.com/apikey。' },
  };

  const hint = activeHint ? HINT_MESSAGES[activeHint] : null;

  const hintStyles = {
    success: 'bg-phantom-success/10 border-phantom-success/30 text-phantom-success',
    info: 'bg-phantom-primary/10 border-phantom-primary/30 text-phantom-primary',
    warn: 'bg-phantom-warning/10 border-phantom-warning/30 text-phantom-warning',
  };

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h2 className="text-xl font-bold">Provider 自動偵測</h2>
        {scanStatus === 'done' && (
          <button
            onClick={() => { setActiveHint(null); void runScan(); }}
            className="text-xs text-phantom-primary hover:text-phantom-primary/80 border border-phantom-primary/30 px-3 py-1.5 rounded transition hover:border-phantom-primary/60"
          >
            重新掃描
          </button>
        )}
      </div>

      {scanStatus === 'scanning' && (
        <div className="flex items-center gap-2 text-sm text-phantom-muted animate-pulse">
          <div className="w-4 h-4 border-2 border-phantom-primary border-t-transparent rounded-full animate-spin" />
          掃描本地憑證中...
        </div>
      )}

      {scanStatus === 'error' && (
        <div className="bg-phantom-danger/20 border border-phantom-danger/40 rounded-lg p-3 text-sm">
          <span className="text-phantom-danger">掃描失敗: {error}</span>
          <button
            onClick={() => void runScan()}
            className="ml-3 text-xs text-phantom-primary hover:underline"
          >
            重試
          </button>
        </div>
      )}

      {/* Auto-detected results */}
      {data.discoveredProviders.length > 0 && (
        <div className="space-y-2">
          <h3 className="text-sm font-medium text-phantom-muted">已偵測到</h3>
          {data.discoveredProviders.map((p) => (
            <label
              key={`${p.name}-${p.source}`}
              className="flex items-center justify-between p-3 rounded-lg border border-phantom-border bg-phantom-card hover:border-phantom-primary/40 cursor-pointer transition"
            >
              <div className="flex items-center gap-2">
                <span className="w-2 h-2 rounded-full bg-phantom-success" />
                <span className="font-medium text-phantom-text">{p.displayLabel}</span>
                {p.models.length > 0 && (
                  <span className="text-xs text-phantom-muted">
                    {p.models.length} 個模型
                  </span>
                )}
              </div>
              <input
                type="checkbox"
                checked={p.enabled}
                onChange={() => toggleProvider(p.name)}
                className="w-4 h-4 accent-phantom-primary"
              />
            </label>
          ))}
        </div>
      )}

      {scanStatus === 'done' && data.discoveredProviders.length === 0 && (
        <div className="text-sm text-phantom-muted bg-phantom-card border border-phantom-border rounded-lg p-4 text-center">
          未偵測到任何已設定的 Provider。請透過下方按鈕登入服務，或跳至下一步手動輸入 API Key。
        </div>
      )}

      {/* Service login buttons */}
      {(!hasGemini || !hasCopilot || !hasClaude) && (
        <div className="space-y-2">
          <h3 className="text-sm font-medium text-phantom-muted">登入服務</h3>
          <div className="grid grid-cols-1 gap-2">
            {/* Google Gemini — OAuth coming soon; use manual API key instead */}
            {!hasGemini && (
              <div className="w-full text-left p-3 rounded-lg border border-phantom-border bg-phantom-card opacity-50 cursor-not-allowed select-none">
                <div className="flex items-center justify-between">
                  <div>
                    <span className="text-sm font-medium text-phantom-text">Google Gemini</span>
                    <span className="text-xs text-phantom-muted ml-2">OAuth 即將支援 — 請用下一步手動輸入 API Key</span>
                  </div>
                  <span className="text-xs bg-phantom-border/50 px-2 py-0.5 rounded text-phantom-muted">即將支援</span>
                </div>
              </div>
            )}
            {/* Copilot — detect token or open subscription page */}
            {!hasCopilot && (
              <button
                onClick={handleCopilotLogin}
                disabled={checking === 'copilot'}
                className="w-full text-left p-3 rounded-lg border border-phantom-border bg-phantom-card hover:border-phantom-primary/50 transition disabled:opacity-60"
              >
                <div className="flex items-center justify-between">
                  <div>
                    <span className="text-sm font-medium text-phantom-text">GitHub Copilot</span>
                    <span className="text-xs text-phantom-muted ml-2">偵測 token / 開啟訂閱頁</span>
                  </div>
                  {checking === 'copilot' ? (
                    <div className="w-4 h-4 border-2 border-phantom-primary border-t-transparent rounded-full animate-spin" />
                  ) : (
                    <span className="text-xs text-phantom-primary font-medium">登入 →</span>
                  )}
                </div>
              </button>
            )}
            {/* Claude — detect CLI token or open Anthropic Console */}
            {!hasClaude && (
              <button
                onClick={handleClaudeLogin}
                disabled={checking === 'claude'}
                className="w-full text-left p-3 rounded-lg border border-phantom-border bg-phantom-card hover:border-phantom-primary/50 transition disabled:opacity-60"
              >
                <div className="flex items-center justify-between">
                  <div>
                    <span className="text-sm font-medium text-phantom-text">Claude / Anthropic</span>
                    <span className="text-xs text-phantom-muted ml-2">偵測 CLI / 開啟 Console</span>
                  </div>
                  {checking === 'claude' ? (
                    <div className="w-4 h-4 border-2 border-phantom-primary border-t-transparent rounded-full animate-spin" />
                  ) : (
                    <span className="text-xs text-phantom-primary font-medium">登入 →</span>
                  )}
                </div>
              </button>
            )}
          </div>
        </div>
      )}

      {/* Hint banner — feedback after clicking a service button */}
      {hint && (
        <div className={`border rounded-lg p-3 text-sm flex items-center justify-between ${hintStyles[hint.type]}`}>
          <span>{hint.text}</span>
          <button
            onClick={() => setActiveHint(null)}
            className="text-xs opacity-60 hover:opacity-100 ml-3 shrink-0"
          >
            關閉
          </button>
        </div>
      )}

      {/* Footer */}
      <div className="text-sm text-phantom-muted">
        已啟用: <span className="text-phantom-text font-medium">{enabledCount}</span> 個 provider
        {hasProvider && <span className="text-phantom-success ml-1">✓</span>}
      </div>
      <div className="flex justify-between items-center">
        <button
          onClick={goBack}
          className="text-phantom-muted hover:text-phantom-text text-sm px-4 py-2 transition"
        >
          ← 上一步
        </button>
        <div className="flex items-center gap-2">
          {hasProvider && (
            <button
              onClick={() => goTo(4 as WizardStep)}
              className="text-phantom-muted hover:text-phantom-text text-xs px-3 py-1.5 transition"
            >
              跳過手動設定 →→
            </button>
          )}
          <button
            onClick={goNext}
            className={`px-6 py-2.5 rounded-lg font-medium transition ${
              hasProvider
                ? 'bg-phantom-primary text-phantom-bg hover:brightness-110'
                : 'bg-phantom-primary/40 text-phantom-bg/70 hover:bg-phantom-primary/60'
            }`}
          >
            手動新增 API Key →
          </button>
        </div>
      </div>
    </div>
  );
}
