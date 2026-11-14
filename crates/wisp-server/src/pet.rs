//! The pet: an animated companion drawn from a folder the user chooses,
//! holding a `pet.json` and a sprite sheet (the Codex-compatible v2
//! layout: 8 columns by 11 rows on a 1536x2288 image).
//!
//! The folder checks follow `src-tauri/src/pet_commands.rs`, which cannot
//! be shared from here because it lives in the Tauri shell. The choice is
//! kept under the WebView's keys, so both desktops show the same pet.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, bail, ensure, Context, Result};
use serde::Deserialize;
use wisp_store::Store;

const ENABLED: &str = "pet_enabled";
const DIRECTORY: &str = "pet_directory";
const MAX_SHEET_BYTES: u64 = 16 * 1024 * 1024;
pub const SHEET_WIDTH: u32 = 1536;
pub const SHEET_HEIGHT: u32 = 2288;

/// What Settings shows and the window draws.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pet {
    pub enabled: bool,
    /// The folder as it was typed; empty when none was chosen.
    pub directory: String,
    /// The pet to draw; `None` when it is off or its folder is unusable.
    pub look: Option<PetLook>,
    /// Why an enabled pet cannot be drawn.
    pub problem: Option<String>,
}

/// A pet that passed the folder checks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PetLook {
    pub name: String,
    pub description: String,
    /// The sprite sheet, inside the folder.
    pub sheet: PathBuf,
    /// How many frames each row's animation has, by state name.
    pub frames: BTreeMap<String, u8>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    id: String,
    display_name: String,
    #[serde(default)]
    description: String,
    sprite_version_number: u8,
    spritesheet_path: String,
}

#[derive(Deserialize)]
struct Validation {
    ok: bool,
    columns: u32,
    rows: u32,
    width: u32,
    height: u32,
    #[serde(default)]
    cells: Vec<Cell>,
}

#[derive(Deserialize)]
struct Cell {
    state: String,
    column: u8,
    used: bool,
}

fn default_frames() -> BTreeMap<String, u8> {
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
    ]
    .into_iter()
    .map(|(name, count)| (name.to_owned(), count))
    .collect()
}

fn u24(bytes: &[u8]) -> u32 {
    u32::from(bytes[0]) | (u32::from(bytes[1]) << 8) | (u32::from(bytes[2]) << 16)
}

fn webp_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 20 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
        return None;
    }
    let mut offset = 12usize;
    while offset.checked_add(8)? <= bytes.len() {
        let kind = &bytes[offset..offset + 4];
        let size = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().ok()?) as usize;
        let start = offset + 8;
        let end = start.checked_add(size)?;
        let data = bytes.get(start..end)?;
        match kind {
            b"VP8X" if data.len() >= 10 => {
                return Some((u24(&data[4..7]) + 1, u24(&data[7..10]) + 1));
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
        offset = end + (size & 1);
    }
    None
}

fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        return None;
    }
    Some((
        u32::from_be_bytes(bytes[16..20].try_into().ok()?),
        u32::from_be_bytes(bytes[20..24].try_into().ok()?),
    ))
}

/// The frame counts a folder's optional `validation.json` gives, over the
/// defaults.
fn frames(root: &Path) -> Result<BTreeMap<String, u8>> {
    let path = root.join("validation.json");
    let mut counts = default_frames();
    if !path.is_file() {
        return Ok(counts);
    }
    let report: Validation = serde_json::from_slice(&fs::read(&path)?)
        .with_context(|| format!("Invalid {}", path.display()))?;
    ensure!(
        report.ok
            && report.columns == 8
            && report.rows == 11
            && report.width == SHEET_WIDTH
            && report.height == SHEET_HEIGHT,
        "Pet validation.json does not describe a valid 8x11 v2 atlas."
    );
    for (state, count) in counts.iter_mut() {
        let used = report
            .cells
            .iter()
            .filter(|cell| &cell.state == state && cell.used)
            .map(|cell| cell.column)
            .max();
        if let Some(last) = used {
            *count = last.saturating_add(1).clamp(1, 8);
        }
    }
    Ok(counts)
}

