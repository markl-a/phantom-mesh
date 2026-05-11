use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

// ── Directory helpers ─────────────────────────────────────────────────────────

fn pages_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    let base = std::env::var("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("C:/Users/Default"));

    #[cfg(not(target_os = "windows"))]
    let base = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"));

    base.join(".phantom-mesh").join("pages")
}

fn page_db_path() -> PathBuf {
    pages_dir().join("page.db")
}

fn ensure_db() -> Result<Connection, String> {
    let db_path = page_db_path();
    if let Some(parent) = db_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create DB directory: {}", e))?;
    }
    let conn = Connection::open(&db_path)
        .map_err(|e| format!("Failed to open SQLite DB: {}", e))?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS page_kv (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );",
    )
    .map_err(|e| format!("Failed to create page_kv table: {}", e))?;
    Ok(conn)
}

// ── Bridge JS ─────────────────────────────────────────────────────────────────

const BRIDGE_JS: &str = r#"<script>
window.phantom = {
  _call: function(method, args) {
    return new Promise(function(resolve, reject) {
      var id = Math.random().toString(36).slice(2);
      var handler = function(e) {
        if (e.data && e.data.phantomId === id) {
          window.removeEventListener('message', handler);
          if (e.data.error) reject(new Error(e.data.error));
          else resolve(e.data.result);
        }
      };
      window.addEventListener('message', handler);
      window.parent.postMessage({ phantom: true, id: id, method: method, args: args }, '*');
    });
  },
  db: {
    get: function(key) { return window.phantom._call('page_db_get', { key: key }); },
    set: function(key, value) { return window.phantom._call('page_db_set', { key: key, value: JSON.stringify(value) }); },
    query: function(sql) { return window.phantom._call('page_db_query', { sql: sql }); },
  },
  agent: { run: function(prompt) { return window.phantom._call('send_message', { prompt: prompt }); } },
  notify: function(title, body) { return window.phantom._call('send_notification', { title: title, body: body }); },
  cluster: { nodes: function() { return window.phantom._call('get_cluster_status'); } },
};
</script>"#;

// ── Data structs ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PageInfo {
    pub name: String,
    pub title: String,
    pub description: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PageManifest {
    pub name: String,
    pub title: String,
    pub description: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
pub struct SavePageArgs {
    pub name: String,
    pub title: String,
    pub description: String,
    pub html: String,
}

#[derive(Debug, Serialize)]
pub struct LoadPageResult {
    pub html: String,
    pub name: String,
}

// ── Commands ──────────────────────────────────────────────────────────────────

/// List all pages by reading subdirectories of pages_dir and parsing manifest.json.
#[tauri::command]
pub fn list_pages() -> Result<Vec<PageInfo>, String> {
    let dir = pages_dir();
    if !dir.exists() {
        return Ok(vec![]);
    }

    let entries = fs::read_dir(&dir)
        .map_err(|e| format!("Failed to read pages directory: {}", e))?;

    let mut pages = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let manifest_path = path.join("manifest.json");
        if !manifest_path.exists() {
            continue;
        }
        let manifest_str = fs::read_to_string(&manifest_path)
            .map_err(|e| format!("Failed to read manifest.json: {}", e))?;
        let manifest: PageManifest = serde_json::from_str(&manifest_str)
            .map_err(|e| format!("Failed to parse manifest.json: {}", e))?;
        pages.push(PageInfo {
            name: manifest.name,
            title: manifest.title,
            description: manifest.description,
            created_at: manifest.created_at,
            updated_at: manifest.updated_at,
        });
    }

    Ok(pages)
}

/// Load a page's index.html, inject BRIDGE_JS into <head>, return html + name.
#[tauri::command]
pub fn load_page(name: String) -> Result<LoadPageResult, String> {
    let page_dir = pages_dir().join(&name);
    let html_path = page_dir.join("index.html");

    if !html_path.exists() {
        return Err(format!("Page '{}' not found", name));
    }

    let html = fs::read_to_string(&html_path)
        .map_err(|e| format!("Failed to read index.html: {}", e))?;

    // Inject BRIDGE_JS after <head> tag (case-insensitive)
    let injected = if let Some(pos) = html.to_lowercase().find("<head>") {
        let insert_at = pos + "<head>".len();
        let mut result = html.clone();
        result.insert_str(insert_at, BRIDGE_JS);
        result
    } else {
        // No <head> tag found — prepend bridge at top
        format!("{}{}", BRIDGE_JS, html)
    };

    Ok(LoadPageResult {
        html: injected,
        name,
    })
}

