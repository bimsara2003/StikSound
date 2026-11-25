use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Cursor, Read, Write};
use std::path::{Component, Path};

use serde_json::Value;
use thiserror::Error;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

pub const MAX_PACKAGE_FILES: usize = 32;
pub const MAX_PACKAGE_UNCOMPRESSED_BYTES: u64 = 100 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum ProjectStoreError {
    #[error("package-invalid: {0}")]
    PackageInvalid(String),
    #[error("io-error: {0}")]
    Io(#[from] std::io::Error),
}

/// Basename only, with anything outside `[A-Za-z0-9._-]` replaced by `_`, so it can't escape a target dir.
pub fn sanitize_file_name(raw: &str) -> Result<String, String> {
    let base = Path::new(raw)
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| "Invalid file name".to_string())?;
    let safe: String = base
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' })
        .collect();
    if safe.is_empty() {
        return Err("Empty sanitized file name".to_string());
    }
    Ok(safe)
}

pub fn validate_zip_entry_path(entry: &str) -> Result<(), ProjectStoreError> {
    if entry.is_empty() || entry.contains('\\') || entry.starts_with('/') {
        return Err(ProjectStoreError::PackageInvalid(format!(
            "Illegal zip entry path: {entry}"
        )));
    }
    if entry.len() >= 2 && entry.as_bytes()[1] == b':' {
        return Err(ProjectStoreError::PackageInvalid(format!(
            "Drive prefix forbidden in zip entry: {entry}"
        )));
    }
    let path = Path::new(entry);
    for comp in path.components() {
        if matches!(
            comp,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        ) {
            return Err(ProjectStoreError::PackageInvalid(format!(
                "Path traversal in zip entry: {entry}"
            )));
        }
    }
    if entry == "manifest.json" {
        return Ok(());
    }
    let parts: Vec<&str> = entry.split('/').collect();
    if parts.len() == 2
        && parts[0] == "assets"
        && !parts[1].is_empty()
        && parts[1]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
    {
        return Ok(());
    }
    Err(ProjectStoreError::PackageInvalid(format!(
        "Disallowed zip entry: {entry}"
    )))
}

pub fn write_zip_bytes(entries: &[(&str, &[u8])]) -> Result<Vec<u8>, ProjectStoreError> {
    if entries.len() > MAX_PACKAGE_FILES {
        return Err(ProjectStoreError::PackageInvalid(
            "Too many zip entries".to_string(),
        ));
    }
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut writer = ZipWriter::new(&mut cursor);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        for &(path, bytes) in entries {
            validate_zip_entry_path(path)?;
            writer
                .start_file(path, options)
                .map_err(|e| ProjectStoreError::PackageInvalid(format!("Zip start_file: {e}")))?;
            writer.write_all(bytes)?;
        }
        writer
            .finish()
            .map_err(|e| ProjectStoreError::PackageInvalid(format!("Zip finish: {e}")))?;
    }
    Ok(cursor.into_inner())
}

pub fn read_zip_package(zip_bytes: &[u8]) -> Result<HashMap<String, Vec<u8>>, ProjectStoreError> {
    let mut archive = ZipArchive::new(Cursor::new(zip_bytes))
        .map_err(|e| ProjectStoreError::PackageInvalid(format!("Invalid zip archive: {e}")))?;

    if archive.len() > MAX_PACKAGE_FILES {
        return Err(ProjectStoreError::PackageInvalid(
            "Package exceeds 32 entry limit".to_string(),
        ));
    }

    let mut total_uncompressed: u64 = 0;
    let mut out = HashMap::new();

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| ProjectStoreError::PackageInvalid(format!("Zip read entry: {e}")))?;
        let name = file.name().to_string();
        validate_zip_entry_path(&name)?;

        let mut buf = Vec::new();
        let mut limited = (&mut file).take(MAX_PACKAGE_UNCOMPRESSED_BYTES + 1);
        limited.read_to_end(&mut buf)?;
        total_uncompressed = total_uncompressed.saturating_add(buf.len() as u64);
        if total_uncompressed > MAX_PACKAGE_UNCOMPRESSED_BYTES {
            return Err(ProjectStoreError::PackageInvalid(
                "Package exceeds 100 MB uncompressed limit".to_string(),
            ));
        }
        out.insert(name, buf);
    }

    let manifest_bytes = out
        .get("manifest.json")
        .ok_or_else(|| ProjectStoreError::PackageInvalid("Missing manifest.json".to_string()))?;
    let manifest: Value = serde_json::from_slice(manifest_bytes)
        .map_err(|_| ProjectStoreError::PackageInvalid("Invalid manifest JSON".to_string()))?;
    if manifest.get("schema_version").and_then(Value::as_str) != Some("1.0") {
        return Err(ProjectStoreError::PackageInvalid(
            "Unsupported or missing schema_version".to_string(),
        ));
    }

    Ok(out)
}

/// Atomically publishes `bytes` to `destination` via a sibling `.tmp` file so a crash
/// or failure leaves any previous file intact.
pub fn atomic_publish_file(destination: &Path, bytes: &[u8]) -> Result<(), ProjectStoreError> {
    let parent = destination
        .parent()
        .ok_or_else(|| ProjectStoreError::PackageInvalid("Destination has no parent".to_string()))?;
    fs::create_dir_all(parent)?;
    let file_name = destination
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| ProjectStoreError::PackageInvalid("Invalid destination file name".to_string()))?;
    let tmp_path = parent.join(format!("{file_name}.tmp"));

    {
        let mut f = File::create(&tmp_path)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }

    if destination.exists() {
        fs::remove_file(destination)?;
    }
    fs::rename(&tmp_path, destination)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sec006_sec007_rejects_zip_slip_entries() {
        assert!(validate_zip_entry_path("../outside.txt").is_err());
        assert!(validate_zip_entry_path("assets/../../secret").is_err());
        assert!(validate_zip_entry_path("/etc/passwd").is_err());
        assert!(validate_zip_entry_path("C:\\evil.txt").is_err());

        // Craft raw zip containing "../outside.txt"
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut w = ZipWriter::new(&mut cursor);
            w.start_file("../outside.txt", SimpleFileOptions::default())
                .unwrap();
            w.write_all(b"pwn").unwrap();
            w.finish().unwrap();
        }
        let err = read_zip_package(&cursor.into_inner()).unwrap_err();
        assert!(matches!(err, ProjectStoreError::PackageInvalid(_)));
    }

    #[test]
    fn sanitize_file_name_cases() {
        assert_eq!(sanitize_file_name("sticker.mp4").unwrap(), "sticker.mp4");
        assert_eq!(sanitize_file_name("../../evil.mp4").unwrap(), "evil.mp4");
        assert_eq!(sanitize_file_name("mi sticker ñ.mp4").unwrap(), "mi_sticker__.mp4");
        assert!(sanitize_file_name("").is_err());
        assert!(sanitize_file_name("..").is_err());
    }

    #[test]
    fn round_trips_package_and_atomic_publish() {
        let manifest = br#"{"schema_version":"1.0","project":{"id":"prj_1234"}}"#;
        let sticker = b"pngbytes";
        let zip = write_zip_bytes(&[("manifest.json", manifest), ("assets/stk_1.png", sticker)]).unwrap();
        let read_back = read_zip_package(&zip).unwrap();
        assert_eq!(read_back.get("assets/stk_1.png").unwrap(), sticker);

        let dir = std::env::temp_dir().join("stiksound_store_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("project.stiksound.zip");
        atomic_publish_file(&dest, &zip).unwrap();
        assert!(dest.exists());
        assert!(!dir.join("project.stiksound.zip.tmp").exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
