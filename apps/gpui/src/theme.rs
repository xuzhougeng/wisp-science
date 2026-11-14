//! Wisp's palette for the component library, light and dark. The values are
//! the WebView's design tokens (`ui/src/styles/base.css`), so both frontends
//! read as the same product without sharing a renderer.
use std::rc::Rc;

use gpui::{App, Window};
use gpui_component::{Theme, ThemeSet};

const THEMES: &str = include_str!("../assets/wisp-theme.json");

fn bundled() -> ThemeSet {
    serde_json::from_str(THEMES).expect("the bundled theme is valid JSON")
}

/// Replaces the library's default themes and follows the system appearance.
pub fn install(cx: &mut App) {
    for config in bundled().themes {
        let theme = Theme::global_mut(cx);
        if config.mode.is_dark() {
            theme.dark_theme = Rc::new(config);
        } else {
            theme.light_theme = Rc::new(config);
        }
    }
    Theme::sync_system_appearance(None, cx);
}

/// Re-applies the palette when the system switches between light and dark.
pub fn follow_system(window: &mut Window, cx: &mut App) {
    Theme::sync_system_appearance(Some(window), cx);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_theme_has_one_light_and_one_dark_palette() {
        let themes = bundled().themes;
        assert_eq!(themes.len(), 2);
        assert_eq!(themes.iter().filter(|t| t.mode.is_dark()).count(), 1);
        for theme in &themes {
            // The accent and the page colours are what make it Wisp; a typo in
            // a key would silently fall back to the library's grey.
            for color in [
                &theme.colors.background,
                &theme.colors.foreground,
                &theme.colors.primary,
                &theme.colors.sidebar,
                &theme.colors.border,
            ] {
                let color = color.as_ref().expect("a core colour is missing");
                assert!(color.starts_with('#'), "{color}");
            }
        }
    }

    #[test]
    fn every_colour_key_is_one_the_library_reads() {
        // Unknown keys are ignored by serde; compare against a round trip so a
        // misspelt key fails here instead of being dropped.
        let raw: serde_json::Value = serde_json::from_str(THEMES).unwrap();
        for (index, theme) in bundled().themes.iter().enumerate() {
            let written = raw["themes"][index]["colors"].as_object().unwrap();
            let read = serde_json::to_value(&theme.colors).unwrap();
            for (key, value) in written {
                assert_eq!(read.get(key), Some(value), "unknown theme key {key}");
            }
        }
    }
}
