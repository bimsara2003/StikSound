use std::fs;
use std::net::IpAddr;
use std::path::PathBuf;
use std::time::Duration;

use stiksound_audio_engine::{probe_audio, AudioProbeInfo};
use stiksound_image_engine::{probe_image, ImageProbeInfo};
use stiksound_project_store::{atomic_publish_file, sanitize_file_name};

#[cfg(target_os = "android")]
mod mobile_share;
use stiksound_video_engine::{run_ffmpeg_mux, VideoContainerProfile, VideoMuxRequest};
use url::Url;

const MAX_MYINSTANTS_BYTES: usize = 25 * 1024 * 1024;
const MAX_MYINSTANTS_REDIRECTS: usize = 2;

fn is_disallowed_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, _, _] = v4.octets();
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_unspecified()
                || v4.is_multicast()
                || (a == 100 && (64..=127).contains(&b))
                || a >= 224
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || ((v6.segments()[0] & 0xfe00) == 0xfc00) // ULA fc00::/7
                || ((v6.segments()[0] & 0xffc0) == 0xfe80) // link-local fe80::/10
        }
    }
}

fn validate_myinstants_url(raw: &str) -> Result<Url, String> {
    let parsed = Url::parse(raw.trim()).map_err(|_| "url-rejected: malformed URL".to_string())?;
    if parsed.scheme() != "https" {
        return Err("url-rejected: only https allowed".to_string());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("url-rejected: userinfo forbidden".to_string());
    }
    if let Some(port) = parsed.port() {
        if port != 443 {
            return Err("url-rejected: non-443 port forbidden".to_string());
        }
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "url-rejected: missing host".to_string())?
        .to_ascii_lowercase();
    if host != "www.myinstants.com" && host != "myinstants.com" {
        return Err(format!("url-rejected: host not allowed ({host})"));
    }
    Ok(parsed)
}

#[tauri::command]
async fn cmd_fetch_myinstants(url: String) -> Result<Vec<u8>, String> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(20))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) StikSound/1.0")
        .build()
        .map_err(|e| format!("remote-unavailable: {e}"))?;

    let mut current = validate_myinstants_url(&url)?;

    for hop in 0..=MAX_MYINSTANTS_REDIRECTS {
        let host = current
            .host_str()
            .ok_or_else(|| "url-rejected: missing host".to_string())?;
        let addrs = tokio::net::lookup_host((host, 443))
            .await
            .map_err(|e| format!("remote-unavailable: DNS failed ({e})"))?;
        let mut saw_addr = false;
        for addr in addrs {
            saw_addr = true;
            if is_disallowed_ip(addr.ip()) {
                return Err("url-rejected: private or reserved IP address".to_string());
            }
        }
        if !saw_addr {
            return Err("remote-unavailable: DNS returned no addresses".to_string());
        }

        let resp = client
            .get(current.as_str())
            .send()
            .await
            .map_err(|e| format!("remote-unavailable: {e}"))?;

        let status = resp.status();
        if status.is_redirection() {
            if hop == MAX_MYINSTANTS_REDIRECTS {
                return Err("url-rejected: too many redirects".to_string());
            }
            let loc = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| "url-rejected: missing redirect location".to_string())?;
            let next_url = current
                .join(loc)
                .map_err(|_| "url-rejected: invalid redirect location".to_string())?;
            current = validate_myinstants_url(next_url.as_str())?;
            continue;
        }

        if !status.is_success() {
            return Err(format!("remote-unavailable: HTTP {}", status.as_u16()));
        }

        if let Some(len) = resp.content_length() {
            if len as usize > MAX_MYINSTANTS_BYTES {
                return Err("file-too-large: remote response exceeds 25 MB".to_string());
            }
        }

        let bytes = resp
            .bytes()
            .await
            .map_err(|e| format!("remote-unavailable: {e}"))?;
        if bytes.len() > MAX_MYINSTANTS_BYTES {
            return Err("file-too-large: remote body exceeds 25 MB".to_string());
        }
        return Ok(bytes.to_vec());
    }

    Err("url-rejected: redirect limit exceeded".to_string())
}

#[tauri::command]
fn cmd_probe_image(bytes: Vec<u8>) -> Result<ImageProbeInfo, String> {
    probe_image(&bytes).map_err(|e| e.to_string())
}

