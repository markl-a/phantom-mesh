// Tauri command surface for SPEC-20 food capture.
//
// Wraps `phantom_mesh::capture_food_wire` so the React food-log surface can
// drive photo/text → macro-estimate analysis through Tauri's invoke channel.
// Mirrors commands/capture_focus_wire.rs + capture_habit_wire.rs.
//
// analyze_food dispatches through `gemini_multimodal_pseudo` (SPEC-20 Stage-4
// vision fallback, still unimplemented), so we catch_unwind and surface a
// stable "food.not_yet_wired" string. Typed errors (image_too_large,
// no_food_detected, …) pass through verbatim.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

use phantom_mesh::capture_food_wire::{
    self, FoodAnalysisResult, FoodCaptureError, FoodCaptureRequest,
};

const NOT_YET_WIRED: &str =
    "food.not_yet_wired: SPEC-20 Stage 4 deferred — vision provider chain still unimplemented";

fn err_string(e: FoodCaptureError) -> String {
    e.to_string()
}

fn run_or_unimplemented<T>(f: impl FnOnce() -> Result<T, FoodCaptureError>) -> Result<T, String> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(err_string(e)),
        Err(_) => Err(NOT_YET_WIRED.to_string()),
    }
}

#[tauri::command]
pub async fn food_analyze(request: FoodCaptureRequest) -> Result<FoodAnalysisResult, String> {
    run_or_unimplemented(|| capture_food_wire::analyze_food(&request))
}

#[tauri::command]
pub async fn food_validate_image(path: String) -> Result<(), String> {
    run_or_unimplemented(|| capture_food_wire::validate_image_size(Path::new(&path)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_request() -> FoodCaptureRequest {
        FoodCaptureRequest {
            text: Some("中午的鮭魚便當".to_string()),
            image_path: None,
            kind: capture_food_wire::FOOD_LOG_KIND.to_string(),
            tag: vec!["fat_loss".to_string()],
            timestamp_ms: 1_716_000_000_000,
        }
    }

    // The wrapper's contract is panic-safety + a stable `food.` error prefix —
    // not a specific Ok/Err split (Stage-4 pseudo helpers may change).
    #[tokio::test]
    async fn food_analyze_returns_wellformed_result() {
        if let Err(e) = food_analyze(make_request()).await {
            assert!(e.starts_with("food."), "got {e}");
        }
    }

    #[tokio::test]
    async fn food_validate_image_returns_wellformed_result() {
        if let Err(e) = food_validate_image("/nonexistent/x.jpg".to_string()).await {
            assert!(e.starts_with("food."), "got {e}");
        }
    }

    #[test]
    fn err_string_carries_kebab_code_prefix() {
        let e = FoodCaptureError::ImageTooLarge { bytes: 99, max: 10 };
        let s = err_string(e);
        assert!(s.starts_with("food.image_too_large:"), "got {s}");
    }

    #[test]
    fn food_request_deserializes_from_camelcase_json() {
        let json = r#"{
            "text": "晚餐",
            "imagePath": null,
            "kind": "food_log",
            "tag": ["fat_loss"],
            "timestampMs": 1716000000000
        }"#;
        let req: FoodCaptureRequest = serde_json::from_str(json).expect("parse ok");
        assert_eq!(req.kind, "food_log");
        assert_eq!(req.tag, vec!["fat_loss".to_string()]);
    }
}
