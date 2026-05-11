import React, { useEffect, useState } from 'react';
import { useProjectStore, ProjectInfo } from '../../stores/projectStore';
import { FolderOpen, ChevronDown, GitBranch } from 'lucide-react';

export function ProjectSelector() {
  const { currentProject, recentProjects, loadCurrentProject, loadRecentProjects, setProject } = useProjectStore();
  const [open, setOpen] = useState(false);
  const [customPath, setCustomPath] = useState('');

  useEffect(() => {
    loadCurrentProject();
    loadRecentProjects();
  }, []);

  const projectTypeIcon = (type: ProjectInfo['project_type']) => {
    const icons: Record<string, string> = {
      rust: '🦀', node: '⬡', python: '🐍', go: '🐹', unknown: '📁'
    };
    return icons[type] ?? '📁';
  };

  return (
    <div className="relative">
      {/* Trigger */}
      <button
        onClick={() => setOpen(!open)}
        className="flex items-center gap-1.5 px-2 py-1 rounded text-xs text-phantom-muted hover:text-phantom-text hover:bg-phantom-card transition"
      >
        <FolderOpen size={12} />
        <span className="max-w-32 truncate">
          {currentProject ? currentProject.name : 'no project'}
        </span>
        {currentProject?.has_git && <GitBranch size={10} className="text-phantom-primary" />}
        <ChevronDown size={10} />
      </button>

      {/* Dropdown */}
      {open && (
        <div className="absolute top-full left-0 mt-1 w-72 bg-phantom-card border border-phantom-border rounded-lg shadow-xl z-50 p-2">
          {/* Current */}
          {currentProject && (
            <div className="px-2 py-1.5 mb-2 border-b border-phantom-border">
              <div className="text-xs text-phantom-muted">current project</div>
              <div className="text-sm text-phantom-text font-medium flex items-center gap-1">
                <span>{projectTypeIcon(currentProject.project_type)}</span>
                {currentProject.name}
              </div>
              <div className="text-xs text-phantom-muted truncate">{currentProject.cwd}</div>
            </div>
          )}

          {/* Recent */}
          {recentProjects.length > 0 && (
            <>
              <div className="text-xs text-phantom-muted px-2 mb-1">recent</div>
              {recentProjects.slice(0, 5).map((p) => (
                <button
                  key={p.cwd}
                  onClick={() => { setProject(p.cwd); setOpen(false); }}
                  className="w-full flex items-center gap-2 px-2 py-1.5 rounded hover:bg-phantom-bg text-left"
                >
                  <span>{projectTypeIcon(p.project_type)}</span>
                  <div className="min-w-0">
                    <div className="text-xs text-phantom-text truncate">{p.name}</div>
                    <div className="text-xs text-phantom-muted truncate">{p.cwd}</div>
                  </div>
                </button>
              ))}
            </>
          )}

          {/* Custom path */}
          <div className="mt-2 pt-2 border-t border-phantom-border">
            <div className="flex gap-1">
              <input
                value={customPath}
                onChange={e => setCustomPath(e.target.value)}
                onKeyDown={e => { if (e.key === 'Enter' && customPath) { setProject(customPath); setOpen(false); setCustomPath(''); }}}
                placeholder="/path/to/project"
                className="flex-1 text-xs bg-phantom-bg border border-phantom-border rounded px-2 py-1 text-phantom-text placeholder-phantom-muted"
              />
              <button
                onClick={() => { if (customPath) { setProject(customPath); setOpen(false); setCustomPath(''); }}}
                disabled={!customPath}
                className="text-xs bg-phantom-primary text-phantom-bg px-2 py-1 rounded disabled:opacity-40"
              >
                open
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Backdrop */}
      {open && <div className="fixed inset-0 z-40" onClick={() => setOpen(false)} />}
    </div>
  );
}

export default ProjectSelector;
