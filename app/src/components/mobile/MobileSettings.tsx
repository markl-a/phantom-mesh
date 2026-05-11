import { useState, useEffect } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import { ChevronLeft } from "lucide-react";
import ProvidersPanel from "../settings/ProvidersPanel";
import AgentsPanel from "../settings/AgentsPanel";
import SecurityPanel from "../settings/SecurityPanel";
import UpdatePanel from "../settings/UpdatePanel";
import MobileOnboarding from "./MobileOnboarding";
import MobileClusterSettings from "./MobileClusterSettings";
import MobileBrokerLogin from "./MobileBrokerLogin";
import MobileLocalKeys from "./MobileLocalKeys";
import MobileDiagnostics from "./MobileDiagnostics";

type Section = null | "broker" | "diag" | "localKeys" | "providers" | "agents" | "security" | "update" | "onboarding" | "cluster";

const VALID_SECTIONS: Section[] = ["broker", "diag", "localKeys", "providers", "agents", "security", "update", "onboarding", "cluster"];

const SECTIONS: { id: Exclude<Section, null>; title: string; subtitle: string }[] = [
  { id: "broker",     title: "登入 phantommesh.io", subtitle: "Google / Apple 登入 → broker_token 存進 app sandbox" },
  { id: "diag",       title: "診斷：LLM 不通？", subtitle: "看 auth + vault sync chain 卡在哪一步" },
  { id: "localKeys",  title: "手動填 LLM API key", subtitle: "不想登入？直接貼 OPENAI / GROQ key — 馬上能用" },
  { id: "onboarding", title: "從 Mac 匯入設定", subtitle: "用 token 從電腦同步 cluster + API keys" },
  { id: "cluster",    title: "Cluster 派送",     subtitle: "讓 chat 訊息走協調者 → 任一節點執行" },
  { id: "providers",  title: "Providers",       subtitle: "Groq / Gemini / Anthropic / OpenRouter" },
  { id: "agents",     title: "Agents",          subtitle: "master / coder / reviewer / researcher" },
  { id: "security",   title: "Security",        subtitle: "cluster_secret / API key 檢查" },
  { id: "update",     title: "更新",            subtitle: "OTA 檢查與安裝" },
];

export default function MobileSettings() {
  const location = useLocation();
  const navigate = useNavigate();
  // Read deep-link section from URL: /settings/cluster → section=cluster.
  // Lets MobileConversation's "尚未設定" CTA route straight here.
  const initialFromUrl = (() => {
    const m = location.pathname.match(/^\/settings\/([a-z]+)$/);
    const s = m?.[1] as Section | undefined;
    return s && VALID_SECTIONS.includes(s) ? s : null;
  })();
  const [section, setSection] = useState<Section>(initialFromUrl);

  useEffect(() => {
    if (initialFromUrl && initialFromUrl !== section) setSection(initialFromUrl);
  }, [initialFromUrl]);

  const exitToList = () => {
    setSection(null);
    if (location.pathname !== "/settings") navigate("/settings", { replace: true });
  };

  if (section === null) {
    return (
      <div className="flex flex-col h-full overflow-y-auto">
        <div className="p-3 space-y-2">
          {SECTIONS.map((s) => (
            <button
              key={s.id}
              onClick={() => setSection(s.id)}
              className="w-full text-left bg-phantom-card border border-phantom-border rounded-lg px-4 py-3 hover:border-phantom-primary transition"
            >
              <div className="text-sm font-medium text-phantom-text">{s.title}</div>
              <div className="text-xs text-phantom-muted mt-0.5">{s.subtitle}</div>
            </button>
          ))}
        </div>
        <div className="p-4 mt-auto text-[11px] text-phantom-muted text-center">
          Phantom Mesh — mobile mode
        </div>
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center px-2 py-2.5 border-b border-phantom-border flex-shrink-0">
        <button
          onClick={exitToList}
          className="text-phantom-text p-2 -m-2 flex items-center gap-1"
        >
          <ChevronLeft size={20} />
          <span className="text-sm">設定</span>
        </button>
        <span className="text-sm font-medium text-phantom-text mx-auto pr-8">
          {SECTIONS.find(s => s.id === section)?.title}
        </span>
      </div>
      <div className="flex-1 overflow-y-auto p-3">
        {section === "broker"     && <MobileBrokerLogin />}
        {section === "diag"       && <MobileDiagnostics />}
        {section === "localKeys"  && <MobileLocalKeys />}
        {section === "onboarding" && <MobileOnboarding />}
        {section === "cluster"    && <MobileClusterSettings />}
        {section === "providers"  && <ProvidersPanel />}
        {section === "agents"     && <AgentsPanel />}
        {section === "security"   && <SecurityPanel />}
        {section === "update"     && <UpdatePanel />}
      </div>
    </div>
  );
}
