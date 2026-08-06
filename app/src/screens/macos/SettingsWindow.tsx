import { useEffect, useState, type ReactNode } from "react";
import {
  Cloud,
  Database,
  Download,
  FileText,
  Home,
  KeyRound,
  Keyboard,
  Languages,
  Lock,
  Palette,
  Plus,
  RotateCcw,
  ShieldCheck,
  Trash2,
  Upload,
  type LucideIcon,
} from "lucide-react";
import type { SettingsWindowProps, SettingsTab } from "./types";
import ClusterStatusDashboard from "./ClusterStatusDashboard";

const tabs: Array<{ id: SettingsTab; label: string; Icon: LucideIcon }> = [
  { id: "general", label: "一般", Icon: Home },
  { id: "cluster", label: "叢集", Icon: Cloud },
  { id: "providers", label: "供應商", Icon: KeyRound },
  { id: "privacy", label: "隱私", Icon: ShieldCheck },
];

function Note({ children }: { children: ReactNode }) {
  return <span className="text-xs text-spectyn-muted">{children}</span>;
}

function Section({
  title,
  icon: Icon,
  children,
}: {
  title: string;
  icon: LucideIcon;
  children: ReactNode;
}) {
  return (
    <section className="rounded-lg border border-spectyn-border bg-spectyn-bg/40 p-4">
      <h2 className="mb-4 flex items-center gap-2 text-sm font-semibold text-spectyn-text">
        <Icon size={16} className="text-spectyn-primary" />
        {title}
      </h2>
      {children}
    </section>
  );
}

function SelectRow({
  label,
  value,
  children,
}: {
  label: string;
  value: string;
  children: ReactNode;
}) {
  return (
    <label className="grid grid-cols-[150px_1fr] items-center gap-4 py-2">
      <span className="text-sm text-spectyn-text">{label}</span>
      <div className="flex items-center gap-3">
        <select
          disabled
          value={value}
          className="w-64 rounded-md border border-spectyn-border bg-spectyn-card px-3 py-2 text-sm text-spectyn-muted opacity-70"
        >
          <option value={value}>{children}</option>
        </select>
        <Note>尚未實作</Note>
      </div>
    </label>
  );
}

function DisabledButton({ children }: { children: ReactNode }) {
  return (
    <button
      type="button"
      disabled
      className="rounded-md border border-spectyn-border px-3 py-1.5 text-sm text-spectyn-muted opacity-60"
    >
      {children}
    </button>
  );
}

