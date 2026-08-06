// MIUI (小米系統) compatibility guide — native detection + dismiss persistence.
// SPEC-33 §6(E) / SPEC-34 §9 + G6.
//
// MIUI / Redmi kill background apps aggressively, so spectyn's foreground node
// service is reaped overnight unless the user whitelists auto-start + battery
// optimization. We can only GUIDE (no public MIUI API), but we CAN detect MIUI
// (the `ro.miui.ui.version.code` system property exists only on MIUI) and
// remember whether the user dismissed the guide.
//
// This module implements the SPEC-34 §9 command matrix:
//   miui_guide_check_should_show {} -> { should_show, is_miui, last_dismissed_ms }
//   miui_guide_dismiss { dont_show_again } -> { ok }
//   miui_guide_open_autostart {} -> { ok }            (MIUI 安全中心 自啟動)
//   miui_guide_open_battery_optimization {} -> { ok } (system battery-opt prompt)
//
// Stage 2 (the two open_* commands) launches the relevant Settings Activity via
// a small JNI bridge (no Kotlin needed): ndk_context gives us the JavaVM + the
// app Activity, and we build + startActivity an Intent from Rust. A failed
// launch (e.g. ActivityNotFoundException for the MIUI-only autostart component
// on a non-MIUI device) returns ok:false so the React dialog falls back to its
// always-visible manual steps. The "open the guide automatically when the
// foreground service is actually reaped" trigger remains deferred.

use serde::Serialize;
use std::path::PathBuf;
use std::sync::OnceLock;

/// Where the dismiss flag is persisted. Mirrors local_keys.rs's choice of
/// `~/.spectyn-mesh/` so all per-user spectyn state sits in one dir (on Android
/// this resolves under the app sandbox home).
fn flag_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".spectyn-mesh")
        .join("miui_guide.json")
}

#[derive(Serialize)]
pub struct MiuiGuideStatus {
    /// Whether the app should surface the guide: MIUI detected AND the user
    /// hasn't ticked "don't show again". (The richer SPEC trigger — only after
    /// a foreground-service start actually fails — is deferred to the Kotlin
    /// service stage; first-launch detection is the v1 behaviour.)
    pub should_show: bool,
    /// True when this device runs MIUI (Xiaomi / Redmi / POCO).
    pub is_miui: bool,
    /// Epoch-ms of the last dismissal, if any.
    pub last_dismissed_ms: Option<i64>,
}

/// MIUI exposes `ro.miui.ui.version.code` (e.g. "14"); it's absent on stock
/// Android and on desktop. Read it via `getprop` — the standard, app-callable
/// way to read system properties on Android (every app's SELinux domain may
/// exec `/system/bin/getprop`; device-info libraries rely on this, MIUI
/// included). Absolute path guards against an empty PATH. Cached because a
/// device's MIUI-ness can't change within a process — avoids repeat fork/exec.
fn detect_miui() -> bool {
    static CACHED: OnceLock<bool> = OnceLock::new();
    *CACHED.get_or_init(|| {
        let Ok(out) = std::process::Command::new("/system/bin/getprop")
            .arg("ro.miui.ui.version.code")
            .output()
        else {
            return false;
        };
        if !out.status.success() {
            return false;
        }
        !String::from_utf8_lossy(&out.stdout).trim().is_empty()
    })
}

#[derive(serde::Deserialize, Default)]
struct DismissFlag {
    #[serde(default)]
    dont_show_again: bool,
    #[serde(default)]
    last_dismissed_ms: Option<i64>,
}

fn read_flag() -> DismissFlag {
    std::fs::read_to_string(flag_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

#[tauri::command]
pub fn miui_guide_check_should_show() -> MiuiGuideStatus {
    let is_miui = detect_miui();
    let flag = read_flag();
    MiuiGuideStatus {
        should_show: is_miui && !flag.dont_show_again,
        is_miui,
        last_dismissed_ms: flag.last_dismissed_ms,
    }
}

#[derive(Serialize)]
pub struct MiuiDismissResult {
    pub ok: bool,
}

#[tauri::command]
pub fn miui_guide_dismiss(dont_show_again: bool) -> Result<MiuiDismissResult, String> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let path = flag_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {parent:?}: {e}"))?;
    }
    let body = serde_json::json!({
        "dont_show_again": dont_show_again,
        "last_dismissed_ms": now_ms,
    });
    std::fs::write(&path, body.to_string()).map_err(|e| format!("write {path:?}: {e}"))?;
    Ok(MiuiDismissResult { ok: true })
}

#[derive(Serialize)]
pub struct MiuiOpenResult {
    /// True if startActivity didn't throw. False → the dialog leans on its
    /// always-visible manual steps (no native jump available on this device).
    pub ok: bool,
}

/// Open MIUI's auto-start management screen (安全中心 → 應用管理 → 自啟動).
/// MIUI-only component — on stock Android / desktop the launch fails and we
/// return ok:false so the React dialog shows its manual fallback.
#[tauri::command]
pub fn miui_guide_open_autostart() -> MiuiOpenResult {
    MiuiOpenResult {
        ok: open_autostart_impl(),
    }
}

/// Open the system "ignore battery optimizations" request for this app. Unlike
/// autostart this is a STANDARD Android action, so it also works off MIUI.
#[tauri::command]
pub fn miui_guide_open_battery_optimization() -> MiuiOpenResult {
    MiuiOpenResult {
        ok: open_battery_impl(),
    }
}

