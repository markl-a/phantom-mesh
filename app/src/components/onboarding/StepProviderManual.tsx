import { useState } from 'react';
import { safeInvoke as invoke } from '../../lib/tauri-compat';
import type { OnboardingData, ProviderConfig, ValidationResult, WizardStep } from './types';

interface Props {
  data: OnboardingData;
  updateData: (partial: Partial<OnboardingData>) => void;
  goNext: () => void;
  goBack: () => void;
  goTo: (step: WizardStep) => void;
}

const CLOUD_PROVIDERS = [
  { id: 'openai', label: 'OpenAI', type: 'openai' },
  { id: 'anthropic', label: 'Anthropic', type: 'anthropic' },
  { id: 'gemini', label: 'Google Gemini', type: 'gemini' },
  { id: 'groq', label: 'Groq', type: 'groq' },
  { id: 'deepseek', label: 'DeepSeek', type: 'deepseek' },
  { id: 'mistral', label: 'Mistral', type: 'mistral' },
  { id: 'xai', label: 'xAI (Grok)', type: 'xai' },
  { id: 'openrouter', label: 'OpenRouter', type: 'openai_compat' },
  { id: 'codex', label: 'Codex (OpenAI)', type: 'codex' },
  { id: 'opencode', label: 'OpenCode', type: 'opencode' },
] as const;

const BEDROCK_REGIONS = [
  'us-east-1',
  'us-west-2',
  'eu-west-1',
  'ap-northeast-1',
  'ap-southeast-1',
] as const;

