//! Embedded UI assets. The icons are Wisp's one shared set: the drawings in
//! `compose_icon` (`ui/src/app_support/messages.rs`), one file per kind under
//! `assets/icons`. A test keeps the files equal to that source while it exists.
//!
//! The two faces the design language bundles are here too, each with its
//! SIL Open Font License beside it in `assets/fonts`:
//! - Source Serif 4, for titles: static Regular and Medium instances (opsz
//!   20) of the variable font in google/fonts (`ofl/sourceserif4`), which
//!   carries no Reserved Font Name. The upstream static set has no Medium.
//! - IBM Plex Mono, for facts: Regular and Medium as IBM ships them
//!   (`IBM/plex`, `packages/plex-mono/fonts/complete/ttf`).
use std::borrow::Cow;

use gpui::{App, AssetSource, SharedString, Styled, Svg, svg};

/// The family name of the bundled serif.
pub const SERIF_FACE: &str = "Source Serif 4";

/// The family name of the bundled monospace face of facts.
pub const MONO_FACE: &str = "IBM Plex Mono";

macro_rules! fonts {
    ($($name:literal)*) => {
        const FONTS: &[(&str, &[u8])] = &[$((
            $name,
            include_bytes!(concat!("../assets/fonts/", $name)) as &[u8],
        )),*];
    };
}

fonts! {
    "SourceSerif4-Regular.ttf" "SourceSerif4-Medium.ttf"
    "IBMPlexMono-Regular.ttf" "IBMPlexMono-Medium.ttf"
}

/// Makes the bundled faces known to the window's text system, so a title
/// or a fact reads the same on every system. A face that cannot be read is
/// said on stderr and the system's own face stands in.
pub fn register_fonts(cx: &App) {
    let faces = FONTS
        .iter()
        .map(|(_, bytes)| Cow::Borrowed(*bytes))
        .collect();
    if let Err(error) = cx.text_system().add_fonts(faces) {
        eprintln!("Cannot load the bundled fonts: {error:#}");
    }
}

macro_rules! icons {
    ($($name:literal)*) => {
        const ICONS: &[(&str, &[u8])] = &[$((
            concat!("icons/", $name, ".svg"),
            include_bytes!(concat!("../assets/icons/", $name, ".svg")) as &[u8],
        )),*];
    };
}

#[rustfmt::skip]
icons! {
    "activity-orbit" "adjustments" "archive" "archive-export" "archive-import" "arrow-left"
    "arrow-right" "arrow-up" "attach" "ban" "bell" "bolt" "book" "bot" "brain" "branch"
    "bubble" "calendar" "camera" "chat" "check" "check-square" "chevron-down" "chevron-left"
    "chevron-right" "chevron-up" "circle" "circle-alert" "circle-check" "circle-ellipsis"
    "circle-help" "circle-minus" "clipboard" "clock" "close" "code" "code-decrease"
    "code-increase" "compact" "compass" "computer" "context-compact" "controls"
    "conversation-import" "conversation-move" "copy" "crop" "database" "dna" "doc" "dock"
    "download" "edit" "expand" "external-link" "eye" "eye-off" "file-play" "flask" "folder"
    "folder-move" "folder-plus" "fork" "gauge" "gear" "grid" "grip-vertical" "hand" "highlighter"
    "history" "image" "inbox" "layers" "lightbulb" "link" "list" "loader" "lock" "memory"
    "minus" "monitor" "moon" "more" "note-plus" "notebook" "panel" "panel-left" "pause"
    "pin" "plan" "play" "plus" "refresh" "research-trail" "resize" "review" "runtime-panel"
    "save" "search" "server" "share" "shield" "skill" "sort" "sparkles" "split" "square"
    "square-pen" "star" "star-filled" "stop" "sun" "swarm" "sync" "table" "terminal"
    "timeline" "trash" "type-decrease" "type-increase" "undo" "undo-compact" "up" "upload"
    "user" "video" "wrench"
}

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        Ok(ICONS
            .iter()
            .find(|(name, _)| *name == path)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(ICONS
            .iter()
            .filter(|(name, _)| name.starts_with(path))
            .map(|(name, _)| SharedString::from(*name))
            .collect())
    }
}

pub fn icon_path(name: &str) -> SharedString {
    debug_assert!(
        ICONS.iter().any(|(path, _)| path
            .strip_prefix("icons/")
            .and_then(|p| p.strip_suffix(".svg"))
            == Some(name)),
        "unknown icon {name}"
    );
    format!("icons/{name}.svg").into()
}

