//! Calendar tool — manage calendar events stored as JSON in workspace.

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;

use super::{Tool, ToolResult};

/// Path to the calendar events file in workspace.
fn calendar_path() -> PathBuf {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(format!("{}/.clawtex/workspace/calendar_events.json", home))
}

/// A single calendar event.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct CalendarEvent {
    id: String,
    title: String,
    date: String,
    time: Option<String>,
    description: Option<String>,
}

/// Load all events from the JSON file.
fn load_events(path: &PathBuf) -> Vec<CalendarEvent> {
    match std::fs::read_to_string(path) {
        Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

/// Save events to the JSON file.
fn save_events(path: &PathBuf, events: &[CalendarEvent]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let data = serde_json::to_string_pretty(events)?;
    std::fs::write(path, data)?;
    Ok(())
}

/// Generate a simple unique ID based on timestamp.
fn generate_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("evt_{}", ts)
}

/// Validate a date string is YYYY-MM-DD format and has valid ranges.
fn validate_date(date: &str) -> bool {
    if date.len() != 10 {
        return false;
    }
    let parts: Vec<&str> = date.split('-').collect();
    if parts.len() != 3 {
        return false;
    }
    let year = parts[0].parse::<u32>();
    let month = parts[1].parse::<u32>();
    let day = parts[2].parse::<u32>();
    match (year, month, day) {
        (Ok(y), Ok(m), Ok(d)) => y >= 2000 && y <= 2100 && m >= 1 && m <= 12 && d >= 1 && d <= 31,
        _ => false,
    }
}

/// Validate a time string is HH:MM format.
fn validate_time(time: &str) -> bool {
    if time.len() != 5 {
        return false;
    }
    let parts: Vec<&str> = time.split(':').collect();
    if parts.len() != 2 {
        return false;
    }
    let hour = parts[0].parse::<u32>();
    let minute = parts[1].parse::<u32>();
    match (hour, minute) {
        (Ok(h), Ok(m)) => h < 24 && m < 60,
        _ => false,
    }
}

/// Calculate the date range for "this week" (Mon-Sun) given a reference date.
fn week_range(today: &str) -> Option<(String, String)> {
    let parts: Vec<u32> = today.split('-').filter_map(|s| s.parse().ok()).collect();
    if parts.len() != 3 {
        return None;
    }
    let (year, month, day) = (parts[0] as i32, parts[1], parts[2]);

    // Simple day-of-week calculation (Zeller-like for Gregorian).
    // Using Tomohiko Sakamoto's algorithm.
    let t = [0i32, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let mut y = year;
    if month < 3 {
        y -= 1;
    }
    let dow = ((y + y / 4 - y / 100 + y / 400 + t[(month - 1) as usize] + day as i32) % 7) as i32;
    // dow: 0=Sun, 1=Mon, ..., 6=Sat
    // We want Mon=0 .. Sun=6
    let weekday = if dow == 0 { 6 } else { dow - 1 };

    // Calculate start of week (Monday)
    let start_offset = weekday;
    let end_offset = 6 - weekday;

    // Simple date arithmetic using days since epoch-ish
    let to_days = |y: i32, m: u32, d: u32| -> i32 {
        let mut total = 0i32;
        // Rough: days = y*365 + leap_days + month_days + d
        total += y * 365 + y / 4 - y / 100 + y / 400;
        let month_days = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
        total += month_days[(m - 1) as usize];
        if m > 2 && (y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)) {
            total += 1;
        }
        total += d as i32;
        total
    };

    let from_days = |mut days: i32| -> (i32, u32, u32) {
        // Inverse of to_days (approximate Gregorian)
        let mut y = (days * 400) / 146097;
        let mut remainder = days - to_days(y, 1, 1) + 1;
        if remainder <= 0 {
            y -= 1;
            remainder = days - to_days(y, 1, 1) + 1;
        }
        let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
        let month_lens: [i32; 12] = [
            31,
            if leap { 29 } else { 28 },
            31, 30, 31, 30, 31, 31, 30, 31, 30, 31,
        ];
        let mut m = 0u32;
        for (i, &ml) in month_lens.iter().enumerate() {
            if remainder <= ml {
                m = (i + 1) as u32;
                break;
            }
            remainder -= ml;
        }
        if m == 0 {
            m = 12;
        }
        (y, m, remainder as u32)
    };

    let today_days = to_days(year, month, day);
    let start_days = today_days - start_offset;
    let end_days = today_days + end_offset;

    let (sy, sm, sd) = from_days(start_days);
    let (ey, em, ed) = from_days(end_days);

    Some((
        format!("{:04}-{:02}-{:02}", sy, sm, sd),
        format!("{:04}-{:02}-{:02}", ey, em, ed),
    ))
}

/// Get today's date as YYYY-MM-DD.
fn today_date() -> String {
    // Use chrono-free approach
    let now = std::time::SystemTime::now();
    let since_epoch = now.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let secs = since_epoch.as_secs() as i64;

    // Convert epoch seconds to date
    let days = secs / 86400;
    let mut y = 1970i32;
    let mut remaining_days = days;

    loop {
        let year_days = if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) {
            366i64
        } else {
            365i64
        };
        if remaining_days < year_days {
            break;
        }
        remaining_days -= year_days;
        y += 1;
    }

    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let month_days: [i64; 12] = [
        31,
        if leap { 29 } else { 28 },
        31, 30, 31, 30, 31, 31, 30, 31, 30, 31,
    ];
    let mut m = 1u32;
    for &md in &month_days {
        if remaining_days < md {
            break;
        }
        remaining_days -= md;
        m += 1;
    }
    let d = remaining_days + 1;

    format!("{:04}-{:02}-{:02}", y, m, d)
}

