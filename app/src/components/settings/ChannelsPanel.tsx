import { useState, useEffect, useCallback } from "react";
import { safeInvoke as invoke } from "../../lib/tauri-compat";

interface Channel {
  id: string;
  name: string;
  icon: string;
  status: "connected" | "disconnected";
  messageCount: number;
  description: string;
}

type DaemonStatus = "checking" | "online" | "offline";

const MOCK_CHANNELS: Channel[] = [
  {
    id: "telegram",
    name: "Telegram",
    icon: "TG",
    status: "connected",
    messageCount: 1247,
    description: "主要通訊頻道。支援文字、圖片、檔案傳送與 Bot 指令。",
  },
  {
    id: "slack",
    name: "Slack",
    icon: "SL",
    status: "disconnected",
    messageCount: 0,
    description: "團隊協作頻道。支援 Workspace 整合與 App 安裝。",
  },
  {
    id: "discord",
    name: "Discord",
    icon: "DC",
    status: "disconnected",
    messageCount: 0,
    description: "社群與語音頻道。支援 Bot 整合與 Webhook。",
  },
  {
    id: "email",
    name: "Email",
    icon: "EM",
    status: "disconnected",
    messageCount: 0,
    description: "電子郵件頻道。支援 IMAP/SMTP 收發與自動回覆。",
  },
];

export default function ChannelsPanel() {
  const [showConfig, setShowConfig] = useState<string | null>(null);
  const [daemonStatus, setDaemonStatus] = useState<DaemonStatus>("checking");

  const checkDaemon = useCallback(async () => {
    setDaemonStatus("checking");
    try {
      await invoke("get_health");
      setDaemonStatus("online");
    } catch {
      setDaemonStatus("offline");
    }
  }, []);

  useEffect(() => {
    checkDaemon();
  }, [checkDaemon]);

  const daemonBadge = () => {
    switch (daemonStatus) {
      case "checking":
        return (
          <span className="inline-flex items-center gap-1.5 text-xs px-2.5 py-1 rounded-full bg-phantom-border/50 text-phantom-muted">
            <span className="w-2 h-2 rounded-full bg-phantom-muted animate-pulse" />
            檢查中...
          </span>
        );
      case "online":
        return (
          <span className="inline-flex items-center gap-1.5 text-xs px-2.5 py-1 rounded-full bg-phantom-success/20 text-phantom-success">
            <span className="w-2 h-2 rounded-full bg-phantom-success" />
            連線中
          </span>
        );
      case "offline":
        return (
          <span className="inline-flex items-center gap-1.5 text-xs px-2.5 py-1 rounded-full bg-phantom-warning/20 text-phantom-warning">
            <span className="w-2 h-2 rounded-full bg-phantom-warning" />
            離線模式
          </span>
        );
    }
  };

  return (
    <div>
      <div className="flex items-center justify-between mb-6">
        <div className="flex items-center gap-3">
          <h1 className="text-2xl font-bold">通訊頻道</h1>
          {daemonBadge()}
        </div>
        <div className="flex items-center gap-3">
          <div className="text-sm text-phantom-muted">
            已連線: {MOCK_CHANNELS.filter((c) => c.status === "connected").length} / {MOCK_CHANNELS.length}
          </div>
          {daemonStatus !== "checking" && (
            <button
              onClick={checkDaemon}
              className="text-xs text-phantom-muted hover:text-phantom-text border border-phantom-border rounded px-2 py-1"
            >
              重新檢查
            </button>
          )}
        </div>
      </div>

      <div className="space-y-4">
        {MOCK_CHANNELS.map((channel) => (
          <div
            key={channel.id}
            className="bg-phantom-card border border-phantom-border rounded-lg p-4"
          >
            <div className="flex items-center gap-4">
              {/* Icon */}
              <div
                className={`w-12 h-12 rounded-lg flex items-center justify-center font-bold text-sm ${
                  channel.status === "connected"
                    ? "bg-phantom-primary/20 text-phantom-primary"
                    : "bg-phantom-border/50 text-phantom-muted"
                }`}
              >
                {channel.icon}
              </div>

              {/* Info */}
              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-2 mb-1">
                  <h3 className="font-semibold">{channel.name}</h3>
                  <span
                    className={`text-xs px-2 py-0.5 rounded font-medium ${
                      channel.status === "connected"
                        ? "bg-phantom-success/20 text-phantom-success"
                        : "bg-phantom-border/50 text-phantom-muted"
                    }`}
                  >
                    {channel.status === "connected" ? "已連線" : "未設定"}
                  </span>
                </div>
                <p className="text-sm text-phantom-muted">{channel.description}</p>
              </div>

              {/* Stats + Action */}
              <div className="flex items-center gap-4 shrink-0">
                {channel.status === "connected" && (
                  <div className="text-right">
                    <p className="text-lg font-bold">{channel.messageCount.toLocaleString()}</p>
                    <p className="text-xs text-phantom-muted">訊息數</p>
                  </div>
                )}
                <button
                  onClick={() => setShowConfig(showConfig === channel.id ? null : channel.id)}
                  className={`px-3 py-1.5 rounded text-xs font-medium border ${
                    channel.status === "connected"
                      ? "border-phantom-primary text-phantom-primary hover:bg-phantom-primary/10"
                      : "border-phantom-border text-phantom-muted hover:text-phantom-text hover:border-phantom-text"
                  }`}
                >
                  設定
                </button>
              </div>
            </div>

            {/* Config Panel */}
            {showConfig === channel.id && (
              <div className="mt-4 pt-4 border-t border-phantom-border">
                {channel.status === "connected" ? (
                  <div className="grid grid-cols-1 md:grid-cols-3 gap-4 text-sm">
                    <div>
                      <label className="block text-phantom-muted text-xs mb-1">Bot Token</label>
                      <div className="bg-phantom-bg border border-phantom-border rounded px-3 py-2 font-mono text-phantom-muted">
                        ••••••••••••:•••••••••••
                      </div>
                    </div>
                    <div>
                      <label className="block text-phantom-muted text-xs mb-1">Chat ID</label>
                      <div className="bg-phantom-bg border border-phantom-border rounded px-3 py-2 font-mono text-phantom-muted">
                        -100**********
                      </div>
                    </div>
                    <div>
                      <label className="block text-phantom-muted text-xs mb-1">狀態</label>
                      <div className="bg-phantom-bg border border-phantom-border rounded px-3 py-2">
                        <span className="text-phantom-success">Webhook 活躍</span>
                      </div>
                    </div>
                  </div>
                ) : (
                  <div className="text-center py-4">
                    <p className="text-sm text-phantom-muted mb-3">尚未設定此頻道</p>
                    <button className="bg-phantom-primary text-phantom-bg px-4 py-2 rounded text-sm font-medium hover:opacity-90">
                      開始設定
                    </button>
                  </div>
                )}
              </div>
            )}
          </div>
        ))}
      </div>

      {/* Integration Info */}
      <div className="mt-6 bg-phantom-card border border-phantom-border rounded-lg p-4">
        <h3 className="text-sm font-medium mb-2">通訊架構</h3>
        <p className="text-xs text-phantom-muted leading-relaxed">
          所有頻道透過統一的 Message Bus 連接。入站訊息經過 NLU 解析後分派給 Master Agent，
          回覆則透過對應頻道發送。目前 Telegram 為主要生產頻道，其他頻道可依需求啟用。
        </p>
      </div>
    </div>
  );
}