/// One icon from the shared set at text-row size. It takes the colour given
/// with `text_color`, so it follows the theme like the text beside it.
pub fn icon(name: &str) -> Svg {
    svg().path(icon_path(name)).size_4().flex_none()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn drawing(bytes: &[u8]) -> String {
        let text = std::str::from_utf8(bytes).unwrap().trim();
        let start = text.find('>').unwrap() + 1;
        text[start..text.rfind("</svg>").unwrap()].to_owned()
    }

    #[test]
    fn every_icon_file_is_embedded_and_well_formed() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/icons");
        let mut files: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| format!("icons/{}", entry.unwrap().file_name().to_string_lossy()))
            .collect();
        files.sort();
        let mut embedded: Vec<_> = ICONS.iter().map(|(name, _)| name.to_string()).collect();
        embedded.sort();
        assert_eq!(files, embedded);
        for (name, bytes) in ICONS {
            let text = std::str::from_utf8(bytes).unwrap();
            assert!(
                text.starts_with("<svg ") && text.contains("viewBox=\"0 0 24 24\""),
                "{name}"
            );
            assert!(!drawing(bytes).is_empty(), "{name}");
            // One stroke for the whole set: 1.5 on the 24 grid, in the
            // colour of the text beside it, with no width of its own inside.
            let open = &text[..text.find('>').unwrap()];
            assert!(open.contains("stroke=\"currentColor\""), "{name}");
            assert!(open.contains("stroke-width=\"1.5\""), "{name}");
            assert_eq!(text.matches("stroke-width").count(), 1, "{name}");
            assert_eq!(Assets.load(name).unwrap().as_deref(), Some(*bytes));
        }
        assert!(Assets.load("icons/missing.svg").unwrap().is_none());
        assert_eq!(Assets.list("icons/").unwrap().len(), ICONS.len());
    }

    /// The table of a font file named `tag`: where it starts and how long
    /// it is.
    fn table<'a>(font: &'a [u8], tag: &[u8; 4]) -> &'a [u8] {
        let be16 = |at: usize| u16::from_be_bytes([font[at], font[at + 1]]) as usize;
        let be32 = |at: usize| {
            u32::from_be_bytes([font[at], font[at + 1], font[at + 2], font[at + 3]]) as usize
        };
        for index in 0..be16(4) {
            let record = 12 + index * 16;
            if &font[record..record + 4] == tag {
                let (start, length) = (be32(record + 8), be32(record + 12));
                return &font[start..start + length];
            }
        }
        panic!("no {} table", String::from_utf8_lossy(tag));
    }

    /// The Windows-platform English name `id` of a font file.
    fn font_name(font: &[u8], id: u16) -> Option<String> {
        let names = table(font, b"name");
        let be16 = |at: usize| u16::from_be_bytes([names[at], names[at + 1]]);
        let strings = be16(4) as usize;
        (0..be16(2) as usize).find_map(|index| {
            let record = 6 + index * 12;
            let field = |at: usize| be16(record + at);
            (field(0) == 3 && field(6) == id && field(4) == 0x409).then(|| {
                let start = strings + field(10) as usize;
                let units: Vec<u16> = names[start..start + field(8) as usize]
                    .chunks(2)
                    .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                    .collect();
                String::from_utf16(&units).unwrap()
            })
        })
    }

    #[test]
    fn the_bundled_faces_are_the_families_and_weights_the_views_ask_for() {
        // Every file in the folder is embedded, and each family keeps its
        // licence beside it.
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/fonts");
        let mut files: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".ttf"))
            .collect();
        files.sort();
        let mut embedded: Vec<_> = FONTS.iter().map(|(name, _)| name.to_string()).collect();
        embedded.sort();
        assert_eq!(files, embedded);
        for licence in ["SourceSerif4-OFL.txt", "IBMPlexMono-OFL.txt"] {
            let text = std::fs::read_to_string(dir.join(licence)).unwrap();
            assert!(text.contains("SIL Open Font License"), "{licence}");
        }
        // A family name the text system does not find falls back to the
        // system's face without a word, so the names are checked here: the
        // typographic family (16) when there is one, else the family (1),
        // and the weight class in OS/2.
        for (name, bytes) in FONTS {
            let family = font_name(bytes, 16).or_else(|| font_name(bytes, 1));
            let expected = if name.starts_with("SourceSerif4") {
                SERIF_FACE
            } else {
                MONO_FACE
            };
            assert_eq!(family.as_deref(), Some(expected), "{name}");
            let os2 = table(bytes, b"OS/2");
            let weight = u16::from_be_bytes([os2[4], os2[5]]);
            let expected = if name.ends_with("-Medium.ttf") {
                500
            } else {
                400
            };
            assert_eq!(weight, expected, "{name}");
        }
    }

    /// "More actions" is three horizontal dots, filled, a little heavier
    /// than a hairline so they weigh as much as the stroke icons beside
    /// them; the close cross is inset so it is optically their size.
    #[test]
    fn more_lies_flat_and_close_is_inset() {
        let file = |name: &str| {
            let (_, bytes) = ICONS
                .iter()
                .find(|(path, _)| *path == format!("icons/{name}.svg"))
                .expect(name);
            std::str::from_utf8(bytes).unwrap().to_owned()
        };
        let more = file("more");
        let circles: Vec<&str> = more.split("<circle").skip(1).collect();
        assert_eq!(circles.len(), 3);
        for (circle, cx) in circles.iter().zip(["5", "12", "19"]) {
            assert!(circle.contains(&format!("cx=\"{cx}\"")), "{circle}");
            assert!(circle.contains("cy=\"12\""), "the dots share one line");
            assert!(circle.contains("r=\"1.25\""));
            assert!(circle.contains("fill=\"currentColor\" stroke=\"none\""));
        }
        let close = file("close");
        assert!(close.contains("M17 7 7 17") && close.contains("m7 7 10 10"));
        assert!(!close.contains("M18 6"));
    }

    /// While the WebView sources exist, both frontends draw the same icons.
    #[test]
    fn icons_match_the_shared_webview_set() {
        let source =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui/src/app_support/messages.rs");
        let Ok(source) = std::fs::read_to_string(source) else {
            return;
        };
        let squeezed: String = source.split_whitespace().collect::<Vec<_>>().join(" ");
        let squeezed = squeezed.replace("> <", "><");
        // The WebView draws them with the same stroke.
        assert!(squeezed.contains("stroke-width=\"1.5\" stroke-linecap=\"round\""));
        assert!(!squeezed.contains(
            "stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\">{body}"
        ));
        for (path, bytes) in ICONS {
            let name = path.trim_start_matches("icons/").trim_end_matches(".svg");
            assert!(
                squeezed.contains(&format!("\"{name}\"")),
                "{name} is not a shared icon"
            );
            let arm = format!("view! {{ {} }}.into_view()", drawing(bytes));
            assert!(
                squeezed.contains(&arm),
                "{name} differs from the shared set"
            );
        }
    }
}
