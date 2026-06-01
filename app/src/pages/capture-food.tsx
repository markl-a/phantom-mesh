// SPEC-31 capture-food — route /capture/food; commands_used: capture_food_* (via lib/captureFood)

import { useState } from "react";
import { Camera, Check, Loader2, Utensils } from "lucide-react";
import {
  analyzeFood,
  buildFoodRequest,
  describeFoodError,
} from "../lib/captureFood";
import type { FoodAnalysisResult } from "../lib/generated/capture_food/FoodAnalysisResult";
import { useHaptics } from "../lib/useHaptics";

export default function CaptureFood() {
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<FoodAnalysisResult | null>(null);
  const { impact } = useHaptics();

  const canSubmit = text.trim().length > 0;

  async function submit() {
    if (busy || !canSubmit) return;

    setBusy(true);
    setError(null);
    setResult(null);

    try {
      // Camera/photo capture is a native bridge (SPEC-30 deferred), so we send a
      // text-only request with no imagePath. buildFoodRequest trims + builds the wire shape.
      const request = buildFoodRequest(text);
      const analysis = await analyzeFood(request);
      setResult(analysis);
      impact("medium");
    } catch (e) {
      setError(describeFoodError(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div
      data-testid="capture-food"
      className="min-h-screen bg-phantom-bg text-phantom-text pt-[env(safe-area-inset-top)] pb-[env(safe-area-inset-bottom)] px-[env(safe-area-inset-left)] pr-[env(safe-area-inset-right)]"
    >
      <div className="flex min-h-screen flex-col">
        <main className="flex-1 overflow-y-auto px-4 pb-6 pt-5">
          <header className="mb-5">
            <div className="mb-2 flex items-center gap-2 text-phantom-primary">
              <Utensils aria-hidden="true" className="size-5" />
              <span className="text-base font-medium">飲食 / Food</span>
            </div>
            <h1 className="text-lg font-semibold">記錄這餐 / Log a meal</h1>
            <p className="mt-2 text-base text-phantom-muted">
              描述吃了什麼，估算熱量與營養。
            </p>
          </header>

          {/* Honest not-ready note for camera capture (native bridge deferred). */}
          <section
            aria-label="拍照尚未支援 / Camera capture not yet supported"
            className="mb-5 flex items-start gap-3 rounded-lg border border-phantom-border bg-phantom-card px-3 py-3"
          >
            <Camera aria-hidden="true" className="mt-0.5 size-5 shrink-0 text-phantom-warning" />
            <p className="text-base text-phantom-muted">
              拍照需相機權限與原生橋接（稍後支援）/ Camera capture needs native bridge (coming soon)
            </p>
          </section>

          <section className="space-y-3">
            <label className="block">
              <span className="mb-2 block text-base text-phantom-muted">
                描述這餐 / Describe this meal
              </span>
              <textarea
                value={text}
                onChange={(e) => {
                  setText(e.target.value);
                  setError(null);
                  setResult(null);
                }}
                aria-label="描述這餐 / Describe this meal"
                placeholder="例：雞胸肉沙拉、一碗白飯、半顆酪梨"
                className="min-h-[120px] w-full resize-none rounded-lg border border-phantom-border bg-phantom-card px-3 py-2 text-base text-phantom-text placeholder:text-phantom-muted"
              />
            </label>

            {error && (
              <p role="alert" className="text-base text-phantom-warning">
                {error}
              </p>
            )}

            {result && (
              <div role="status" className="space-y-3 rounded-lg border border-phantom-border bg-phantom-card px-3 py-3">
                <p className="flex items-center gap-2 text-base font-medium text-phantom-success">
                  <Check aria-hidden="true" className="size-5" />
                  分析完成 / Analysis ready
                </p>

                <p className="text-base text-phantom-text">{result.summary}</p>

                {result.macroEstimate ? (
                  <dl className="grid grid-cols-2 gap-x-4 gap-y-1 text-base">
                    <dt className="text-phantom-muted">熱量 / Calories</dt>
                    <dd className="text-right text-phantom-text">
                      {Math.round(result.macroEstimate.calories)} kcal
                    </dd>
                    <dt className="text-phantom-muted">蛋白質 / Protein</dt>
                    <dd className="text-right text-phantom-text">
                      {Math.round(result.macroEstimate.proteinG)} g
                    </dd>
                    <dt className="text-phantom-muted">碳水 / Carbs</dt>
                    <dd className="text-right text-phantom-text">
                      {Math.round(result.macroEstimate.carbsG)} g
                    </dd>
                    <dt className="text-phantom-muted">脂肪 / Fat</dt>
                    <dd className="text-right text-phantom-text">
                      {Math.round(result.macroEstimate.fatG)} g
                    </dd>
                  </dl>
                ) : (
                  <p className="text-base text-phantom-muted">
                    信心不足，未提供營養估算 / Low confidence, no macro estimate
                  </p>
                )}

                {/* suggestion is typed `string` but a partial/unwired backend result can
                    omit it at runtime — guard before .trim() so the screen never white-screens. */}
                {(result.suggestion ?? "").trim().length > 0 && (
                  <p className="text-base text-phantom-muted">
                    建議 / Suggestion: {result.suggestion}
                  </p>
                )}
              </div>
            )}
          </section>
        </main>

        <footer className="sticky bottom-0 border-t border-phantom-border bg-phantom-bg px-4 py-3">
          <button
            type="button"
            aria-label="分析這餐 / Analyze meal"
            disabled={busy || !canSubmit}
            onClick={submit}
            className="flex min-h-[48px] w-full items-center justify-center gap-2 rounded-lg bg-phantom-primary px-4 py-3 text-base font-semibold text-phantom-bg transition disabled:opacity-50 motion-reduce:transition-none"
          >
            {busy && <Loader2 aria-hidden="true" className="size-5 animate-spin motion-reduce:animate-none" />}
            {busy ? "分析中 / Analyzing" : "分析這餐 / Analyze meal"}
          </button>
        </footer>
      </div>
    </div>
  );
}
