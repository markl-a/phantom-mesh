import { useState } from "react";

type AdaptationLevel = "Safe" | "Normal" | "Dangerous";

interface Skill {
  name: string;
  version: string;
  source: "官方" | "社群";
  capability: string;
  status: "active" | "update";
}

interface Plugin {
  name: string;
  version: string;
  engine: string;
  size: string;
  status: "active" | "error";
}

interface AdaptationEntry {
  time: string;
  type: string;
  action: string;
  level: AdaptationLevel;
  status: "已套用" | "待確認" | "已拒絕";
}

const LEVEL_CONFIG: Record<AdaptationLevel, { color: string }> = {
  Safe: { color: "bg-spectyn-success/20 text-spectyn-success" },
  Normal: { color: "bg-spectyn-warning/20 text-spectyn-warning" },
  Dangerous: { color: "bg-spectyn-danger/20 text-spectyn-danger" },
};

const STATUS_STYLE: Record<string, string> = {
  "已套用": "text-spectyn-success",
  "待確認": "text-spectyn-warning",
  "已拒絕": "text-spectyn-danger",
};

const MOCK_SKILLS: Skill[] = [
  { name: "web-search", version: "v1.2.0", source: "官方", capability: "網路搜尋與摘要", status: "active" },
  { name: "code-review", version: "v2.0.1", source: "官方", capability: "程式碼審查與建議", status: "active" },
  { name: "image-gen", version: "v0.9.0", source: "社群", capability: "圖片生成", status: "active" },
  { name: "data-analysis", version: "v1.1.0", source: "官方", capability: "資料分析與視覺化", status: "active" },
  { name: "translation", version: "v1.0.0", source: "社群", capability: "多語言翻譯", status: "update" },
];

const MOCK_PLUGINS: Plugin[] = [
  { name: "markdown-renderer", version: "v1.0.0", engine: "Extism", size: "128KB", status: "active" },
  { name: "csv-parser", version: "v0.8.0", engine: "Extism", size: "96KB", status: "active" },
  { name: "pdf-extractor", version: "v1.1.0", engine: "Extism", size: "256KB", status: "error" },
];

const MOCK_ADAPTATIONS: AdaptationEntry[] = [
  { time: "10:30", type: "AdjustScaling", action: "+2 SubAgents", level: "Safe", status: "已套用" },
  { time: "09:15", type: "ReorderProviderTier", action: "Ollama → 第一", level: "Safe", status: "已套用" },
  { time: "08:00", type: "InstallCapability", action: "image_generation", level: "Normal", status: "待確認" },
  { time: "昨天", type: "RemoveNode", action: "node-b-01", level: "Dangerous", status: "已拒絕" },
];

