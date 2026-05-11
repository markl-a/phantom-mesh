import { useState, useEffect } from 'react';
import { safeInvoke as invoke } from '../../lib/tauri-compat';
import { QRCodeSVG } from 'qrcode.react';
import { OnboardingData, QrPayload } from './types';

interface Props {
  data: OnboardingData;
  updateData: (partial: Partial<OnboardingData>) => void;
  onNext: () => void;
  onBack: () => void;
}

export default function StepNetwork({ data, updateData, onNext, onBack }: Props) {
  const [clusterEnabled, setClusterEnabled] = useState(data.clusterEnabled);
  const [telegramEnabled, setTelegramEnabled] = useState(!!data.telegramToken);
  const [telegramToken, setTelegramToken] = useState(data.telegramToken);
  const [telegramValid, setTelegramValid] = useState<boolean | null>(null);
  const [telegramValidating, setTelegramValidating] = useState(false);
  const [qrPayload, setQrPayload] = useState<QrPayload | null>(data.qrPayload);

  // Generate QR data on mount — resolve LAN IP for mobile access
  useEffect(() => {
    if (qrPayload) return;
    const port = data.hardwareScan?.available_port ?? 7878;
    const authKey = crypto.randomUUID().replace(/-/g, '').slice(0, 32);
    const nodeId = `desktop-${crypto.randomUUID().slice(0, 8)}`;
    invoke<string>('get_local_ip').then(ip => {
      return invoke<QrPayload>('generate_qr_data', {
        hubUrl: `http://${ip}:${port}`,
        authKey,
        nodeId,
      });
    }).then(payload => {
      setQrPayload(payload);
      updateData({ qrPayload: payload });
    }).catch(() => {});
  }, []);

  const validateTelegram = async () => {
    if (!telegramToken) return;
    setTelegramValidating(true);
    try {
      const resp = await fetch(`https://api.telegram.org/bot${telegramToken}/getMe`);
      const json = await resp.json();
      setTelegramValid(json.ok === true);
    } catch {
      setTelegramValid(false);
    }
    setTelegramValidating(false);
  };

  const handleNext = () => {
    updateData({
      clusterEnabled,
      telegramToken: telegramEnabled ? telegramToken : '',
      qrPayload,
    });
    onNext();
  };

  return (
    <div>
      <h2 className="text-2xl font-bold text-white mb-2">網路與叢集</h2>
      <p className="text-phantom-muted text-sm mb-6">
        選擇性設定。可以之後再從設定頁調整。
      </p>

      {/* QR Code Section */}
      <div className="bg-phantom-card border border-phantom-border rounded-lg p-4 mb-4">
        <h3 className="text-sm font-semibold text-phantom-text mb-3">Mobile Worker 連線</h3>
        <div className="flex gap-4 items-start">
          <div className="bg-white p-2 rounded-lg shrink-0">
            {qrPayload ? (
              <QRCodeSVG
                value={JSON.stringify(qrPayload)}
                size={120}
                level="M"
              />
            ) : (
              <div className="w-[120px] h-[120px] flex items-center justify-center text-phantom-muted text-xs">
                產生中...
              </div>
            )}
          </div>
          <div className="text-xs space-y-2 min-w-0">
            <p className="text-phantom-muted">用 Phantom Mesh Worker App 掃描此 QR Code 即可連線</p>
            {qrPayload && (
              <>
                <div>
                  <span className="text-phantom-muted">Hub URL: </span>
                  <code className="text-phantom-primary break-all">{qrPayload.hub_url}</code>
                </div>
                <div>
                  <span className="text-phantom-muted">Auth Key: </span>
                  <code className="text-phantom-primary break-all">{qrPayload.auth_key}</code>
                </div>
              </>
            )}
          </div>
        </div>
      </div>

      {/* Cluster Toggle */}
      <div className="bg-phantom-card border border-phantom-border rounded-lg p-4 mb-4">
        <label className="flex items-center justify-between cursor-pointer">
          <span className="text-sm text-phantom-text">要組建叢集嗎？</span>
          <input
            type="checkbox"
            checked={clusterEnabled}
            onChange={e => setClusterEnabled(e.target.checked)}
            className="accent-phantom-primary"
          />
        </label>
        {clusterEnabled && (
          <p className="text-phantom-muted text-xs mt-2">
            叢集功能將自動偵測區域網路中的其他 Phantom Mesh 節點。可稍後在叢集頁面管理。
          </p>
        )}
      </div>

      {/* Telegram Section */}
      <div className="bg-phantom-card border border-phantom-border rounded-lg p-4 mb-6">
        <label className="flex items-center justify-between cursor-pointer mb-2">
          <span className="text-sm text-phantom-text">Telegram Bot</span>
          <input
            type="checkbox"
            checked={telegramEnabled}
            onChange={e => setTelegramEnabled(e.target.checked)}
            className="accent-phantom-primary"
          />
        </label>
        {telegramEnabled && (
          <div className="flex gap-2">
            <input
              type="password"
              value={telegramToken}
              onChange={e => { setTelegramToken(e.target.value); setTelegramValid(null); }}
              placeholder="Bot Token (from @BotFather)"
              className="flex-1 bg-phantom-bg border border-phantom-border rounded px-3 py-1.5 text-white text-xs
                         focus:outline-none focus:border-phantom-primary"
            />
            <button
              onClick={validateTelegram}
              disabled={!telegramToken || telegramValidating}
              className="bg-phantom-primary/20 text-phantom-primary px-3 py-1.5 rounded text-xs
                         disabled:opacity-40 hover:bg-phantom-primary/30 transition"
            >
              {telegramValidating ? '...' : '驗證'}
            </button>
          </div>
        )}
        {telegramValid === true && (
          <p className="text-phantom-success text-xs mt-1">✓ Bot Token 有效</p>
        )}
        {telegramValid === false && (
          <p className="text-phantom-danger text-xs mt-1">✗ Token 無效</p>
        )}
      </div>

      <div className="flex justify-between">
        <button
          onClick={onBack}
          className="text-phantom-muted hover:text-white text-sm px-4 py-2 transition"
        >
          ← 上一步
        </button>
        <div className="flex gap-2">
          <button
            onClick={() => { updateData({ clusterEnabled: false, telegramToken: '' }); onNext(); }}
            className="text-phantom-muted hover:text-white text-sm px-4 py-2 transition"
          >
            跳過
          </button>
          <button
            onClick={handleNext}
            className="bg-phantom-primary text-phantom-bg px-6 py-2.5 rounded-lg font-medium
                       hover:brightness-110 transition"
          >
            下一步 →
          </button>
        </div>
      </div>
    </div>
  );
}
