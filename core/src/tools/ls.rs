use serde_json::Value;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::tools::file::safe_path;

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

#[allow(dead_code)]
fn format_size(size: u64) -> String {
    if size < 1024 {
        format!("{}", size)
    } else if size < 1024 * 1024 {
        format!("{:.1}K", size as f64 / 1024.0)
    } else if size < 1024 * 1024 * 1024 {
        format!("{:.1}M", size as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1}G", size as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

fn format_mtime(mtime: SystemTime) -> String {
    let secs = mtime
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Format as YYYY-MM-DD using integer arithmetic (no chrono dependency)
    let (year, month, day) = secs_to_ymd(secs);
    format!("{:04}-{:02}-{:02}", year, month, day)
}

fn format_datetime(t: SystemTime) -> String {
    let secs = t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let (year, month, day) = secs_to_ymd(secs);
    let rem = secs % 86400;
    let hour = rem / 3600;
    let min = (rem % 3600) / 60;
    let sec = rem % 60;
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
        year, month, day, hour, min, sec
    )
}

/// Convert Unix timestamp (seconds) to (year, month, day) using Gregorian calendar.
fn secs_to_ymd(secs: u64) -> (u32, u32, u32) {
    // Days since epoch
    let days = (secs / 86400) as u32;
    // Shift epoch from 1970-01-01 to 0000-03-01 for easier leap-year math
    let days = days + 719468;
    let era = days / 146097;
    let doe = days - era * 146097; // day of era [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

#[cfg(unix)]
fn mode_string(meta: &fs::Metadata) -> String {
    use std::os::unix::fs::PermissionsExt;
    let mode = meta.permissions().mode();
    let is_dir = meta.is_dir();
    let type_char = if is_dir { 'd' } else { '-' };
    let chars = [
        (mode & 0o400 != 0, 'r'),
        (mode & 0o200 != 0, 'w'),
        (mode & 0o100 != 0, 'x'),
        (mode & 0o040 != 0, 'r'),
        (mode & 0o020 != 0, 'w'),
        (mode & 0o010 != 0, 'x'),
        (mode & 0o004 != 0, 'r'),
        (mode & 0o002 != 0, 'w'),
        (mode & 0o001 != 0, 'x'),
    ];
    let bits: String = chars
        .iter()
        .map(|(set, ch)| if *set { *ch } else { '-' })
        .collect();
    format!("{}{}", type_char, bits)
}

#[cfg(not(unix))]
fn mode_string(meta: &fs::Metadata) -> String {
    if meta.is_dir() {
        "drwxr-xr-x".to_string()
    } else {
        "-rw-r--r--".to_string()
    }
}

#[cfg(unix)]
fn perms_octal(meta: &fs::Metadata) -> String {
    use std::os::unix::fs::PermissionsExt;
    format!("{:03o}", meta.permissions().mode() & 0o777)
}

#[cfg(not(unix))]
fn perms_octal(meta: &fs::Metadata) -> String {
    if meta.permissions().readonly() {
        "444".to_string()
    } else {
        "644".to_string()
    }
}

// ---------------------------------------------------------------------------
// list (ls)
// ---------------------------------------------------------------------------

pub async fn list(args: &Value) -> String {
    let path_str = args["path"].as_str().unwrap_or(".");
    let long = args["long"].as_bool().unwrap_or(false);
    let tree = args["tree"].as_bool().unwrap_or(false);
    let hidden = args["hidden"].as_bool().unwrap_or(false);
    let max_entries = args["max_entries"].as_u64().unwrap_or(200) as usize;

    // [T7f] Workspace-boundary check (PR #75 audit H-7). Without this,
    // `ls /etc` or `ls C:\Windows\System32` dumps an entire system
    // directory tree — fingerprinting target host + SSH-key file
    // recon in one shot.
    let root = match safe_path(path_str) {
        Ok(p) => p,
        Err(e) => return format!("Error: invalid path: {}", e),
    };

    if !root.exists() {
        return format!("Error: path does not exist: {}", path_str);
    }

    if tree {
        let mut out = format!("{}\n", root.display());
        render_tree(&root, "", 0, 3, hidden, max_entries, &mut 0usize, &mut out);
        return out;
    }

    let entries = match read_dir_sorted(&root, hidden) {
        Ok(e) => e,
        Err(err) => return format!("Error reading {}: {}", path_str, err),
    };

    let entries: Vec<_> = entries.into_iter().take(max_entries).collect();
    let truncated = {
        let total = read_dir_sorted(&root, hidden).map(|e| e.len()).unwrap_or(0);
        total > max_entries
    };

    let mut lines: Vec<String> = Vec::new();

    if long {
        for (name, meta) in &entries {
            let mode = mode_string(meta);
            let size = if meta.is_dir() {
                "-".to_string()
            } else {
                format!("{}", meta.len())
            };
            let mtime = meta
                .modified()
                .map(format_mtime)
                .unwrap_or_else(|_| "unknown".to_string());
            let display_name = if meta.is_dir() {
                format!("{}/", name)
            } else {
                name.clone()
            };
            lines.push(format!(
                "{:<10}  {:>8}  {}  {}",
                mode, size, mtime, display_name
            ));
        }
    } else {
        for (name, meta) in &entries {
            if meta.is_dir() {
                lines.push(format!("{}/", name));
            } else {
                lines.push(name.clone());
            }
        }
    }

    let mut out = lines.join("\n");
    if truncated {
        out.push_str(&format!(
            "\n\n[truncated — showing {} of {} entries; use max_entries to see more]",
            max_entries,
            read_dir_sorted(&root, hidden)
                .map(|e| e.len())
                .unwrap_or(max_entries)
        ));
    }
    out
}

/// Returns (name, metadata) pairs sorted: dirs first, then files, both alphabetical.
fn read_dir_sorted(path: &Path, hidden: bool) -> std::io::Result<Vec<(String, fs::Metadata)>> {
    let rd = fs::read_dir(path)?;
    let mut dirs: Vec<(String, fs::Metadata)> = Vec::new();
    let mut files: Vec<(String, fs::Metadata)> = Vec::new();

    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !hidden && name.starts_with('.') {
            continue;
        }
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.is_dir() {
            dirs.push((name, meta));
        } else {
            files.push((name, meta));
        }
    }

    dirs.sort_by_key(|a| a.0.to_lowercase());
    files.sort_by_key(|a| a.0.to_lowercase());
    dirs.extend(files);
    Ok(dirs)
}

/// Recursive tree renderer.
fn render_tree(
    path: &Path,
    prefix: &str,
    depth: usize,
    max_depth: usize,
    hidden: bool,
    max_entries: usize,
    count: &mut usize,
    out: &mut String,
) {
    if depth >= max_depth {
        return;
    }

    let entries = match read_dir_sorted(path, hidden) {
        Ok(e) => e,
        Err(err) => {
            out.push_str(&format!("{}[error: {}]\n", prefix, err));
            return;
        }
    };

    let len = entries.len();
    for (i, (name, meta)) in entries.into_iter().enumerate() {
        if *count >= max_entries {
            out.push_str(&format!("{}[... truncated]\n", prefix));
            return;
        }
        *count += 1;

        let is_last = i == len - 1;
        let connector = if is_last { "└── " } else { "├── " };
        let display = if meta.is_dir() {
            format!("{}/", name)
        } else {
            name.clone()
        };
        out.push_str(&format!("{}{}{}\n", prefix, connector, display));

        if meta.is_dir() {
            let child_prefix = if is_last {
                format!("{}    ", prefix)
            } else {
                format!("{}│   ", prefix)
            };
            render_tree(
                &path.join(&name),
                &child_prefix,
                depth + 1,
                max_depth,
                hidden,
                max_entries,
                count,
                out,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// stat
// ---------------------------------------------------------------------------

pub async fn stat(args: &Value) -> String {
    let path_str = match args["path"].as_str() {
        Some(p) => p,
        None => return "Error: missing 'path' argument".into(),
    };

    // [T7f] Workspace-boundary check (PR #75 audit H-7). Stat on
    // arbitrary system paths leaks size/mtime/permissions/line counts
    // of sensitive files (e.g. `~/.ssh/id_rsa`) even without a read.
    let path = match safe_path(path_str) {
        Ok(p) => p,
        Err(e) => return format!("Error: invalid path: {}", e),
    };
    let meta = match fs::metadata(&path) {
        Ok(m) => m,
        Err(e) => return format!("Error: cannot stat {}: {}", path_str, e),
    };

    let abs_path = path
        .canonicalize()
        .unwrap_or_else(|_| path.clone())
        .display()
        .to_string();

    let file_type = if meta.is_dir() {
        "directory"
    } else if meta.is_symlink() {
        "symlink"
    } else {
        "file"
    };

    let size_bytes = meta.len();
    let size_human = format_size_stat(size_bytes);
    let size_str = format!("{} bytes ({})", size_bytes, size_human);

    let modified = meta
        .modified()
        .map(format_datetime)
        .unwrap_or_else(|_| "unavailable".to_string());

    let created = meta
        .created()
        .map(format_datetime)
        .unwrap_or_else(|_| "unavailable".to_string());

    let perms = perms_octal(&meta);

    let mut out = format!(
        "Path:     {}\nType:     {}\nSize:     {}\nModified: {}\nCreated:  {}\nPerms:    {}",
        abs_path, file_type, size_str, modified, created, perms
    );

    // Line count for small text files
    if meta.is_file() && size_bytes < 1_000_000 {
        match fs::read(&path) {
            Ok(bytes) => {
                // Only count lines if it looks like text (no null bytes in first 512)
                let is_text = !bytes[..bytes.len().min(512)].contains(&0u8);
                if is_text {
                    let line_count = bytes.iter().filter(|&&b| b == b'\n').count()
                        + if bytes.last().map(|&b| b != b'\n').unwrap_or(false) {
                            1
                        } else {
                            0
                        };
                    out.push_str(&format!("\nLines:    {}", line_count));
                }
            }
            Err(_) => {} // ignore read errors for line count
        }
    }

    out
}

fn format_size_stat(size: u64) -> String {
    if size < 1024 {
        format!("{} B", size)
    } else if size < 1024 * 1024 {
        format!("{:.1} KB", size as f64 / 1024.0)
    } else if size < 1024 * 1024 * 1024 {
        format!("{:.1} MB", size as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", size as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}
