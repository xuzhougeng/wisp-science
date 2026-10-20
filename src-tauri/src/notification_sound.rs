//! Sound cue for task-status notifications (#1595).
//!
//! The banner follows the OS notification settings, so it disappears on its
//! own, stays hidden during screen sharing, and is suppressed by Focus / Do Not
//! Disturb. This cue is a separate in-app switch that plays a short system
//! sound through each platform's own player:
//!
//! - macOS: `afplay` with one of the system sounds in `/System/Library/Sounds`.
//! - Windows: `PlaySoundW` with a system event alias from the active sound
//!   scheme. The toast is silenced while the cue is on, so it never plays twice.
//! - Linux: `canberra-gtk-play` with a freedesktop sound-theme event id.
//!
//! Sound ids come from a fixed per-platform catalog. A stored value never names
//! an arbitrary file path or command argument.

use wisp_dto::NotificationSound;
use wisp_store::Store;

pub(crate) const SOUND_ENABLED_KEY: &str = "notification_sound_enabled";
pub(crate) const SOUND_KEY: &str = "notification_sound";

struct SoundCatalog {
    /// `(id, label)` pairs. Labels are English fallbacks; the UI localizes
    /// the ids it knows.
    sounds: &'static [(&'static str, &'static str)],
    default: &'static str,
}

/// The 14 alert sounds shipped in `/System/Library/Sounds/<id>.aiff`.
const MACOS: SoundCatalog = SoundCatalog {
    sounds: &[
        ("Basso", "Basso"),
        ("Blow", "Blow"),
        ("Bottle", "Bottle"),
        ("Frog", "Frog"),
        ("Funk", "Funk"),
        ("Glass", "Glass"),
        ("Hero", "Hero"),
        ("Morse", "Morse"),
        ("Ping", "Ping"),
        ("Pop", "Pop"),
        ("Purr", "Purr"),
        ("Sosumi", "Sosumi"),
        ("Submarine", "Submarine"),
        ("Tink", "Tink"),
    ],
    default: "Glass",
};

/// System event aliases under `HKCU\AppEvents\Schemes\Apps\.Default`. They
/// follow the user's sound scheme, including sounds the user has muted.
const WINDOWS: SoundCatalog = SoundCatalog {
    sounds: &[
        ("Notification.Default", "Notification"),
        ("Notification.IM", "Instant message"),
        ("Notification.Mail", "Mail"),
        ("Notification.Reminder", "Reminder"),
        ("Notification.SMS", "Text message"),
        ("SystemAsterisk", "Asterisk"),
        ("SystemExclamation", "Exclamation"),
        ("SystemNotification", "System notification"),
    ],
    default: "Notification.Default",
};

/// freedesktop sound-theme event ids, resolved by the desktop's sound theme.
const LINUX: SoundCatalog = SoundCatalog {
    sounds: &[
        ("complete", "Complete"),
        ("message", "Message"),
        ("message-new-instant", "Instant message"),
        ("bell", "Bell"),
        ("dialog-information", "Information"),
        ("dialog-warning", "Warning"),
    ],
    default: "complete",
};

const UNSUPPORTED: SoundCatalog = SoundCatalog {
    sounds: &[],
    default: "",
};

fn platform_catalog() -> &'static SoundCatalog {
    if cfg!(target_os = "macos") {
        &MACOS
    } else if cfg!(target_os = "windows") {
        &WINDOWS
    } else if cfg!(target_os = "linux") {
        &LINUX
    } else {
        &UNSUPPORTED
    }
}

impl SoundCatalog {
    fn find(&self, id: &str) -> Option<&'static str> {
        self.sounds
            .iter()
            .map(|(candidate, _)| *candidate)
            .find(|candidate| *candidate == id)
    }

    /// The sound to play for a stored choice. A blank value or an id this
    /// platform does not have (for example a macOS name in a database copied to
    /// Windows) falls back to the platform default. `None` means this platform
    /// has no sounds.
    fn resolve(&self, stored: &str) -> Option<&'static str> {
        self.find(stored.trim())
            .or_else(|| (!self.default.is_empty()).then_some(self.default))
    }

    fn normalize(&self, id: &str) -> Result<String, String> {
        let id = id.trim();
        if id.is_empty() {
            return Ok(String::new());
        }
        self.find(id).map(str::to_string).ok_or_else(|| {
            if self.sounds.is_empty() {
                "Notification sounds are not supported on this platform.".to_string()
            } else {
                format!(
                    "Unknown notification sound '{id}'. Choose one of: {}.",
                    self.ids().join(", ")
                )
            }
        })
    }

    fn ids(&self) -> Vec<&'static str> {
        self.sounds.iter().map(|(id, _)| *id).collect()
    }

    fn list(&self) -> Vec<NotificationSound> {
        self.sounds
            .iter()
            .map(|(id, label)| NotificationSound {
                id: (*id).to_string(),
                label: (*label).to_string(),
            })
            .collect()
    }
}