/// Write index.html + manifest.json for a page, return PageInfo.
#[tauri::command]
pub fn save_page(args: SavePageArgs) -> Result<PageInfo, String> {
    let page_dir = pages_dir().join(&args.name);
    fs::create_dir_all(&page_dir)
        .map_err(|e| format!("Failed to create page directory: {}", e))?;

    let now = chrono::Utc::now().to_rfc3339();

    // Read existing manifest for created_at if it exists
    let created_at = {
        let manifest_path = page_dir.join("manifest.json");
        if manifest_path.exists() {
            fs::read_to_string(&manifest_path)
                .ok()
                .and_then(|s| serde_json::from_str::<PageManifest>(&s).ok())
                .map(|m| m.created_at)
                .unwrap_or_else(|| now.clone())
        } else {
            now.clone()
        }
    };

    // Write index.html
    let html_path = page_dir.join("index.html");
    fs::write(&html_path, &args.html)
        .map_err(|e| format!("Failed to write index.html: {}", e))?;

    // Write manifest.json
    let manifest = PageManifest {
        name: args.name.clone(),
        title: args.title.clone(),
        description: args.description.clone(),
        created_at: created_at.clone(),
        updated_at: now.clone(),
    };
    let manifest_json = serde_json::to_string_pretty(&manifest)
        .map_err(|e| format!("Failed to serialize manifest: {}", e))?;
    let manifest_path = page_dir.join("manifest.json");
    fs::write(&manifest_path, manifest_json)
        .map_err(|e| format!("Failed to write manifest.json: {}", e))?;

    Ok(PageInfo {
        name: args.name,
        title: args.title,
        description: args.description,
        created_at,
        updated_at: now,
    })
}

/// Remove a page directory, return true on success.
#[tauri::command]
pub fn delete_page(name: String) -> Result<bool, String> {
    let page_dir = pages_dir().join(&name);
    if !page_dir.exists() {
        return Ok(false);
    }
    fs::remove_dir_all(&page_dir)
        .map_err(|e| format!("Failed to delete page '{}': {}", name, e))?;
    Ok(true)
}

/// Get a value by key from the page_kv SQLite table.
#[tauri::command]
pub fn page_db_get(key: String) -> Result<Option<String>, String> {
    let conn = ensure_db()?;
    let result: rusqlite::Result<Option<String>> = conn.query_row(
        "SELECT value FROM page_kv WHERE key = ?1",
        params![key],
        |row| row.get(0),
    ).map(Some).or_else(|e| {
        if matches!(e, rusqlite::Error::QueryReturnedNoRows) {
            Ok(None)
        } else {
            Err(e)
        }
    });
    result.map_err(|e| format!("DB get error: {}", e))
}

/// INSERT OR REPLACE a key-value pair into the page_kv table.
#[tauri::command]
pub fn page_db_set(key: String, value: String) -> Result<(), String> {
    let conn = ensure_db()?;
    conn.execute(
        "INSERT OR REPLACE INTO page_kv (key, value) VALUES (?1, ?2)",
        params![key, value],
    )
    .map_err(|e| format!("DB set error: {}", e))?;
    Ok(())
}

/// Execute a SELECT query — rejects INSERT/UPDATE/DELETE/DROP/ALTER/CREATE for security.
#[tauri::command]
pub fn page_db_query(sql: String) -> Result<Vec<serde_json::Value>, String> {
    // Security: only allow SELECT statements
    let trimmed = sql.trim().to_uppercase();
    let blocked = ["INSERT", "UPDATE", "DELETE", "DROP", "ALTER", "CREATE"];
    for keyword in &blocked {
        if trimmed.starts_with(keyword) {
            return Err(format!(
                "Security: '{}' queries are not allowed. Only SELECT is permitted.",
                keyword
            ));
        }
    }
    if !trimmed.starts_with("SELECT") {
        return Err("Security: Only SELECT queries are permitted.".to_string());
    }

    let conn = ensure_db()?;
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("DB query prepare error: {}", e))?;

    let column_names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();

    let rows_result: rusqlite::Result<Vec<serde_json::Value>> = stmt
        .query_map([], |row| {
            let mut obj = serde_json::Map::new();
            for (i, col) in column_names.iter().enumerate() {
                let val: rusqlite::types::Value = row.get(i)?;
                let json_val = match val {
                    rusqlite::types::Value::Null => serde_json::Value::Null,
                    rusqlite::types::Value::Integer(n) => serde_json::Value::Number(n.into()),
                    rusqlite::types::Value::Real(f) => serde_json::json!(f),
                    rusqlite::types::Value::Text(s) => serde_json::Value::String(s),
                    rusqlite::types::Value::Blob(b) => {
                        serde_json::Value::String(format!("<blob {} bytes>", b.len()))
                    }
                };
                obj.insert(col.clone(), json_val);
            }
            Ok(serde_json::Value::Object(obj))
        })
        .map_err(|e| format!("DB query error: {}", e))?
        .collect();

    rows_result.map_err(|e| format!("DB row error: {}", e))
}