#[tauri::command]
fn cmd_probe_audio(bytes: Vec<u8>) -> Result<AudioProbeInfo, String> {
    probe_audio(&bytes).map_err(|e| e.to_string())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
fn cmd_render_video(
    app: tauri::AppHandle,
    sticker_bytes: Vec<u8>,
    sticker_format: String,
    animated: bool,
    audio_bytes: Vec<u8>,
    audio_format: String,
    start_ms: u32,
    end_ms: u32,
    matte_color: String,
    container: String,
) -> Result<Vec<u8>, String> {
    use tauri::Manager;
    let base_temp = app
        .path()
        .app_local_data_dir()
        .map_err(|e| e.to_string())?
        .join("work");
    let export_temp = base_temp.join("export_tmp");
    let assets_dir = base_temp.join("assets");
    fs::create_dir_all(&assets_dir).map_err(|e| e.to_string())?;
    fs::create_dir_all(&export_temp).map_err(|e| e.to_string())?;

    let sticker_path = assets_dir.join(format!("sticker.{sticker_format}"));
    let audio_path = assets_dir.join(format!("audio.{audio_format}"));
    let ext = if container == "webm-alpha" { "webm" } else { "mp4" };
    let output_path = export_temp.join(format!("out.{ext}"));

    fs::write(&sticker_path, &sticker_bytes).map_err(|e| e.to_string())?;
    fs::write(&audio_path, &audio_bytes).map_err(|e| e.to_string())?;

    let profile = if container == "webm-alpha" {
        VideoContainerProfile::VideoWebmAlpha
    } else {
        VideoContainerProfile::VideoMp4
    };

    let req = VideoMuxRequest {
        project_dir: base_temp.clone(),
        temp_dir: export_temp.clone(),
        sticker_path,
        sticker_animated_gif: animated && sticker_format == "gif",
        audio_path,
        output_path: output_path.clone(),
        start_ms,
        end_ms,
        matte_color_hex: matte_color,
        profile,
    };

    let ffmpeg_bin = PathBuf::from("ffmpeg");
    run_ffmpeg_mux(&ffmpeg_bin, &req).map_err(|e| e.to_string())?;
    let out = fs::read(&output_path).map_err(|e| e.to_string())?;
    let _ = fs::remove_dir_all(&export_temp);
    Ok(out)
}

#[tauri::command]
fn cmd_save_export_to_downloads(
    app: tauri::AppHandle,
    file_name: String,
    bytes: Vec<u8>,
) -> Result<String, String> {
    use tauri::Manager;
    let safe_name = PathBuf::from(&file_name)
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| "Invalid file name".to_string())?
        .to_string();

    let downloads_dir = app
        .path()
        .download_dir()
        .map_err(|e| e.to_string())?
        .join("StikSound");
    let dest = downloads_dir.join(&safe_name);
    atomic_publish_file(&dest, &bytes).map_err(|e| e.to_string())?;
    Ok(dest.to_string_lossy().into_owned())
}

#[tauri::command]
fn cmd_copy_artifact_to_clipboard(file_name: String, bytes: Vec<u8>) -> Result<String, String> {
    let safe_name = sanitize_file_name(&file_name)?;

    let clipboard_dir = std::env::temp_dir().join("StikSoundClipboard");
    let dest = clipboard_dir.join(&safe_name);
    atomic_publish_file(&dest, &bytes).map_err(|e| e.to_string())?;
    let dest_str = dest.to_string_lossy().into_owned();

    #[cfg(target_os = "windows")]
    {
        let escaped = dest_str.replace('\'', "''");
        let ps_script = format!("Set-Clipboard -LiteralPath '{escaped}'");
        let status = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &ps_script])
            .status()
            .map_err(|e| format!("Clipboard command failed: {e}"))?;
        if !status.success() {
            return Err("Failed to copy file to Windows clipboard".to_string());
        }
    }

    #[cfg(target_os = "macos")]
    {
        let escaped = dest_str.replace('\\', "\\\\").replace('"', "\\\"");
        let osa_script = format!("set the clipboard to (POSIX file \"{escaped}\")");
        let status = std::process::Command::new("osascript")
            .args(["-e", &osa_script])
            .status()
            .map_err(|e| format!("Clipboard command failed: {e}"))?;
        if !status.success() {
            return Err("Failed to copy file to macOS clipboard".to_string());
        }
    }

    Ok(dest_str)
}

#[cfg(target_os = "windows")]
fn ensure_isolated_webview2_data_dir() {
    if std::env::var_os("WEBVIEW2_USER_DATA_FOLDER").is_some() {
        return;
    }
    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        let base = PathBuf::from(local_app_data).join("StikSound").join("wv2");
        let _ = fs::create_dir_all(&base);
        // Best-effort cleanup of old unlocked session folders
        if let Ok(entries) = fs::read_dir(&base) {
            for entry in entries.flatten() {
                let _ = fs::remove_dir_all(entry.path());
            }
        }
        let session_dir = base.join(format!("s_{}", std::process::id()));
        let _ = fs::create_dir_all(&session_dir);
        std::env::set_var("WEBVIEW2_USER_DATA_FOLDER", &session_dir);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "windows")]
    ensure_isolated_webview2_data_dir();

    let builder = tauri::Builder::default();

    #[cfg(not(target_os = "android"))]
    let builder = builder.invoke_handler(tauri::generate_handler![
        cmd_probe_image,
        cmd_probe_audio,
        cmd_render_video,
        cmd_save_export_to_downloads,
        cmd_copy_artifact_to_clipboard,
        cmd_fetch_myinstants
    ]);

    #[cfg(target_os = "android")]
    let builder = builder.plugin(mobile_share::init()).invoke_handler(tauri::generate_handler![
        cmd_probe_image,
        cmd_probe_audio,
        cmd_fetch_myinstants,
        mobile_share::cmd_share_file,
        mobile_share::cmd_save_to_downloads
    ]);

    builder
        .run(tauri::generate_context!())
        .expect("error while running StikSound desktop application");
}
