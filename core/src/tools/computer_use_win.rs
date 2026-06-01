//! Windows computer-use MCP tools: screen capture + mouse + keyboard.
//!
//! Closes part of the doc 28 §5 v0.6.0 gap: phantom can read text, run
//! shells, fetch the web — but had no way to drive a GUI. This module
//! exposes three Win32-native tools that let an agent loop perceive
//! and act on the desktop:
//!
//! 1. `screen_capture` — BitBlt the primary monitor to a PNG file.
//!    Returns the path so the agent can `@`-attach it on the next
//!    turn (multimodal sentinel → vision-capable LLM).
//! 2. `mouse_click` — `SetCursorPos` + `mouse_event` to click at
//!    absolute screen coordinates.
//! 3. `keystroke` — `SendInput INPUT_KEYBOARD` to type Unicode text
//!    OR send a single named key (Enter / Tab / Escape / F1-F12 /
//!    Up / Down / Left / Right / Backspace / Delete) with optional
//!    modifiers (Ctrl/Shift/Alt/Win).
//!
//! ## Safety
//!
//! These tools manipulate the **real** logged-in desktop. There is
//! no sandbox between phantom and Excel. Treat them like `shell` —
//! gated by the agent's permission model; never invoke from an
//! untrusted prompt. The module is `#[cfg(target_os = "windows")]`
//! so non-Win builds get an empty stub.
//!
//! ## Coordinates
//!
//! All coordinates are **physical pixels on the primary monitor**,
//! origin top-left. Multi-monitor configurations capture only the
//! primary screen for now (extended virtual desktop is a v0.7.0
//! follow-up; needs `EnumDisplayMonitors` + virtual-screen
//! rectangle math).

#![cfg(target_os = "windows")]

use serde_json::Value;
use std::path::PathBuf;

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
    ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HDC, SRCCOPY,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
    MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
    MOUSEINPUT, MOUSE_EVENT_FLAGS, VIRTUAL_KEY, VK_BACK, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END,
    VK_ESCAPE, VK_F1, VK_F10, VK_F11, VK_F12, VK_F2, VK_F3, VK_F4, VK_F5, VK_F6, VK_F7, VK_F8,
    VK_F9, VK_HOME, VK_LEFT, VK_LWIN, VK_MENU, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT,
    VK_SPACE, VK_TAB, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SetCursorPos, SM_CXSCREEN, SM_CYSCREEN,
};

/// Where screenshots land.
fn capture_dir() -> std::io::Result<PathBuf> {
    let home = dirs::home_dir()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "home dir not found"))?;
    let dir = home.join(".phantom-mesh").join("captures");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

// ─── screen_capture ──────────────────────────────────────────────────────────

/// MCP tool entry-point. Captures the primary monitor and writes a PNG.
///
/// Optional args:
/// - `path` (string) — override output path. Default is
///   `~/.phantom-mesh/captures/<unix-ts>.png`.
///
/// Returns the absolute path to the saved PNG.
pub async fn screen_capture(args: &Value) -> String {
    let override_path = args.get("path").and_then(|v| v.as_str()).map(PathBuf::from);
    match tokio::task::spawn_blocking(move || capture_primary_to_png(override_path)).await {
        Ok(Ok(p)) => format!("Captured screen to:\n{}", p.display()),
        Ok(Err(e)) => format!("Error: {e}"),
        Err(e) => format!("Error: capture task join: {e}"),
    }
}

fn capture_primary_to_png(override_path: Option<PathBuf>) -> Result<PathBuf, String> {
    let dest = match override_path {
        Some(p) => p,
        None => {
            let dir = capture_dir().map_err(|e| format!("preparing capture dir: {e}"))?;
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            dir.join(format!("{ts}.png"))
        }
    };

    let (width, height, bgra) = grab_primary_bgra()?;
    write_png_from_bgra(&dest, width, height, &bgra)?;
    Ok(dest)
}

