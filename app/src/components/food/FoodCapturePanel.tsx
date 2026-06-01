// SPEC-20 capture-food — desktop food-log panel (/food).
//
// Text-first food capture: describe a meal → analyze → macro estimate +
// fat-loss score + shame-free suggestion. Reuses lib/captureFood.ts over the
// capture_food_wire backend (vision chain Stage-4 deferred → graceful
// "尚未實作" note). Image capture is a mobile-first path (SPEC-31) — desktop
// stays text-only for v0.6.0. Design lineage: BIG-GOAL P2 multimodal → SPEC-20.

import { useState } from "react";
import { Utensils, Sparkles, Loader2 } from "lucide-react";
import { buildFoodRequest, analyzeFood, describeFoodError } from "../../lib/captureFood";
import type { FoodAnalysisResult } from "../../lib/generated/capture_food/FoodAnalysisResult";

export default function FoodCapturePanel() {
  const [text, setText] = useState("");
  const [analyzing, setAnalyzing] = useState(false);
  const [result, setResult] = useState<FoodAnalysisResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  const analyze = async () => {
    if (!text.trim()) return;
    setAnalyzing(true);
    setError(null);
    setResult(null);
    try {
      const res = await analyzeFood(buildFoodRequest(text));
      setResult(res);
    } catch (e) {
      setError(describeFoodError(e));
    } finally {
      setAnalyzing(false);
    }
  };

  const macro = result?.macroEstimate ?? null;

  return (
    <div className="max-w-2xl mx-auto space-y-6" data-testid="food-capture-panel">
      <header className="flex items-center gap-3">
        <div className="w-10 h-10 rounded-lg bg-phantom-primary/15 flex items-center justify-center">
          <Utensils size={20} className="text-phantom-primary" />
        </div>
        <div>
          <h1 className="text-xl font-bold text-phantom-text">飲食記錄</h1>
          <p className="text-xs text-phantom-muted">Food log · SPEC-20 capture-food</p>
        </div>
      </header>

      <div className="bg-phantom-card border border-phantom-border rounded-lg p-5 space-y-3">
        <textarea
          value={text}
          onChange={(e) => setText(e.target.value)}
          placeholder="今天吃了什麼？（例：中午的鮭魚便當 + 一杯無糖豆漿）"
          rows={3}
          className="w-full bg-phantom-bg border border-phantom-border rounded px-3 py-2 text-sm text-phantom-text placeholder-phantom-muted focus:outline-none focus:border-phantom-primary resize-none"
        />
        <div className="flex items-center justify-between">
          <span className="text-[11px] text-phantom-muted">標籤：fat_loss（減脂）</span>
          <button
            onClick={() => void analyze()}
            disabled={analyzing || !text.trim()}
            className="flex items-center gap-2 bg-phantom-primary text-phantom-bg px-4 py-2 rounded-lg text-sm font-medium hover:brightness-110 disabled:opacity-40 transition"
          >
            {analyzing ? <Loader2 size={15} className="animate-spin" /> : <Sparkles size={15} />}
            {analyzing ? "分析中…" : "分析"}
          </button>
        </div>
        {error && <p className="text-xs text-phantom-warning" role="alert">{error}</p>}
      </div>

      {result && (
        <div className="bg-phantom-card border border-phantom-border rounded-lg p-5 space-y-4">
          {result.summary && (
            <p className="text-sm text-phantom-text">{result.summary}</p>
          )}

          {macro ? (
            <div className="grid grid-cols-5 gap-2 text-center">
              {[
                { v: macro.calories, u: "kcal", l: "熱量" },
                { v: macro.proteinG, u: "g", l: "蛋白質" },
                { v: macro.carbsG, u: "g", l: "碳水" },
                { v: macro.fatG, u: "g", l: "脂肪" },
                { v: macro.fiberG, u: "g", l: "纖維" },
              ].map((m) => (
                <div key={m.l}>
                  <p className="text-base font-bold text-phantom-text tabular-nums">{m.v}<span className="text-[10px] text-phantom-muted">{m.u}</span></p>
                  <p className="text-[10px] text-phantom-muted">{m.l}</p>
                </div>
              ))}
            </div>
          ) : (
            <p className="text-xs text-phantom-muted">無法估算巨量營養素（信心不足或純飲料）</p>
          )}

          <div>
            <div className="flex items-center justify-between mb-1">
              <span className="text-xs text-phantom-muted">減脂對齊分數</span>
              <span className="text-xs text-phantom-primary">{Math.round((result.fatLossScore ?? 0) * 100)}%</span>
            </div>
            <div className="w-full h-1.5 bg-phantom-bg rounded-full overflow-hidden">
              <div className="h-full bg-phantom-primary rounded-full transition-all" style={{ width: `${Math.min(100, Math.max(0, (result.fatLossScore ?? 0) * 100))}%` }} />
            </div>
          </div>

          {result.suggestion && (
            <div className="bg-phantom-primary/10 border border-phantom-primary/30 rounded p-3 flex gap-2">
              <Sparkles size={14} className="text-phantom-primary flex-shrink-0 mt-0.5" />
              <p className="text-sm text-phantom-text">{result.suggestion}</p>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
