import { useState, useEffect, useCallback } from 'react';
import { safeInvoke as invoke } from '../../lib/tauri-compat';
import type { HardwareScanResult, DiscoveredProvider, DaemonStatus, UserIdentity } from './types';
import { ONBOARDED_KEY } from './types';

interface Props {
  onComplete: () => void;
}

type Phase = 'scanning' | 'ready' | 'launching' | 'success' | 'error';

interface ScanResult {
  hardware: HardwareScanResult | null;
  providers: DiscoveredProvider[];
}

const IDENTITY_KEY = 'phantom_mesh_identity';

const MANUAL_PROVIDERS = [
  { key: 'openrouter', label: 'OpenRouter', desc: '一個 key 用所有模型（推薦）', url: 'https://openrouter.ai/keys', providerType: 'openai_compat', tier: 'payg' },
  { key: 'openai',     label: 'OpenAI',     desc: 'GPT-4o、o1 等',             url: 'https://platform.openai.com/api-keys',         providerType: 'openai',       tier: 'payg' },
  { key: 'anthropic',  label: 'Anthropic',  desc: 'Claude 3.5 / 4 等',         url: 'https://console.anthropic.com/settings/keys',  providerType: 'anthropic',    tier: 'payg' },
  { key: 'gemini',     label: 'Google Gemini', desc: 'Gemini 2.5 Flash / Pro', url: 'https://aistudio.google.com/apikey',           providerType: 'gemini',       tier: 'payg' },
  { key: 'groq',       label: 'Groq',       desc: '超快推理，免費額度',         url: 'https://console.groq.com/keys',                providerType: 'groq',         tier: 'free' },
];

function saveIdentity(id: UserIdentity) {
  localStorage.setItem(IDENTITY_KEY, JSON.stringify(id));
}

function loadIdentity(): UserIdentity | null {
  try {
    const raw = localStorage.getItem(IDENTITY_KEY);
    return raw ? JSON.parse(raw) : null;
  } catch { return null; }
}

export function clearSession() {
  localStorage.removeItem(IDENTITY_KEY);
  localStorage.removeItem(ONBOARDED_KEY);
}

