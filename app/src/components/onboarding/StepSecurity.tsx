import { useState } from 'react';
import { OnboardingData, UserIdentity } from './types';

interface Props {
  data: OnboardingData;
  updateData: (partial: Partial<OnboardingData>) => void;
  onNext: () => void;
  onBack: () => void;
}

export default function StepSecurity({ data, updateData, onNext, onBack }: Props) {
  const [identity, setIdentity] = useState<UserIdentity | null>(data.identity);
  const [pin, setPin] = useState('');
  const [confirmPin, setConfirmPin] = useState('');

  const canProceed = pin.length === 6 && pin === confirmPin;

  const handlePinInput = (value: string) => {
    const digits = value.replace(/\D/g, '').slice(0, 6);
    setPin(digits);
  };

  const handleConfirmInput = (value: string) => {
    const digits = value.replace(/\D/g, '').slice(0, 6);
    setConfirmPin(digits);
  };

  const handleSubmit = () => {
    if (!canProceed) return;
    updateData({ vaultPin: pin });
    onNext();
  };

  return (
    <div>
      <h2 className="text-2xl font-bold text-white mb-2">KeyVault 安全設定</h2>
      <p className="text-phantom-muted text-sm mb-6">
        登入帳號以識別此 Hub 的擁有者，並設定 Vault PIN。
      </p>

      {/* OAuth Sign-In */}
      <div className="bg-phantom-card border border-phantom-border rounded-lg p-4 mb-4">
        <div className="text-sm text-phantom-text mb-3">帳號登入</div>

        {identity ? (
          <div className="flex items-center gap-3 py-2">
            {identity.avatar_url && (
              <img src={identity.avatar_url} alt="" className="w-8 h-8 rounded-full" />
            )}
            <div>
              <div className="text-sm text-white">{identity.display_name}</div>
              <div className="text-xs text-phantom-muted">{identity.email}</div>
            </div>
            <span className="text-phantom-success text-xs ml-auto">已登入</span>
          </div>
        ) : (
          <div className="space-y-2">
            {/* Google / Apple OAuth — coming soon */}
            <div className="flex items-center gap-2 bg-white/5 text-phantom-muted px-4 py-2.5 rounded-lg
                           font-medium text-sm border border-phantom-border cursor-not-allowed select-none">
              <span>G</span>
              <span className="flex-1">使用 Google 登入</span>
              <span className="text-xs bg-phantom-border/50 px-1.5 py-0.5 rounded">即將支援</span>
            </div>
            <div className="flex items-center gap-2 bg-white/5 text-phantom-muted px-4 py-2.5 rounded-lg
                           font-medium text-sm border border-phantom-border cursor-not-allowed select-none">
              <span></span>
              <span className="flex-1">使用 Apple 登入</span>
              <span className="text-xs bg-phantom-border/50 px-1.5 py-0.5 rounded">即將支援</span>
            </div>
          </div>
        )}

        {!identity && (
          <p className="text-phantom-muted text-xs mt-3">
            可選：不登入也能繼續，但部分功能（如多裝置同步）將無法使用。
          </p>
        )}
      </div>

      {/* Vault PIN */}
      <div className="bg-phantom-card border border-phantom-border rounded-lg p-4 mb-4">
        <label className="block text-sm text-phantom-text mb-3">Vault PIN（6 位數字）</label>
        <input
          type="password"
          inputMode="numeric"
          maxLength={6}
          value={pin}
          onChange={e => handlePinInput(e.target.value)}
          placeholder="● ● ● ● ● ●"
          className="w-full bg-phantom-bg border border-phantom-border rounded px-3 py-2 text-white text-center text-lg tracking-[0.5em]
                     focus:outline-none focus:border-phantom-primary font-mono"
        />

        <label className="block text-sm text-phantom-text mt-4 mb-1.5">確認 PIN</label>
        <input
          type="password"
          inputMode="numeric"
          maxLength={6}
          value={confirmPin}
          onChange={e => handleConfirmInput(e.target.value)}
          placeholder="● ● ● ● ● ●"
          className="w-full bg-phantom-bg border border-phantom-border rounded px-3 py-2 text-white text-center text-lg tracking-[0.5em]
                     focus:outline-none focus:border-phantom-primary font-mono"
        />
        {confirmPin.length > 0 && confirmPin.length === 6 && pin !== confirmPin && (
          <p className="text-phantom-danger text-xs mt-1">PIN 不一致</p>
        )}
      </div>

      <div className="bg-phantom-warning/10 border border-phantom-warning/30 rounded-lg px-4 py-3 mb-6">
        <p className="text-phantom-warning text-xs">
          PIN 遺失後需重新設定，已加密的資料將無法恢復。
        </p>
      </div>

      <div className="flex justify-between">
        <button
          onClick={onBack}
          className="text-phantom-muted hover:text-white text-sm px-4 py-2 transition"
        >
          ← 上一步
        </button>
        <button
          onClick={handleSubmit}
          disabled={!canProceed}
          className="bg-phantom-primary text-phantom-bg px-6 py-2.5 rounded-lg font-medium
                     disabled:opacity-40 disabled:cursor-not-allowed hover:brightness-110 transition"
        >
          設定完成 →
        </button>
      </div>
    </div>
  );
}
