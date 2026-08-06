// MobileJoinCluster — 3-step form to join an existing spectyn-mesh cluster.
// Per spec v2 §4.3. Triggered by mode picker [B].
//
// Step 1: Coordinator URL (mDNS-discovered or paste)
// Step 2: Cluster secret (paste or scan QR — QR deferred to v0.6.1)
// Step 3: Device name (auto-filled, editable)
//
// On Done: HMAC-ping the coordinator's /rpc/ping to verify; commit to
// clusterModeStore on success.

import { useState } from "react";
import { ChevronLeft, ChevronRight, Loader2 } from "lucide-react";
import { useClusterModeStore } from "../../stores/clusterModeStore";
// Wire helper around the Wave H1.1 Tauri commands `onboarding_advance`
// + `onboarding_rollback` (registered in src-tauri/src/lib.rs:572-573).
import { advanceUntil, rollbackOnboarding, loadSnapshot } from "../../lib/onboardingFsm";

interface DiscoveredHost { host: string; port: number; url: string }
interface Props {
  discovered?: DiscoveredHost;
  onDone: () => void;
  onCancel: () => void;
}

async function hmacSha256Hex(secret: string, body: string): Promise<string> {
  const enc = new TextEncoder();
  const key = await crypto.subtle.importKey(
    "raw", enc.encode(secret), { name: "HMAC", hash: "SHA-256" }, false, ["sign"],
  );
  const sig = await crypto.subtle.sign("HMAC", key, enc.encode(body));
  return Array.from(new Uint8Array(sig)).map(b => b.toString(16).padStart(2, "0")).join("");
}

// Derive a stable hex slug for the joined cluster — feeds the
// `OnboardingContext.clusterIdHash` field. SPEC-28 §7.5 explicitly
// requires this to be a derived/sanitised value (SHA-256 of the
// coordinator URL is fine; we are NOT persisting the raw URL or secret).
async function sha256Hex(input: string): Promise<string> {
  const enc = new TextEncoder();
  const buf = await crypto.subtle.digest("SHA-256", enc.encode(input));
  return Array.from(new Uint8Array(buf)).map(b => b.toString(16).padStart(2, "0")).join("");
}

async function testCoordinator(url: string, secret: string): Promise<{ ok: boolean; error?: string; name?: string }> {
  const base = url.replace(/\/+$/, "");
  const body = JSON.stringify({ node_name: "iphone-join-test", wire_version: 1 });
  try {
    const auth = await hmacSha256Hex(secret, body);
    // On iOS use the native fetch bridge; on Android use plugin-http.
    const isIOS = /iPad|iPhone|iPod/.test(navigator.userAgent || "");
    let respJson: { name?: string; spectyn_version?: string } | null = null;
    if (isIOS) {
      const { invoke } = await import("@tauri-apps/api/core");
      const r = await invoke<{ status: number; body: string }>("swift_cluster_fetch", {
        url: `${base}/rpc/ping`, method: "POST", body, auth,
      });
      if (r.status !== 200) return { ok: false, error: `HTTP ${r.status}: ${r.body.slice(0, 200)}` };
      respJson = JSON.parse(r.body);
    } else {
      const { fetch: tauriFetch } = await import("@tauri-apps/plugin-http");
      const r = await tauriFetch(`${base}/rpc/ping`, {
        method: "POST",
        headers: { "Content-Type": "application/json", "X-Cluster-Auth": auth },
        body,
      });
      if (!r.ok) return { ok: false, error: `HTTP ${r.status}: ${(await r.text()).slice(0, 200)}` };
      respJson = await r.json() as { name?: string; spectyn_version?: string };
    }
    return { ok: true, name: respJson?.name ?? "unknown" };
  } catch (e) {
    return { ok: false, error: String(e).slice(0, 200) };
  }
}

