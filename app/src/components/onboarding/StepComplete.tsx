import { useState, useEffect, useCallback } from 'react';
import { safeInvoke as invoke } from '../../lib/tauri-compat';
import { OnboardingData, DaemonStatus } from './types';

interface Props {
  data: OnboardingData;
  completeWizard: () => void;
  onComplete: () => void;
  onBack: () => void;
}

type LaunchPhase = 'idle' | 'writing_config' | 'starting_daemon' | 'storing_keys' | 'success' | 'error';

export default function StepComplete({ data, completeWizard, onComplete, onBack }: Props) {
  const [phase, setPhase] = useState<LaunchPhase>('idle');
  const [error, setError] = useState<string | null>(null);
  const [countdown, setCountdown] = useState(3);

  const port = data.hardwareScan?.available_port ?? 7878;

  const summaryItems = [
    { label: '帳號', value: data.identity?.email ?? '未登入' },
    { label: 'Daemon Port', value: String(port) },
    { label: 'KeyVault', value: '✓ 已設定 PIN' },
    { label: 'Providers', value: [
      ...(data.ollamaEnabled ? ['Ollama'] : []),
      ...data.discoveredProviders.filter(p => p.enabled).map(p => p.displayLabel),
      ...data.manualProviders.filter(p => p.validated).map(p => p.name),
    ].join(', ') || '無' },
    { label: '叢集', value: data.clusterEnabled ? '啟用' : '關閉' },
    { label: 'Telegram', value: data.telegramToken ? '已設定' : '未設定' },
    { label: '主 Hub', value: '✓ 此裝置' },
  ];

  const launch = useCallback(async () => {
    setPhase('writing_config');
    setError(null);

    try {
      // 1. Write config files
      const firstDiscovered = data.discoveredProviders.find(p => p.enabled);
      const firstManual = data.manualProviders.find(p => p.validated);
      const defaultProvider = data.ollamaEnabled
        ? 'ollama'
        : firstDiscovered?.name ?? firstManual?.name ?? 'ollama';
      const defaultModel = data.ollamaEnabled
        ? (data.hardwareScan?.ollama_models[0] ?? 'llama3')
        : firstDiscovered?.models?.[0] ?? firstManual?.models?.[0] ?? 'llama3';

      await invoke('write_config', {
        data: {
          port,
          discovered_providers: data.discoveredProviders
            .filter(p => p.enabled)
            .map(p => ({
              name: p.name,
              provider_type: p.providerType,
              tier: p.tier,
              token_source: p.source,
              base_url: null,
              env_key_name: null,
            })),
          manual_providers: data.manualProviders
            .filter(p => p.validated)
            .map(p => ({
              name: p.name,
              provider_type: p.providerType,
              api_key: p.apiKey,
              tier: 'payg',
              base_url: p.baseUrl ?? null,
              endpoint: p.endpoint ?? null,
              region: p.region ?? null,
            })),
          ollama_endpoint: data.ollamaEnabled ? data.ollamaEndpoint : null,
          default_agent_provider: defaultProvider,
          default_agent_model: defaultModel,
          auth_key: data.qrPayload?.auth_key ?? crypto.randomUUID().replace(/-/g, ''),
          telegram_token: data.telegramToken || null,
          identity_provider: data.identity?.provider ?? null,
          identity_sub: data.identity?.sub ?? null,
          identity_email: data.identity?.email ?? null,
          is_primary: true,  // First device is automatically Primary Hub
        },
      });

      // 2. Check runtime health (library mode — no sidecar needed)
      setPhase('starting_daemon');
      try {
        await invoke('get_conversations');
        // Runtime is ready — in-process mode
      } catch {
        // Try legacy daemon fallback
        const binaryPath = data.hardwareScan?.daemon_binary_path;
        if (binaryPath) {
          const status = await invoke<DaemonStatus>('launch_daemon', {
            vaultPin: data.vaultPin,
            port,
            binaryPath,
          });
          if (!status.ok) throw new Error('Runtime 啟動失敗，請檢查日誌');
        }
        // No binary + no runtime = proceed anyway (may init in background)
      }

      // 3. Try to store API keys in vault (best-effort)
      setPhase('storing_keys');
      try {
        const keysToStore: Record<string, string> = {};
        for (const p of data.manualProviders.filter(pr => pr.validated)) {
          keysToStore[`${p.name.toUpperCase()}_API_KEY`] = p.apiKey;
        }
        if (Object.keys(keysToStore).length > 0 && port) {
          const authToken = data.qrPayload?.auth_key ?? '';
          try {
            const response = await fetch(`http://localhost:${port}/vault/store-keys`, {
              method: 'POST',
              headers: {
                'Content-Type': 'application/json',
                ...(authToken && { 'Authorization': `Bearer ${authToken}` }),
              },
              body: JSON.stringify(keysToStore),
            });
            if (!response.ok) {
              console.warn(`Vault store returned ${response.status} — keys saved to .env as fallback`);
            }
          } catch (vaultErr) {
            console.warn('Vault store unavailable — keys saved to .env as fallback:', vaultErr);
          }
        }
      } catch {
        // Vault store is best-effort; .env fallback already written
      }

      // Persist identity to Tauri Store for post-onboarding access
      if (data.identity) {
        try {
          const { load } = await import('@tauri-apps/plugin-store');
          const store = await load('phantom-store.json');
          await store.set('user_identity', data.identity);
          await store.save();
        } catch {
          // Tauri Store is best-effort
        }
      }

      setPhase('success');
      completeWizard();
    } catch (e) {
      setError(String(e));
      setPhase('error');
    }
  }, [data, port, completeWizard]);

  // Countdown after success
  useEffect(() => {
    if (phase !== 'success') return;
    if (countdown <= 0) { onComplete(); return; }
    const timer = setTimeout(() => setCountdown(c => c - 1), 1000);
    return () => clearTimeout(timer);
  }, [phase, countdown, onComplete]);

  return (
    <div>
      <h2 className="text-2xl font-bold text-white mb-2">確認並啟動</h2>
      <p className="text-phantom-muted text-sm mb-6">確認設定後啟動 Phantom Mesh Daemon</p>

      {/* Summary */}
      <div className="bg-phantom-card border border-phantom-border rounded-lg p-4 mb-4">
        {summaryItems.map((item, i) => (
          <div key={i} className="flex justify-between py-1.5 text-sm border-b border-phantom-border last:border-0">
            <span className="text-phantom-muted">{item.label}</span>
            <span className="text-phantom-text">{item.value}</span>
          </div>
        ))}
      </div>

      {/* Launch status */}
      {phase !== 'idle' && phase !== 'error' && phase !== 'success' && (
        <div className="bg-phantom-card border border-phantom-border rounded-lg p-4 mb-4">
          <div className="flex items-center gap-3 text-sm">
            <div className="w-4 h-4 border-2 border-phantom-primary border-t-transparent rounded-full animate-spin" />
            <span className="text-phantom-text">
              {phase === 'writing_config' && '寫入設定檔...'}
              {phase === 'starting_daemon' && '啟動 Daemon...'}
              {phase === 'storing_keys' && '儲存金鑰至 Vault...'}
            </span>
          </div>
        </div>
      )}

      {/* Success */}
      {phase === 'success' && (
        <div className="bg-phantom-success/10 border border-phantom-success/30 rounded-lg p-4 mb-4 text-center">
          <div className="text-3xl mb-2">🎉</div>
          <p className="text-phantom-success font-semibold mb-1">Phantom Mesh 已成功啟動！</p>
          <p className="text-phantom-muted text-xs">
            {countdown > 0 ? `${countdown} 秒後自動跳轉...` : '跳轉中...'}
          </p>
          <button
            onClick={onComplete}
            className="text-phantom-primary text-xs mt-2 hover:underline"
          >
            立即前往
          </button>
        </div>
      )}

      {/* Error */}
      {phase === 'error' && (
        <div className="bg-phantom-danger/10 border border-phantom-danger/30 rounded-lg p-4 mb-4">
          <p className="text-phantom-danger text-sm mb-2">啟動失敗</p>
          <p className="text-phantom-muted text-xs mb-3">{error}</p>
          <button
            onClick={launch}
            className="bg-phantom-danger/20 text-phantom-danger px-4 py-1.5 rounded text-xs hover:bg-phantom-danger/30 transition"
          >
            重試
          </button>
        </div>
      )}

      {/* Action buttons */}
      {phase === 'idle' && (
        <div className="flex justify-between">
          <button
            onClick={onBack}
            className="text-phantom-muted hover:text-white text-sm px-4 py-2 transition"
          >
            ← 上一步
          </button>
          <button
            onClick={launch}
            className="bg-phantom-primary text-phantom-bg px-6 py-2.5 rounded-lg font-medium
                       hover:brightness-110 transition"
          >
            🚀 啟動 Phantom Mesh
          </button>
        </div>
      )}
    </div>
  );
}
