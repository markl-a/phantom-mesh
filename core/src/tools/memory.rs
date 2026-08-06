use serde_json::Value;

fn mem_path() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("SPECTYN_MEMORY_FILE") {
        return std::path::PathBuf::from(p);
    }
    crate::cli_config::spectyn_data_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join("memory.json")
}

/// Resolve the full storage key, incorporating an optional namespace prefix.
/// Namespaced keys are stored as `{namespace}/{key}`.
fn resolve_key(key: &str, namespace: Option<&str>) -> String {
    match namespace {
        Some(ns) if !ns.is_empty() => format!("{}/{}", ns, key),
        _ => key.to_string(),
    }
}

async fn load_map() -> serde_json::Map<String, Value> {
    let path = mem_path();
    if let Ok(data) = tokio::fs::read_to_string(&path).await {
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        serde_json::Map::new()
    }
}

async fn save_map(map: &serde_json::Map<String, Value>) {
    let path = mem_path();
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    let temp_path = path.with_extension("tmp");
    let _ = tokio::fs::write(
        &temp_path,
        serde_json::to_string_pretty(map).unwrap_or_default(),
    )
    .await;
    let _ = tokio::fs::rename(&temp_path, &path).await;
}

/// Store a key/value pair in memory.
/// If `namespace` is provided, the key is stored as `{namespace}/{key}`.
pub async fn store(args: &Value) -> String {
    let key = args["key"].as_str().unwrap_or("unknown");
    let value = args["value"].as_str().unwrap_or("");
    let namespace = args["namespace"].as_str();

    let full_key = resolve_key(key, namespace);
    let mut map = load_map().await;
    map.insert(full_key.clone(), Value::String(value.to_string()));
    save_map(&map).await;
    format!("Stored: {} = {}", full_key, value)
}

/// Recall a value by key from memory.
/// If `namespace` is provided, the key is looked up as `{namespace}/{key}`.
pub async fn recall(args: &Value) -> String {
    let key = args["key"].as_str().unwrap_or("");
    let namespace = args["namespace"].as_str();

    let full_key = resolve_key(key, namespace);
    let map = load_map().await;
    if let Some(val) = map.get(&full_key) {
        return val.as_str().unwrap_or(&val.to_string()).to_string();
    }
    format!("No memory found for key: {}", full_key)
}

/// List all stored keys, optionally filtered by a namespace prefix.
/// Values are truncated to 50 characters. Keys are returned in sorted order.
pub async fn list(args: &Value) -> String {
    let namespace = args["namespace"].as_str();
    let map = load_map().await;

    let prefix = match namespace {
        Some(ns) if !ns.is_empty() => Some(format!("{}/", ns)),
        _ => None,
    };

    let mut entries: Vec<(&String, &Value)> = map
        .iter()
        .filter(|(k, _)| match &prefix {
            Some(p) => k.starts_with(p.as_str()),
            None => true,
        })
        .collect();

    if entries.is_empty() {
        return match &prefix {
            Some(p) => format!(
                "No memory entries in namespace '{}'.",
                p.trim_end_matches('/')
            ),
            None => "No memory entries stored.".to_string(),
        };
    }

    entries.sort_by_key(|(k, _)| k.as_str());

    let lines: Vec<String> = entries
        .iter()
        .map(|(k, v)| {
            let raw = v
                .as_str()
                .map(|s| s.to_string())
                .unwrap_or_else(|| v.to_string());
            let truncated = if raw.chars().count() > 50 {
                format!("{}…", raw.chars().take(50).collect::<String>())
            } else {
                raw
            };
            format!("{}: {}", k, truncated)
        })
        .collect();

    lines.join("\n")
}

/// Delete a key from memory.
/// If `namespace` is provided, the key is resolved as `{namespace}/{key}`.
/// Returns "deleted" or "key not found".
pub async fn delete(args: &Value) -> String {
    let key = match args["key"].as_str() {
        Some(k) => k,
        None => return "Error: missing 'key' argument".to_string(),
    };
    let namespace = args["namespace"].as_str();

    let full_key = resolve_key(key, namespace);
    let mut map = load_map().await;

    if map.remove(&full_key).is_none() {
        return "key not found".to_string();
    }
    save_map(&map).await;
    "deleted".to_string()
}

/// Search memory entries whose values contain `query` (case-insensitive).
/// Optionally filter by namespace prefix. Returns a formatted list of matching entries.
pub async fn search(args: &Value) -> String {
    let query = match args["query"].as_str() {
        Some(q) => q.to_lowercase(),
        None => return "Error: missing 'query' argument".to_string(),
    };
    let namespace = args["namespace"].as_str();
    let map = load_map().await;

    let prefix = match namespace {
        Some(ns) if !ns.is_empty() => Some(format!("{}/", ns)),
        _ => None,
    };

    let mut results: Vec<String> = map
        .iter()
        .filter(|(k, v)| {
            let in_ns = match &prefix {
                Some(p) => k.starts_with(p.as_str()),
                None => true,
            };
            if !in_ns {
                return false;
            }
            let val_str = v
                .as_str()
                .map(|s| s.to_string())
                .unwrap_or_else(|| v.to_string());
            val_str.to_lowercase().contains(&query)
        })
        .map(|(k, v)| {
            let val_str = v
                .as_str()
                .map(|s| s.to_string())
                .unwrap_or_else(|| v.to_string());
            format!("{}: {}", k, val_str)
        })
        .collect();

    if results.is_empty() {
        return format!("No memory entries matching '{}'.", query);
    }

    results.sort();
    results.join("\n")
}