export default function SettingsWindow({
  targetTab,
  onTabChange,
  onOpenAddPeer,
  onOpenCoachReviews,
  onRestartDaemon,
  onOpenDaemonLog,
  onWipeAllData,
  onClose,
}: SettingsWindowProps) {
  const [activeTab, setActiveTab] = useState<SettingsTab>(targetTab ?? "general");

  // useState only seeds `activeTab` on mount; sync post-mount `targetTab`
  // prop changes (e.g. caller re-opens settings on a different tab).
  useEffect(() => {
    if (targetTab) setActiveTab(targetTab);
  }, [targetTab]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose?.();
    };

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  const selectTab = (tab: SettingsTab) => {
    setActiveTab(tab);
    onTabChange?.(tab);
  };

  return (
    <div
      data-testid="settings-window"
      className="w-[820px] max-w-[calc(100vw-2rem)] min-h-[560px] overflow-hidden rounded-lg border border-spectyn-border bg-spectyn-card text-spectyn-text shadow-xl"
    >
      <div
        role="tablist"
        aria-label="設定分頁"
        className="flex items-center gap-1 border-b border-spectyn-border bg-spectyn-bg/40 px-3 py-2"
      >
        {tabs.map(({ id, label, Icon }) => {
          const selected = activeTab === id;
          return (
            <button
              key={id}
              id={`settings-tab-${id}`}
              type="button"
              role="tab"
              aria-selected={selected}
              aria-controls={`settings-panel-${id}`}
              onClick={() => selectTab(id)}
              className={`flex flex-1 items-center justify-center gap-2 rounded-md px-3 py-2 text-sm transition ${
                selected
                  ? "bg-spectyn-primary/15 text-spectyn-primary"
                  : "text-spectyn-muted hover:bg-spectyn-primary/10 hover:text-spectyn-text"
              }`}
            >
              <Icon size={16} />
              {label}
            </button>
          );
        })}
      </div>

      <main
        id={`settings-panel-${activeTab}`}
        role="tabpanel"
        aria-labelledby={`settings-tab-${activeTab}`}
        className="p-5"
      >
        {activeTab === "general" && (
          <div className="space-y-4">
            <Section title="偏好設定" icon={Palette}>
              <div className="space-y-1">
                <SelectRow label="語言" value="zh-TW">
                  繁中 (zh-TW)
                </SelectRow>
                <SelectRow label="主題" value="system">
                  跟隨系統
                </SelectRow>
                <label className="grid grid-cols-[150px_1fr] items-center gap-4 py-2">
                  <span className="text-sm text-spectyn-text">登入時啟動</span>
                  <div className="flex items-center gap-3">
                    <input
                      type="checkbox"
                      disabled
                      checked
                      readOnly
                      className="h-4 w-4 accent-spectyn-primary opacity-60"
                    />
                    <Note>由 launchd LaunchAgent 管理</Note>
                  </div>
                </label>
              </div>
            </Section>

            <Section title="全域快捷鍵" icon={Keyboard}>
              <div className="divide-y divide-spectyn-border rounded-md border border-spectyn-border">
                {[
                  ["⌘⇧H", "快速習慣記錄"],
                  ["⌘⇧F", "開始專注"],
                ].map(([shortcut, label]) => (
                  <div key={shortcut} className="flex items-center justify-between gap-4 px-3 py-3">
                    <div className="flex items-center gap-3">
                      <kbd className="rounded border border-spectyn-border bg-spectyn-card px-2 py-1 text-xs text-spectyn-primary">
                        {shortcut}
                      </kbd>
                      <span className="text-sm text-spectyn-text">{label}</span>
                    </div>
                    <div className="flex items-center gap-3">
                      <DisabledButton>變更…</DisabledButton>
                      <Note>尚未實作</Note>
                    </div>
                  </div>
                ))}
              </div>
            </Section>

            <div className="flex justify-end gap-3 pt-2">
              <button
                type="button"
                onClick={() => onOpenDaemonLog?.()}
                className="flex items-center gap-2 rounded-md border border-spectyn-border px-3 py-2 text-sm text-spectyn-text hover:border-spectyn-primary/40 hover:bg-spectyn-primary/10"
              >
                <FileText size={16} />
                開啟背景程式日誌…
              </button>
              <button
                type="button"
                onClick={() => onRestartDaemon?.()}
                className="flex items-center gap-2 rounded-md bg-spectyn-primary px-3 py-2 text-sm text-white hover:bg-spectyn-primary/90"
              >
                <RotateCcw size={16} />
                重啟背景程式
              </button>
            </div>
          </div>
        )}

        {activeTab === "cluster" && (
          <div className="space-y-4">
            <ClusterStatusDashboard />
            <div className="flex items-center gap-3 rounded-lg border border-spectyn-border bg-spectyn-bg/40 p-4">
              <button
                type="button"
                onClick={() => onOpenAddPeer?.()}
                className="flex items-center gap-2 rounded-md bg-spectyn-primary px-3 py-2 text-sm text-white hover:bg-spectyn-primary/90"
              >
                <Plus size={16} />
                新增對等節點
              </button>
              <DisabledButton>重設 cluster_secret</DisabledButton>
              <Note>尚未實作</Note>
            </div>
          </div>
        )}

        {activeTab === "providers" && (
          <div className="flex min-h-[420px] items-center justify-center">
            <div className="max-w-sm rounded-lg border border-spectyn-border bg-spectyn-bg/40 p-6 text-center">
              <KeyRound size={28} className="mx-auto mb-3 text-spectyn-primary" />
              <p className="text-sm font-medium text-spectyn-text">供應商設定尚未接上後端</p>
              <p className="mt-2 text-xs text-spectyn-muted">
                這裡會顯示模型供應商與金鑰狀態；目前不建立假資料。
              </p>
              <button
                type="button"
                disabled
                className="mx-auto mt-4 flex items-center gap-2 rounded-md border border-spectyn-border px-3 py-2 text-sm text-spectyn-muted opacity-60"
              >
                <Plus size={16} />
                新增供應商
              </button>
            </div>
          </div>
        )}

        {activeTab === "privacy" && (
          <div className="space-y-4">
            <Section title="TCC 權限" icon={ShieldCheck}>
              <div className="flex items-center gap-3 rounded-md border border-spectyn-border bg-spectyn-card px-3 py-3">
                <Lock size={16} className="text-spectyn-muted" />
                <span className="text-sm text-spectyn-muted">11 項系統權限狀態尚未接上後端</span>
              </div>
            </Section>

            <Section title="Keychain" icon={Database}>
              <div className="flex items-center gap-3">
                <button
                  type="button"
                  disabled
                  className="flex items-center gap-2 rounded-md border border-spectyn-border px-3 py-2 text-sm text-spectyn-muted opacity-60"
                >
                  <Upload size={16} />
                  匯入識別碼
                </button>
                <button
                  type="button"
                  disabled
                  className="flex items-center gap-2 rounded-md border border-spectyn-border px-3 py-2 text-sm text-spectyn-muted opacity-60"
                >
                  <Download size={16} />
                  匯出識別碼
                </button>
                <Note>尚未實作</Note>
              </div>
            </Section>

            <div className="flex items-center justify-between rounded-lg border border-spectyn-border bg-spectyn-bg/40 p-4">
              <button
                type="button"
                onClick={() => onOpenCoachReviews?.()}
                className="rounded-md border border-spectyn-border px-3 py-2 text-sm text-spectyn-text hover:border-spectyn-primary/40 hover:bg-spectyn-primary/10"
              >
                檢視教練回顧
              </button>
              <button
                type="button"
                onClick={() => onWipeAllData?.()}
                className="flex items-center gap-2 rounded-md border border-spectyn-danger px-3 py-2 text-sm text-spectyn-danger hover:bg-spectyn-danger/10"
              >
                <Trash2 size={16} />
                清除所有資料…
              </button>
            </div>
          </div>
        )}
      </main>
    </div>
  );
}
