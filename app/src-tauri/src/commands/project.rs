use serde_json::Value;
use std::path::PathBuf;

/// Get info about the current working directory / active project
#[tauri::command]
pub async fn get_project_info() -> Result<Value, String> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let info = phantom_mesh::project_info(&cwd);
    serde_json::to_value(info).map_err(|e| e.to_string())
}

/// Set the working directory for the agent session
#[tauri::command]
pub async fn set_project_cwd(path: String) -> Result<Value, String> {
    let p = PathBuf::from(&path);
    if !p.exists() {
        return Err(format!("Path does not exist: {}", path));
    }
    if !p.is_dir() {
        return Err(format!("Path is not a directory: {}", path));
    }
    std::env::set_current_dir(&p).map_err(|e| e.to_string())?;
    let info = phantom_mesh::project_info(&p);
    serde_json::to_value(info).map_err(|e| e.to_string())
}

/// List recently used projects from ~/.phantom-mesh/recent_projects.json
#[tauri::command]
pub async fn list_recent_projects() -> Result<Vec<Value>, String> {
    let path = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".phantom-mesh")
        .join("recent_projects.json");

    if !path.exists() {
        return Ok(vec![]);
    }

    let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let projects: Vec<Value> = serde_json::from_str(&content).unwrap_or_default();
    Ok(projects)
}

/// Add a path to the recent projects list
#[tauri::command]
pub async fn add_recent_project(cwd: String) -> Result<(), String> {
    let path = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".phantom-mesh")
        .join("recent_projects.json");

    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;

    let mut projects: Vec<Value> = if path.exists() {
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        vec![]
    };

    projects.retain(|p| p["cwd"].as_str() != Some(&cwd));
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    projects.insert(0, serde_json::json!({"cwd": cwd, "added_at": ts}));
    projects.truncate(20);

    let content = serde_json::to_string_pretty(&projects).map_err(|e| e.to_string())?;
    std::fs::write(&path, content).map_err(|e| e.to_string())
}