/// Validate a submitted sound id. Blank keeps the platform default.
pub(crate) fn normalize_choice(id: &str) -> Result<String, String> {
    platform_catalog().normalize(id)
}

/// The id Settings should show for a stored choice; empty when this platform
/// has no sounds.
pub(crate) fn effective_choice(stored: &str) -> String {
    platform_catalog()
        .resolve(stored)
        .unwrap_or_default()
        .to_string()
}

pub(crate) async fn load_enabled(store: &Store) -> bool {
    store
        .get_setting(SOUND_ENABLED_KEY)
        .await
        .ok()
        .flatten()
        .is_some_and(|value| value == "true")
}

pub(crate) async fn load_choice(store: &Store) -> String {
    store
        .get_setting(SOUND_KEY)
        .await
        .ok()
        .flatten()
        .unwrap_or_default()
}

/// The sound a task notification should play, or `None` when the cue is off
/// or this platform has no sounds.
pub(crate) async fn load_cue(store: &Store) -> Option<&'static str> {
    if !load_enabled(store).await {
        return None;
    }
    platform_catalog().resolve(&load_choice(store).await)
}

pub(crate) async fn save(store: &Store, enabled: bool, choice: &str) -> Result<(), String> {
    let choice = normalize_choice(choice)?;
    store
        .set_setting(SOUND_ENABLED_KEY, if enabled { "true" } else { "false" })
        .await
        .map_err(|error| error.to_string())?;
    store
        .set_setting(SOUND_KEY, &choice)
        .await
        .map_err(|error| error.to_string())
}

/// How one task-status event is announced once its owning window is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Announcement {
    pub banner: bool,
    pub sound: Option<&'static str>,
}

impl Announcement {
    /// `None` when both the banner and the sound are off, so the caller can
    /// skip choosing an owning window.
    pub(crate) fn new(banner: bool, sound: Option<&'static str>) -> Option<Self> {
        (banner || sound.is_some()).then_some(Self { banner, sound })
    }

    /// The app's own cue replaces the banner's system sound, so the user
    /// never hears two sounds for one event.
    pub(crate) fn silent_banner(&self) -> bool {
        self.sound.is_some()
    }
}

/// Start playing `id` without waiting for it to finish.
pub(crate) fn play(id: &'static str) -> Result<(), String> {
    play_platform_sound(id)
}