/// Checks a pet folder and says what to draw from it. Only headers are
/// read from the sheet; the window decodes it.
pub(crate) fn load(directory: &Path) -> Result<PetLook> {
    ensure!(
        directory.is_absolute(),
        "Pet directory must be an absolute path."
    );
    let root = directory
        .canonicalize()
        .map_err(|e| anyhow!("Cannot open pet directory {}: {e}", directory.display()))?;
    ensure!(
        root.is_dir(),
        "Pet path is not a directory: {}",
        root.display()
    );
    let manifest_path = root.join("pet.json");
    let manifest: Manifest = serde_json::from_slice(
        &fs::read(&manifest_path)
            .map_err(|e| anyhow!("Cannot read {}: {e}", manifest_path.display()))?,
    )
    .map_err(|e| anyhow!("Invalid {}: {e}", manifest_path.display()))?;
    ensure!(
        !manifest.id.trim().is_empty() && !manifest.display_name.trim().is_empty(),
        "Pet id and displayName are required."
    );
    ensure!(
        manifest.sprite_version_number == 2,
        "Only spriteVersionNumber 2 pets are supported."
    );
    let relative = Path::new(manifest.spritesheet_path.trim());
    ensure!(
        !relative.as_os_str().is_empty() && !relative.is_absolute(),
        "Pet spritesheetPath must be a relative file path."
    );
    let sheet = root
        .join(relative)
        .canonicalize()
        .map_err(|e| anyhow!("Cannot open pet spritesheet: {e}"))?;
    ensure!(
        sheet.starts_with(&root) && sheet.is_file(),
        "Pet spritesheetPath must point to a file inside the pet directory."
    );
    ensure!(
        fs::metadata(&sheet)?.len() <= MAX_SHEET_BYTES,
        "Pet spritesheet exceeds the 16 MB limit."
    );
    let bytes = fs::read(&sheet).map_err(|e| anyhow!("Cannot read {}: {e}", sheet.display()))?;
    let Some(size) = webp_size(&bytes).or_else(|| png_size(&bytes)) else {
        bail!("Pet spritesheet must be a valid PNG or WebP image.");
    };
    ensure!(
        size == (SHEET_WIDTH, SHEET_HEIGHT),
        "Pet spritesheet must be {SHEET_WIDTH}x{SHEET_HEIGHT}; found {}x{}.",
        size.0,
        size.1
    );
    Ok(PetLook {
        name: manifest.display_name.trim().to_owned(),
        description: manifest.description.trim().to_owned(),
        sheet,
        frames: frames(&root)?,
    })
}

pub(crate) async fn read(store: &Store) -> Pet {
    let enabled = store.get_setting(ENABLED).await.ok().flatten().as_deref() == Some("true");
    let directory = store
        .get_setting(DIRECTORY)
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    let (look, problem) = match enabled.then(|| load(Path::new(&directory))) {
        Some(Ok(look)) => (Some(look), None),
        Some(Err(problem)) => (None, Some(problem.to_string())),
        None => (None, None),
    };
    Pet {
        enabled,
        directory,
        look,
        problem,
    }
}