export default function EvolutionPanel() {
  const [activeSection, setActiveSection] = useState<"skills" | "plugins" | "adaptation">("skills");

  const stats = {
    skillCount: MOCK_SKILLS.length,
    pluginCount: MOCK_PLUGINS.length,
    todayAdaptations: MOCK_ADAPTATIONS.length,
    lastUpdateCheck: "2 小時前",
  };

  return (
    <div>
      <div className="flex items-center justify-between mb-6">
        <h1 className="text-2xl font-bold">進化系統</h1>
      </div>

      {/* Stats */}
      <div className="grid grid-cols-2 md:grid-cols-4 gap-4 mb-6">
        <div className="bg-spectyn-card border border-spectyn-border rounded-lg p-4">
          <p className="text-spectyn-muted text-xs">技能數量</p>
          <p className="text-2xl font-bold mt-1">{stats.skillCount}</p>
        </div>
        <div className="bg-spectyn-card border border-spectyn-border rounded-lg p-4">
          <p className="text-spectyn-muted text-xs">Plugin 數量</p>
          <p className="text-2xl font-bold mt-1">{stats.pluginCount}</p>
        </div>
        <div className="bg-spectyn-card border border-spectyn-border rounded-lg p-4">
          <p className="text-spectyn-muted text-xs">今日自動調適</p>
          <p className="text-2xl font-bold mt-1">{stats.todayAdaptations}</p>
        </div>
        <div className="bg-spectyn-card border border-spectyn-border rounded-lg p-4">
          <p className="text-spectyn-muted text-xs">上次更新檢查</p>
          <p className="text-2xl font-bold mt-1 text-base">{stats.lastUpdateCheck}</p>
        </div>
      </div>

      {/* Section Tabs */}
      <div className="flex gap-1 mb-6">
        {([
          { key: "skills" as const, label: "已安裝技能" },
          { key: "plugins" as const, label: "已安裝 Plugin" },
          { key: "adaptation" as const, label: "自動調適記錄" },
        ]).map((tab) => (
          <button
            key={tab.key}
            onClick={() => setActiveSection(tab.key)}
            className={`px-3 py-1.5 rounded text-xs font-medium transition-colors ${
              activeSection === tab.key
                ? "bg-spectyn-primary text-spectyn-bg"
                : "bg-spectyn-card border border-spectyn-border text-spectyn-muted hover:text-spectyn-text"
            }`}
          >
            {tab.label}
          </button>
        ))}
      </div>

      {/* Installed Skills */}
      {activeSection === "skills" && (
        <section>
          <div className="flex items-center gap-2 mb-3">
            <span className="w-3 h-3 rounded-full bg-spectyn-primary" />
            <h2 className="text-lg font-bold">已安裝技能</h2>
            <span className="text-xs text-spectyn-muted">— Installed Skills</span>
          </div>
          <div className="bg-spectyn-card border border-spectyn-border rounded-lg overflow-hidden">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b border-spectyn-border">
                  <th className="text-left px-4 py-3 text-spectyn-muted font-medium">名稱</th>
                  <th className="text-left px-4 py-3 text-spectyn-muted font-medium">版本</th>
                  <th className="text-left px-4 py-3 text-spectyn-muted font-medium">來源</th>
                  <th className="text-left px-4 py-3 text-spectyn-muted font-medium">能力</th>
                  <th className="text-left px-4 py-3 text-spectyn-muted font-medium">狀態</th>
                </tr>
              </thead>
              <tbody>
                {MOCK_SKILLS.map((skill, i) => (
                  <tr
                    key={skill.name}
                    className={`border-b border-spectyn-border last:border-0 ${
                      i % 2 === 1 ? "bg-spectyn-bg/50" : ""
                    }`}
                  >
                    <td className="px-4 py-3">
                      <span className="font-mono text-xs bg-spectyn-bg px-1.5 py-0.5 rounded border border-spectyn-border">
                        {skill.name}
                      </span>
                    </td>
                    <td className="px-4 py-3 font-mono text-xs text-spectyn-muted">{skill.version}</td>
                    <td className="px-4 py-3">
                      <span
                        className={`text-xs px-2 py-0.5 rounded ${
                          skill.source === "官方"
                            ? "bg-spectyn-primary/10 text-spectyn-primary"
                            : "bg-spectyn-warning/10 text-spectyn-warning"
                        }`}
                      >
                        {skill.source}
                      </span>
                    </td>
                    <td className="px-4 py-3 text-spectyn-muted text-xs">{skill.capability}</td>
                    <td className="px-4 py-3">
                      {skill.status === "active" ? (
                        <span className="text-spectyn-success text-xs font-medium">&#10003; 正常</span>
                      ) : (
                        <span className="text-spectyn-warning text-xs font-medium">&#9888; 需更新</span>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </section>
      )}

      {/* WASM Plugins */}
      {activeSection === "plugins" && (
        <section>
          <div className="flex items-center gap-2 mb-3">
            <span className="w-3 h-3 rounded-full bg-spectyn-success" />
            <h2 className="text-lg font-bold">已安裝 Plugin</h2>
            <span className="text-xs text-spectyn-muted">— WASM Plugins</span>
          </div>
          <div className="bg-spectyn-card border border-spectyn-border rounded-lg overflow-hidden">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b border-spectyn-border">
                  <th className="text-left px-4 py-3 text-spectyn-muted font-medium">名稱</th>
                  <th className="text-left px-4 py-3 text-spectyn-muted font-medium">版本</th>
                  <th className="text-left px-4 py-3 text-spectyn-muted font-medium">引擎</th>
                  <th className="text-left px-4 py-3 text-spectyn-muted font-medium">大小</th>
                  <th className="text-left px-4 py-3 text-spectyn-muted font-medium">狀態</th>
                </tr>
              </thead>
              <tbody>
                {MOCK_PLUGINS.map((plugin, i) => (
                  <tr
                    key={plugin.name}
                    className={`border-b border-spectyn-border last:border-0 ${
                      i % 2 === 1 ? "bg-spectyn-bg/50" : ""
                    }`}
                  >
                    <td className="px-4 py-3">
                      <span className="font-mono text-xs bg-spectyn-bg px-1.5 py-0.5 rounded border border-spectyn-border">
                        {plugin.name}
                      </span>
                    </td>
                    <td className="px-4 py-3 font-mono text-xs text-spectyn-muted">{plugin.version}</td>
                    <td className="px-4 py-3">
                      <span className="text-xs bg-spectyn-primary/10 text-spectyn-primary px-2 py-0.5 rounded">
                        {plugin.engine}
                      </span>
                    </td>
                    <td className="px-4 py-3 text-spectyn-muted text-xs">{plugin.size}</td>
                    <td className="px-4 py-3">
                      {plugin.status === "active" ? (
                        <span className="text-spectyn-success text-xs font-medium">&#10003; 正常</span>
                      ) : (
                        <span className="text-spectyn-danger text-xs font-medium">&#9888; 沙箱錯誤</span>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </section>
      )}

      {/* Auto Adaptation Log */}
      {activeSection === "adaptation" && (
        <section>
          <div className="flex items-center gap-2 mb-3">
            <span className="w-3 h-3 rounded-full bg-spectyn-warning" />
            <h2 className="text-lg font-bold">自動調適記錄</h2>
            <span className="text-xs text-spectyn-muted">— Auto Adaptation Log</span>
          </div>
          <div className="bg-spectyn-card border border-spectyn-border rounded-lg overflow-hidden">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b border-spectyn-border">
                  <th className="text-left px-4 py-3 text-spectyn-muted font-medium">時間</th>
                  <th className="text-left px-4 py-3 text-spectyn-muted font-medium">類型</th>
                  <th className="text-left px-4 py-3 text-spectyn-muted font-medium">動作</th>
                  <th className="text-left px-4 py-3 text-spectyn-muted font-medium">等級</th>
                  <th className="text-left px-4 py-3 text-spectyn-muted font-medium">狀態</th>
                </tr>
              </thead>
              <tbody>
                {MOCK_ADAPTATIONS.map((entry, i) => (
                  <tr
                    key={`${entry.time}-${entry.type}`}
                    className={`border-b border-spectyn-border last:border-0 ${
                      i % 2 === 1 ? "bg-spectyn-bg/50" : ""
                    }`}
                  >
                    <td className="px-4 py-3 text-spectyn-muted font-mono text-xs">{entry.time}</td>
                    <td className="px-4 py-3">
                      <span className="font-mono text-xs bg-spectyn-bg px-1.5 py-0.5 rounded border border-spectyn-border">
                        {entry.type}
                      </span>
                    </td>
                    <td className="px-4 py-3 text-xs">{entry.action}</td>
                    <td className="px-4 py-3">
                      <span
                        className={`inline-block px-2 py-0.5 rounded text-xs font-medium ${
                          LEVEL_CONFIG[entry.level].color
                        }`}
                      >
                        {entry.level}
                      </span>
                    </td>
                    <td className="px-4 py-3">
                      <span className={`text-xs font-medium ${STATUS_STYLE[entry.status] || ""}`}>
                        {entry.status}
                      </span>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </section>
      )}
    </div>
  );
}