pub struct CalendarTool;

impl CalendarTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for CalendarTool {
    fn name(&self) -> &str {
        "calendar"
    }

    fn description(&self) -> &str {
        "Manage calendar events. Operations: add, list, delete, today, week. Events stored in workspace as JSON."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "description": "One of: add, list, delete, today, week"
                },
                "title": {
                    "type": "string",
                    "description": "Event title (required for add)"
                },
                "date": {
                    "type": "string",
                    "description": "Event date in YYYY-MM-DD format (required for add, optional for list)"
                },
                "time": {
                    "type": "string",
                    "description": "Event time in HH:MM format (optional)"
                },
                "description": {
                    "type": "string",
                    "description": "Event description (optional)"
                },
                "id": {
                    "type": "string",
                    "description": "Event ID (required for delete)"
                }
            },
            "required": ["operation"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let operation = args["operation"].as_str().unwrap_or("").trim();
        if operation.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: "Missing required parameter: operation".into(),
            });
        }

        let path = calendar_path();

        match operation {
            "add" => {
                let title = args["title"].as_str().unwrap_or("").trim();
                let date = args["date"].as_str().unwrap_or("").trim();
                if title.is_empty() {
                    return Ok(ToolResult {
                        success: false,
                        output: "Missing required parameter: title".into(),
                    });
                }
                if date.is_empty() {
                    return Ok(ToolResult {
                        success: false,
                        output: "Missing required parameter: date".into(),
                    });
                }
                if !validate_date(date) {
                    return Ok(ToolResult {
                        success: false,
                        output: format!("Invalid date format: '{}'. Expected YYYY-MM-DD.", date),
                    });
                }
                let time = args["time"].as_str().map(|s| s.trim().to_string());
                if let Some(ref t) = time {
                    if !t.is_empty() && !validate_time(t) {
                        return Ok(ToolResult {
                            success: false,
                            output: format!("Invalid time format: '{}'. Expected HH:MM.", t),
                        });
                    }
                }
                let description = args["description"]
                    .as_str()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());

                let id = generate_id();
                let event = CalendarEvent {
                    id: id.clone(),
                    title: title.to_string(),
                    date: date.to_string(),
                    time: time.filter(|t| !t.is_empty()),
                    description,
                };

                let mut events = load_events(&path);
                events.push(event.clone());
                save_events(&path, &events)?;

                Ok(ToolResult {
                    success: true,
                    output: json!({
                        "message": "Event added successfully",
                        "id": id,
                        "title": event.title,
                        "date": event.date,
                        "time": event.time,
                        "total_events": events.len()
                    })
                    .to_string(),
                })
            }
            "list" => {
                let events = load_events(&path);
                let date_filter = args["date"].as_str().map(|s| s.trim());

                let filtered: Vec<&CalendarEvent> = if let Some(d) = date_filter {
                    events.iter().filter(|e| e.date == d).collect()
                } else {
                    events.iter().collect()
                };

                // Sort by date and time
                let mut sorted = filtered.clone();
                sorted.sort_by(|a, b| {
                    a.date
                        .cmp(&b.date)
                        .then(a.time.as_deref().unwrap_or("").cmp(b.time.as_deref().unwrap_or("")))
                });

                Ok(ToolResult {
                    success: true,
                    output: json!({
                        "count": sorted.len(),
                        "events": sorted
                    })
                    .to_string(),
                })
            }
            "delete" => {
                let id = args["id"].as_str().unwrap_or("").trim();
                if id.is_empty() {
                    return Ok(ToolResult {
                        success: false,
                        output: "Missing required parameter: id".into(),
                    });
                }

                let mut events = load_events(&path);
                let original_len = events.len();
                events.retain(|e| e.id != id);
                let removed = original_len - events.len();

                if removed == 0 {
                    return Ok(ToolResult {
                        success: false,
                        output: format!("Event with id '{}' not found", id),
                    });
                }

                save_events(&path, &events)?;
                Ok(ToolResult {
                    success: true,
                    output: json!({
                        "message": "Event deleted",
                        "deleted_id": id,
                        "remaining_events": events.len()
                    })
                    .to_string(),
                })
            }
            "today" => {
                let today = today_date();
                let events = load_events(&path);
                let todays: Vec<&CalendarEvent> =
                    events.iter().filter(|e| e.date == today).collect();

                Ok(ToolResult {
                    success: true,
                    output: json!({
                        "date": today,
                        "count": todays.len(),
                        "events": todays
                    })
                    .to_string(),
                })
            }
            "week" => {
                let today = today_date();
                let events = load_events(&path);

                if let Some((start, end)) = week_range(&today) {
                    let week_events: Vec<&CalendarEvent> = events
                        .iter()
                        .filter(|e| e.date >= start && e.date <= end)
                        .collect();

                    Ok(ToolResult {
                        success: true,
                        output: json!({
                            "week_start": start,
                            "week_end": end,
                            "count": week_events.len(),
                            "events": week_events
                        })
                        .to_string(),
                    })
                } else {
                    Ok(ToolResult {
                        success: false,
                        output: "Failed to calculate week range".into(),
                    })
                }
            }
            _ => Ok(ToolResult {
                success: false,
                output: format!(
                    "Unknown operation: '{}'. Use: add, list, delete, today, week",
                    operation
                ),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    // Use a unique temp file per test to avoid conflicts
    static TEST_COUNTER: AtomicU32 = AtomicU32::new(0);

    fn test_calendar_path() -> PathBuf {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join("clawtex_calendar_test");
        let _ = std::fs::create_dir_all(&dir);
        dir.join(format!("calendar_test_{}.json", id))
    }

    // Helper to run calendar operations against a temp file
    async fn exec_with_path(path: &PathBuf, args: Value) -> ToolResult {
        let operation = args["operation"].as_str().unwrap_or("").trim().to_string();

        match operation.as_str() {
            "add" => {
                let title = args["title"].as_str().unwrap_or("").trim().to_string();
                let date = args["date"].as_str().unwrap_or("").trim().to_string();
                if title.is_empty() {
                    return ToolResult { success: false, output: "Missing required parameter: title".into() };
                }
                if date.is_empty() {
                    return ToolResult { success: false, output: "Missing required parameter: date".into() };
                }
                if !validate_date(&date) {
                    return ToolResult { success: false, output: format!("Invalid date format: '{}'. Expected YYYY-MM-DD.", date) };
                }
                let time = args["time"].as_str().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
                if let Some(ref t) = time {
                    if !validate_time(t) {
                        return ToolResult { success: false, output: format!("Invalid time format: '{}'. Expected HH:MM.", t) };
                    }
                }
                let description = args["description"].as_str().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
                let id = generate_id();
                let event = CalendarEvent { id: id.clone(), title, date, time, description };
                let mut events = load_events(path);
                events.push(event);
                save_events(path, &events).unwrap();
                ToolResult { success: true, output: json!({"message": "Event added", "id": id, "total_events": events.len()}).to_string() }
            }
            "list" => {
                let events = load_events(path);
                let date_filter = args["date"].as_str().map(|s| s.trim().to_string());
                let filtered: Vec<&CalendarEvent> = if let Some(ref d) = date_filter {
                    events.iter().filter(|e| e.date == *d).collect()
                } else {
                    events.iter().collect()
                };
                ToolResult { success: true, output: json!({"count": filtered.len(), "events": filtered}).to_string() }
            }
            "delete" => {
                let id = args["id"].as_str().unwrap_or("").trim().to_string();
                if id.is_empty() {
                    return ToolResult { success: false, output: "Missing required parameter: id".into() };
                }
                let mut events = load_events(path);
                let orig = events.len();
                events.retain(|e| e.id != id);
                if events.len() == orig {
                    return ToolResult { success: false, output: format!("Event with id '{}' not found", id) };
                }
                save_events(path, &events).unwrap();
                ToolResult { success: true, output: json!({"message": "Event deleted", "remaining": events.len()}).to_string() }
            }
            _ => ToolResult { success: false, output: format!("Unknown operation: '{}'", operation) },
        }
    }

    #[test]
    fn test_name() {
        assert_eq!(CalendarTool::new().name(), "calendar");
    }

    #[test]
    fn test_description() {
        let tool = CalendarTool::new();
        assert!(tool.description().contains("calendar"));
    }

    #[test]
    fn test_schema() {
        let tool = CalendarTool::new();
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["operation"].is_object());
        assert!(schema["properties"]["title"].is_object());
        assert!(schema["properties"]["date"].is_object());
        assert!(schema["properties"]["time"].is_object());
    }

    #[test]
    fn test_validate_date_valid() {
        assert!(validate_date("2026-03-18"));
        assert!(validate_date("2026-01-01"));
        assert!(validate_date("2026-12-31"));
    }

    #[test]
    fn test_validate_date_invalid() {
        assert!(!validate_date("2026-13-01"));
        assert!(!validate_date("2026-00-01"));
        assert!(!validate_date("2026-01-00"));
        assert!(!validate_date("not-a-date"));
        assert!(!validate_date("20260318"));
        assert!(!validate_date(""));
    }

    #[test]
    fn test_validate_time_valid() {
        assert!(validate_time("00:00"));
        assert!(validate_time("23:59"));
        assert!(validate_time("12:30"));
    }

    #[test]
    fn test_validate_time_invalid() {
        assert!(!validate_time("24:00"));
        assert!(!validate_time("12:60"));
        assert!(!validate_time("noon"));
        assert!(!validate_time(""));
    }

    #[tokio::test]
    async fn test_add_event() {
        let path = test_calendar_path();
        let result = exec_with_path(&path, json!({
            "operation": "add",
            "title": "Team Meeting",
            "date": "2026-03-20",
            "time": "10:00",
            "description": "Weekly sync"
        })).await;
        assert!(result.success);
        assert!(result.output.contains("Event added"));
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn test_add_missing_title() {
        let path = test_calendar_path();
        let result = exec_with_path(&path, json!({
            "operation": "add",
            "date": "2026-03-20"
        })).await;
        assert!(!result.success);
        assert!(result.output.contains("title"));
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn test_add_missing_date() {
        let path = test_calendar_path();
        let result = exec_with_path(&path, json!({
            "operation": "add",
            "title": "Meeting"
        })).await;
        assert!(!result.success);
        assert!(result.output.contains("date"));
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn test_add_invalid_date() {
        let path = test_calendar_path();
        let result = exec_with_path(&path, json!({
            "operation": "add",
            "title": "Meeting",
            "date": "not-valid"
        })).await;
        assert!(!result.success);
        assert!(result.output.contains("Invalid date"));
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn test_add_invalid_time() {
        let path = test_calendar_path();
        let result = exec_with_path(&path, json!({
            "operation": "add",
            "title": "Meeting",
            "date": "2026-03-20",
            "time": "25:00"
        })).await;
        assert!(!result.success);
        assert!(result.output.contains("Invalid time"));
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn test_list_events() {
        let path = test_calendar_path();
        // Add two events
        exec_with_path(&path, json!({"operation": "add", "title": "A", "date": "2026-03-20"})).await;
        exec_with_path(&path, json!({"operation": "add", "title": "B", "date": "2026-03-21"})).await;

        let result = exec_with_path(&path, json!({"operation": "list"})).await;
        assert!(result.success);
        let v: Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(v["count"].as_u64().unwrap(), 2);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn test_list_filter_by_date() {
        let path = test_calendar_path();
        exec_with_path(&path, json!({"operation": "add", "title": "A", "date": "2026-03-20"})).await;
        exec_with_path(&path, json!({"operation": "add", "title": "B", "date": "2026-03-21"})).await;

        let result = exec_with_path(&path, json!({"operation": "list", "date": "2026-03-20"})).await;
        assert!(result.success);
        let v: Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(v["count"].as_u64().unwrap(), 1);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn test_delete_event() {
        let path = test_calendar_path();
        let add_result = exec_with_path(&path, json!({"operation": "add", "title": "ToDelete", "date": "2026-03-20"})).await;
        let v: Value = serde_json::from_str(&add_result.output).unwrap();
        let id = v["id"].as_str().unwrap();

        let del_result = exec_with_path(&path, json!({"operation": "delete", "id": id})).await;
        assert!(del_result.success);
        assert!(del_result.output.contains("deleted"));

        let list_result = exec_with_path(&path, json!({"operation": "list"})).await;
        let v2: Value = serde_json::from_str(&list_result.output).unwrap();
        assert_eq!(v2["count"].as_u64().unwrap(), 0);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn test_delete_nonexistent() {
        let path = test_calendar_path();
        let result = exec_with_path(&path, json!({"operation": "delete", "id": "nonexistent"})).await;
        assert!(!result.success);
        assert!(result.output.contains("not found"));
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn test_delete_missing_id() {
        let path = test_calendar_path();
        let result = exec_with_path(&path, json!({"operation": "delete"})).await;
        assert!(!result.success);
        assert!(result.output.contains("id"));
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn test_unknown_operation() {
        let tool = CalendarTool::new();
        let result = tool.execute(json!({"operation": "invalid"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Unknown operation"));
    }

    #[tokio::test]
    async fn test_missing_operation() {
        let tool = CalendarTool::new();
        let result = tool.execute(json!({})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Missing"));
    }

    #[test]
    fn test_week_range_calculation() {
        // 2026-03-18 is a Wednesday
        let range = week_range("2026-03-18");
        assert!(range.is_some());
        let (start, end) = range.unwrap();
        // Monday should be 2026-03-16, Sunday should be 2026-03-22
        assert_eq!(start, "2026-03-16");
        assert_eq!(end, "2026-03-22");
    }

    #[test]
    fn test_load_events_empty_file() {
        let path = test_calendar_path();
        let events = load_events(&path);
        assert!(events.is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_save_and_load_events() {
        let path = test_calendar_path();
        let events = vec![CalendarEvent {
            id: "test1".into(),
            title: "Test Event".into(),
            date: "2026-03-20".into(),
            time: Some("14:00".into()),
            description: Some("A test".into()),
        }];
        save_events(&path, &events).unwrap();
        let loaded = load_events(&path);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].title, "Test Event");
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn test_today_operation() {
        let tool = CalendarTool::new();
        let result = tool.execute(json!({"operation": "today"})).await.unwrap();
        assert!(result.success);
        let v: Value = serde_json::from_str(&result.output).unwrap();
        assert!(v["date"].is_string());
        assert!(v["count"].is_number());
    }

    #[tokio::test]
    async fn test_week_operation() {
        let tool = CalendarTool::new();
        let result = tool.execute(json!({"operation": "week"})).await.unwrap();
        assert!(result.success);
        let v: Value = serde_json::from_str(&result.output).unwrap();
        assert!(v["week_start"].is_string());
        assert!(v["week_end"].is_string());
    }
}