export default function OnboardingQuickStart({ onComplete }: Props) {
  const [phase, setPhase] = useState<Phase>('scanning');
  const [scan, setScan] = useState<ScanResult>({ hardware: null, providers: [] });
  const [identity, setIdentity] = useState<UserIdentity | null>(loadIdentity);
  const [oauthLoading, setOauthLoading] = useState<string | null>(null);
  const [oauthError, setOauthError] = useState<string | null>(null);
  const [launchMsg, setLaunchMsg] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [countdown, setCountdown] = useState(3);

  // ── Persist identity whenever it changes ──
  useEffect(() => {
    if (identity) saveIdentity(identity);
  }, [identity]);

  // ── Pick up OAuth redirect result from URL ──
  useEffect(() => {
    const params = new URLSearchParams(window.location.search);
    const oauthData = params.get('oauth_identity');
    if (oauthData) {
      try {
        const parsed = JSON.parse(decodeURIComponent(oauthData)) as UserIdentity;
        setIdentity(parsed);
        window.history.replaceState({}, '', window.location.pathname);
      } catch { /* ignore parse errors */ }
    }
  }, []);

  // ── Auto-scan on mount ──
  useEffect(() => {
    (async () => {
      try {
        const [hw, creds] = await Promise.all([
          invoke<HardwareScanResult>('scan_hardware').catch(() => null),
          invoke<Array<{
            name: string;
            providerType: string;
            source: string;
            tier: string;
            displayLabel: string;
            models: string[];
          }>>('scan_credentials').catch(() => []),
        ]);

        const providers: DiscoveredProvider[] = (creds ?? []).map((r: any) => ({
          name: r.name,
          providerType: r.providerType,
          source: r.source as DiscoveredProvider['source'],
          enabled: true,
          tier: r.tier as DiscoveredProvider['tier'],
          models: r.models,
          displayLabel: r.displayLabel,
        }));

        setScan({ hardware: hw, providers });
        setPhase('ready');
      } catch (e) {
        setError(String(e));
        setPhase('error');
      }
    })();
  }, []);

  // ── Quick login (email-based, no OAuth needed) ──
  const [loginEmail, setLoginEmail] = useState('');
  const [loginName, setLoginName] = useState('');

  const handleQuickLogin = () => {
    if (!loginEmail.trim()) return;
    setIdentity({
      provider: 'local',
      sub: `local-${Date.now()}`,
      email: loginEmail.trim(),
      display_name: loginName.trim() || loginEmail.split('@')[0],
      avatar_url: null,
      id_token: null,
    });
    setOauthError(null);
  };

  // ── OAuth: open system browser via daemon, poll for result ──
  const handleOAuth = async (provider: 'google' | 'apple') => {
    setOauthLoading(provider);
    setOauthError(null);
    const DAEMON = 'http://localhost:7878';
    const oauthUrl = `${DAEMON}/oauth/${provider}`;

    try {
      // Open system browser — try multiple methods
      let opened = false;

      // Method 1: Tauri shell plugin (most reliable for opening system browser)
      if (!opened) {
        try {
          const { open } = await import('@tauri-apps/plugin-shell');
          await open(oauthUrl);
          opened = true;
        } catch { /* not in Tauri or plugin not available */ }
      }

      // Method 2: Tauri invoke command
      if (!opened) {
        try {
          await invoke('open_external_url', { url: oauthUrl });
          opened = true;
        } catch { /* command failed */ }
      }

      // Method 3: window.open fallback (browser mode)
      if (!opened) {
        window.open(oauthUrl, '_blank');
      }

      // Poll daemon for OAuth result
      const result = await new Promise<UserIdentity>((resolve, reject) => {
        let attempts = 0;
        const poll = setInterval(async () => {
          attempts++;
          try {
            const resp = await fetch(`${DAEMON}/oauth/result`);
            const data = await resp.json();
            if (data.ok && data.identity) {
              clearInterval(poll);
              resolve(data.identity as UserIdentity);
            } else if (data.error && data.error !== 'no result yet') {
              clearInterval(poll);
              reject(new Error(data.error));
            }
          } catch { /* keep trying */ }
          if (attempts >= 120) {
            clearInterval(poll);
            reject(new Error('Login timeout'));
          }
        }, 1000);
      });

      setIdentity(result);
    } catch (e) {
      setOauthError(String(e));
    }
    setOauthLoading(null);
  };

  // ── Launch daemon ──
  const launch = useCallback(async () => {
    setPhase('launching');
    setError(null);

    try {
      const hw = scan.hardware;
      const port = hw?.available_port ?? 7878;
      const ollamaOnline = hw?.ollama_status === 'online';

      const firstProvider = scan.providers.find((p) => p.enabled);
      const defaultProvider = ollamaOnline
        ? 'ollama'
        : firstProvider?.name ?? 'ollama';
      const defaultModel = ollamaOnline
        ? (hw?.ollama_models[0] ?? 'llama3')
        : firstProvider?.models?.[0] ?? 'llama3';

      // 1. Write config
      setLaunchMsg('寫入設定檔...');
      // Build manual providers from hand-entered API keys
      const manualProvidersList = MANUAL_PROVIDERS
        .filter(p => manualKeys[p.key]?.trim())
        .map(p => ({
          name: p.key,
          provider_type: p.providerType,
          api_key: manualKeys[p.key].trim(),
          tier: p.tier,
          base_url: null as string | null,
          endpoint: null as string | null,
          region: null as string | null,
        }));
      // Pick default provider from manual keys if nothing auto-detected
      const firstManual = manualProvidersList[0];
      const effectiveProvider = defaultProvider === 'ollama' && !ollamaOnline
        ? (firstManual?.name ?? 'openrouter')
        : defaultProvider;
      const effectiveModel = effectiveProvider === firstManual?.name
        ? (effectiveProvider === 'openrouter' ? 'google/gemini-2.5-flash'
          : effectiveProvider === 'openai' ? 'gpt-4o-mini'
          : effectiveProvider === 'anthropic' ? 'claude-haiku-4-5-20251001'
          : effectiveProvider === 'gemini' ? 'gemini-2.0-flash'
          : effectiveProvider === 'groq' ? 'llama-3.3-70b-versatile'
          : defaultModel)
        : defaultModel;

      await invoke('write_config', {
        data: {
          port,
          discovered_providers: scan.providers
            .filter((p) => p.enabled)
            .map((p) => ({
              name: p.name,
              provider_type: p.providerType,
              tier: p.tier,
              token_source: p.source,
              base_url: null,
              env_key_name: null,
            })),
          manual_providers: manualProvidersList,
          ollama_endpoint: ollamaOnline ? 'http://localhost:11434' : null,
          default_agent_provider: effectiveProvider,
          default_agent_model: effectiveModel,
          auth_key: crypto.randomUUID().replace(/-/g, ''),
          telegram_token: null,
          identity_provider: identity?.provider ?? null,
          identity_sub: identity?.sub ?? null,
          identity_email: identity?.email ?? null,
          is_primary: true,
        },
      });

      // 2. Check runtime health (library mode — no sidecar needed)
      setLaunchMsg('驗證 Runtime...');
      try {
        await invoke('get_conversations');
        // Runtime is ready
      } catch {
        // Runtime not ready yet — try legacy daemon as fallback
        const binaryPath = hw?.daemon_binary_path;
        if (binaryPath) {
          const status = await invoke<DaemonStatus>('launch_daemon', {
            vaultPin: '',
            port,
            binaryPath,
          });
          if (!status.ok) throw new Error('Runtime 啟動失敗，請檢查日誌');
        }
        // If no binary path and runtime not ready, just proceed anyway
        // (runtime may still be initializing in background)
      }

      // 3. Done!
      localStorage.setItem(ONBOARDED_KEY, 'true');
      setPhase('success');
    } catch (e) {
      setError(String(e));
      setPhase('error');
    }
  }, [scan, identity]);

  // Countdown after success
  useEffect(() => {
    if (phase !== 'success') return;
    if (countdown <= 0) { onComplete(); return; }
    const timer = setTimeout(() => setCountdown((c) => c - 1), 1000);
    return () => clearTimeout(timer);
  }, [phase, countdown, onComplete]);

  // ── Open external login page ──
  const openLogin = async (url: string) => {
    await invoke('open_external_url', { url }).catch(() => {});
  };

  const hw = scan.hardware;
  const ollamaOnline = hw?.ollama_status === 'online';
  const providerCount = scan.providers.filter((p) => p.enabled).length;
  const hasAnyProvider = ollamaOnline || providerCount > 0;

  const detectedNames = new Set(scan.providers.map((p) => p.name));
  const [manualKeys, setManualKeys] = useState<Record<string, string>>({});
  const [expandedProvider, setExpandedProvider] = useState<string | null>(null);

  return (
    <div className="h-screen bg-phantom-bg flex flex-col items-center justify-center p-8">
      <div className="w-full max-w-lg">
        {/* Header */}
        <div className="text-center mb-8">
          <h1 className="text-3xl font-bold text-phantom-text mb-2">Phantom Mesh</h1>
          <p className="text-phantom-muted text-sm">
            {phase === 'scanning' && '正在偵測你的環境...'}
            {phase === 'ready' && '環境偵測完成，準備啟動'}
            {phase === 'launching' && launchMsg}
            {phase === 'success' && '啟動成功！'}
            {phase === 'error' && '發生錯誤'}
          </p>
        </div>

        {/* Scanning spinner */}
        {phase === 'scanning' && (
          <div className="flex flex-col items-center gap-4 py-12">
            <div className="w-10 h-10 border-3 border-phantom-primary border-t-transparent rounded-full animate-spin" />
            <span className="text-phantom-muted text-sm">掃描硬體與 Provider...</span>
          </div>
        )}

        {/* ── Ready: show scan results + social login + launch ── */}
        {phase === 'ready' && (
          <>
            {/* Account Login */}
            <div className="bg-phantom-card border border-phantom-border rounded-lg p-4 mb-4">
              <div className="text-sm text-phantom-text mb-3">帳號登入</div>
              {identity ? (
                <div className="flex items-center gap-3 py-1">
                  {identity.avatar_url && (
                    <img src={identity.avatar_url} alt="" className="w-8 h-8 rounded-full" />
                  )}
                  <div className="flex-1 min-w-0">
                    <div className="text-sm text-white truncate">{identity.display_name}</div>
                    <div className="text-xs text-phantom-muted truncate">{identity.email}</div>
                  </div>
                  <span className="text-phantom-success text-xs flex-shrink-0">已登入</span>
                  <button
                    onClick={() => {
                      setIdentity(null);
                      localStorage.removeItem(IDENTITY_KEY);
                    }}
                    className="text-phantom-muted hover:text-phantom-danger text-xs flex-shrink-0 ml-1 transition"
                  >
                    登出
                  </button>
                </div>
              ) : (
                <>
                  {/* Quick email login */}
                  <div className="space-y-2 mb-3">
                    <input
                      type="text"
                      placeholder="你的名稱"
                      value={loginName}
                      onChange={(e) => setLoginName(e.target.value)}
                      className="w-full bg-phantom-bg border border-phantom-border rounded-lg px-3 py-2 text-sm text-phantom-text placeholder:text-phantom-muted focus:border-phantom-primary outline-none"
                    />
                    <input
                      type="email"
                      placeholder="Email"
                      value={loginEmail}
                      onChange={(e) => setLoginEmail(e.target.value)}
                      onKeyDown={(e) => e.key === 'Enter' && handleQuickLogin()}
                      className="w-full bg-phantom-bg border border-phantom-border rounded-lg px-3 py-2 text-sm text-phantom-text placeholder:text-phantom-muted focus:border-phantom-primary outline-none"
                    />
                    <button
                      onClick={handleQuickLogin}
                      disabled={!loginEmail.trim()}
                      className="w-full bg-phantom-primary text-phantom-bg py-2 rounded-lg font-medium text-sm hover:brightness-110 transition disabled:opacity-50"
                    >
                      登入
                    </button>
                  </div>

                  {/* Divider */}
                  <div className="flex items-center gap-2 my-3">
                    <div className="flex-1 border-t border-phantom-border" />
                    <span className="text-phantom-muted text-xs">或使用第三方帳號</span>
                    <div className="flex-1 border-t border-phantom-border" />
                  </div>

                  {/* OAuth waiting state */}
                  {oauthLoading && (
                    <div className="flex items-center gap-3 bg-phantom-primary/10 border border-phantom-primary/30 rounded-lg p-3 mb-3">
                      <div className="w-4 h-4 border-2 border-phantom-primary border-t-transparent rounded-full animate-spin flex-shrink-0" />
                      <div className="flex-1">
                        <p className="text-sm text-phantom-text">等待瀏覽器登入...</p>
                        <p className="text-xs text-phantom-muted">請在瀏覽器完成 {oauthLoading === 'google' ? 'Google' : 'Apple'} 登入</p>
                      </div>
                      <button
                        onClick={() => setOauthLoading(null)}
                        className="text-xs text-phantom-muted hover:text-phantom-text"
                      >
                        取消
                      </button>
                    </div>
                  )}

                  {/* Google / Apple OAuth — coming soon */}
                  <div className="flex gap-2">
                    <div className="flex-1 flex items-center justify-center gap-2 bg-white/10 text-phantom-muted px-3 py-2 rounded-lg
                                font-medium text-sm border border-phantom-border cursor-not-allowed select-none">
                      <span>G</span>
                      Google
                      <span className="text-xs bg-phantom-border/50 px-1.5 py-0.5 rounded">即將支援</span>
                    </div>
                    <div className="flex-1 flex items-center justify-center gap-2 bg-white/5 text-phantom-muted px-3 py-2 rounded-lg
                                font-medium text-sm border border-phantom-border cursor-not-allowed select-none">
                      <span></span>
                      Apple
                      <span className="text-xs bg-phantom-border/50 px-1.5 py-0.5 rounded">即將支援</span>
                    </div>
                  </div>
                </>
              )}
              {oauthError && (
                <p className="text-phantom-danger text-xs mt-2">{oauthError}</p>
              )}
              {!identity && (
                <p className="text-phantom-muted text-xs mt-2">
                  可選 — 不登入也能使用，部分同步功能需要帳號。
                </p>
              )}
            </div>

            {/* Scan Results */}
            {hw && (
              <div className="bg-phantom-card border border-phantom-border rounded-lg p-4 mb-4 space-y-3">
                <div className="flex items-center justify-between text-sm">
                  <span className="text-phantom-muted">GPU</span>
                  <span className="text-phantom-text">{hw.gpu || '未偵測到'}</span>
                </div>
                {hw.vram_mb > 0 && (
                  <div className="flex items-center justify-between text-sm">
                    <span className="text-phantom-muted">VRAM</span>
                    <span className="text-phantom-text">{(hw.vram_mb / 1024).toFixed(1)} GB</span>
                  </div>
                )}
                <div className="flex items-center justify-between text-sm">
                  <span className="text-phantom-muted">RAM</span>
                  <span className="text-phantom-text">{(hw.ram_mb / 1024).toFixed(0)} GB</span>
                </div>
                <div className="flex items-center justify-between text-sm">
                  <span className="text-phantom-muted">Ollama</span>
                  <span className={ollamaOnline ? 'text-phantom-success' : 'text-phantom-muted'}>
                    {ollamaOnline ? `在線 (${hw.ollama_models.length} 個模型)` : '未偵測到'}
                  </span>
                </div>

                {/* Providers */}
                {scan.providers.length > 0 && (
                  <>
                    <div className="border-t border-phantom-border pt-3">
                      <span className="text-xs text-phantom-muted">已偵測到的 Provider</span>
                    </div>
                    <div className="flex flex-wrap gap-2">
                      {scan.providers.map((p) => (
                        <span
                          key={`${p.name}-${p.source}`}
                          className="inline-flex items-center gap-1 bg-phantom-success/10 text-phantom-success text-xs px-2.5 py-1 rounded-full border border-phantom-success/20"
                        >
                          <span className="w-1.5 h-1.5 rounded-full bg-phantom-success" />
                          {p.displayLabel}
                        </span>
                      ))}
                    </div>
                  </>
                )}

                {!hasAnyProvider && (
                  <div className="border-t border-phantom-border pt-3">
                    <p className="text-xs text-phantom-warning">
                      未偵測到任何 Provider。啟動後可透過聊天設定。
                    </p>
                  </div>
                )}
              </div>
            )}

            {/* Manual API key entry */}
            <div className="bg-phantom-card border border-phantom-border rounded-lg p-4 mb-4">
              <div className="text-sm text-phantom-text mb-1">新增 API Key</div>
              <p className="text-xs text-phantom-muted mb-3">至少需要一個 Provider 才能使用 AI 功能</p>
              <div className="space-y-1.5">
                {MANUAL_PROVIDERS.filter(p => !detectedNames.has(p.key)).map((p) => {
                  const isExpanded = expandedProvider === p.key;
                  const savedKey = manualKeys[p.key] ?? '';
                  return (
                    <div key={p.key} className="border border-phantom-border rounded-lg overflow-hidden">
                      <button
                        onClick={() => setExpandedProvider(isExpanded ? null : p.key)}
                        className="w-full flex items-center justify-between px-3 py-2.5 bg-phantom-bg hover:bg-phantom-card transition"
                      >
                        <div className="flex items-center gap-2">
                          <span className={`w-1.5 h-1.5 rounded-full ${savedKey ? 'bg-phantom-success' : 'bg-phantom-muted'}`} />
                          <span className="text-sm text-phantom-text">{p.label}</span>
                          <span className="text-xs text-phantom-muted">{p.desc}</span>
                        </div>
                        <span className="text-xs text-phantom-muted">{isExpanded ? '▲' : '▼'}</span>
                      </button>
                      {isExpanded && (
                        <div className="px-3 pb-3 pt-2 bg-phantom-bg border-t border-phantom-border space-y-2">
                          <div className="flex gap-2">
                            <input
                              type="password"
                              placeholder={`貼上 ${p.label} API Key`}
                              value={savedKey}
                              onChange={(e) => setManualKeys(k => ({ ...k, [p.key]: e.target.value }))}
                              className="flex-1 bg-phantom-card border border-phantom-border rounded px-2.5 py-1.5 text-xs text-phantom-text placeholder:text-phantom-muted focus:border-phantom-primary outline-none font-mono"
                            />
                            <button
                              onClick={() => openLogin(p.url)}
                              className="text-xs text-phantom-primary hover:underline whitespace-nowrap px-1"
                            >
                              取得 Key →
                            </button>
                          </div>
                          {savedKey && (
                            <p className="text-phantom-success text-xs">✓ 已輸入，啟動時會寫入設定</p>
                          )}
                        </div>
                      )}
                    </div>
                  );
                })}
              </div>
            </div>

            {/* Launch button */}
            <button
              onClick={launch}
              className="w-full bg-phantom-primary text-phantom-bg py-3 rounded-lg font-semibold text-lg hover:brightness-110 transition"
            >
              啟動 Phantom Mesh
            </button>
            <button
              onClick={() => {
                localStorage.setItem(ONBOARDED_KEY, 'true');
                onComplete();
              }}
              className="w-full mt-2 text-phantom-muted text-sm py-2 hover:text-phantom-text transition"
            >
              跳過設定，直接進入
            </button>
            <p className="text-center text-phantom-muted text-xs mt-4">
              啟動後可透過聊天隨時新增 Provider、設定叢集、連接 Telegram 等
            </p>
          </>
        )}

        {/* Launching spinner */}
        {phase === 'launching' && (
          <div className="bg-phantom-card border border-phantom-border rounded-lg p-4 mb-4">
            <div className="flex items-center gap-3 text-sm">
              <div className="w-4 h-4 border-2 border-phantom-primary border-t-transparent rounded-full animate-spin" />
              <span className="text-phantom-text">{launchMsg}</span>
            </div>
          </div>
        )}

        {/* Success */}
        {phase === 'success' && (
          <div className="bg-phantom-success/10 border border-phantom-success/30 rounded-lg p-6 mb-4 text-center">
            <p className="text-phantom-success font-semibold text-lg mb-1">Phantom Mesh 已啟動！</p>
            <p className="text-phantom-muted text-xs mb-3">
              {countdown > 0 ? `${countdown} 秒後進入主介面...` : '跳轉中...'}
            </p>
            <button onClick={onComplete} className="text-phantom-primary text-sm hover:underline">
              立即進入
            </button>
          </div>
        )}

        {/* Error */}
        {phase === 'error' && (
          <div className="bg-phantom-danger/10 border border-phantom-danger/30 rounded-lg p-4 mb-4">
            <p className="text-phantom-danger text-sm mb-2">啟動失敗</p>
            <p className="text-phantom-muted text-xs mb-3">{error}</p>
            <div className="flex gap-2">
              <button
                onClick={() => { setError(null); setPhase('ready'); }}
                className="bg-phantom-danger/20 text-phantom-danger px-4 py-1.5 rounded text-xs hover:bg-phantom-danger/30 transition"
              >
                重試
              </button>
              <button
                onClick={() => {
                  localStorage.setItem(ONBOARDED_KEY, 'true');
                  onComplete();
                }}
                className="bg-phantom-card text-phantom-muted px-4 py-1.5 rounded text-xs hover:text-phantom-text transition"
              >
                跳過，直接進入
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
