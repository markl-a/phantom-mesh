use serde::Serialize;

#[derive(Serialize)]
pub struct BrowserNavigateResult {
    pub success: bool,
    pub output: String,
    pub screenshot_path: Option<String>,
}

#[derive(Serialize)]
pub struct BrowserSnapshotResult {
    pub success: bool,
    pub text: String,
}

#[derive(Serialize)]
pub struct BrowserStatusResult {
    pub active: bool,
    pub current_url: Option<String>,
}

#[tauri::command]
pub async fn browser_navigate(url: String) -> Result<BrowserNavigateResult, String> {
    // Browser tool uses web_search internally; direct navigation deferred to v0.2
    Ok(BrowserNavigateResult {
        success: true,
        output: format!("Navigation to {} queued (browser tool not yet integrated)", url),
        screenshot_path: None,
    })
}

#[tauri::command]
pub async fn browser_screenshot() -> Result<String, String> {
    Err("Browser screenshot not yet available (v0.2)".to_string())
}

#[tauri::command]
pub async fn browser_snapshot() -> Result<BrowserSnapshotResult, String> {
    Ok(BrowserSnapshotResult {
        success: false,
        text: "Browser snapshot not yet available (v0.2)".to_string(),
    })
}

#[tauri::command]
pub async fn browser_status() -> Result<BrowserStatusResult, String> {
    Ok(BrowserStatusResult { active: false, current_url: None })
}

#[tauri::command]
pub async fn browser_close() -> Result<bool, String> {
    Ok(true)
}
