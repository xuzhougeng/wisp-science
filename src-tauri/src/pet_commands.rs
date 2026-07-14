use super::AppState;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::Path};
use tauri::State;

const MAX_SPRITESHEET_BYTES: u64 = 16 * 1024 * 1024;
const SPRITE_WIDTH: u32 = 1536;
const SPRITE_HEIGHT: u32 = 2288;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PetManifest {
    id: String,
    display_name: String,
    #[serde(default)]
    description: String,
    sprite_version_number: u8,
    spritesheet_path: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PetAsset {
    id: String,
    display_name: String,
    description: String,
    sprite_version_number: u8,
    spritesheet_data_url: String,
    frame_counts: BTreeMap<String, u8>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PetStatus {
    enabled: bool,
    directory: String,
    asset: Option<PetAsset>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct ValidationReport {
    ok: bool,
    columns: u32,
    rows: u32,
    width: u32,
    height: u32,
    #[serde(default)]
    cells: Vec<ValidationCell>,
}

#[derive(Deserialize)]
struct ValidationCell {
    state: String,
    column: u8,
    used: bool,
}

fn default_frame_counts() -> BTreeMap<String, u8> {
    [
        ("idle", 7),
        ("running-right", 8),
        ("running-left", 8),
        ("waving", 4),
        ("jumping", 5),
        ("failed", 8),
        ("waiting", 6),
        ("running", 6),
        ("review", 6),
        ("look-000-to-157.5", 8),
        ("look-180-to-337.5", 8),
    ]
    .into_iter()
    .map(|(name, count)| (name.to_string(), count))
    .collect()
}

fn read_u24_le(bytes: &[u8]) -> Option<u32> {
    (bytes.len() >= 3)
        .then(|| u32::from(bytes[0]) | (u32::from(bytes[1]) << 8) | (u32::from(bytes[2]) << 16))
}

fn webp_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 20 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
        return None;
    }
    let mut offset = 12usize;
    while offset.checked_add(8)? <= bytes.len() {
        let kind = &bytes[offset..offset + 4];
        let size = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().ok()?) as usize;
        let data_start = offset + 8;
        let data_end = data_start.checked_add(size)?;
        if data_end > bytes.len() {
            return None;
        }
        let data = &bytes[data_start..data_end];
        match kind {
            b"VP8X" if data.len() >= 10 => {
                return Some((
                    read_u24_le(&data[4..7])? + 1,
                    read_u24_le(&data[7..10])? + 1,
                ));
            }
            b"VP8 " if data.len() >= 10 && data[3..6] == [0x9d, 0x01, 0x2a] => {
                let width = u16::from_le_bytes([data[6], data[7]]) & 0x3fff;
                let height = u16::from_le_bytes([data[8], data[9]]) & 0x3fff;
                return Some((u32::from(width), u32::from(height)));
            }
            b"VP8L" if data.len() >= 5 && data[0] == 0x2f => {
                let bits = u32::from_le_bytes([data[1], data[2], data[3], data[4]]);
                return Some(((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1));
            }
            _ => {}
        }
        offset = data_end + (size & 1);
    }
    None
}

fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    if bytes.len() < 24 || &bytes[..8] != SIGNATURE || &bytes[12..16] != b"IHDR" {
        return None;
    }
    Some((
        u32::from_be_bytes(bytes[16..20].try_into().ok()?),
        u32::from_be_bytes(bytes[20..24].try_into().ok()?),
    ))
}

fn image_format_and_dimensions(bytes: &[u8]) -> Option<(&'static str, (u32, u32))> {
    webp_dimensions(bytes)
        .map(|dimensions| ("image/webp", dimensions))
        .or_else(|| png_dimensions(bytes).map(|dimensions| ("image/png", dimensions)))
}