/// Grab the primary monitor as a BGRA byte buffer (one byte per channel,
/// row-major, top-down).
fn grab_primary_bgra() -> Result<(u32, u32, Vec<u8>), String> {
    unsafe {
        let width = GetSystemMetrics(SM_CXSCREEN);
        let height = GetSystemMetrics(SM_CYSCREEN);
        if width <= 0 || height <= 0 {
            return Err(format!(
                "GetSystemMetrics returned non-positive dims: {width}x{height}"
            ));
        }
        let w = width as u32;
        let h = height as u32;

        let screen_dc: HDC = GetDC(HWND(std::ptr::null_mut()));
        if screen_dc.is_invalid() {
            return Err("GetDC(NULL) returned invalid HDC".into());
        }
        let mem_dc = CreateCompatibleDC(screen_dc);
        if mem_dc.is_invalid() {
            ReleaseDC(HWND(std::ptr::null_mut()), screen_dc);
            return Err("CreateCompatibleDC returned invalid HDC".into());
        }
        let bitmap = CreateCompatibleBitmap(screen_dc, width, height);
        if bitmap.is_invalid() {
            let _ = DeleteDC(mem_dc);
            ReleaseDC(HWND(std::ptr::null_mut()), screen_dc);
            return Err("CreateCompatibleBitmap returned invalid HBITMAP".into());
        }
        let old = SelectObject(mem_dc, bitmap);

        if BitBlt(mem_dc, 0, 0, width, height, screen_dc, 0, 0, SRCCOPY).is_err() {
            let _ = SelectObject(mem_dc, old);
            let _ = DeleteObject(bitmap);
            let _ = DeleteDC(mem_dc);
            ReleaseDC(HWND(std::ptr::null_mut()), screen_dc);
            return Err("BitBlt failed".into());
        }

        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height, // negative → top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            ..Default::default()
        };
        let mut pixels = vec![0u8; (w as usize) * (h as usize) * 4];
        let rows = GetDIBits(
            mem_dc,
            bitmap,
            0,
            h,
            Some(pixels.as_mut_ptr() as *mut _),
            &mut info,
            DIB_RGB_COLORS,
        );

        let _ = SelectObject(mem_dc, old);
        let _ = DeleteObject(bitmap);
        let _ = DeleteDC(mem_dc);
        ReleaseDC(HWND(std::ptr::null_mut()), screen_dc);

        if rows == 0 {
            return Err("GetDIBits copied 0 rows".into());
        }
        Ok((w, h, pixels))
    }
}

/// Encode a top-down BGRA buffer to PNG (RGBA8) and write to disk.
fn write_png_from_bgra(dest: &PathBuf, w: u32, h: u32, bgra: &[u8]) -> Result<(), String> {
    // BGRA → RGBA swap in place on a copy. (Cheap; capture cost dominates.)
    let mut rgba = bgra.to_vec();
    for px in rgba.chunks_exact_mut(4) {
        px.swap(0, 2);
    }

    let file =
        std::fs::File::create(dest).map_err(|e| format!("creating {}: {e}", dest.display()))?;
    let w_writer = std::io::BufWriter::new(file);
    let mut encoder = png::Encoder::new(w_writer, w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|e| format!("PNG header: {e}"))?;
    writer
        .write_image_data(&rgba)
        .map_err(|e| format!("PNG body: {e}"))?;
    Ok(())
}

// ─── mouse_click ─────────────────────────────────────────────────────────────

/// MCP tool entry-point. Move cursor to absolute (x, y) and click.
///
/// Required args:
/// - `x` (integer, physical pixels from left of primary monitor)
/// - `y` (integer, physical pixels from top of primary monitor)
///
/// Optional args:
/// - `button` (string: `"left"` | `"right"` | `"middle"`; default `"left"`)
/// - `double` (boolean; default `false` — emits two click events 50ms apart)
pub async fn mouse_click(args: &Value) -> String {
    let x = match args.get("x").and_then(|v| v.as_i64()) {
        Some(v) => v as i32,
        None => return "Error: missing required integer argument 'x'".into(),
    };
    let y = match args.get("y").and_then(|v| v.as_i64()) {
        Some(v) => v as i32,
        None => return "Error: missing required integer argument 'y'".into(),
    };
    let button = args
        .get("button")
        .and_then(|v| v.as_str())
        .unwrap_or("left")
        .to_lowercase();
    let double = args
        .get("double")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let (down, up) = match button.as_str() {
        "left" => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
        "right" => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
        "middle" => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
        other => return format!("Error: unknown button '{other}' (expect left/right/middle)"),
    };

    let res = tokio::task::spawn_blocking(move || -> Result<(), String> {
        unsafe {
            if SetCursorPos(x, y).is_err() {
                return Err(format!("SetCursorPos({x},{y}) failed"));
            }
        }
        send_mouse(down, up)?;
        if double {
            std::thread::sleep(std::time::Duration::from_millis(50));
            send_mouse(down, up)?;
        }
        Ok(())
    })
    .await;
    match res {
        Ok(Ok(())) => format!(
            "Clicked {button} at ({x},{y}){}",
            if double { " (double)" } else { "" }
        ),
        Ok(Err(e)) => format!("Error: {e}"),
        Err(e) => format!("Error: click task join: {e}"),
    }
}

