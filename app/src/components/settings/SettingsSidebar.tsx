import { NavLink } from "react-router-dom";
import { Cpu, Workflow, Wrench, Sparkles, Brain, KeyRound, Radio, Shield, ScrollText, RefreshCw } from "lucide-react";

const SETTINGS_GROUPS = [
  {
    title: "叢集",
    items: [
      { path: "/settings/agents", icon: Cpu, label: "Agent 監控" },
      { path: "/settings/hands", icon: Workflow, label: "工作流" },
      { path: "/settings/tools", icon: Wrench, label: "工具管理" },
      { path: "/settings/evolution", icon: Sparkles, label: "進化" },
    ],
  },
  {
    title: "對話",
    items: [
      { path: "/settings/memory", icon: Brain, label: "記憶" },
      { path: "/settings/providers", icon: KeyRound, label: "API 金鑰" },
      { path: "/settings/channels", icon: Radio, label: "頻道" },
    ],
  },
  {
    title: "系統",
    items: [
      { path: "/settings/security", icon: Shield, label: "安全" },
      { path: "/settings/logs", icon: ScrollText, label: "日誌" },
      { path: "/settings/update", icon: RefreshCw, label: "更新" },
    ],
  },
];

export default function SettingsSidebar() {
  return (
    <aside className="w-full md:w-48 flex-shrink-0 border-b md:border-b-0 md:border-r border-phantom-border overflow-x-auto md:overflow-y-auto">
      <nav className="flex md:flex-col md:px-2 md:py-4 px-2 py-2 gap-1 md:gap-0 overflow-x-auto">
        {SETTINGS_GROUPS.map((group) => (
          <div key={group.title} className="mb-0 md:mb-4 flex md:flex-col gap-1">
            <p className="hidden md:block px-3 py-1 text-[10px] uppercase tracking-wider text-phantom-muted">
              {group.title}
            </p>
            {group.items.map((item) => (
              <NavLink
                key={item.path}
                to={item.path}
                className={({ isActive }) =>
                  `flex items-center gap-2 px-3 py-1.5 rounded text-sm transition-colors ${
                    isActive
                      ? "bg-phantom-primary/15 text-phantom-primary"
                      : "text-phantom-text hover:bg-phantom-card"
                  }`
                }
              >
                <item.icon size={16} />
                {item.label}
              </NavLink>
            ))}
          </div>
        ))}
      </nav>
    </aside>
  );
}