export default function MobileJoinCluster({ discovered, onDone, onCancel }: Props) {
  const store = useClusterModeStore();
  const [step, setStep] = useState(1);
  const [url, setUrl] = useState(discovered?.url ?? "");
  const [secret, setSecret] = useState("");
  const [deviceName, setDeviceName] = useState(() => {
    if (typeof navigator === "undefined") return "phone";
    if (/iPad/.test(navigator.userAgent)) return "iPad";
    if (/iPhone/.test(navigator.userAgent)) return "iPhone";
    return "mobile-device";
  });
  const [testing, setTesting] = useState(false);
  const [testError, setTestError] = useState<string | null>(null);

  const handleSubmit = async () => {
    setTesting(true);
    setTestError(null);
    const trimmedUrl = url.trim();
    const trimmedSecret = secret.trim();
    const res = await testCoordinator(trimmedUrl, trimmedSecret);
    if (!res.ok) {
      setTesting(false);
      setTestError(res.error ?? "unknown error");
      return;
    }
    // Persist client-side state
    store.setCoordinatorUrl(trimmedUrl);
    store.setClusterSecret(trimmedSecret);
    store.setEnabled(true);
    try {
      localStorage.setItem("spectyn_mesh_v2_onboarded", "true");
      localStorage.setItem("spectyn_mesh_v2_onboarded_mode", "join");
      localStorage.setItem("spectyn_mesh_device_name", deviceName);
    } catch (_e) { /* ignore */ }
    // Drive the SPEC-28 FSM up to `joined_cluster`, patching the
    // sanitised cluster_id_hash so downstream telemetry can attribute
    // events to this cluster without leaking the coordinator URL.
    try {
      const clusterIdHash = await sha256Hex(trimmedUrl);
      await advanceUntil("joined_cluster", { clusterIdHash });
    } catch (e) {
      // Don't block the user — they have a working cluster connection.
      // eslint-disable-next-line no-console
      console.warn("[MobileJoinCluster] advanceUntil failed (non-fatal)", e);
    }
    setTesting(false);
    onDone();
  };

  // Header back/cancel arrow. When the user backs out of step 1 we have
  // potentially just bumped the FSM into `joined_cluster` on a previous
  // submit attempt — fire the sanctioned `JoinedCluster → CreatedIdentity`
  // rollback so the FSM stays truthful. For step 2/3 we just step the
  // wizard UI back (no FSM change needed).
  const handleBackOrCancel = async () => {
    if (step !== 1) {
      setStep((s) => s - 1);
      return;
    }
    try {
      if (loadSnapshot().currentState === "joined_cluster") {
        await rollbackOnboarding();
      }
    } catch (e) {
      // eslint-disable-next-line no-console
      console.warn("[MobileJoinCluster] rollback failed (non-fatal)", e);
    }
    onCancel();
  };

  return (
    <div className="h-[100dvh] bg-spectyn-bg flex flex-col">
      {/* Header */}
      <header
        className="flex items-center px-4 py-3 border-b border-spectyn-border flex-shrink-0"
        style={{ paddingTop: "calc(env(safe-area-inset-top) + 0.75rem)" }}
      >
        <button onClick={handleBackOrCancel}
                aria-label="返回"
                className="p-2 -ml-2 text-spectyn-muted">
          <ChevronLeft size={22} />
        </button>
        <h2 className="flex-1 text-base font-semibold text-spectyn-text text-center">
          加入 cluster · 步驟 {step} / 3
        </h2>
        <div className="w-9" />
      </header>

      {/* Body */}
      <main className="flex-1 overflow-y-auto px-4 py-6">
        {step === 1 && (
          <>
            <h3 className="text-lg font-semibold text-spectyn-text mb-1">Coordinator URL</h3>
            <p className="text-sm text-spectyn-muted mb-4">
              你 mac 上 spectyn serve 監聽的位址 (mac 預設 :7878)
            </p>
            {discovered && (
              <button
                onClick={() => setUrl(discovered.url)}
                className="w-full mb-3 p-3 border border-spectyn-primary/50 bg-spectyn-primary/10 rounded-lg text-left"
              >
                <div className="text-xs text-spectyn-muted">本機網路發現</div>
                <div className="text-spectyn-text font-mono text-sm truncate">{discovered.host}:{discovered.port}</div>
              </button>
            )}
            <input
              type="url"
              value={url}
              onChange={e => setUrl(e.target.value)}
              placeholder="https://my-mac.local:7878"
              className="w-full p-3 bg-spectyn-bg-elevated border border-spectyn-border rounded-lg text-spectyn-text font-mono text-sm"
            />
            <p className="text-xs text-spectyn-muted mt-2">
              支援 http://、https:// 任一。如果 mac 在 Tailnet 上、用 Tailscale magic hostname 最穩。
            </p>
          </>
        )}

        {step === 2 && (
          <>
            <h3 className="text-lg font-semibold text-spectyn-text mb-1">Cluster Secret</h3>
            <p className="text-sm text-spectyn-muted mb-4">
              從 mac 上 <code className="bg-spectyn-bg-elevated px-1 rounded">~/.spectyn-mesh/agents.toml</code> 找
              <code className="bg-spectyn-bg-elevated px-1 rounded">cluster_secret = "..."</code>
            </p>
            <input
              type="password"
              value={secret}
              onChange={e => setSecret(e.target.value)}
              placeholder="貼上 secret"
              className="w-full p-3 bg-spectyn-bg-elevated border border-spectyn-border rounded-lg text-spectyn-text font-mono text-sm"
              autoComplete="off"
            />
            <p className="text-xs text-spectyn-muted mt-2">
              這個 secret 用 HMAC-SHA256 簽 RPC，不會明文傳。儲存在 iOS Keychain。
            </p>
          </>
        )}

        {step === 3 && (
          <>
            <h3 className="text-lg font-semibold text-spectyn-text mb-1">這個裝置叫什麼？</h3>
            <p className="text-sm text-spectyn-muted mb-4">
              其他裝置會看到這個名字 (e.g. mesh peer list 顯示)
            </p>
            <input
              type="text"
              value={deviceName}
              onChange={e => setDeviceName(e.target.value)}
              placeholder="iPhone-13-mini"
              className="w-full p-3 bg-spectyn-bg-elevated border border-spectyn-border rounded-lg text-spectyn-text"
            />
            <div className="mt-6 p-3 bg-spectyn-bg-elevated border border-spectyn-border rounded-lg text-xs text-spectyn-muted space-y-1">
              <div><span className="text-spectyn-muted">Coordinator:</span> <code className="text-spectyn-text">{url}</code></div>
              <div><span className="text-spectyn-muted">Secret:</span> <code className="text-spectyn-text">{"●".repeat(Math.min(secret.length, 16))}</code></div>
              <div><span className="text-spectyn-muted">Capabilities:</span> <code className="text-spectyn-text">camera, microphone, mobile-ios</code></div>
            </div>
            {testError && (
              <div className="mt-4 p-3 bg-red-500/10 border border-red-500/30 rounded-lg text-red-400 text-sm">
                ✗ {testError}
              </div>
            )}
          </>
        )}
      </main>

      {/* Footer button */}
      <footer className="border-t border-spectyn-border p-4 flex-shrink-0" style={{ paddingBottom: "calc(env(safe-area-inset-bottom) + 1rem)" }}>
        {step < 3 ? (
          <button
            onClick={() => setStep(s => s + 1)}
            disabled={step === 1 ? !url.trim() : !secret.trim()}
            className="w-full py-3 bg-spectyn-primary text-white rounded-lg font-semibold disabled:opacity-40 disabled:cursor-not-allowed flex items-center justify-center gap-2"
          >
            下一步 <ChevronRight size={18} />
          </button>
        ) : (
          <button
            onClick={handleSubmit}
            disabled={testing}
            className="w-full py-3 bg-spectyn-primary text-white rounded-lg font-semibold disabled:opacity-40 flex items-center justify-center gap-2"
          >
            {testing ? <><Loader2 size={18} className="animate-spin" /> 連線測試中...</> : "✓ 加入 cluster"}
          </button>
        )}
      </footer>
    </div>
  );
}
