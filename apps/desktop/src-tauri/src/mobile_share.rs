//! Android share sheet + save to Downloads, backed by StikSharePlugin.kt in gen/android.
use std::fs;

use serde::Deserialize;
use serde_json::json;
use stiksound_project_store::sanitize_file_name;
use tauri::plugin::{Builder, PluginHandle, TauriPlugin};
use tauri::{AppHandle, Manager, Runtime, Wry};

pub struct StikShare<R: Runtime>(PluginHandle<R>);

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("stikshare")
        .setup(|app, api| {
            let handle = api.register_android_plugin("app.stiksound.android", "StikSharePlugin")?;
            app.manage(StikShare(handle));
            Ok(())
        })
        .build()
}

/// Writes bytes to cache/share/<sanitized name> (the only dir FileProvider exposes).
fn write_to_share_cache(app: &AppHandle, file_name: &str, bytes: &[u8]) -> Result<String, String> {
    let dir = app.path().app_cache_dir().map_err(|e| e.to_string())?.join("share");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(sanitize_file_name(file_name)?);
    fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
pub fn cmd_share_file(app: AppHandle, file_name: String, mime_type: String, bytes: Vec<u8>) -> Result<(), String> {
    let path = write_to_share_cache(&app, &file_name, &bytes)?;
    app.state::<StikShare<Wry>>()
        .0
        .run_mobile_plugin::<serde_json::Value>("share", json!({ "path": path, "mimeType": mime_type }))
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[derive(Deserialize)]
struct Saved {
    name: String,
}

#[tauri::command]
pub fn cmd_save_to_downloads(app: AppHandle, file_name: String, mime_type: String, bytes: Vec<u8>) -> Result<String, String> {
    let path = write_to_share_cache(&app, &file_name, &bytes)?;
    let safe_name = sanitize_file_name(&file_name)?;
    app.state::<StikShare<Wry>>()
        .0
        .run_mobile_plugin::<Saved>(
            "save",
            json!({ "path": path, "fileName": safe_name, "mimeType": mime_type }),
        )
        .map(|s| s.name)
        .map_err(|e| e.to_string())
}
