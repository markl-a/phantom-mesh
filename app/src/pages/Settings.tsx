import { Routes, Route, Navigate } from "react-router-dom";
import SettingsSidebar from "../components/settings/SettingsSidebar";
import AgentsPanel from "../components/settings/AgentsPanel";
import HandsPanel from "../components/settings/HandsPanel";
import ToolsPanel from "../components/settings/ToolsPanel";
import EvolutionPanel from "../components/settings/EvolutionPanel";
import MemoryPanel from "../components/settings/MemoryPanel";
import ProvidersPanel from "../components/settings/ProvidersPanel";
import ChannelsPanel from "../components/settings/ChannelsPanel";
import SecurityPanel from "../components/settings/SecurityPanel";
import LogsPanel from "../components/settings/LogsPanel";
import UpdatePanel from "../components/settings/UpdatePanel";

export default function SettingsPage() {
  return (
    <div className="flex flex-col md:flex-row h-full">
      <SettingsSidebar />
      <main className="flex-1 overflow-y-auto p-4 md:p-6">
        <Routes>
          <Route index element={<Navigate to="agents" replace />} />
          <Route path="agents" element={<AgentsPanel />} />
          <Route path="hands" element={<HandsPanel />} />
          <Route path="tools" element={<ToolsPanel />} />
          <Route path="evolution" element={<EvolutionPanel />} />
          <Route path="memory" element={<MemoryPanel />} />
          <Route path="providers" element={<ProvidersPanel />} />
          <Route path="channels" element={<ChannelsPanel />} />
          <Route path="security" element={<SecurityPanel />} />
          <Route path="logs" element={<LogsPanel />} />
          <Route path="update" element={<UpdatePanel />} />
        </Routes>
      </main>
    </div>
  );
}