#[cfg(target_os = "macos")]
fn play_platform_sound(id: &'static str) -> Result<(), String> {
    let path = format!("/System/Library/Sounds/{id}.aiff");
    if !std::path::Path::new(&path).is_file() {
        return Err(format!("System sound {path} is missing."));
    }
    let mut child = std::process::Command::new("/usr/bin/afplay")
        .arg(&path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|error| format!("Could not start afplay: {error}"))?;
    // Reap the player off the async runtime so it never lingers as a zombie.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(target_os = "windows")]
fn play_platform_sound(id: &'static str) -> Result<(), String> {
    use windows::core::HSTRING;
    use windows::Win32::Media::Audio::{
        PlaySoundW, SND_ALIAS, SND_ASYNC, SND_NODEFAULT, SND_SYSTEM,
    };
    let alias = HSTRING::from(id);
    // SND_ASYNC returns at once; SND_SYSTEM follows the "System sounds" volume
    // slider; SND_NODEFAULT stays silent instead of beeping for a missing alias.
    let played = unsafe {
        PlaySoundW(
            &alias,
            None,
            SND_ALIAS | SND_ASYNC | SND_NODEFAULT | SND_SYSTEM,
        )
    };
    if played.as_bool() {
        Ok(())
    } else {
        Err(format!("Windows could not play the {id} sound."))
    }
}

#[cfg(target_os = "linux")]
fn play_platform_sound(id: &'static str) -> Result<(), String> {
    let player = which::which("canberra-gtk-play").map_err(|_| {
        "Notification sounds need canberra-gtk-play (libcanberra) on Linux.".to_string()
    })?;
    let mut child = std::process::Command::new(player)
        .arg(format!("--id={id}"))
        .arg("--description=Wisp Science")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|error| format!("Could not start canberra-gtk-play: {error}"))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn play_platform_sound(_id: &'static str) -> Result<(), String> {
    Err("Notification sounds are not supported on this platform.".into())
}

/// Sounds the Settings picker offers on this platform. Empty hides the picker.
#[tauri::command]
pub(crate) fn list_notification_sounds() -> Vec<NotificationSound> {
    platform_catalog().list()
}

/// Play a sound from Settings so the user can hear it before saving. Unlike
/// task notifications, this ignores window focus and the saved switch.
#[tauri::command]
pub(crate) async fn preview_notification_sound(sound: String) -> Result<(), String> {
    let catalog = platform_catalog();
    let choice = catalog.normalize(&sound)?;
    let id = catalog
        .resolve(&choice)
        .ok_or_else(|| "Notification sounds are not supported on this platform.".to_string())?;
    play(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [&SoundCatalog; 3] = [&MACOS, &WINDOWS, &LINUX];

    #[test]
    fn every_catalog_has_unique_ids_and_contains_its_default() {
        for catalog in ALL {
            let ids = catalog.ids();
            let mut unique = ids.clone();
            unique.sort_unstable();
            unique.dedup();
            assert_eq!(unique.len(), ids.len(), "duplicate id in {ids:?}");
            assert_eq!(catalog.find(catalog.default), Some(catalog.default));
        }
    }

    #[test]
    fn macos_lists_the_fourteen_system_alert_sounds_as_safe_file_stems() {
        assert_eq!(MACOS.sounds.len(), 14);
        for id in MACOS.ids() {
            assert!(
                id.chars().all(|ch| ch.is_ascii_alphanumeric()),
                "{id} must stay a plain file stem"
            );
        }
    }

    #[test]
    fn sound_ids_never_carry_paths_or_command_arguments() {
        for catalog in ALL {
            for id in catalog.ids() {
                assert!(!id.starts_with('-'), "{id}");
                assert!(!id.contains(['/', '\\', ' ', '=']), "{id}");
            }
        }
    }

    #[test]
    fn resolve_falls_back_to_the_platform_default() {
        assert_eq!(MACOS.resolve(""), Some("Glass"));
        assert_eq!(MACOS.resolve(" Ping "), Some("Ping"));
        // A Windows alias stored on macOS (copied database) plays the default.
        assert_eq!(MACOS.resolve("Notification.IM"), Some("Glass"));
        assert_eq!(WINDOWS.resolve("Glass"), Some("Notification.Default"));
        assert_eq!(LINUX.resolve("message"), Some("message"));
        assert_eq!(UNSUPPORTED.resolve(""), None);
        assert_eq!(UNSUPPORTED.resolve("Glass"), None);
    }

    #[test]
    fn normalize_accepts_blank_and_known_ids_and_rejects_the_rest() {
        assert_eq!(MACOS.normalize("  ").unwrap(), "");
        assert_eq!(MACOS.normalize(" Hero ").unwrap(), "Hero");
        // Ids are exact; they double as file stems on macOS.
        let error = MACOS.normalize("hero").unwrap_err();
        assert!(
            error.contains("Unknown notification sound 'hero'"),
            "{error}"
        );
        assert!(error.contains("Glass"), "{error}");
        let error = MACOS.normalize("../../tmp/evil").unwrap_err();
        assert!(error.contains("Unknown notification sound"), "{error}");
        assert!(UNSUPPORTED
            .normalize("Glass")
            .unwrap_err()
            .contains("not supported"));
        assert_eq!(UNSUPPORTED.normalize("").unwrap(), "");
    }

    #[test]
    fn announcement_keeps_banner_and_sound_independent() {
        assert_eq!(Announcement::new(false, None), None);
        let banner_only = Announcement::new(true, None).unwrap();
        assert!(banner_only.banner);
        assert!(!banner_only.silent_banner());
        let sound_only = Announcement::new(false, Some("Glass")).unwrap();
        assert!(!sound_only.banner);
        assert_eq!(sound_only.sound, Some("Glass"));
        let both = Announcement::new(true, Some("Glass")).unwrap();
        assert!(both.banner);
        assert!(
            both.silent_banner(),
            "the cue replaces the banner's own sound"
        );
    }

    #[tokio::test]
    async fn the_cue_is_off_by_default_and_follows_saved_settings() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("wisp.sqlite")).await.unwrap();
        assert!(!load_enabled(&store).await);
        assert_eq!(load_cue(&store).await, None);

        let catalog = platform_catalog();
        let Some(&(choice, _)) = catalog.sounds.last() else {
            assert!(save(&store, true, "Glass").await.is_err());
            return;
        };
        save(&store, true, choice).await.unwrap();
        assert!(load_enabled(&store).await);
        assert_eq!(load_choice(&store).await, choice);
        assert_eq!(load_cue(&store).await, Some(choice));

        save(&store, true, "").await.unwrap();
        assert_eq!(load_cue(&store).await, Some(catalog.default));

        save(&store, false, choice).await.unwrap();
        assert_eq!(load_cue(&store).await, None);

        assert!(save(&store, true, "not-a-sound").await.is_err());
        assert_eq!(
            load_choice(&store).await,
            choice,
            "a rejected id is not stored"
        );
    }
}
