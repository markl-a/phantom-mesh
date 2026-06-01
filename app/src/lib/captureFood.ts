// Helper for SPEC-20 food capture — thin wrapper over the Tauri commands in
// `app/src-tauri/src/commands/capture_food_wire.rs`:
//   - food_analyze(request)      → FoodAnalysisResult
//   - food_validate_image(path)  → void
//
// The vision provider chain is SPEC-20 Stage-4 deferred, so food_analyze
// surfaces a stable `food.not_yet_wired:` prefix until it lands; callers gate
// UI on describeFoodError. Mirrors lib/captureFocus.ts + captureHabit.ts.

import { safeInvoke as invoke } from "./tauri-compat";
import type { FoodCaptureRequest } from "./generated/capture_food/FoodCaptureRequest";
import type { FoodAnalysisResult } from "./generated/capture_food/FoodAnalysisResult";

export const FOOD_LOG_KIND = "food_log";

/** Build a text-based food capture request for "now". */
export function buildFoodRequest(
  text: string,
  opts: { imagePath?: string | null; tag?: string[] } = {},
): FoodCaptureRequest {
  return {
    text: text.trim() || null,
    imagePath: opts.imagePath ?? null,
    kind: FOOD_LOG_KIND,
    tag: opts.tag && opts.tag.length > 0 ? opts.tag : ["fat_loss"],
    timestampMs: BigInt(Date.now()),
  };
}

/** Analyze a food capture → macro estimate + fat-loss score + suggestion. */
export async function analyzeFood(request: FoodCaptureRequest): Promise<FoodAnalysisResult> {
  // `timestampMs` is a BigInt (ts-rs maps Rust i64), but Tauri's invoke serializes
  // args as JSON which cannot encode BigInt → "Do not know how to serialize a BigInt"
  // crashes food capture. Coerce to a plain number on the wire (epoch-ms « 2^53).
  // Same BigInt-invoke class as onboarding/focus; systemic fix = coerceBigInts() in tauri-compat.
  const wire = { ...request, timestampMs: Number(request.timestampMs) };
  return invoke<FoodAnalysisResult>("food_analyze", { request: wire });
}

/** Map a food.<code>:<detail> wire error to a UI-friendly Chinese string. */
export function describeFoodError(err: unknown): string {
  const s = String(err ?? "").trim();
  if (s.startsWith("food.not_yet_wired")) return "影像分析後端尚未實作（SPEC-20 Stage 4 deferred）";
  if (s.startsWith("food.image_too_large")) return "圖片太大（上限 10 MB）";
  if (s.startsWith("food.image_unreadable")) return "圖片無法讀取，請重拍";
  if (s.startsWith("food.no_food_detected")) return "畫面中沒偵測到食物";
  if (s.startsWith("food.provider_failed")) return "分析服務暫時無法使用，稍後再試";
  if (s.startsWith("food.decrypt")) return "解密失敗（請先解鎖金鑰）";
  return s || "未知錯誤";
}