#[cfg(target_os = "android")]
fn open_autostart_impl() -> bool {
    android_intent::start_activity_component(
        "com.miui.securitycenter",
        "com.miui.permcenter.autostart.AutoStartManagementActivity",
    )
}

#[cfg(target_os = "android")]
fn open_battery_impl() -> bool {
    android_intent::start_activity_action_with_pkg_data(
        "android.settings.REQUEST_IGNORE_BATTERY_OPTIMIZATIONS",
    )
}

// Off Android there is no Activity to start — the commands resolve to ok:false
// (the dialog's manual steps are the fallback) and the JNI module is absent.
#[cfg(not(target_os = "android"))]
fn open_autostart_impl() -> bool {
    false
}
#[cfg(not(target_os = "android"))]
fn open_battery_impl() -> bool {
    false
}

#[cfg(target_os = "android")]
mod android_intent {
    use jni::objects::{JObject, JString, JValue};
    use jni::JavaVM;

    /// Run a closure with an attached JNIEnv + the app Activity. Returns false on
    /// any JNI failure OR a pending Java exception (e.g. ActivityNotFoundException
    /// when the MIUI-only component is missing on stock Android). Never panics
    /// across the FFI boundary — a failed launch just means "use the manual
    /// steps the dialog already shows".
    fn with_activity<F>(f: F) -> bool
    where
        F: FnOnce(&mut jni::JNIEnv, &JObject) -> jni::errors::Result<()>,
    {
        let ctx = ndk_context::android_context();
        let vm_ptr = ctx.vm();
        let act_ptr = ctx.context();
        if vm_ptr.is_null() || act_ptr.is_null() {
            return false;
        }
        let vm = match unsafe { JavaVM::from_raw(vm_ptr.cast()) } {
            Ok(v) => v,
            Err(_) => return false,
        };
        let mut env = match vm.attach_current_thread() {
            Ok(e) => e,
            Err(_) => return false,
        };
        let activity = unsafe { JObject::from_raw(act_ptr.cast()) };
        // Run inside a bounded local frame so the Intent / String / ComponentName
        // locals the closure creates are reclaimed on return — regardless of
        // whether attach_current_thread freshly attached this thread (refs freed
        // on the detach-on-drop) or found it ALREADY attached (no detach → the
        // locals would otherwise linger in a long-lived frame). Caps the JNI
        // local-reference table either way. (`activity` wraps ndk_context's
        // global ref, which stays valid across the frame.)
        let ran = env
            .with_local_frame(16, |env| f(env, &activity))
            .is_ok();
        // A thrown Java exception survives the frame pop; clear it so the next
        // JNI use isn't poisoned, and report failure.
        if env.exception_check().unwrap_or(false) {
            let _ = env.exception_clear();
            return false;
        }
        ran
    }

    /// startActivity(new Intent().setComponent(ComponentName(pkg, cls)).addFlags(NEW_TASK))
    pub fn start_activity_component(pkg: &str, cls: &str) -> bool {
        with_activity(|env, activity| {
            let intent = env.new_object("android/content/Intent", "()V", &[])?;
            let pkg_s = env.new_string(pkg)?;
            let cls_s = env.new_string(cls)?;
            let component = env.new_object(
                "android/content/ComponentName",
                "(Ljava/lang/String;Ljava/lang/String;)V",
                &[(&pkg_s).into(), (&cls_s).into()],
            )?;
            env.call_method(
                &intent,
                "setComponent",
                "(Landroid/content/ComponentName;)Landroid/content/Intent;",
                &[(&component).into()],
            )?;
            // FLAG_ACTIVITY_NEW_TASK (0x1000_0000) — needed to start an Activity
            // from the non-Activity Tauri worker thread context.
            env.call_method(
                &intent,
                "addFlags",
                "(I)Landroid/content/Intent;",
                &[JValue::Int(0x1000_0000)],
            )?;
            env.call_method(
                activity,
                "startActivity",
                "(Landroid/content/Intent;)V",
                &[(&intent).into()],
            )?;
            Ok(())
        })
    }

    /// startActivity(new Intent(action).setData(Uri.parse("package:<self>")).addFlags(NEW_TASK))
    pub fn start_activity_action_with_pkg_data(action: &str) -> bool {
        with_activity(|env, activity| {
            let action_s = env.new_string(action)?;
            let intent = env.new_object(
                "android/content/Intent",
                "(Ljava/lang/String;)V",
                &[(&action_s).into()],
            )?;
            let pkg_obj = env
                .call_method(activity, "getPackageName", "()Ljava/lang/String;", &[])?
                .l()?;
            let pkg_jstr = JString::from(pkg_obj);
            let pkg: String = env.get_string(&pkg_jstr)?.into();
            let uri_s = env.new_string(format!("package:{pkg}"))?;
            let uri = env
                .call_static_method(
                    "android/net/Uri",
                    "parse",
                    "(Ljava/lang/String;)Landroid/net/Uri;",
                    &[(&uri_s).into()],
                )?
                .l()?;
            env.call_method(
                &intent,
                "setData",
                "(Landroid/net/Uri;)Landroid/content/Intent;",
                &[(&uri).into()],
            )?;
            env.call_method(
                &intent,
                "addFlags",
                "(I)Landroid/content/Intent;",
                &[JValue::Int(0x1000_0000)],
            )?;
            env.call_method(
                activity,
                "startActivity",
                "(Landroid/content/Intent;)V",
                &[(&intent).into()],
            )?;
            Ok(())
        })
    }
}