fn send_mouse(down: MOUSE_EVENT_FLAGS, up: MOUSE_EVENT_FLAGS) -> Result<(), String> {
    let inputs = [
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: 0,
                    dy: 0,
                    mouseData: 0,
                    dwFlags: down,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        },
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: 0,
                    dy: 0,
                    mouseData: 0,
                    dwFlags: up,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        },
    ];
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent as usize != inputs.len() {
        Err(format!("SendInput accepted {sent}/{} events", inputs.len()))
    } else {
        Ok(())
    }
}

// ─── keystroke ───────────────────────────────────────────────────────────────

/// MCP tool entry-point. Type Unicode text OR send a single named key
/// with optional modifiers.
///
/// One of these is required:
/// - `text` (string) — every char is sent as a Unicode keystroke
///   (works for arbitrary text including CJK / emoji).
/// - `key` (string) — single named key, see [`name_to_vk`].
///
/// Optional args (only meaningful with `key`):
/// - `modifiers` (array of strings: any of `"ctrl"`, `"shift"`, `"alt"`,
///   `"win"`). Modifiers are pressed before the key and released after.
pub async fn keystroke(args: &Value) -> String {
    let text = args
        .get("text")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let key = args.get("key").and_then(|v| v.as_str()).map(str::to_string);
    if text.is_none() && key.is_none() {
        return "Error: provide either 'text' (string) or 'key' (named key)".into();
    }
    if text.is_some() && key.is_some() {
        return "Error: provide 'text' OR 'key', not both".into();
    }

    let mods: Vec<String> = args
        .get("modifiers")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_lowercase()))
                .collect()
        })
        .unwrap_or_default();

    let res = tokio::task::spawn_blocking(move || -> Result<String, String> {
        if let Some(t) = text {
            send_unicode_text(&t)?;
            Ok(format!("Typed {} chars", t.chars().count()))
        } else {
            let key = key.unwrap();
            let vk = name_to_vk(&key).ok_or_else(|| format!("unknown key name '{key}'"))?;
            let mod_vks: Vec<VIRTUAL_KEY> = mods
                .iter()
                .map(|m| modifier_to_vk(m))
                .collect::<Result<_, _>>()?;
            send_key_with_modifiers(vk, &mod_vks)?;
            Ok(format!(
                "Sent {} + {key}",
                if mod_vks.is_empty() {
                    "(no mods)".into()
                } else {
                    mods.join("+")
                }
            ))
        }
    })
    .await;
    match res {
        Ok(Ok(msg)) => msg,
        Ok(Err(e)) => format!("Error: {e}"),
        Err(e) => format!("Error: keystroke task join: {e}"),
    }
}

/// Send `text` as a sequence of Unicode-scancode INPUT_KEYBOARD events.
fn send_unicode_text(text: &str) -> Result<(), String> {
    for code_unit in text.encode_utf16() {
        let down = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0),
                    wScan: code_unit,
                    dwFlags: KEYEVENTF_UNICODE,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let up = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0),
                    wScan: code_unit,
                    dwFlags: KEYEVENTF_UNICODE | KEYEVENTF_KEYUP,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let inputs = [down, up];
        let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
        if sent as usize != inputs.len() {
            return Err(format!(
                "SendInput accepted {sent}/2 events for U+{:04X}",
                code_unit
            ));
        }
    }
    Ok(())
}

fn send_key_with_modifiers(vk: VIRTUAL_KEY, mods: &[VIRTUAL_KEY]) -> Result<(), String> {
    let mut inputs: Vec<INPUT> = Vec::with_capacity((mods.len() + 1) * 2);
    // Press modifiers in order.
    for m in mods {
        inputs.push(key_input(*m, KEYBD_EVENT_FLAGS(0)));
    }
    // Press + release the main key.
    inputs.push(key_input(vk, KEYBD_EVENT_FLAGS(0)));
    inputs.push(key_input(vk, KEYEVENTF_KEYUP));
    // Release modifiers in reverse order.
    for m in mods.iter().rev() {
        inputs.push(key_input(*m, KEYEVENTF_KEYUP));
    }

    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent as usize != inputs.len() {
        Err(format!("SendInput accepted {sent}/{} events", inputs.len()))
    } else {
        Ok(())
    }
}