fn validation_frame_counts(directory: &Path) -> Result<BTreeMap<String, u8>, String> {
    let path = directory.join("validation.json");
    if !path.is_file() {
        return Ok(default_frame_counts());
    }
    let report: ValidationReport = serde_json::from_slice(
        &fs::read(&path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?,
    )
    .map_err(|e| format!("Invalid {}: {e}", path.display()))?;
    if !report.ok
        || report.columns != 8
        || report.rows != 11
        || report.width != SPRITE_WIDTH
        || report.height != SPRITE_HEIGHT
    {
        return Err("Pet validation.json does not describe a valid 8x11 v2 atlas.".into());
    }
    let mut counts = default_frame_counts();
    for state in counts.keys().cloned().collect::<Vec<_>>() {
        let used = report
            .cells
            .iter()
            .filter(|cell| cell.state == state && cell.used)
            .map(|cell| cell.column)
            .max()
            .map(|column| column + 1);
        if let Some(used) = used {
            counts.insert(state, used.clamp(1, 8));
        }
    }
    Ok(counts)
}

pub(super) fn load_pet_asset(directory: &Path) -> Result<PetAsset, String> {
    if !directory.is_absolute() {
        return Err("Pet directory must be an absolute path.".into());
    }
    let root = directory
        .canonicalize()
        .map_err(|e| format!("Cannot open pet directory {}: {e}", directory.display()))?;
    if !root.is_dir() {
        return Err(format!("Pet path is not a directory: {}", root.display()));
    }
    let manifest_path = root.join("pet.json");
    let manifest: PetManifest = serde_json::from_slice(
        &fs::read(&manifest_path)
            .map_err(|e| format!("Cannot read {}: {e}", manifest_path.display()))?,
    )
    .map_err(|e| format!("Invalid {}: {e}", manifest_path.display()))?;
    if manifest.id.trim().is_empty() || manifest.display_name.trim().is_empty() {
        return Err("Pet id and displayName are required.".into());
    }
    if manifest.sprite_version_number != 2 {
        return Err("Only spriteVersionNumber 2 pets are supported.".into());
    }
    let relative = Path::new(manifest.spritesheet_path.trim());
    if relative.as_os_str().is_empty() || relative.is_absolute() {
        return Err("Pet spritesheetPath must be a relative file path.".into());
    }
    let spritesheet_path = root
        .join(relative)
        .canonicalize()
        .map_err(|e| format!("Cannot open pet spritesheet: {e}"))?;
    if !spritesheet_path.starts_with(&root) || !spritesheet_path.is_file() {
        return Err("Pet spritesheetPath must point to a file inside the pet directory.".into());
    }
    let metadata = fs::metadata(&spritesheet_path).map_err(|e| e.to_string())?;
    if metadata.len() > MAX_SPRITESHEET_BYTES {
        return Err("Pet spritesheet exceeds the 16 MB limit.".into());
    }
    let spritesheet = fs::read(&spritesheet_path)
        .map_err(|e| format!("Cannot read {}: {e}", spritesheet_path.display()))?;
    let (mime, dimensions) = image_format_and_dimensions(&spritesheet)
        .ok_or_else(|| "Pet spritesheet must be a valid PNG or WebP image.".to_string())?;
    if dimensions != (SPRITE_WIDTH, SPRITE_HEIGHT) {
        return Err(format!(
            "Pet spritesheet must be {SPRITE_WIDTH}x{SPRITE_HEIGHT}; found {}x{}.",
            dimensions.0, dimensions.1
        ));
    }
    let frame_counts = validation_frame_counts(&root)?;
    Ok(PetAsset {
        id: manifest.id,
        display_name: manifest.display_name,
        description: manifest.description,
        sprite_version_number: manifest.sprite_version_number,
        spritesheet_data_url: format!("data:{mime};base64,{}", STANDARD.encode(spritesheet)),
        frame_counts,
    })
}

#[tauri::command]
pub(super) async fn get_pet(state: State<'_, AppState>) -> Result<PetStatus, String> {
    let enabled = state
        .store
        .get_setting("pet_enabled")
        .await
        .map_err(|e| e.to_string())?
        .map(|value| value == "true")
        .unwrap_or(false);
    let directory = state
        .store
        .get_setting("pet_directory")
        .await
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    if !enabled {
        return Ok(PetStatus {
            enabled,
            directory,
            asset: None,
            error: None,
        });
    }
    match load_pet_asset(Path::new(&directory)) {
        Ok(asset) => Ok(PetStatus {
            enabled,
            directory,
            asset: Some(asset),
            error: None,
        }),
        Err(error) => Ok(PetStatus {
            enabled,
            directory,
            asset: None,
            error: Some(error),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn temp_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("wisp-pet-{name}-{nonce}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn vp8x(width: u32, height: u32) -> Vec<u8> {
        let mut data = vec![0; 10];
        for (offset, value) in [(4, width - 1), (7, height - 1)] {
            data[offset] = value as u8;
            data[offset + 1] = (value >> 8) as u8;
            data[offset + 2] = (value >> 16) as u8;
        }
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&(22u32).to_le_bytes());
        bytes.extend_from_slice(b"WEBPVP8X");
        bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&data);
        bytes
    }

    fn write_pet(directory: &Path, version: u8, sprite_path: &str) {
        fs::write(
            directory.join("pet.json"),
            serde_json::json!({
                "id": "wispy",
                "displayName": "Wispy",
                "description": "test pet",
                "spriteVersionNumber": version,
                "spritesheetPath": sprite_path,
            })
            .to_string(),
        )
        .unwrap();
    }

    #[test]
    fn loads_a_valid_v2_pet_and_uses_standard_frame_counts() {
        let dir = temp_dir("valid");
        write_pet(&dir, 2, "spritesheet.webp");
        fs::write(
            dir.join("spritesheet.webp"),
            vp8x(SPRITE_WIDTH, SPRITE_HEIGHT),
        )
        .unwrap();

        let pet = load_pet_asset(&dir).unwrap();
        assert_eq!(pet.id, "wispy");
        assert_eq!(pet.frame_counts["idle"], 7);
        assert!(pet
            .spritesheet_data_url
            .starts_with("data:image/webp;base64,"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn uses_validated_frame_counts_when_the_package_provides_them() {
        let dir = temp_dir("validation");
        write_pet(&dir, 2, "spritesheet.webp");
        fs::write(
            dir.join("spritesheet.webp"),
            vp8x(SPRITE_WIDTH, SPRITE_HEIGHT),
        )
        .unwrap();
        fs::write(
            dir.join("validation.json"),
            serde_json::json!({
                "ok": true,
                "columns": 8,
                "rows": 11,
                "width": SPRITE_WIDTH,
                "height": SPRITE_HEIGHT,
                "cells": [
                    {"state": "idle", "column": 0, "used": true},
                    {"state": "idle", "column": 1, "used": true},
                    {"state": "idle", "column": 2, "used": true},
                    {"state": "idle", "column": 3, "used": false}
                ]
            })
            .to_string(),
        )
        .unwrap();

        let pet = load_pet_asset(&dir).unwrap();
        assert_eq!(pet.frame_counts["idle"], 3);
        assert_eq!(pet.frame_counts["running"], 6);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn rejects_wrong_version_and_wrong_dimensions() {
        let dir = temp_dir("invalid");
        write_pet(&dir, 1, "spritesheet.webp");
        fs::write(dir.join("spritesheet.webp"), vp8x(100, 100)).unwrap();
        assert!(load_pet_asset(&dir)
            .unwrap_err()
            .contains("spriteVersionNumber 2"));

        write_pet(&dir, 2, "spritesheet.webp");
        assert!(load_pet_asset(&dir).unwrap_err().contains("1536x2288"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn rejects_a_spritesheet_outside_the_pet_directory() {
        let dir = temp_dir("traversal");
        let outside = dir.parent().unwrap().join(format!(
            "outside-{}.webp",
            dir.file_name().unwrap().to_string_lossy()
        ));
        fs::write(&outside, vp8x(SPRITE_WIDTH, SPRITE_HEIGHT)).unwrap();
        write_pet(
            &dir,
            2,
            &format!("../{}", outside.file_name().unwrap().to_string_lossy()),
        );

        assert!(load_pet_asset(&dir)
            .unwrap_err()
            .contains("inside the pet directory"));
        let _ = fs::remove_file(outside);
        let _ = fs::remove_dir_all(dir);
    }
}