/// Keeps the choice. A pet is only turned on with a folder that passes
/// the checks; a refused choice changes nothing.
pub(crate) async fn save(store: &Store, enabled: bool, directory: &str) -> Result<Pet> {
    let directory = directory.trim();
    ensure!(
        directory.is_empty() || Path::new(directory).is_absolute(),
        "Pet directory must be an absolute path."
    );
    if enabled {
        ensure!(
            !directory.is_empty(),
            "Choose a pet directory before enabling the pet."
        );
        load(Path::new(directory))?;
    }
    store
        .set_setting(ENABLED, if enabled { "true" } else { "false" })
        .await?;
    store.set_setting(DIRECTORY, directory).await?;
    Ok(read(store).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file that starts like a PNG of the given size; the checks read
    /// no further.
    fn png_header(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        bytes.extend(width.to_be_bytes());
        bytes.extend(height.to_be_bytes());
        bytes
    }

    fn write_pet(folder: &Path, sheet: &str, size: (u32, u32)) {
        fs::write(
            folder.join("pet.json"),
            format!(
                r#"{{"id":"mochi","displayName":" Mochi ","spriteVersionNumber":2,"spritesheetPath":"{sheet}"}}"#
            ),
        )
        .unwrap();
        if !sheet.starts_with("..") {
            fs::write(folder.join(sheet), png_header(size.0, size.1)).unwrap();
        }
    }

    #[test]
    fn a_folder_is_checked_before_anything_is_drawn_from_it() {
        let folder = tempfile::tempdir().unwrap();
        assert!(load(Path::new("relative/pet")).is_err());
        assert!(load(folder.path()).is_err(), "no pet.json");

        write_pet(folder.path(), "sheet.png", (SHEET_WIDTH, SHEET_HEIGHT));
        let look = load(folder.path()).unwrap();
        assert_eq!(look.name, "Mochi");
        assert!(look.sheet.ends_with("sheet.png"));
        assert_eq!(look.frames["idle"], 7);
        assert_eq!(look.frames["waving"], 4);

        // A validation report says how many frames a row really has.
        fs::write(
            folder.path().join("validation.json"),
            r#"{"ok":true,"columns":8,"rows":11,"width":1536,"height":2288,
                "cells":[{"state":"idle","column":0,"used":true},
                         {"state":"idle","column":2,"used":true},
                         {"state":"idle","column":5,"used":false}]}"#,
        )
        .unwrap();
        assert_eq!(load(folder.path()).unwrap().frames["idle"], 3);
        fs::write(
            folder.path().join("validation.json"),
            r#"{"ok":false,"columns":8,"rows":11,"width":1536,"height":2288}"#,
        )
        .unwrap();
        assert!(load(folder.path()).is_err());
        fs::remove_file(folder.path().join("validation.json")).unwrap();

        // The wrong size, something that is no image, and a sheet outside
        // the folder are all refused.
        write_pet(folder.path(), "sheet.png", (512, 512));
        assert!(load(folder.path())
            .unwrap_err()
            .to_string()
            .contains("512x512"));
        fs::write(folder.path().join("sheet.png"), b"not an image").unwrap();
        assert!(load(folder.path()).is_err());
        let outside = folder.path().parent().unwrap().join("wisp-pet-outside.png");
        fs::write(&outside, png_header(SHEET_WIDTH, SHEET_HEIGHT)).unwrap();
        write_pet(folder.path(), "../wisp-pet-outside.png", (0, 0));
        let refused = load(folder.path()).unwrap_err().to_string();
        fs::remove_file(&outside).unwrap();
        assert!(refused.contains("inside the pet directory"), "{refused}");
    }

    #[tokio::test]
    async fn a_pet_is_only_turned_on_with_a_folder_that_holds_one() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(&temp.path().join("wisp.sqlite")).await.unwrap();
        assert_eq!(read(&store).await, Pet::default());

        let folder = temp.path().join("mochi");
        fs::create_dir(&folder).unwrap();
        let typed = folder.to_string_lossy().into_owned();
        // Off, a folder is kept without being read.
        let off = save(&store, false, &format!(" {typed} ")).await.unwrap();
        assert_eq!(
            (off.enabled, off.directory.as_str()),
            (false, typed.as_str())
        );
        assert!(off.look.is_none() && off.problem.is_none());
        // On, it has to hold a pet; a refusal changes nothing.
        assert!(save(&store, true, &typed).await.is_err());
        assert!(save(&store, true, "").await.is_err());
        assert!(save(&store, false, "relative").await.is_err());
        assert_eq!(read(&store).await, off);

        write_pet(&folder, "sheet.png", (SHEET_WIDTH, SHEET_HEIGHT));
        let on = save(&store, true, &typed).await.unwrap();
        assert_eq!(
            on.look.as_ref().map(|look| look.name.as_str()),
            Some("Mochi")
        );
        // A folder that goes bad later is reported, not drawn.
        fs::remove_file(folder.join("pet.json")).unwrap();
        let broken = read(&store).await;
        assert!(broken.enabled && broken.look.is_none() && broken.problem.is_some());
    }

    #[test]
    fn webp_sizes_are_read_from_each_kind_of_header() {
        let mut lossless = b"RIFF\0\0\0\0WEBPVP8L".to_vec();
        lossless.extend(5u32.to_le_bytes());
        let bits = (SHEET_WIDTH - 1) | ((SHEET_HEIGHT - 1) << 14);
        lossless.push(0x2f);
        lossless.extend(bits.to_le_bytes());
        lossless.push(0);
        assert_eq!(webp_size(&lossless), Some((SHEET_WIDTH, SHEET_HEIGHT)));
        assert_eq!(webp_size(b"RIFF\0\0\0\0WEBPVP8L\xff\xff\xff\xff...."), None);
        assert_eq!(webp_size(b"GIF89a"), None);
    }
}
