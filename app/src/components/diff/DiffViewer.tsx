import React from 'react';

interface DiffLine {
  type: 'add' | 'remove' | 'context';
  content: string;
  lineNo?: number;
}

interface DiffHunk {
  header: string;  // e.g. "@@ -10,5 +10,6 @@"
  lines: DiffLine[];
}

interface DiffViewerProps {
  path: string;
  hunks: DiffHunk[];
  className?: string;
}

export function DiffViewer({ path, hunks, className = '' }: DiffViewerProps) {
  if (hunks.length === 0) return null;

  return (
    <div className={`rounded-lg overflow-hidden border border-spectyn-border text-xs font-mono ${className}`}>
      {/* File header */}
      <div className="bg-spectyn-card px-3 py-1.5 border-b border-spectyn-border flex items-center gap-2">
        <span className="text-spectyn-muted">~</span>
        <span className="text-spectyn-text">{path}</span>
      </div>

      {/* Hunks */}
      {hunks.map((hunk, hi) => (
        <div key={hi}>
          {/* Hunk header */}
          <div className="bg-blue-500/10 px-3 py-0.5 text-blue-400 text-xs">
            {hunk.header}
          </div>
          {/* Lines */}
          {hunk.lines.map((line, li) => (
            <div
              key={li}
              className={`flex px-0 ${
                line.type === 'add' ? 'bg-green-500/10' :
                line.type === 'remove' ? 'bg-red-500/10' : ''
              }`}
            >
              <span className={`w-6 text-center select-none flex-shrink-0 ${
                line.type === 'add' ? 'text-green-400' :
                line.type === 'remove' ? 'text-red-400' :
                'text-spectyn-muted'
              }`}>
                {line.type === 'add' ? '+' : line.type === 'remove' ? '-' : ' '}
              </span>
              <span className={`flex-1 px-2 whitespace-pre-wrap break-all ${
                line.type === 'add' ? 'text-green-300' :
                line.type === 'remove' ? 'text-red-300' :
                'text-spectyn-muted'
              }`}>
                {line.content}
              </span>
            </div>
          ))}
        </div>
      ))}
    </div>
  );
}

/// Parse a unified diff string into DiffHunk[]
export function parseDiff(diffText: string): { path: string; hunks: DiffHunk[] } {
  const lines = diffText.split('\n');
  let path = '';
  const hunks: DiffHunk[] = [];
  let currentHunk: DiffHunk | null = null;

  for (const line of lines) {
    if (line.startsWith('+++ b/') || line.startsWith('+++ ')) {
      path = line.replace(/^\+\+\+ (b\/)?/, '');
    } else if (line.startsWith('@@')) {
      currentHunk = { header: line, lines: [] };
      hunks.push(currentHunk);
    } else if (currentHunk) {
      if (line.startsWith('+')) {
        currentHunk.lines.push({ type: 'add', content: line.slice(1) });
      } else if (line.startsWith('-')) {
        currentHunk.lines.push({ type: 'remove', content: line.slice(1) });
      } else if (!line.startsWith('\\')) {
        currentHunk.lines.push({ type: 'context', content: line.slice(1) });
      }
    }
  }

  return { path, hunks };
}

export default DiffViewer;
