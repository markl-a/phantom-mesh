import { useState } from 'react';

interface FailureNarrative {
  summary: string;
  narrative: string;
  rawLogs: string;
  suggestedAction: string;
  errorCode?: string;
}

interface FailureCardProps {
  taskId: string;
  narrative: FailureNarrative;
  onRetry?: (taskId: string) => void;
  onRetryDifferentDevice?: (taskId: string) => void;
}

export function FailureCard({ taskId, narrative, onRetry, onRetryDifferentDevice }: FailureCardProps) {
  const [layer3Expanded, setLayer3Expanded] = useState(false);

  return (
    <div className="rounded-lg border border-red-500/30 bg-red-950/20 p-4 space-y-3">
      {/* Layer 1: Summary */}
      <div className="text-red-400 font-medium text-sm">
        {narrative.errorCode && (
          <span className="bg-red-500/20 px-1.5 py-0.5 rounded text-xs mr-2">
            {narrative.errorCode}
          </span>
        )}
        {narrative.summary}
      </div>

      {/* Layer 2: Narrative (default expanded) */}
      <div className="text-white/80 text-sm whitespace-pre-wrap leading-relaxed">
        {narrative.narrative}
      </div>

      {/* Suggested action */}
      <div className="text-amber-400/90 text-sm">
        Suggested: {narrative.suggestedAction}
      </div>

      {/* Layer 3: Raw logs (collapsed) */}
      <div>
        <button
          onClick={() => setLayer3Expanded(!layer3Expanded)}
          className="text-white/40 text-xs hover:text-white/60 transition-colors"
        >
          {layer3Expanded ? '▼ Hide raw logs' : '▶ Show raw logs'}
        </button>
        <div
          className={`overflow-hidden transition-all duration-300 ease-out ${
            layer3Expanded ? 'max-h-[500px] mt-2' : 'max-h-0'
          }`}
        >
          <pre className="bg-black/40 rounded p-3 text-xs text-white/60 overflow-x-auto">
            {narrative.rawLogs || 'No execution logs available'}
          </pre>
        </div>
      </div>

      {/* Action buttons */}
      <div className="flex gap-2 pt-1">
        {onRetry && (
          <button
            onClick={() => onRetry(taskId)}
            className="px-3 py-1.5 text-xs rounded bg-indigo-600 hover:bg-indigo-500 text-white transition-colors"
          >
            Retry
          </button>
        )}
        {onRetryDifferentDevice && (
          <button
            onClick={() => onRetryDifferentDevice(taskId)}
            className="px-3 py-1.5 text-xs rounded bg-white/10 hover:bg-white/20 text-white/80 transition-colors"
          >
            Retry on different device
          </button>
        )}
      </div>
    </div>
  );
}

/** Merge 10+ failures into a single summary card */
export function MergedFailureCard({
  failures,
  onRetryAll,
}: {
  failures: { taskId: string; narrative: FailureNarrative }[];
  onRetryAll?: () => void;
}) {
  if (failures.length <= 10) return null;

  return (
    <div className="rounded-lg border border-red-500/30 bg-red-950/20 p-4">
      <div className="text-red-400 font-medium text-sm mb-2">
        {failures.length} tasks failed
      </div>
      <ul className="text-white/60 text-xs space-y-1 mb-3">
        {failures.slice(0, 5).map((f) => (
          <li key={f.taskId}>- {f.narrative.summary}</li>
        ))}
        {failures.length > 5 && (
          <li className="text-white/40">...and {failures.length - 5} more</li>
        )}
      </ul>
      {onRetryAll && (
        <button
          onClick={onRetryAll}
          className="px-3 py-1.5 text-xs rounded bg-red-600 hover:bg-red-500 text-white transition-colors"
        >
          Retry all ({failures.length})
        </button>
      )}
    </div>
  );
}
