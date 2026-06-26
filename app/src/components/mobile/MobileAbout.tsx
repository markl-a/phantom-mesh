import { useEffect, useState } from "react";
import { ExternalLink, RotateCcw } from "lucide-react";
import { getVersion as getAppVersion } from "@tauri-apps/api/app";
import { getVersion } from "../../lib/api";

// SPEC-34 Screen 14 (About / 關於): version + build SHA, OSS link rows,
// and a "restart onboarding" entrance. Backend-free except the optional
// get_version probe (degrades gracefully on a thin mobile client).

const LINKS: { label: string; url: string }[] = [
  { label: "Source（GitHub）", url: "https://github.com/markl-a/phantom-mesh" },
  { label: "License（AGPL-3.0）", url: "https://github.com/markl-a/phantom-mesh/blob/main/LICENSE" },
];

export default function MobileAbout() {
  const [version, setVersion] = useState<string | null>(null);
  const [commit, setCommit] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    getVersion()
      .then((v) => {
        if (!alive) return;
        setVersion(v.version);
        setCommit(v.commit);
      })
      .catch(async () => {
        if (!alive) return; // unmounted while get_version was pending
        // get_version command not wired (common on the thin mobile client).
        // Fall back to the Tauri app version from tauri.conf so we still show
        // a real version instead of a bare dash (commit stays unknown).
        try {
          const v = await getAppVersion();
          if (alive) setVersion(v);
        } catch { /* not a Tauri runtime — leave the dash */ }
      });
    return () => { alive = false; };
  }, []);

  const restartOnboarding = () => {
    try {
      localStorage.removeItem("phantom_mesh_v2_onboarded");
      localStorage.removeItem("phantom_mesh_v2_onboarded_mode");
    } catch { /* localStorage may be restricted */ }
    // Reload so App re-evaluates onboarding state and shows MobileFirstLaunch.
    window.location.assign("/");
  };

  return (
    <div className="flex flex-col h-full overflow-y-auto space-y-4">
      {/* Version block */}
      <div className="bg-phantom-card border border-phantom-border rounded-lg px-4 py-3">
        <div className="text-sm font-medium text-phantom-text">Phantom Mesh</div>
        <div className="text-xs text-phantom-muted mt-1 font-mono">
          {version ? `v${version}` : "—"}
          {commit ? ` · ${commit.slice(0, 8)}` : ""}
        </div>
        <div className="text-xs text-phantom-muted mt-0.5 font-mono">ai.phantommesh.app</div>
      </div>

      {/* OSS link rows */}
      <div className="space-y-2">
        {LINKS.map((l) => (
          <a
            key={l.url}
            href={l.url}
            target="_blank"
            rel="noreferrer noopener"
            aria-label={`${l.label}，外部連結`}
            className="flex items-center justify-between bg-phantom-card border border-phantom-border rounded-lg px-4 py-3 hover:border-phantom-primary transition"
          >
            <span className="text-sm text-phantom-text">{l.label}</span>
            <ExternalLink size={16} className="text-phantom-muted" />
          </a>
        ))}
      </div>

      {/* Restart onboarding */}
      <button
        onClick={restartOnboarding}
        className="flex items-center gap-2 bg-phantom-card border border-phantom-border rounded-lg px-4 py-3 text-sm text-phantom-text hover:border-phantom-primary transition"
      >
        <RotateCcw size={16} className="text-phantom-muted" />
        重啟 onboarding（重新設定）
      </button>
    </div>
  );
}
