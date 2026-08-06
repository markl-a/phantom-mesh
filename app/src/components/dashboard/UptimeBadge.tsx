import { useState, useEffect } from "react";

// ─── Helpers ──────────────────────────────────────────────────────────────────

function formatUptime(seconds: number): string {
  if (seconds < 60) return `${Math.floor(seconds)}s`;
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  if (h === 0) return `${m}m`;
  return `${h}h ${m}m`;
}

// ─── Component ────────────────────────────────────────────────────────────────

export default function UptimeBadge() {
  const [uptime, setUptime] = useState<number | null>(null);

  useEffect(() => {
    const fetchUptime = async () => {
      try {
        const res = await fetch("http://localhost:7878/health");
        if (!res.ok) return;
        const data = await res.json() as { uptime_seconds?: number; uptime?: number };
        const secs = data.uptime_seconds ?? data.uptime ?? null;
        if (typeof secs === "number") setUptime(secs);
      } catch {
        // Daemon may not be running — silently skip
      }
    };

    void fetchUptime();
    const interval = setInterval(() => void fetchUptime(), 30_000);
    return () => clearInterval(interval);
  }, []);

  if (uptime === null) return null;

  return (
    <span className="text-xs text-spectyn-muted flex items-center gap-1">
      <span className="w-1.5 h-1.5 rounded-full bg-spectyn-success" />
      {formatUptime(uptime)}
    </span>
  );
}
