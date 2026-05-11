import { useEffect, useState } from 'react';
import { HardwareScanResult, GpuInfo, NpuInfo } from './types';
import { ScanStatus } from './useHardwareScan';

interface Props {
  scan: { result: HardwareScanResult | null; status: ScanStatus; error: string | null };
  onNext: () => void;
}

interface ScanItem {
  label: string;
  value: string;
  ok: boolean;
}

export default function StepWelcome({ scan, onNext }: Props) {
  const [visibleItems, setVisibleItems] = useState<number>(0);

  const formatGpu = (g: GpuInfo): string => {
    const vram = g.dedicated_mb > 0 ? Math.round(g.dedicated_mb / 1024) : 0;
    return vram > 0 ? `${g.name} (${vram} GB)` : g.name;
  };

  const gpuItems: ScanItem[] = scan.result
    ? (scan.result.gpus && scan.result.gpus.length > 0
        ? scan.result.gpus.map((g, i) => ({
            label: scan.result!.gpus.length === 1 ? 'GPU' : `GPU ${i + 1}`,
            value: formatGpu(g),
            ok: g.name !== 'CPU-only',
          }))
        : [{ label: 'GPU', value: scan.result.vram_mb > 0
              ? `${scan.result.gpu} (${Math.round(scan.result.vram_mb / 1024)} GB)`
              : scan.result.gpu, ok: scan.result.gpu !== 'CPU-only' }])
    : [];

  const npuItems: ScanItem[] = scan.result?.npus?.length
    ? scan.result.npus.map((n: NpuInfo) => ({
        label: 'NPU',
        value: n.tops > 0 ? `${n.name} (${n.tops} TOPS)` : n.name,
        ok: true,
      }))
    : [];

  const items: ScanItem[] = scan.result
    ? [
        ...gpuItems,
        ...npuItems,
        { label: 'RAM', value: `${Math.round(scan.result.ram_mb / 1024)} GB`, ok: scan.result.ram_mb >= 8192 },
        { label: 'Ollama', value: scan.result.ollama_status === 'online'
            ? `${scan.result.ollama_models.length} 個模型`
            : '未偵測到', ok: scan.result.ollama_status === 'online' },
        { label: 'Daemon', value: scan.result.daemon_binary_path ? '已找到' : '未找到', ok: !!scan.result.daemon_binary_path },
        { label: 'Port', value: `${scan.result.available_port}`, ok: true },
      ]
    : [];

  // Animate items appearing one by one
  useEffect(() => {
    if (scan.status !== 'done' || items.length === 0) return;
    if (visibleItems >= items.length) return;
    const timer = setTimeout(() => setVisibleItems(v => v + 1), 300);
    return () => clearTimeout(timer);
  }, [scan.status, visibleItems, items.length]);

  return (
    <div className="text-center">
      <h1 className="text-3xl font-bold text-white mb-2">歡迎來到 Phantom Mesh</h1>
      <p className="text-phantom-muted mb-8">你的 AI Agent 叢集，從這裡開始</p>

      <div className="grid grid-cols-3 gap-4 mb-8 text-sm">
        <div className="bg-phantom-card border border-phantom-border rounded-lg p-4">
          <div className="text-phantom-primary text-lg mb-1">53</div>
          <div className="text-phantom-muted">內建工具</div>
        </div>
        <div className="bg-phantom-card border border-phantom-border rounded-lg p-4">
          <div className="text-phantom-primary text-lg mb-1">11</div>
          <div className="text-phantom-muted">AI 引擎</div>
        </div>
        <div className="bg-phantom-card border border-phantom-border rounded-lg p-4">
          <div className="text-phantom-primary text-lg mb-1">48</div>
          <div className="text-phantom-muted">工作流程</div>
        </div>
      </div>

      <div className="bg-phantom-card border border-phantom-border rounded-lg p-4 mb-6 text-left">
        <div className="text-sm text-phantom-muted mb-3">
          {scan.status === 'scanning' && '正在掃描系統環境...'}
          {scan.status === 'done' && '系統掃描完成'}
          {scan.status === 'error' && `掃描失敗: ${scan.error}`}
          {scan.status === 'idle' && '準備掃描...'}
        </div>
        {scan.status === 'scanning' && (
          <div className="flex items-center gap-2 text-phantom-primary text-sm">
            <div className="w-4 h-4 border-2 border-phantom-primary border-t-transparent rounded-full animate-spin" />
            偵測中...
          </div>
        )}
        {items.slice(0, visibleItems).map((item, i) => (
          <div
            key={i}
            className="flex items-center justify-between py-1.5 text-sm border-b border-phantom-border last:border-0"
          >
            <span className="text-phantom-muted">{item.label}</span>
            <span className={`flex items-center gap-1.5 ${item.ok ? 'text-phantom-success' : 'text-phantom-warning'}`}>
              <span>{item.ok ? '✓' : '✗'}</span>
              {item.value}
            </span>
          </div>
        ))}
      </div>

      <button
        onClick={onNext}
        disabled={scan.status !== 'done'}
        className="bg-phantom-primary text-phantom-bg px-6 py-2.5 rounded-lg font-medium
                   disabled:opacity-40 disabled:cursor-not-allowed hover:brightness-110 transition"
      >
        開始設定 →
      </button>
    </div>
  );
}