fn key_input(vk: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// Map a friendly key name (case-insensitive) to a Win32 virtual key.
/// Returns `None` for unrecognised names — the caller surfaces this
/// as a user-facing "unknown key" error so the agent can correct.
pub fn name_to_vk(name: &str) -> Option<VIRTUAL_KEY> {
    Some(match name.to_lowercase().as_str() {
        "enter" | "return" => VK_RETURN,
        "tab" => VK_TAB,
        "escape" | "esc" => VK_ESCAPE,
        "space" => VK_SPACE,
        "backspace" | "bs" => VK_BACK,
        "delete" | "del" => VK_DELETE,
        "up" => VK_UP,
        "down" => VK_DOWN,
        "left" => VK_LEFT,
        "right" => VK_RIGHT,
        "home" => VK_HOME,
        "end" => VK_END,
        "pageup" | "pgup" => VK_PRIOR,
        "pagedown" | "pgdn" => VK_NEXT,
        "f1" => VK_F1,
        "f2" => VK_F2,
        "f3" => VK_F3,
        "f4" => VK_F4,
        "f5" => VK_F5,
        "f6" => VK_F6,
        "f7" => VK_F7,
        "f8" => VK_F8,
        "f9" => VK_F9,
        "f10" => VK_F10,
        "f11" => VK_F11,
        "f12" => VK_F12,
        _ => return None,
    })
}

fn modifier_to_vk(name: &str) -> Result<VIRTUAL_KEY, String> {
    Ok(match name {
        "ctrl" | "control" => VK_CONTROL,
        "shift" => VK_SHIFT,
        "alt" => VK_MENU,
        "win" | "super" => VK_LWIN,
        other => return Err(format!("unknown modifier '{other}'")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // These tests are pure-logic only (no real screen / mouse / keyboard
    // events). The real Win32 round-trip tests would need an interactive
    // session and would steal focus from the dev — they live in
    // scripts/test-windows.ps1 as a manual phase.

    #[test]
    fn name_to_vk_known_keys() {
        assert!(name_to_vk("enter").is_some());
        assert!(name_to_vk("Return").is_some());
        assert!(name_to_vk("F1").is_some());
        assert!(name_to_vk("F12").is_some());
        assert!(name_to_vk("PgUp").is_some());
        assert!(name_to_vk("escape").is_some());
    }

    #[test]
    fn name_to_vk_unknown_returns_none() {
        assert!(name_to_vk("xyzzy").is_none());
        assert!(name_to_vk("ctrl").is_none(), "modifiers are NOT keys");
        assert!(name_to_vk("").is_none());
    }

    #[test]
    fn modifier_to_vk_aliases() {
        assert!(modifier_to_vk("ctrl").is_ok());
        assert!(modifier_to_vk("control").is_ok());
        assert!(modifier_to_vk("shift").is_ok());
        assert!(modifier_to_vk("alt").is_ok());
        assert!(modifier_to_vk("win").is_ok());
        assert!(modifier_to_vk("super").is_ok());
        assert!(modifier_to_vk("xyz").is_err());
    }

    #[tokio::test]
    async fn mouse_click_missing_coords() {
        let out = mouse_click(&json!({})).await;
        assert!(
            out.starts_with("Error: missing required integer argument 'x'"),
            "got: {out}"
        );
        let out = mouse_click(&json!({"x": 0})).await;
        assert!(
            out.starts_with("Error: missing required integer argument 'y'"),
            "got: {out}"
        );
    }

    #[tokio::test]
    async fn mouse_click_unknown_button() {
        let out = mouse_click(&json!({"x": 0, "y": 0, "button": "fingerprint"})).await;
        assert!(out.starts_with("Error: unknown button"), "got: {out}");
    }

    #[tokio::test]
    async fn keystroke_neither_text_nor_key() {
        let out = keystroke(&json!({})).await;
        assert!(
            out.starts_with("Error: provide either 'text'"),
            "got: {out}"
        );
    }

    #[tokio::test]
    async fn keystroke_both_text_and_key() {
        let out = keystroke(&json!({"text": "hi", "key": "enter"})).await;
        assert!(
            out.starts_with("Error: provide 'text' OR 'key'"),
            "got: {out}"
        );
    }

    #[tokio::test]
    async fn keystroke_unknown_named_key() {
        let out = keystroke(&json!({"key": "fingerprint"})).await;
        assert!(out.starts_with("Error: unknown key name"), "got: {out}");
    }
}
