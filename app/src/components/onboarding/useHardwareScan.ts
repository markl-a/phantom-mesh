import { useState, useEffect } from 'react';
import { safeInvoke as invoke } from '../../lib/tauri-compat';
import { HardwareScanResult } from './types';

export type ScanStatus = 'idle' | 'scanning' | 'done' | 'error';

export function useHardwareScan(autoStart: boolean) {
  const [result, setResult] = useState<HardwareScanResult | null>(null);
  const [status, setStatus] = useState<ScanStatus>('idle');
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!autoStart) return;

    setStatus('scanning');
    invoke<HardwareScanResult>('scan_hardware')
      .then((r) => {
        setResult(r);
        setStatus('done');
      })
      .catch((e) => {
        setError(String(e));
        setStatus('error');
      });
  }, [autoStart]);

  return { result, status, error };
}
