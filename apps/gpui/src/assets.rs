//! Embedded UI assets. The icons are Wisp's one shared set: the drawings in
//! `compose_icon` (`ui/src/app_support/messages.rs`), one file per kind under
//! `assets/icons`. A test keeps the files equal to that source while it exists.
use std::borrow::Cow;

use gpui::{AssetSource, SharedString, Styled, Svg, svg};

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
    "folder-move" "folder-plus" "fork" "gauge" "gear" "grid" "grip-vertical" "hand"
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
            assert_eq!(Assets.load(name).unwrap().as_deref(), Some(*bytes));
        }
        assert!(Assets.load("icons/missing.svg").unwrap().is_none());
        assert_eq!(Assets.list("icons/").unwrap().len(), ICONS.len());
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
