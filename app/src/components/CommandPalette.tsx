import { useEffect, useRef, useState, useCallback } from 'react';
import { usePageStore } from '../stores/pageStore';

interface CommandItem {
  id: string;
  label: string;
  category: 'navigation' | 'task' | 'setting';
  action: () => void;
}

const defaultCommands: CommandItem[] = [
  { id: 'nav-tasks', label: 'Go to Tasks', category: 'navigation', action: () => {} },
  { id: 'nav-devices', label: 'Go to Devices', category: 'navigation', action: () => {} },
  { id: 'nav-settings', label: 'Go to Settings', category: 'navigation', action: () => {} },
];

export function CommandPalette() {
  const { commandPaletteOpen, closeCommandPalette, setArea } = usePageStore();
  const [query, setQuery] = useState('');
  const [selectedIndex, setSelectedIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  const commands: CommandItem[] = [
    { ...defaultCommands[0], action: () => { setArea('tasks'); closeCommandPalette(); } },
    { ...defaultCommands[1], action: () => { setArea('devices'); closeCommandPalette(); } },
    { ...defaultCommands[2], action: () => { setArea('settings'); closeCommandPalette(); } },
  ];

  // Fuzzy filter
  const filtered = query
    ? commands.filter((c) => c.label.toLowerCase().includes(query.toLowerCase()))
    : commands;

  // Keyboard shortcut: Cmd+K / Ctrl+K
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === 'k') {
        e.preventDefault();
        if (commandPaletteOpen) {
          closeCommandPalette();
        } else {
          usePageStore.getState().openCommandPalette();
        }
      }
      if (e.key === 'Escape' && commandPaletteOpen) {
        closeCommandPalette();
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [commandPaletteOpen, closeCommandPalette]);

  // Focus input when opened
  useEffect(() => {
    if (commandPaletteOpen) {
      setQuery('');
      setSelectedIndex(0);
      setTimeout(() => inputRef.current?.focus(), 50);
    }
  }, [commandPaletteOpen]);

  // Arrow key navigation
  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === 'ArrowDown') {
        e.preventDefault();
        setSelectedIndex((i) => Math.min(i + 1, filtered.length - 1));
      } else if (e.key === 'ArrowUp') {
        e.preventDefault();
        setSelectedIndex((i) => Math.max(i - 1, 0));
      } else if (e.key === 'Enter' && filtered[selectedIndex]) {
        filtered[selectedIndex].action();
      }
    },
    [filtered, selectedIndex]
  );

  if (!commandPaletteOpen) return null;

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center pt-[20vh]"
      onClick={closeCommandPalette}
    >
      <div
        className="w-[500px] rounded-lg border border-white/10 bg-[#22223c]/95 backdrop-blur-xl shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <input
          ref={inputRef}
          type="text"
          value={query}
          onChange={(e) => { setQuery(e.target.value); setSelectedIndex(0); }}
          onKeyDown={handleKeyDown}
          placeholder="Type a command..."
          className="w-full bg-transparent px-4 py-3 text-white outline-none placeholder:text-white/40"
        />
        <div className="border-t border-white/10 max-h-[300px] overflow-y-auto">
          {filtered.map((cmd, i) => (
            <div
              key={cmd.id}
              onClick={cmd.action}
              className={`px-4 py-2 cursor-pointer text-sm ${
                i === selectedIndex ? 'bg-indigo-600/30 text-white' : 'text-white/70 hover:bg-white/5'
              }`}
            >
              <span className="text-white/40 text-xs mr-2">{cmd.category}</span>
              {cmd.label}
            </div>
          ))}
          {filtered.length === 0 && (
            <div className="px-4 py-3 text-white/40 text-sm">No results</div>
          )}
        </div>
      </div>
    </div>
  );
}