export default function StepProviderManual({ data, updateData, goNext, goBack }: Props) {
  const [providers, setProviders] = useState<ProviderConfig[]>(data.manualProviders);
  const [ollamaEnabled, setOllamaEnabled] = useState(data.ollamaEnabled);
  const [ollamaEndpoint, setOllamaEndpoint] = useState(data.ollamaEndpoint);
  const [validating, setValidating] = useState<string | null>(null);

  // Azure local state
  const [azureEndpoint, setAzureEndpoint] = useState('');
  const [azureKey, setAzureKey] = useState('');
  const [azureValidated, setAzureValidated] = useState(false);

  // Bedrock local state
  const [bedrockRegion, setBedrockRegion] = useState('us-east-1');
  const [bedrockValidated, setBedrockValidated] = useState(false);

  const ollamaOnline = data.hardwareScan?.ollama_status === 'online';
  const discoveredCount = data.discoveredProviders.filter((p) => p.enabled).length;
  const hasProvider =
    providers.some((p) => p.validated) || ollamaEnabled || discoveredCount > 0 || azureValidated || bedrockValidated;

  const toggleProvider = (id: string, type: string) => {
    setProviders((prev) => {
      const exists = prev.find((p) => p.name === id);
      if (exists) return prev.filter((p) => p.name !== id);
      return [...prev, { name: id, apiKey: '', providerType: type, validated: false, models: [] }];
    });
  };

  const updateKey = (name: string, apiKey: string) => {
    setProviders((prev) =>
      prev.map((p) => (p.name === name ? { ...p, apiKey, validated: false, models: [] } : p))
    );
  };

  const validateKey = async (name: string) => {
    const provider = providers.find((p) => p.name === name);
    if (!provider || !provider.apiKey) return;

    setValidating(name);
    try {
      const result = await invoke<ValidationResult>('validate_api_key', {
        provider: provider.providerType,
        key: provider.apiKey,
      });
      setProviders((prev) =>
        prev.map((p) =>
          p.name === name ? { ...p, validated: result.ok, models: result.models } : p
        )
      );
    } catch {
      setProviders((prev) =>
        prev.map((p) => (p.name === name ? { ...p, validated: false, models: [] } : p))
      );
    }
    setValidating(null);
  };

  const validateAzure = async () => {
    if (!azureEndpoint || !azureKey) return;
    setValidating('azure');
    try {
      const result = await invoke<ValidationResult>('validate_api_key', {
        provider: 'azure',
        key: `${azureEndpoint}|${azureKey}`,
      });
      setAzureValidated(result.ok);
      if (result.ok) {
        // Add as a manual provider entry
        setProviders((prev) => {
          const filtered = prev.filter((p) => p.name !== 'azure_openai');
          return [
            ...filtered,
            {
              name: 'azure_openai',
              apiKey: azureKey,
              providerType: 'azure_openai',
              validated: true,
              models: result.models,
              endpoint: azureEndpoint,
            },
          ];
        });
      }
    } catch {
      setAzureValidated(false);
    }
    setValidating(null);
  };

  const validateBedrock = async () => {
    setValidating('bedrock');
    try {
      const result = await invoke<ValidationResult>('validate_api_key', {
        provider: 'bedrock',
        key: bedrockRegion,
      });
      setBedrockValidated(result.ok);
      if (result.ok) {
        setProviders((prev) => {
          const filtered = prev.filter((p) => p.name !== 'bedrock');
          return [
            ...filtered,
            {
              name: 'bedrock',
              apiKey: '',
              providerType: 'bedrock',
              validated: true,
              models: result.models,
              region: bedrockRegion,
            },
          ];
        });
      }
    } catch {
      setBedrockValidated(false);
    }
    setValidating(null);
  };

  const handleNext = () => {
    updateData({ manualProviders: providers, ollamaEnabled, ollamaEndpoint });
    goNext();
  };

  return (
    <div>
      <h2 className="text-2xl font-bold text-white mb-2">手動新增 API Key</h2>
      <p className="text-phantom-muted text-sm mb-6">
        輸入 API Key 來新增雲端 Provider。已自動偵測的 Provider 不需重複設定。
      </p>

      {/* Discovered providers badges */}
      {discoveredCount > 0 && (
        <div className="flex flex-wrap gap-2 mb-4">
          {data.discoveredProviders
            .filter((p) => p.enabled)
            .map((p) => (
              <span
                key={`${p.name}-${p.source}`}
                className="inline-flex items-center gap-1 bg-green-900/30 text-green-400 text-xs px-2.5 py-1 rounded-full border border-green-700/50"
              >
                <span>✓</span>
                {p.displayLabel}
              </span>
            ))}
        </div>
      )}

      {/* Ollama Section */}
      {ollamaOnline && (
        <div className="bg-phantom-card border border-phantom-border rounded-lg p-4 mb-4">
          <div className="flex items-center justify-between mb-2">
            <div className="flex items-center gap-2">
              <span className="text-phantom-success text-sm">✓ 已偵測到 Ollama</span>
              <span className="text-phantom-muted text-xs">
                {data.hardwareScan?.ollama_models.length} 個模型
              </span>
            </div>
            <label className="flex items-center gap-2 cursor-pointer">
              <input
                type="checkbox"
                checked={ollamaEnabled}
                onChange={(e) => setOllamaEnabled(e.target.checked)}
                className="accent-phantom-primary"
              />
              <span className="text-sm text-phantom-text">啟用</span>
            </label>
          </div>
          {ollamaEnabled && (
            <div>
              <input
                type="text"
                value={ollamaEndpoint}
                onChange={(e) => setOllamaEndpoint(e.target.value)}
                className="w-full bg-phantom-bg border border-phantom-border rounded px-3 py-1.5 text-white text-xs
                           focus:outline-none focus:border-phantom-primary mt-1"
              />
              {data.hardwareScan?.ollama_models && data.hardwareScan.ollama_models.length > 0 && (
                <div className="flex flex-wrap gap-1 mt-2">
                  {data.hardwareScan.ollama_models.map((m) => (
                    <span
                      key={m}
                      className="bg-phantom-bg text-phantom-muted text-xs px-2 py-0.5 rounded"
                    >
                      {m}
                    </span>
                  ))}
                </div>
              )}
            </div>
          )}
        </div>
      )}

      {/* Cloud Providers */}
      <h3 className="text-sm font-medium text-gray-300 mb-2">雲端 Provider</h3>
      <div className="grid grid-cols-3 gap-3 mb-4">
        {CLOUD_PROVIDERS.map((cp) => {
          const active = providers.find((p) => p.name === cp.id);
          return (
            <div key={cp.id} className="bg-phantom-card border border-phantom-border rounded-lg p-3">
              <label className="flex items-center gap-2 cursor-pointer mb-2">
                <input
                  type="checkbox"
                  checked={!!active}
                  onChange={() => toggleProvider(cp.id, cp.type)}
                  className="accent-phantom-primary"
                />
                <span className="text-sm text-phantom-text">{cp.label}</span>
                {active?.validated && (
                  <span className="text-phantom-success text-xs ml-auto">✓</span>
                )}
              </label>
              {active && (
                <div className="flex gap-2">
                  <input
                    type="password"
                    value={active.apiKey}
                    onChange={(e) => updateKey(cp.id, e.target.value)}
                    placeholder="API Key"
                    className="flex-1 bg-phantom-bg border border-phantom-border rounded px-2 py-1 text-white text-xs
                               focus:outline-none focus:border-phantom-primary"
                  />
                  <button
                    onClick={() => validateKey(cp.id)}
                    disabled={!active.apiKey || validating === cp.id}
                    className="bg-phantom-primary/20 text-phantom-primary px-3 py-1 rounded text-xs
                               disabled:opacity-40 hover:bg-phantom-primary/30 transition"
                  >
                    {validating === cp.id ? '...' : '驗證'}
                  </button>
                </div>
              )}
            </div>
          );
        })}
      </div>

      {/* Azure OpenAI */}
      <h3 className="text-sm font-medium text-gray-300 mb-2">Azure OpenAI</h3>
      <div className="bg-phantom-card border border-phantom-border rounded-lg p-4 mb-4">
        <div className="space-y-2">
          <input
            type="text"
            value={azureEndpoint}
            onChange={(e) => setAzureEndpoint(e.target.value)}
            placeholder="https://your-resource.openai.azure.com"
            className="w-full bg-phantom-bg border border-phantom-border rounded px-3 py-1.5 text-white text-xs
                       focus:outline-none focus:border-phantom-primary"
          />
          <div className="flex gap-2">
            <input
              type="password"
              value={azureKey}
              onChange={(e) => setAzureKey(e.target.value)}
              placeholder="API Key"
              className="flex-1 bg-phantom-bg border border-phantom-border rounded px-2 py-1 text-white text-xs
                         focus:outline-none focus:border-phantom-primary"
            />
            <button
              onClick={validateAzure}
              disabled={!azureEndpoint || !azureKey || validating === 'azure'}
              className="bg-phantom-primary/20 text-phantom-primary px-3 py-1 rounded text-xs
                         disabled:opacity-40 hover:bg-phantom-primary/30 transition"
            >
              {validating === 'azure' ? '...' : '驗證'}
            </button>
          </div>
          {azureValidated && (
            <span className="text-phantom-success text-xs">✓ 已驗證</span>
          )}
        </div>
      </div>

      {/* AWS Bedrock */}
      <h3 className="text-sm font-medium text-gray-300 mb-2">AWS Bedrock</h3>
      <div className="bg-phantom-card border border-phantom-border rounded-lg p-4 mb-4">
        <div className="flex gap-2 items-center">
          <select
            value={bedrockRegion}
            onChange={(e) => {
              setBedrockRegion(e.target.value);
              setBedrockValidated(false);
            }}
            className="bg-phantom-bg border border-phantom-border rounded px-3 py-1.5 text-white text-xs
                       focus:outline-none focus:border-phantom-primary"
          >
            {BEDROCK_REGIONS.map((r) => (
              <option key={r} value={r}>
                {r}
              </option>
            ))}
          </select>
          <button
            onClick={validateBedrock}
            disabled={validating === 'bedrock'}
            className="bg-phantom-primary/20 text-phantom-primary px-3 py-1 rounded text-xs
                       disabled:opacity-40 hover:bg-phantom-primary/30 transition"
          >
            {validating === 'bedrock' ? '...' : '檢查 AWS 憑證'}
          </button>
          {bedrockValidated && (
            <span className="text-phantom-success text-xs">✓ 已驗證</span>
          )}
        </div>
        <p className="text-phantom-muted text-xs mt-2">
          使用本地 AWS 憑證鏈 (環境變數 / ~/.aws/credentials)
        </p>
      </div>

      <p className="text-phantom-muted text-xs mb-6">
        Phantom Mesh 支援多級路由：本地 Ollama → 雲端 Primary → 雲端 Fallback → OpenRouter 聚合 → 離線快取
      </p>

      <div className="flex justify-between items-center">
        <button
          onClick={() => { updateData({ manualProviders: providers, ollamaEnabled, ollamaEndpoint }); goBack(); }}
          className="text-phantom-muted hover:text-white text-sm px-4 py-2 transition"
        >
          ← 上一步
        </button>
        <div className="flex items-center gap-3">
          {!hasProvider && (
            <button
              onClick={handleNext}
              className="text-phantom-muted hover:text-white text-xs px-3 py-1.5 transition"
            >
              跳過
            </button>
          )}
          <button
            onClick={handleNext}
            className={`px-6 py-2.5 rounded-lg font-medium transition ${
              hasProvider
                ? 'bg-phantom-primary text-phantom-bg hover:brightness-110'
                : 'bg-phantom-primary/40 text-phantom-bg/70 hover:bg-phantom-primary/60'
            }`}
          >
            下一步 →
          </button>
        </div>
      </div>
    </div>
  );
}
